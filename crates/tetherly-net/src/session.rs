// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::codec::{read_len_prefixed, write_len_prefixed};
use crate::error::NetError;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tetherly_core::dedup::TcpLimiter;
use tetherly_core::frame::{decode_unix_ms, ping, pong, InnerFrame, TYPE_PING, TYPE_PONG};
use tetherly_core::hello::Hello;
use tetherly_core::ports::{Clock, MemoryTrustStore, SystemClock, TrustStore};
use tetherly_core::replay::ReplayGuard;
use tetherly_core::session::{
    commit_trust, dispose_hello, HelloDisposition, LockoutTable, PIN_TTL,
};
use tetherly_core::{CoreError, DeviceId};
use tetherly_crypto::identity::Identity;
use tetherly_crypto::noise::{prologue, NoiseHandshake, NoiseTransport};
use tetherly_crypto::pairbind::{decrypt_pairbind, encrypt_pairbind, PairBind, PairBindRole};
use tetherly_crypto::spake::SpakeRole;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::TcpStream;
use tracing::{debug, warn};
use zeroize::Zeroize;

pub const CONTROL_PORT: u16 = 45717;
pub const FILE_PORT: u16 = 45718;
pub const INPUT_PORT: u16 = 45719;

/// Handshake I/O budget. Argon2id itself is ~hundreds of ms; 20s covers a
/// slow LAN plus KDF without hanging forever on a half-open TCP.
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(20);

pub struct ActiveSession {
    pub peer_id: DeviceId,
    pub peer_hello: Hello,
    pub peer_addr: SocketAddr,
    reader: OwnedReadHalf,
    writer: OwnedWriteHalf,
    transport: NoiseTransport,
    next_msg_id: u64,
    replay: ReplayGuard,
}

impl ActiveSession {
    pub async fn send_inner(&mut self, mut frame: InnerFrame) -> Result<(), NetError> {
        if self.transport.needs_rekey(unix_now()) {
            return Err(NetError::RekeyRequired);
        }
        frame.msg_id = self.next_msg_id();
        let plain = frame.encode()?;
        let ct = self.transport.encrypt(&plain)?;
        write_len_prefixed(&mut self.writer, &ct).await
    }

    pub async fn recv_inner(&mut self) -> Result<InnerFrame, NetError> {
        if self.transport.needs_rekey(unix_now()) {
            return Err(NetError::RekeyRequired);
        }
        let ct = read_len_prefixed(&mut self.reader).await?;
        let plain = self.transport.decrypt(&ct)?;
        let frame = InnerFrame::decode(&plain)?;
        if !self.replay.check_and_record(frame.msg_id) {
            return Err(NetError::Replay);
        }
        Ok(frame)
    }

    pub fn handshake_hash(&self) -> &[u8; 32] {
        self.transport.handshake_hash()
    }

    pub fn file_token(&self, transfer_id: &str) -> [u8; 32] {
        tetherly_crypto::file_token(self.transport.handshake_hash(), transfer_id.as_bytes())
    }

    pub fn peer_is_desktop(&self) -> bool {
        tetherly_core::is_desktop_platform(&self.peer_hello.platform)
    }

    pub async fn recv_event(&mut self) -> Result<crate::dispatch::SessionEvent, NetError> {
        crate::dispatch::SessionEvent::from_frame(self.recv_inner().await?)
    }

    pub async fn ping_pong(&mut self, unix_ms: u64) -> Result<u64, NetError> {
        self.send_inner(ping(unix_ms)).await?;
        let reply = self.recv_inner().await?;
        if reply.ty != TYPE_PONG {
            return Err(NetError::Handshake);
        }
        decode_unix_ms(&reply.payload).ok_or(NetError::Handshake)
    }

    fn next_msg_id(&mut self) -> u64 {
        let id = self.next_msg_id;
        self.next_msg_id = self.next_msg_id.saturating_add(1);
        id
    }
}

