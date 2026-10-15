// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::codec::{
    read_len_prefixed, read_tagged, write_len_prefixed, write_tagged, TAG_DATA, TAG_REKEY,
};
use crate::error::NetError;
use std::collections::VecDeque;
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
pub const SCREEN_PORT: u16 = 45720;

/// Handshake I/O budget. Argon2id itself is ~hundreds of ms; 20s covers a
/// slow LAN plus KDF without hanging forever on a half-open TCP.
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(20);

/// Budget for one inner-frame write. A peer that stops reading leaves
/// `write_all` parked on a full kernel buffer with no error and no EOF, so an
/// unbounded write would wedge the session task forever while the caller keeps
/// queueing frames into the void. 15s is far past any healthy LAN round trip.
pub const SESSION_WRITE_TIMEOUT: Duration = Duration::from_secs(15);

pub struct ActiveSession {
    pub peer_id: DeviceId,
    pub peer_hello: Hello,
    pub peer_addr: SocketAddr,
    reader: OwnedReadHalf,
    writer: OwnedWriteHalf,
    transport: NoiseTransport,
    next_msg_id: u64,
    replay: ReplayGuard,
    /// Everything needed to rebuild a fresh IK handshake in band. `snow` 0.9
    /// `TransportState` has no `rekey()`, so the SSOT mandates a new IK
    /// handshake (same static keys) after 2^16 messages or 3600s.
    rekey: RekeyContext,
    /// In-flight rekey. `send_inner`/`recv_inner` run inside `select!`, which
    /// may cancel them at an await; a rekey therefore cannot be a single
    /// `await`-heavy function but must resume from where it stopped.
    rekey_state: Option<RekeyInFlight>,
    /// Frames the peer sent under the old transport while our rekey handshake
    /// was in flight. Drained by `recv_inner` before reading the wire again.
    pending: VecDeque<InnerFrame>,
}

/// Static material for an in-band rekey. `Identity` and `Hello` are cheap to
/// clone and the values are fixed for the whole session.
struct RekeyContext {
    identity: Identity,
    local_hello: Hello,
    remote_hello: Hello,
    remote_n_pk: [u8; 32],
    /// Only the Noise initiator drives a rekey, so the two sides can never
    /// start one simultaneously. The responder reacts to the initiator's
    /// handshake message.
    we_initiate_noise: bool,
}

/// A rekey handshake resumable across cancellations. The `NoiseHandshake`
/// keeps its own ratchet state; we only track which step comes next so a
/// `select!`-cancelled send/recv can re-enter without re-ratcheting.
struct RekeyInFlight {
    hs: NoiseHandshake,
    phase: RekeyPhase,
}

enum RekeyPhase {
    /// Initiator: `m1` is ready to write, then we wait for the peer's `m2`.
    SendInit { m1: Vec<u8> },
    /// Initiator: `m1` is on the wire, awaiting `m2`.
    AwaitReply,
    /// Responder: `m1` was read; write this `m2`, then swap transports.
    SendReply { m2: Vec<u8> },
}

impl ActiveSession {
    /// A rekey is due when the live transport hit its counter/time limit, or
    /// when a rekey was already started and cancelled before finishing (the
    /// responder's old transport may itself not be "due", so the in-flight
    /// state has to force the resume).
    fn rekey_due(&self, now_unix: u64) -> bool {
        self.rekey_state.is_some() || self.transport.needs_rekey(now_unix)
    }

    pub async fn send_inner(&mut self, mut frame: InnerFrame) -> Result<(), NetError> {
        if self.rekey_due(unix_now()) {
            self.drive_rekey().await?;
        }
        frame.msg_id = self.next_msg_id();
        let plain = frame.encode()?;
        // A fresh transport never needs a rekey, so this does not re-enter.
        let ct = self.transport.encrypt(&plain)?;
        write_tagged(&mut self.writer, TAG_DATA, &ct).await
    }

    /// [`send_inner`] under [`SESSION_WRITE_TIMEOUT`]. Bounding the write turns
    /// a silently wedged peer into a hard [`NetError::WriteStalled`] so the
    /// caller tears the session down instead of buffering into a black hole.
    /// On timeout the write half may be mid-frame, so the session must not be
    /// reused afterwards — every caller treats the error as fatal.
    pub async fn send_bounded(&mut self, frame: InnerFrame) -> Result<(), NetError> {
        match tokio::time::timeout(SESSION_WRITE_TIMEOUT, self.send_inner(frame)).await {
            Ok(res) => res,
            Err(_) => Err(NetError::WriteStalled),
        }
    }

