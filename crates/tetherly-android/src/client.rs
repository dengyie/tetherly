// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The Android client: dial the desktop's 45717 control port, pair with the
//! 8-digit PIN (first run) or resume the Noise IK session (afterwards), then
//! forward notifications as `notify.push`. No OS calls live here — only TCP
//! plus the shared crate stack — so the whole path is testable on the host.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tetherly_core::frame::{
    decode_unix_ms, ping, pong, NotifyReply, TYPE_NOTIFY_REPLY, TYPE_PING, TYPE_PONG,
};
use tetherly_core::ports::{Clock, MemoryTrustStore, SystemClock};
use tetherly_core::session::LockoutTable;
use tetherly_core::{DeviceId, NotifyPush, TcpLimiter};
use tetherly_crypto::identity::Identity;
use tetherly_net::session::{dial_session, ActiveSession, SessionConfig, HANDSHAKE_TIMEOUT};
use tetherly_net::NetError;
use tokio::net::TcpStream;
use tracing::{info, warn};

use crate::store::{load_or_create_identity, load_trust, save_trust, StoreError};

/// Phone capability advertisement: it produces notifications and clipboard and
/// can send files, but it is not a desktop — never `input` or `screen`.
const PHONE_CAPS: &[&str] = &["notify", "clip", "file"];

/// How long a blocking read waits before the loop re-drains the outbox. Keeps
/// notification egress latency well inside the M1.1 budget without a second
/// task fighting the session's single `&mut`.
const RECV_TICK: Duration = Duration::from_millis(150);
const RECONNECT_MIN: Duration = Duration::from_millis(250);
const RECONNECT_MAX: Duration = Duration::from_secs(5);

/// Connection state, mirrored to the UI. Codes are stable across the JNI edge.
pub const STATE_IDLE: i32 = 0;
pub const STATE_CONNECTING: i32 = 1;
pub const STATE_CONNECTED: i32 = 2;
pub const STATE_ERROR: i32 = 3;

#[derive(Debug, Clone)]
enum Inner {
    Idle,
    Connecting,
    Connected(DeviceId),
    Error(String),
}