pub struct SessionConfig {
    pub identity: Identity,
    pub name: String,
    pub platform: String,
    pub trust: Arc<Mutex<MemoryTrustStore>>,
    pub lockout: Arc<Mutex<LockoutTable>>,
    pub tcp_limiter: Arc<Mutex<TcpLimiter>>,
    pub pin: Option<[u8; 8]>,
    pub pin_created_ms: u64,
    pub clock: Arc<dyn Clock>,
    pub handshake_timeout: Duration,
    /// Test-only MITM fixture. Production callers leave this `None`.
    pub hello_override: Option<Hello>,
}

impl Clone for SessionConfig {
    fn clone(&self) -> Self {
        Self {
            identity: self.identity.clone(),
            name: self.name.clone(),
            platform: self.platform.clone(),
            trust: self.trust.clone(),
            lockout: self.lockout.clone(),
            tcp_limiter: self.tcp_limiter.clone(),
            pin: self.pin,
            pin_created_ms: self.pin_created_ms,
            clock: self.clock.clone(),
            handshake_timeout: self.handshake_timeout,
            hello_override: self.hello_override.clone(),
        }
    }
}

impl SessionConfig {
    fn hello(&self) -> Hello {
        self.hello_override.clone().unwrap_or_else(|| {
            self.identity
                .hello(&self.name, &self.platform, &["notify", "clip", "file"])
        })
    }

    fn now_ms(&self) -> u64 {
        self.clock.unix_ms()
    }
}

fn unix_now() -> u64 {
    SystemClock.unix_ms() / 1000
}

fn pair_role(we_initiate_noise: bool) -> PairBindRole {
    if we_initiate_noise {
        PairBindRole::Initiator
    } else {
        PairBindRole::Responder
    }
}

pub async fn accept_session(
    stream: TcpStream,
    peer_addr: SocketAddr,
    cfg: &SessionConfig,
) -> Result<ActiveSession, NetError> {
    {
        let mut lim = cfg.tcp_limiter.lock().expect("tcp limiter");
        if !lim.allow(peer_addr.ip(), cfg.now_ms()) {
            return Err(NetError::RateLimited);
        }
    }
    with_timeout(
        cfg.handshake_timeout,
        handshake(stream, peer_addr, cfg, false),
    )
    .await
}

pub async fn dial_session(
    stream: TcpStream,
    peer_addr: SocketAddr,
    cfg: &SessionConfig,
) -> Result<ActiveSession, NetError> {
    with_timeout(
        cfg.handshake_timeout,
        handshake(stream, peer_addr, cfg, true),
    )
    .await
}

async fn with_timeout<F>(timeout: Duration, fut: F) -> Result<ActiveSession, NetError>
where
    F: std::future::Future<Output = Result<ActiveSession, NetError>>,
{
    match tokio::time::timeout(timeout, fut).await {
        Ok(r) => r,
        Err(_) => Err(NetError::Timeout),
    }
}

async fn write_then_read(
    reader: &mut OwnedReadHalf,
    writer: &mut OwnedWriteHalf,
    outbound: &[u8],
    initiator: bool,
) -> Result<Vec<u8>, NetError> {
    if initiator {
        write_len_prefixed(writer, outbound).await?;
        read_len_prefixed(reader).await
    } else {
        let inbound = read_len_prefixed(reader).await?;
        write_len_prefixed(writer, outbound).await?;
        Ok(inbound)
    }
}