    pub async fn recv_inner(&mut self) -> Result<InnerFrame, NetError> {
        if self.rekey_due(unix_now()) {
            self.drive_rekey().await?;
        }
        let frame = loop {
            // Frames the peer sent under the old transport while a rekey was
            // in flight are already decrypted and ordered; deliver them first.
            if let Some(frame) = self.pending.pop_front() {
                break frame;
            }
            let (tag, body) = read_tagged(&mut self.reader).await?;
            match tag {
                TAG_DATA => {
                    let plain = self.transport.decrypt(&body)?;
                    break InnerFrame::decode(&plain)?;
                }
                TAG_REKEY => {
                    // Only the initiator starts a rekey, so an inbound
                    // handshake message reaches us exclusively as `m1`.
                    self.accept_rekey_start(body).await?;
                    // `m2` is on the wire; loop to read the first frame under
                    // the new transport.
                }
                _ => return Err(NetError::Handshake),
            }
        };
        if !self.replay.check_and_record(frame.msg_id) {
            return Err(NetError::Replay);
        }
        Ok(frame)
    }

    /// Advance an in-band rekey if the transport is due, and run it to
    /// completion. Cancellation-safe: all progress lives in `self.rekey_state`.
    async fn drive_rekey(&mut self) -> Result<(), NetError> {
        if let Some(inflight) = self.rekey_state.take() {
            return self.rekey_step(inflight).await;
        }
        if !self.rekey.we_initiate_noise {
            // The responder never starts a rekey; it waits for the initiator's
            // `m1`. Reusing a spent transport for one more frame is harmless —
            // only the initiator's counter gates the boundary.
            return Ok(());
        }
        let p = prologue(&self.rekey.local_hello, &self.rekey.remote_hello)?;
        let mut hs = NoiseHandshake::initiator(&self.rekey.identity, &self.rekey.remote_n_pk, &p)?;
        let m1 = hs.write_message(b"")?;
        self.rekey_step(RekeyInFlight {
            hs,
            phase: RekeyPhase::SendInit { m1 },
        })
        .await
    }

    /// Run an in-flight rekey one step further. On cancellation the state is
    /// put back first, so the caller re-enters at exactly this step.
    async fn rekey_step(&mut self, mut inflight: RekeyInFlight) -> Result<(), NetError> {
        loop {
            match inflight.phase {
                RekeyPhase::SendInit { m1 } => {
                    // Take the message out before awaiting: a cancel while the
                    // write is pending loses it, so re-enter at `AwaitReply`
                    // only after the write completed.
                    self.rekey_state = Some(RekeyInFlight {
                        hs: inflight.hs,
                        phase: RekeyPhase::AwaitReply,
                    });
                    write_tagged(&mut self.writer, TAG_REKEY, &m1).await?;
                    inflight = self.rekey_state.take().expect("rekey in flight");
                }
                RekeyPhase::AwaitReply => {
                    // Park the state before reading so a select! cancelled read
                    // resumes the same handshake instead of dropping it.
                    self.rekey_state = Some(inflight);
                    let (tag, body) = read_tagged(&mut self.reader).await?;
                    inflight = self.rekey_state.take().expect("rekey in flight");
                    match tag {
                        TAG_REKEY => {
                            inflight.hs.read_message(&body)?;
                            if !inflight.hs.is_handshake_finished() {
                                return Err(NetError::Handshake);
                            }
                            let mut transport = inflight.hs.into_transport()?;
                            transport.set_started(unix_now());
                            self.transport = transport;
                            debug!(peer = %self.peer_id, "noise rekey complete (initiator)");
                            return Ok(());
                        }
                        TAG_DATA => {
                            // Old-transport frame racing ahead of the peer's
                            // `m2`: decrypt under the old transport, deliver
                            // after the swap.
                            let plain = self.transport.decrypt(&body)?;
                            self.pending.push_back(InnerFrame::decode(&plain)?);
                        }
                        _ => return Err(NetError::Handshake),
                    }
                }
                RekeyPhase::SendReply { m2 } => {
                    // `m2` depends on `m1`; rebuilding it would consume the
                    // handshake again, so it is computed once when `m1` is read
                    // and only the write is retried.
                    self.rekey_state = Some(RekeyInFlight {
                        hs: inflight.hs,
                        phase: RekeyPhase::SendReply { m2: m2.clone() },
                    });
                    write_tagged(&mut self.writer, TAG_REKEY, &m2).await?;
                    let done = self.rekey_state.take().expect("rekey in flight");
                    let mut transport = done.hs.into_transport()?;
                    transport.set_started(unix_now());
                    self.transport = transport;
                    debug!(peer = %self.peer_id, "noise rekey complete (responder)");
                    return Ok(());
                }
            }
        }
    }