impl Inner {
    fn code(&self) -> i32 {
        match self {
            Inner::Idle => STATE_IDLE,
            Inner::Connecting => STATE_CONNECTING,
            Inner::Connected(_) => STATE_CONNECTED,
            Inner::Error(_) => STATE_ERROR,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("store: {0}")]
    Store(#[from] StoreError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// A running (or reconnecting) Android session. Cheap to clone and safe to
/// call from any thread: the JNI layer keeps one and the listener enqueues.
#[derive(Clone)]
pub struct SessionHandle {
    outbox: Sender<NotifyPush>,
    /// Inbound quick replies from the desktop, drained by the JNI layer so the
    /// Kotlin service can fire the notification's reply action.
    inbox: Arc<Mutex<Vec<NotifyReply>>>,
    state: Arc<Mutex<Inner>>,
    stop: Arc<AtomicBool>,
}

impl SessionHandle {
    /// Queue a notification. Returns false when no session is connected so the
    /// caller can log honestly instead of pretending it was delivered.
    pub fn notify(&self, push: NotifyPush) -> bool {
        if !matches!(*self.state.lock().expect("state"), Inner::Connected(_)) {
            return false;
        }
        self.outbox.send(push).is_ok()
    }

    /// Take every reply received since the last call. Single-use: a reply is
    /// handed out once so the phone never fires the same quick-reply twice.
    pub fn take_replies(&self) -> Vec<NotifyReply> {
        std::mem::take(&mut *self.inbox.lock().expect("inbox"))
    }

    pub fn state_code(&self) -> i32 {
        self.state.lock().expect("state").code()
    }

    pub fn connected_peer(&self) -> Option<DeviceId> {
        match &*self.state.lock().expect("state") {
            Inner::Connected(id) => Some(id.clone()),
            _ => None,
        }
    }

    pub fn last_error(&self) -> Option<String> {
        match &*self.state.lock().expect("state") {
            Inner::Error(e) => Some(e.clone()),
            _ => None,
        }
    }

    /// Ask the loop to stop. Idempotent; the worker exits within one tick.
    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

/// One Android client. `start` consumes it and spawns the worker thread.
pub struct AndroidClient {
    identity: Identity,
    data_dir: PathBuf,
    name: String,
    trust: Arc<Mutex<MemoryTrustStore>>,
}

impl AndroidClient {
    /// Load the persisted identity (creating it on first run) and trust.
    pub fn load_or_create(
        data_dir: impl Into<PathBuf>,
        name: impl Into<String>,
    ) -> Result<Self, ClientError> {
        let data_dir = data_dir.into();
        let identity = load_or_create_identity(&data_dir)?;
        let trust = load_trust(&data_dir);
        Ok(Self {
            identity,
            data_dir,
            name: name.into(),
            trust: Arc::new(Mutex::new(trust)),
        })
    }

    pub fn device_id(&self) -> DeviceId {
        self.identity.device_id().clone()
    }

    pub fn id_pk(&self) -> [u8; 32] {
        *self.identity.id_pk()
    }

    /// Spawn the connect/reconnect loop on a dedicated thread. Returns a handle
    /// immediately; the thread lives until `handle.stop()` or process exit.
    /// `once` makes the loop return after the first successful dial (tests).
    pub fn start(self, addr: SocketAddr, pin: Option<[u8; 8]>, once: bool) -> SessionHandle {
        let (tx, rx) = std::sync::mpsc::channel::<NotifyPush>();
        let state = Arc::new(Mutex::new(Inner::Idle));
        let stop = Arc::new(AtomicBool::new(false));
        let inbox = Arc::new(Mutex::new(Vec::new()));
        let handle = SessionHandle {
            outbox: tx,
            inbox: inbox.clone(),
            state: state.clone(),
            stop: stop.clone(),
        };
        let worker = Worker {
            identity: self.identity,
            data_dir: self.data_dir,
            name: self.name,
            trust: self.trust,
            addr,
            pin,
            once,
            state,
            stop,
            inbox,
        };
        std::thread::Builder::new()
            .name("tetherly-android".into())
            .spawn(move || {
                let rt = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        warn!(%e, "android runtime build failed");
                        return;
                    }
                };
                rt.block_on(worker.run_loop(rx));
            })
            .expect("spawn android session thread");
        handle
    }
}

/// Build the phone's session config. Shared by the reconnect worker and the
/// one-shot latency bench so both use the identical capability advertisement.
fn session_config(
    identity: Identity,
    name: &str,
    pin: Option<[u8; 8]>,
    trust: Arc<Mutex<MemoryTrustStore>>,
) -> SessionConfig {
    SessionConfig {
        identity,
        name: name.to_string(),
        platform: "android".into(),
        trust,
        lockout: Arc::new(Mutex::new(LockoutTable::default())),
        tcp_limiter: Arc::new(Mutex::new(TcpLimiter::default())),
        pin,
        pin_created_ms: SystemClock.unix_ms(),
        clock: Arc::new(SystemClock),
        handshake_timeout: HANDSHAKE_TIMEOUT,
        hello_override: None,
        resume_only: false,
        screen_only: false,
        caps: Some(PHONE_CAPS.iter().map(|s| (*s).to_string()).collect()),
    }
}

/// Measure the phone→desktop→phone round-trip over the real session, one sample
/// per ping the desktop answers with its pong. This is the transport-latency
/// half of the M1.1 budget: on the phone the listener callback→enqueue and
/// enqueue→socket are microseconds, and the desktop's ingest→candidate is
/// microseconds too, so RTT (and RTT/2) is the honest end-to-end proxy. Runs
/// on host or device; returns per-sample microseconds in send order.
pub async fn rtt_bench(
    addr: SocketAddr,
    data_dir: PathBuf,
    name: &str,
    pin: Option<[u8; 8]>,
    count: usize,
    gap: Duration,
) -> Result<Vec<u64>, String> {
    let identity = load_or_create_identity(&data_dir).map_err(|e| e.to_string())?;
    let trust = Arc::new(Mutex::new(load_trust(&data_dir)));
    let cfg = session_config(identity, name, pin, trust.clone());
    let stream = TcpStream::connect(addr).await.map_err(|e| e.to_string())?;
    let _ = stream.set_nodelay(true);
    let peer_addr = stream.peer_addr().unwrap_or(addr);
    let mut sess = dial_session(stream, peer_addr, &cfg)
        .await
        .map_err(|e| e.to_string())?;
    if let Err(e) = save_trust(&data_dir, &trust.lock().expect("trust")) {
        warn!(%e, "android trust persist failed");
    }
    let mut samples = Vec::with_capacity(count);
    for i in 0..count {
        let sent_ms = SystemClock.unix_ms();
        let t0 = std::time::Instant::now();
        sess.send_inner(ping(sent_ms))
            .await
            .map_err(|e| e.to_string())?;
        // Wait for the pong that echoes sent_ms (ignore unrelated inbound).
        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        loop {
            match tokio::time::timeout_at(deadline, sess.recv_inner()).await {
                Ok(Ok(frame)) if frame.ty == TYPE_PONG => {
                    if decode_unix_ms(&frame.payload) == Some(sent_ms) {
                        samples.push(t0.elapsed().as_micros() as u64);
                        break;
                    }
                }
                Ok(Ok(_)) => {}
                Ok(Err(e)) => return Err(e.to_string()),
                Err(_) => return Err(format!("pong timeout at sample {}", i + 1)),
            }
        }
        if i + 1 < count {
            tokio::time::sleep(gap).await;
        }
    }
    Ok(samples)
}

struct Worker {
    identity: Identity,
    data_dir: PathBuf,
    name: String,
    trust: Arc<Mutex<MemoryTrustStore>>,
    addr: SocketAddr,
    pin: Option<[u8; 8]>,
    once: bool,
    state: Arc<Mutex<Inner>>,
    stop: Arc<AtomicBool>,
    inbox: Arc<Mutex<Vec<NotifyReply>>>,
}

impl Worker {
    fn set(&self, inner: Inner) {
        *self.state.lock().expect("state") = inner;
    }

    fn session_config(&self) -> SessionConfig {
        session_config(
            self.identity.clone(),
            &self.name,
            self.pin,
            self.trust.clone(),
        )
    }

    fn persist_trust(&self) {
        let store = self.trust.lock().expect("trust");
        if let Err(e) = save_trust(&self.data_dir, &store) {
            warn!(%e, "android trust persist failed");
        }
    }

    async fn run_loop(&self, rx: Receiver<NotifyPush>) {
        let mut backoff = RECONNECT_MIN;
        loop {
            if self.stop.load(Ordering::SeqCst) {
                return;
            }
            self.set(Inner::Connecting);
            match self.dial().await {
                Ok(mut sess) => {
                    let peer = sess.peer_id.clone();
                    info!(peer = %peer, "android session up");
                    self.persist_trust();
                    self.set(Inner::Connected(peer));
                    backoff = RECONNECT_MIN;
                    let outcome = self.serve(&mut sess, &rx).await;
                    self.set(Inner::Idle);
                    match outcome {
                        Ok(()) => {}
                        Err(e) => warn!(%e, "android session ended"),
                    }
                    if self.once {
                        return;
                    }
                }
                Err(e) => {
                    warn!(%e, "android dial failed");
                    self.set(Inner::Error(e.to_string()));
                    if self.once {
                        return;
                    }
                }
            }
            if self.stop.load(Ordering::SeqCst) {
                return;
            }
            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(RECONNECT_MAX);
        }
    }

    async fn dial(&self) -> Result<ActiveSession, NetError> {
        let stream = TcpStream::connect(self.addr).await?;
        let _ = stream.set_nodelay(true);
        let peer_addr = stream.peer_addr().unwrap_or(self.addr);
        dial_session(stream, peer_addr, &self.session_config()).await
    }

    async fn serve(
        &self,
        sess: &mut ActiveSession,
        rx: &Receiver<NotifyPush>,
    ) -> Result<(), NetError> {
        let mut sent = 0u64;
        loop {
            if self.stop.load(Ordering::SeqCst) {
                return Ok(());
            }
            // Drain queued notifications first (non-blocking), then wait a
            // short tick for anything inbound. The desktop sends pings; we
            // answer them so the session stays healthy.
            loop {
                match rx.try_recv() {
                    Ok(push) => {
                        sess.send_inner(push.to_frame()?).await?;
                        sent += 1;
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => return Ok(()),
                }
            }
            // Single-shot harness: one delivered notification is the proof.
            if self.once && sent > 0 {
                return Ok(());
            }
            match tokio::time::timeout(RECV_TICK, sess.recv_inner()).await {
                Ok(Ok(frame)) => {
                    if frame.ty == TYPE_PING {
                        let ts = decode_unix_ms(&frame.payload).unwrap_or(0);
                        sess.send_inner(pong(ts)).await?;
                    } else if frame.ty == TYPE_NOTIFY_REPLY {
                        // A quick reply typed on the desktop. Queue it for the
                        // JNI layer to fire the notification's reply action.
                        // The text is user content and is never logged.
                        match NotifyReply::from_payload(&frame.payload) {
                            Ok(reply) => {
                                let mut inbox = self.inbox.lock().expect("inbox");
                                // Idempotent: last write wins for one uid+app.
                                inbox.retain(|r| !(r.uid == reply.uid && r.app_id == reply.app_id));
                                inbox.push(reply);
                            }
                            Err(e) => warn!(%e, "malformed notify.reply dropped"),
                        }
                    }
                    // NotifyDismiss / Caps / Unknown: nothing for a phone to do.
                }
                Ok(Err(e)) => return Err(e),
                Err(_) => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phone_caps_exclude_desktop_roles() {
        assert!(PHONE_CAPS.contains(&"notify"));
        assert!(!PHONE_CAPS.contains(&"input"));
        assert!(!PHONE_CAPS.contains(&"screen"));
    }

    #[test]
    fn state_codes_are_stable() {
        assert_eq!(Inner::Idle.code(), STATE_IDLE);
        assert_eq!(Inner::Connecting.code(), STATE_CONNECTING);
        assert_eq!(
            Inner::Connected(DeviceId::from_id_pk(&[0u8; 32])).code(),
            STATE_CONNECTED
        );
        assert_eq!(Inner::Error("x".into()).code(), STATE_ERROR);
    }
}