async fn handshake(
    stream: TcpStream,
    peer_addr: SocketAddr,
    cfg: &SessionConfig,
    we_initiate_noise: bool,
) -> Result<ActiveSession, NetError> {
    let (mut reader, mut writer) = stream.into_split();
    let local_hello = cfg.hello();
    let remote_bytes = write_then_read(
        &mut reader,
        &mut writer,
        &local_hello.to_canonical_bytes()?,
        we_initiate_noise,
    )
    .await?;
    let remote_hello = match Hello::from_bytes(&remote_bytes) {
        Ok(h) => h,
        Err(CoreError::DeviceIdMismatch) => return Err(NetError::DeviceIdMismatch),
        Err(e) => return Err(e.into()),
    };

    let disposition = {
        let store = cfg.trust.lock().expect("trust");
        match dispose_hello(&remote_hello, &*store) {
            Ok(d) => d,
            Err(CoreError::StaticKeyChanged) => return Err(NetError::StaticKeyChanged),
            Err(e) => return Err(e.into()),
        }
    };

    match disposition {
        HelloDisposition::Pair => {
            pair_then_noise(
                reader,
                writer,
                peer_addr,
                cfg,
                local_hello,
                remote_hello,
                we_initiate_noise,
            )
            .await
        }
        HelloDisposition::ResumeNoise => {
            finish_noise(
                reader,
                writer,
                peer_addr,
                cfg,
                local_hello,
                remote_hello,
                we_initiate_noise,
            )
            .await
        }
    }
}

async fn pair_then_noise(
    mut reader: OwnedReadHalf,
    mut writer: OwnedWriteHalf,
    peer_addr: SocketAddr,
    cfg: &SessionConfig,
    local_hello: Hello,
    remote_hello: Hello,
    we_initiate_noise: bool,
) -> Result<ActiveSession, NetError> {
    let now_ms = cfg.now_ms();
    {
        let table = cfg.lockout.lock().expect("lock");
        if table.check(Some(&remote_hello.device_id), Some(peer_addr.ip()), now_ms) {
            return Err(NetError::LockedOut);
        }
    }
    if now_ms.saturating_sub(cfg.pin_created_ms) > PIN_TTL.as_millis() as u64 {
        return Err(NetError::from(tetherly_crypto::CryptoError::PinExpired));
    }
    let mut pin = cfg.pin.ok_or(NetError::WrongPin)?;
    let pin_ascii = tetherly_core::pin::pin_ascii(&pin);
    pin.zeroize();

    let local_id_pk = *cfg.identity.id_pk();
    let remote_keys = remote_hello.decode_keys()?;

    let (id_lo, id_hi) = if cfg.identity.device_id().as_str() <= remote_hello.device_id.as_str() {
        (
            cfg.identity.device_id().as_str().as_bytes().to_vec(),
            remote_hello.device_id.as_str().as_bytes().to_vec(),
        )
    } else {
        (
            remote_hello.device_id.as_str().as_bytes().to_vec(),
            cfg.identity.device_id().as_str().as_bytes().to_vec(),
        )
    };
    let we_are_alice = cfg.identity.device_id().as_str() <= remote_hello.device_id.as_str();

    let spake = SpakeRole::start(
        we_are_alice,
        &pin_ascii,
        &local_id_pk,
        &remote_keys.id_pk,
        &id_lo,
        &id_hi,
    )?;
    let inbound = write_then_read(
        &mut reader,
        &mut writer,
        spake.outbound_message(),
        we_initiate_noise,
    )
    .await?;
    let mut pair_key = match spake.finish(&inbound) {
        Ok(k) => k,
        Err(_) => {
            fail_lock(cfg, &remote_hello.device_id, peer_addr);
            return Err(NetError::PairingFailed);
        }
    };

    let role = pair_role(we_initiate_noise);
    let bind = PairBind::sign(&cfg.identity);
    let ct = encrypt_pairbind(&pair_key, &bind, role)?;
    let peer_ct = write_then_read(&mut reader, &mut writer, &ct, we_initiate_noise).await?;
    let peer_bind = match decrypt_pairbind(&pair_key, &peer_ct, role.peer()) {
        Ok(b) => b,
        Err(_) => {
            pair_key.zeroize();
            fail_lock(cfg, &remote_hello.device_id, peer_addr);
            return Err(NetError::PairingFailed);
        }
    };
    pair_key.zeroize();

    if peer_bind.device_id != remote_hello.device_id
        || peer_bind.id_pk != remote_keys.id_pk
        || peer_bind.n_pk != remote_keys.n_pk
    {
        fail_lock(cfg, &remote_hello.device_id, peer_addr);
        return Err(NetError::PairingFailed);
    }

    // Noise must succeed before trust is durable. Alias comes from PairBind
    // (signed identity) rather than the unauthenticated Hello name.
    let alias = peer_bind.device_id.as_str().to_string();
    let session = finish_noise(
        reader,
        writer,
        peer_addr,
        cfg,
        local_hello,
        remote_hello.clone(),
        we_initiate_noise,
    )
    .await;
    match session {
        Ok(s) => {
            {
                let mut store = cfg.trust.lock().expect("trust");
                commit_trust(&mut *store, &remote_hello, cfg.now_ms(), alias)?;
            }
            {
                let mut table = cfg.lockout.lock().expect("lock");
                table.success(Some(&remote_hello.device_id), Some(peer_addr.ip()));
            }
            debug!(peer = %remote_hello.device_id, "paired");
            Ok(s)
        }
        Err(e) => {
            let mut store = cfg.trust.lock().expect("trust");
            let _ = store.remove(&remote_hello.device_id);
            Err(e)
        }
    }
}