    /// The peer started a rekey: read `m1`, verify its static, build `m2`, then
    /// hand off to `rekey_step` to write it. The static check is the same as
    /// the initial handshake — a replayed `Hello` must not let an attacker
    /// substitute their own static key mid-session.
    async fn accept_rekey_start(&mut self, m1: Vec<u8>) -> Result<(), NetError> {
        let p = prologue(&self.rekey.local_hello, &self.rekey.remote_hello)?;
        let mut hs = NoiseHandshake::responder(&self.rekey.identity, &p)?;
        hs.read_message(&m1)?;
        match hs.remote_static() {
            Some(k) if k == self.rekey.remote_n_pk.as_slice() => {}
            _ => return Err(NetError::StaticKeyChanged),
        }
        let m2 = hs.write_message(b"")?;
        // In IK the responder's handshake only completes once `m2` is written.
        if !hs.is_handshake_finished() {
            return Err(NetError::Handshake);
        }
        self.rekey_state = None;
        self.rekey_step(RekeyInFlight {
            hs,
            phase: RekeyPhase::SendReply { m2 },
        })
        .await
    }

    pub fn handshake_hash(&self) -> &[u8; 32] {
        self.transport.handshake_hash()
    }

    /// Test-only: force the transport to be "due for rekey" so the in-band
    /// rekey can be exercised without sending 2^16 frames. Production must
    /// never call this.
    #[doc(hidden)]
    pub fn debug_force_rekey_due(&mut self) {
        self.transport.debug_force_rekey_due();
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
    /// Input port 45719: Noise IK resume only. Pairing stays on 45717.
    pub resume_only: bool,
    /// Screen port 45720: Noise IK resume only. Pairing stays on 45717.
    pub screen_only: bool,
    /// Explicit Hello capability list. `None` derives from the port flags, so
    /// a plain caller advertises the full desktop set. The Android client sets
    /// `Some(["notify","clip","file"])` — a phone is not a desktop and must not
    /// claim input or screen.
    pub caps: Option<Vec<String>>,
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
            resume_only: self.resume_only,
            screen_only: self.screen_only,
            caps: self.caps.clone(),
        }
    }
}

impl SessionConfig {
    fn hello(&self) -> Hello {
        self.hello_override.clone().unwrap_or_else(|| {
            let caps: Vec<&str> = if let Some(explicit) = &self.caps {
                explicit.iter().map(String::as_str).collect()
            } else if self.screen_only {
                // The screen channel is also resume_only; screen_only is the
                // more specific one, so it wins the advertised cap.
                vec!["screen"]
            } else if self.resume_only {
                vec!["input"]
            } else {
                vec!["notify", "clip", "file", "input", "screen"]
            };
            self.identity.hello(&self.name, &self.platform, &caps)
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
            // screen_only is also resume_only; report the more specific one.
            if cfg.screen_only {
                return Err(NetError::ScreenRequiresTrust);
            }
            if cfg.resume_only {
                return Err(NetError::InputRequiresTrust);
            }
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

    // Argon2id (64 MiB, t=3) is intentionally expensive; run it off the async
    // executor so pairing never stalls other tasks on this runtime thread.
    let id_pk_left = local_id_pk;
    let id_pk_right = remote_keys.id_pk;
    let pin_owned = pin_ascii;
    let password = tokio::task::spawn_blocking(move || {
        let mut p = pin_owned;
        let out = tetherly_crypto::derive_pair_password(&p, &id_pk_left, &id_pk_right);
        p.zeroize();
        out
    })
    .await
    .map_err(|_| NetError::PairingFailed)??;

    let spake = SpakeRole::start_with_password(we_are_alice, password, &id_lo, &id_hi);
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
        // In IK the responder learns the initiator's static from message 1.
        // `remote_n_pk` is the key we trust for this device id (from the store
        // on the resume path, or the authenticated PairBind on the pair path);
        // if the initiator's actual static does not match, the peer merely
        // replayed a trusted Hello and must be refused.
        match hs.remote_static() {
            Some(k) if k == remote_n_pk.as_slice() => {}
            _ => return Err(NetError::StaticKeyChanged),
        }
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
        peer_hello: remote_hello.clone(),
        peer_addr,
        reader,
        writer,
        transport,
        next_msg_id: 1,
        replay: ReplayGuard::default(),
        rekey: RekeyContext {
            identity: cfg.identity.clone(),
            local_hello,
            remote_hello,
            remote_n_pk,
            we_initiate_noise,
        },
        rekey_state: None,
        pending: VecDeque::new(),
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