fn fail_lock(cfg: &SessionConfig, peer: &DeviceId, addr: SocketAddr) {
    let mut table = cfg.lockout.lock().expect("lock");
    table.fail(Some(peer), Some(addr.ip()), cfg.now_ms());
    warn!(fails = table.fails_for(peer), "pairing failure recorded");
}

async fn finish_noise(
    mut reader: OwnedReadHalf,
    mut writer: OwnedWriteHalf,
    peer_addr: SocketAddr,
    cfg: &SessionConfig,
    local_hello: Hello,
    remote_hello: Hello,
    we_initiate_noise: bool,
) -> Result<ActiveSession, NetError> {
    let remote_n_pk = {
        let store = cfg.trust.lock().expect("trust");
        match store.get(&remote_hello.device_id) {
            Some(peer) => peer.n_pk,
            None => remote_hello.decode_keys()?.n_pk,
        }
    };
    let p = prologue(&local_hello, &remote_hello)?;
    let mut transport = if we_initiate_noise {
        let mut hs = NoiseHandshake::initiator(&cfg.identity, &remote_n_pk, &p)?;
        let m1 = hs.write_message(b"")?;
        write_len_prefixed(&mut writer, &m1).await?;
        let m2 = read_len_prefixed(&mut reader).await?;
        hs.read_message(&m2)?;
        if !hs.is_handshake_finished() {
            return Err(NetError::Handshake);
        }
        hs.into_transport()?
    } else {
        let mut hs = NoiseHandshake::responder(&cfg.identity, &p)?;
        let m1 = read_len_prefixed(&mut reader).await?;
        hs.read_message(&m1)?;
        let m2 = hs.write_message(b"")?;
        write_len_prefixed(&mut writer, &m2).await?;
        if !hs.is_handshake_finished() {
            return Err(NetError::Handshake);
        }
        hs.into_transport()?
    };
    transport.set_started(unix_now());

    Ok(ActiveSession {
        peer_id: remote_hello.device_id.clone(),
        peer_hello: remote_hello,
        peer_addr,
        reader,
        writer,
        transport,
        next_msg_id: 1,
        replay: ReplayGuard::default(),
    })
}

pub async fn serve_pong_once(session: &mut ActiveSession) -> Result<(), NetError> {
    let frame = session.recv_inner().await?;
    if frame.ty == TYPE_PING {
        let ts = decode_unix_ms(&frame.payload).unwrap_or(0);
        session.send_inner(pong(ts)).await?;
    }
    Ok(())
}
