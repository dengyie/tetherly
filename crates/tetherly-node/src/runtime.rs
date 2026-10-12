// SPDX-License-Identifier: Apache-2.0 OR MIT
//! LAN + optional EasyTier sidecar overlay. Overlay is unicast only; no mDNS on TUN.

use crate::events::{file_offered, peer_up, CandidateViewDto, UiEvent};
use crate::lan::{bind_many, bind_many_optional, bind_overlay, control_bind_addrs, lan_ips};
use crate::platform::{insert_foreground, MemoryClip, SystemClip};
use crate::sidecar::{self, OverlayConfig, OverlayStatus};
use crate::store::{load_trust, remember_route, save_trust, RouteTable};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tetherly_core::frame::{
    pong, CapsUpdate, ClipSet, FileDone, FileOffer, InnerFrame, NotifyPush, NotifyReply,
};
use tetherly_core::pin::{format_pin, generate_pin};
use tetherly_core::ports::{Clip, Clock, Rng, SystemClock};
use tetherly_core::session::LockoutTable;
use tetherly_core::session::{backoff_delay, PIN_TTL};
use tetherly_core::{
    ancs_actions, ancs_source_id, is_desktop_platform, path_kind, AncsEvent, AncsIngress,
    AncsState, AncsTransport, ClipApply, ClipHub, ConnectOutcome, ControlMsg, CoreError,
    CursorSeat, DeviceId, FileHub, IngestOutcome, InputClient, InputEvent, InputServer, InputSink,
    MemoryScreenSink, MemorySink, MemoryTrustStore, NotifyHub, OpenAllowlist, PathKind, ReplyView,
    ScreenAssembler, ScreenCodec, ScreenFrame, ScreenReceiver, ScreenSender, ScreenSink,
    ScreenSource, ScreenState, ScreenStats, TcpLimiter, TrustStore, CLIPBOARD_OTP_CLEAR_MS,
    OVERLAY_PROBE_MS,
};
use tetherly_crypto::Identity;
use tetherly_net::dispatch::SessionEvent;
use tetherly_net::filechan::{read_header as read_file_header, recv_chunks_after_header};
use tetherly_net::session::{
    accept_session, dial_session, ActiveSession, SessionConfig, CONTROL_PORT, FILE_PORT,
    HANDSHAKE_TIMEOUT, INPUT_PORT, SCREEN_PORT,
};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, mpsc, watch, Mutex as AsyncMutex};
use tracing::{debug, info, warn};

struct OsRng;

impl Rng for OsRng {
    fn fill_bytes(&self, dest: &mut [u8]) -> Result<(), CoreError> {
        getrandom::getrandom(dest).map_err(|_| CoreError::Rng)
    }
}

/// Hard budget for one `ScreenSource::grab` before the runtime interrupts it.
const SCREEN_GRAB_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

#[derive(Clone)]

pub struct NodeConfig {
    pub data_dir: PathBuf,
    pub name: String,
    pub platform: String,
    pub control_port: u16,
    pub file_port: u16,
    pub input_port: u16,
    pub screen_port: u16,
    pub advertise: bool,
    /// Tests inject a clip. Production uses SystemClip.
    pub memory_clip: bool,
    /// Tests bind 127.0.0.1 only so two nodes can share a host.
    pub loopback_only: bool,
    /// Production reconnects trusted peers. Tests turn this off to avoid races.
    pub reconnect: bool,
    /// Loopback HTTP UI. 0 disables.
    pub ui_port: u16,
    /// EasyTier sidecar. Missing binary/RPC is success (LAN only).
    pub overlay: OverlayConfig,
    /// ANCS link to a phone. `None` until the platform BLE layer is attached;
    /// tests attach a `MemoryAncsTransport`.
    pub ancs: Option<Arc<dyn AncsTransport>>,
    /// BLE peripheral identity of the phone, used to derive its synthetic id.
    pub ancs_peripheral: String,
    /// Screen capture source. `None` defaults to a System stub that returns
    /// ScreenRefused (real capture is Manual-required). Tests inject
    /// MemoryScreenSource for deterministic CI.
    pub screen_source: Option<Arc<dyn ScreenSource>>,
    /// Screen encoder/decoder. `None` defaults to lossless zstd level 3.
    pub screen_codec: Option<Arc<dyn ScreenCodec>>,
    /// Screen presentation sink. `None` defaults to the in-memory observer;
    /// production passes `SystemSink` for real output.
    pub screen_sink: Option<Arc<dyn ScreenSink>>,
    /// Real key/mouse injection for the controlled side of 45719. `None`
    /// keeps CI's MemorySink-only behavior (the observer always records).
    pub input_sink: Option<Arc<Mutex<dyn InputSink>>>,
}

impl Default for NodeConfig {
    fn default() -> Self {
        Self {
            data_dir: crate::store::default_data_dir(),
            name: hostname(),
            platform: std::env::consts::OS.to_string(),
            control_port: CONTROL_PORT,
            file_port: FILE_PORT,
            input_port: INPUT_PORT,
            screen_port: SCREEN_PORT,
            advertise: true,
            memory_clip: false,
            loopback_only: false,
            reconnect: true,
            ui_port: 45716,
            overlay: OverlayConfig::default(),
            ancs: None,
            ancs_peripheral: "local-iphone".into(),
            screen_source: None,
            screen_codec: None,
            screen_sink: None,
            input_sink: None,
        }
    }
}

fn hostname() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "tetherly".into())
}

/// Scan `inbox/` for `.<name>.resume.json` sidecars and rebuild the in-memory
/// resume table. Only meaningful for content-addressed transfers whose
/// `transfer_id` equals the name; a mismatched/unreadable sidecar is skipped.
fn load_resume_table(data_dir: &std::path::Path) -> tetherly_core::ResumeTable {
    let mut table = tetherly_core::ResumeTable::new();
    let inbox = data_dir.join("inbox");
    let Ok(rd) = std::fs::read_dir(&inbox) else {
        return table;
    };
    for entry in rd.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let Some(stem) = name
            .strip_prefix('.')
            .and_then(|s| s.strip_suffix(".resume.json"))
        else {
            continue;
        };
        let Ok(bytes) = std::fs::read(entry.path()) else {
            continue;
        };
        let Ok(dto) = serde_json::from_slice::<ResumeSidecar>(&bytes) else {
            continue;
        };
        let Ok(source) = DeviceId::parse(&dto.device_id) else {
            continue;
        };
        if dto.offset == 0 || dto.offset >= dto.total {
            continue;
        }
        // A sidecar without a matching (or long enough) partial is torn state
        // (crash between chunk write and sidecar write): drop it and restart
        // the transfer from 0 rather than promising a resume we cannot honor.
        let part = inbox.join(format!(".{stem}.part"));
        let len = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
        if len < dto.offset {
            let _ = std::fs::remove_file(entry.path());
            continue;
        }
        table.put(tetherly_core::ResumeState {
            transfer_id: stem.to_string(),
            source,
            total: dto.total,
            offset: dto.offset,
        });
    }
    table
}

#[derive(Clone)]
pub struct LivePeer {
    pub device_id: DeviceId,
    pub name: String,
    pub platform: String,
    pub addr: SocketAddr,
    pub handshake_hash: [u8; 32],
    /// Advertised via caps.update `fileport=`. None until the peer tells us.
    pub file_port: Option<u16>,
    /// Advertised via caps.update `inputport=`. None until the peer tells us.
    pub input_port: Option<u16>,
    /// Advertised via caps.update `screenport=`. None until the peer tells us.
    pub screen_port: Option<u16>,
    pub path: PathKind,
}

struct Hubs {
    notify: NotifyHub,
    clip: ClipHub,
    files: FileHub,
    /// Chunk-aligned progress markers for interrupted incoming transfers.
    /// Loaded from disk at startup; the node persists them as sidecars.
    resume: tetherly_core::ResumeTable,
}

struct OutboundFile {
    peer: DeviceId,
    data: Vec<u8>,
}

/// Sidecar persisted next to a `.part` file: lets an interrupted incoming
/// transfer resume after a process restart. No file content, only progress.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct ResumeSidecar {
    device_id: String,
    total: u64,
    offset: u64,
}

/// Wall-clock p95 of host-side frame send intervals. Real timing lives here in
/// tetherly-node (core is deterministic and OS-free); production verifies the
/// frame budget (M5.1) against this. Microsecond resolution.
struct ScreenIntervalTracker {
    last: Option<std::time::Instant>,
    intervals: Vec<u64>, // micros, newest first
    max: usize,
}

impl ScreenIntervalTracker {
    fn new() -> Self {
        Self {
            last: None,
            intervals: Vec::with_capacity(1024),
            max: 1024,
        }
    }

    /// Mark one frame send. The elapsed time since the previous send is the
    /// interval; the first send only seeds the clock.
    fn record(&mut self) {
        let now = std::time::Instant::now();
        if let Some(prev) = self.last.replace(now) {
            self.intervals
                .push(now.duration_since(prev).as_micros() as u64);
        }
        if self.intervals.len() > self.max {
            let excess = self.intervals.len() - self.max;
            self.intervals.drain(..excess);
        }
    }

    /// 95th-percentile frame interval in microseconds, or `None` before two
    /// frames have shipped.
    fn p95_us(&self) -> Option<u64> {
        if self.intervals.len() < 2 {
            return None;
        }
        let mut sorted = self.intervals.clone();
        sorted.sort_unstable();
        let idx = ((sorted.len() as f64 * 0.95).ceil() as usize).saturating_sub(1);
        Some(sorted[idx.min(sorted.len() - 1)])
    }
}

struct Shared {
    identity: Identity,
    cfg: NodeConfig,
    trust: Arc<Mutex<MemoryTrustStore>>,
    lockout: Arc<Mutex<LockoutTable>>,
    tcp_limiter: Arc<Mutex<TcpLimiter>>,
    clock: Arc<dyn Clock>,
    hubs: Mutex<Hubs>,
    clip: Arc<dyn Clip>,
    pin: Mutex<Option<([u8; 8], u64)>>,
    events: broadcast::Sender<UiEvent>,
    sessions: AsyncMutex<HashMap<DeviceId, mpsc::UnboundedSender<InnerFrame>>>,
    input_out: AsyncMutex<Option<mpsc::UnboundedSender<InnerFrame>>>,
    live: Mutex<HashMap<DeviceId, LivePeer>>,
    pending_out: Mutex<HashMap<String, OutboundFile>>,
    bound_control: AtomicU16,
    bound_file: AtomicU16,
    bound_input: AtomicU16,
    bound_screen: AtomicU16,
    /// Input cursor owner. None until a trusted desktop dials 45719.
    input_seat: Mutex<Option<CursorSeat>>,
    input_sink: Mutex<MemorySink>,
    /// Production injection sink (SendInput). Applied events are mirrored
    /// here after the CI observer records them.
    input_real_sink: Option<Arc<Mutex<dyn InputSink>>>,
    /// Seq lives here so repeated edge crossings do not replay seq 1.
    input_engine: Mutex<InputServer>,
    /// Frames outbound to the screen controller on 45720. None until attached.
    screen_out: AsyncMutex<Option<mpsc::UnboundedSender<InnerFrame>>>,
    /// Sender-side screen state machine: consent, seq, refusal counts.
    screen_engine: Mutex<ScreenSender>,
    /// Where sender-side frames come from. MemorySource in CI; real capture
    /// lives in tetherly-node platform code and stays Manual-required.
    screen_source: Arc<dyn ScreenSource>,
    /// Controller-side trace of presented frames. Clone() shares inner state.
    screen_observer: Mutex<MemoryScreenSink>,
    /// Visitor-side control channel. None until a visitor attaches.
    screen_ctl: AsyncMutex<Option<mpsc::UnboundedSender<ControlMsg>>>,
    /// Visitor-side decompressor. Defaults to zstd lossless.
    screen_codec: Arc<dyn ScreenCodec>,
    /// Visitor-side output. Defaults to the in-memory observer; production
    /// injects `SystemSink` for real display.
    screen_sink: Arc<dyn ScreenSink>,
    /// Host-side frame-interval tracker (p95 budget). Recorded per successful
    /// frame send.
    screen_interval: Mutex<ScreenIntervalTracker>,
    /// Visitor-side receiver counters (received/dropped/bytes). Written by the
    /// screen-client task so the UI can show what actually arrived.
    screen_rx_stats: Mutex<ScreenStats>,
    shutdown: watch::Sender<bool>,
    overlay: Mutex<OverlayStatus>,
    /// ANCS ingress. Present only when a transport was attached.
    ancs: Mutex<Option<AncsIngress>>,
    /// Synthetic peer id for the ANCS phone (ANCS is not a Noise session).
    ancs_source: DeviceId,
    /// Apps allowed to show "open". Empty until config/UI supplies rules.
    allowlist: Mutex<OpenAllowlist>,
}

pub struct Node {
    shared: Arc<Shared>,
}

#[derive(Debug, Clone)]
pub enum UserCommand {
    GeneratePin,
    PairWith { pin: String, addr: SocketAddr },
    CopyCandidate { id: String },
    InsertCandidate { id: String },
    DismissCandidate { id: String },
    AcceptFile { transfer_id: String },
    RejectFile { transfer_id: String },
    OfferFile { path: PathBuf, peer: DeviceId },
    SendClipboard,
    Forget { device_id: DeviceId },
}

/// One accept loop, shared by every listener (control/file/input/screen, LAN
/// and overlay). Each accepted stream is handed to `handler` on its own task;
/// accept errors back off briefly. Ends when the node shuts down.
fn node_accept_loop<F, Fut>(node: Node, listener: TcpListener, label: &'static str, handler: F)
where
    F: Fn(Node, TcpStream, SocketAddr) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = Result<(), tetherly_net::NetError>> + Send,
{
    let mut stop = node.shared.shutdown.subscribe();
    let handler = Arc::new(handler);
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = stop.changed() => {
                    if *stop.borrow() {
                        break;
                    }
                }
                accepted = listener.accept() => {
                    match accepted {
                        Ok((stream, addr)) => {
                            let node = node.clone_handle();
                            let handler = handler.clone();
                            tokio::spawn(async move {
                                if let Err(e) = handler(node, stream, addr).await {
                                    debug!(%e, %addr, label, "channel");
                                }
                            });
                        }
                        Err(e) => {
                            debug!(%e, label, "accept");
                            tokio::time::sleep(Duration::from_millis(200)).await;
                        }
                    }
                }
            }
        }
    });
}

impl Node {
    pub fn start(cfg: NodeConfig) -> Result<Self, crate::store::NodeStoreError> {
        Self::start_with_clock(cfg, Arc::new(SystemClock))
    }

    pub fn start_with_clock(
        cfg: NodeConfig,
        clock: Arc<dyn Clock>,
    ) -> Result<Self, crate::store::NodeStoreError> {
        let identity = crate::cred::load_or_create_identity(&cfg.data_dir)?;
        let trust = load_trust(&cfg.data_dir)?;
        let (events, _) = broadcast::channel(64);
        let (shutdown, _) = watch::channel(false);
        let clip: Arc<dyn Clip> = if cfg.memory_clip {
            Arc::new(MemoryClip::new())
        } else {
            Arc::new(SystemClip)
        };
        let bound_control = AtomicU16::new(cfg.control_port);
        let bound_file = AtomicU16::new(cfg.file_port);
        let bound_input = AtomicU16::new(cfg.input_port);
        let bound_screen = AtomicU16::new(cfg.screen_port);
        let ancs_source = ancs_source_id(&cfg.ancs_peripheral);
        let ancs = cfg.ancs.clone().map(|t| AncsIngress::new(t, clock.clone()));
        let screen_source: Arc<dyn ScreenSource> = cfg.screen_source.clone().unwrap_or_else(|| {
            // Production: real DXGI capture → zstd lossless → wire.
            // Tests inject MemoryScreenSource raw; this default is only
            // active in the bin on Windows.
            Arc::new(crate::screen::EncodedSource::new(
                crate::screen::SystemSource::new(),
                Arc::new(crate::screen::ZstdCodec::default()),
            ))
        });
        let screen_codec: Arc<dyn ScreenCodec> = cfg
            .screen_codec
            .clone()
            .unwrap_or_else(|| Arc::new(crate::screen::ZstdCodec::default()));
        // The observer always records what was presented, so the local UI and
        // CI can assert on the stream without a display.
        let screen_observer = MemoryScreenSink::new();
        // Production passes SystemSink; CI leaves it None and presents to the
        // observer itself (the same shared state), so snapshots stay live.
        let screen_sink: Arc<dyn ScreenSink> = cfg
            .screen_sink
            .clone()
            .unwrap_or_else(|| Arc::new(screen_observer.clone()));
        let input_real_sink = cfg.input_sink.clone();
        let resume = load_resume_table(&cfg.data_dir);
        let shared = Arc::new(Shared {
            identity,
            cfg,
            trust: Arc::new(Mutex::new(trust)),
            lockout: Arc::new(Mutex::new(LockoutTable::default())),
            tcp_limiter: Arc::new(Mutex::new(TcpLimiter::default())),
            clock,
            hubs: Mutex::new(Hubs {
                notify: NotifyHub::default(),
                clip: ClipHub::default(),
                files: FileHub::default(),
                resume,
            }),
            clip,
            pin: Mutex::new(None),
            events,
            sessions: AsyncMutex::new(HashMap::new()),
            input_out: AsyncMutex::new(None),
            live: Mutex::new(HashMap::new()),
            pending_out: Mutex::new(HashMap::new()),
            bound_control,
            bound_file,
            bound_input,
            bound_screen,
            input_seat: Mutex::new(None),
            input_sink: Mutex::new(MemorySink::default()),
            input_real_sink,
            input_engine: Mutex::new(InputServer::new(1, 1)),
            screen_out: AsyncMutex::new(None),
            screen_engine: Mutex::new(ScreenSender::new()),
            screen_source,
            screen_observer: Mutex::new(screen_observer),
            screen_ctl: AsyncMutex::new(None),
            screen_codec,
            screen_sink,
            screen_interval: Mutex::new(ScreenIntervalTracker::new()),
            screen_rx_stats: Mutex::new(ScreenStats::default()),
            shutdown,
            overlay: Mutex::new(OverlayStatus::lan_only()),
            ancs: Mutex::new(ancs),
            ancs_source,
            allowlist: Mutex::new(OpenAllowlist::default()),
        });
        Ok(Self { shared })
    }

    pub fn identity(&self) -> &Identity {
        &self.shared.identity
    }

    pub fn subscribe(&self) -> broadcast::Receiver<UiEvent> {
        self.shared.events.subscribe()
    }

    pub fn session_config(&self) -> SessionConfig {
        self.session_config_inner(false)
    }

    fn session_config_inner(&self, resume_only: bool) -> SessionConfig {
        let pin = *self.shared.pin.lock().expect("pin");
        SessionConfig {
            identity: self.shared.identity.clone(),
            name: self.shared.cfg.name.clone(),
            platform: self.shared.cfg.platform.clone(),
            trust: self.shared.trust.clone(),
            lockout: self.shared.lockout.clone(),
            tcp_limiter: self.shared.tcp_limiter.clone(),
            pin: pin.map(|(p, _)| p),
            pin_created_ms: pin.map(|(_, t)| t).unwrap_or(0),
            clock: self.shared.clock.clone(),
            handshake_timeout: HANDSHAKE_TIMEOUT,
            hello_override: None,
            resume_only,
            screen_only: false,
            caps: None,
        }
    }

    pub fn persist_trust(&self) {
        let store = self.shared.trust.lock().expect("trust");
        if let Err(e) = save_trust(&self.shared.cfg.data_dir, &store) {
            warn!(%e, "trust persist failed");
        }
    }

    pub fn generate_pin(&self) -> Result<String, CoreError> {
        let pin = generate_pin(&OsRng)?;
        let now = self.shared.clock.unix_ms();
        *self.shared.pin.lock().expect("pin") = Some((pin, now));
        Ok(format_pin(&pin))
    }

    pub fn set_pin(&self, digits: [u8; 8]) {
        *self.shared.pin.lock().expect("pin") = Some((digits, self.shared.clock.unix_ms()));
    }

    pub fn pin_if_fresh(&self) -> Option<String> {
        let guard = self.shared.pin.lock().expect("pin");
        let (pin, created) = guard.as_ref()?;
        let age = self.shared.clock.unix_ms().saturating_sub(*created);
        if age > PIN_TTL.as_millis() as u64 {
            return None;
        }
        Some(format_pin(pin))
    }

    pub fn name(&self) -> &str {
        &self.shared.cfg.name
    }

    pub fn platform(&self) -> &str {
        &self.shared.cfg.platform
    }

    pub fn candidates(&self) -> Vec<CandidateViewDto> {
        let mut hubs = self.shared.hubs.lock().expect("hubs");
        let expired = hubs.notify.evict_expired(&*self.shared.clock);
        drop(hubs);
        for id in expired {
            let _ = self.shared.events.send(UiEvent::CandidateExpired { id });
        }
        self.shared
            .hubs
            .lock()
            .expect("hubs")
            .notify
            .views(&*self.shared.clock)
            .into_iter()
            .map(CandidateViewDto::from)
            .collect()
    }

    pub fn live_peers(&self) -> Vec<LivePeer> {
        self.shared
            .live
            .lock()
            .expect("live")
            .values()
            .cloned()
            .collect()
    }

    pub fn trusted(&self) -> Vec<tetherly_core::TrustedPeer> {
        self.shared.trust.lock().expect("trust").all()
    }

    pub fn emit(&self, ev: UiEvent) {
        let _ = self.shared.events.send(ev);
    }

    pub async fn copy_candidate(&self, id: &str) -> Result<(), CoreError> {
        let code = {
            let mut hubs = self.shared.hubs.lock().expect("hubs");
            hubs.notify.copy_otp(id, &*self.shared.clock)?
        };
        self.shared.clip.set_text(&code)?;
        {
            let mut hubs = self.shared.hubs.lock().expect("hubs");
            hubs.clip.note_local_write(&code);
        }
        let clip = self.shared.clip.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(CLIPBOARD_OTP_CLEAR_MS)).await;
            if let Ok(Some(cur)) = clip.get_text() {
                if cur == code {
                    let _ = clip.set_text("");
                }
            }
        });
        Ok(())
    }

    pub fn insert_candidate(&self, id: &str) -> Result<(), CoreError> {
        let code = {
            let mut hubs = self.shared.hubs.lock().expect("hubs");
            hubs.notify.copy_otp(id, &*self.shared.clock)?
        };
        insert_foreground(&code)?;
        Ok(())
    }

    pub fn insert_candidate_with(
        &self,
        id: &str,
        insertor: &dyn tetherly_core::Insertor,
    ) -> Result<(), CoreError> {
        let code = {
            let mut hubs = self.shared.hubs.lock().expect("hubs");
            hubs.notify.copy_otp(id, &*self.shared.clock)?
        };
        insertor.insert(&code)
    }

    pub fn dismiss_candidate(&self, id: &str) -> bool {
        self.shared.hubs.lock().expect("hubs").notify.dismiss(id)
    }

    /// Open the app that produced a candidate, on explicit user click. The url
    /// always comes from the allowlist; a url carried by the notification is
    /// never consulted (M4.2).
    pub fn open_candidate(
        &self,
        id: &str,
        opener: &dyn tetherly_core::Opener,
    ) -> Result<String, CoreError> {
        let app_id = {
            let hubs = self.shared.hubs.lock().expect("hubs");
            hubs.notify
                .get(id)
                .map(|c| c.app_id.clone())
                .ok_or(CoreError::UnknownCandidate)?
        };
        let url = self
            .shared
            .allowlist
            .lock()
            .expect("allowlist")
            .url_for(&app_id)
            .map(str::to_string)
            .ok_or_else(|| CoreError::OpenRefused(format!("{app_id} is not allowlisted")))?;
        opener.open(&url)?;
        Ok(url)
    }

    pub async fn send_notify(&self, peer: &DeviceId, push: NotifyPush) -> Result<(), CoreError> {
        self.send_to(peer, push.to_frame()?).await
    }

    pub async fn ingest_local_push(&self, source: DeviceId, push: NotifyPush) -> IngestOutcome {
        let outcome = {
            let mut hubs = self.shared.hubs.lock().expect("hubs");
            hubs.notify.ingest_push(source, push, &*self.shared.clock)
        };
        if let IngestOutcome::Candidate(v) = &outcome {
            self.emit(UiEvent::Candidate {
                candidate: CandidateViewDto::from(v.clone()),
            });
        }
        outcome
    }

    /// Reply to the notification behind a candidate, addressed to the phone it
    /// came from. Empty text is refused; the text is trimmed and capped by
    /// `NotifyReply::truncate` on the wire. Never executed, only delivered.
    pub async fn reply_to_candidate(&self, id: &str, text: &str) -> Result<(), CoreError> {
        let (source, app_id, uid) = {
            let hubs = self.shared.hubs.lock().expect("hubs");
            let c = hubs.notify.get(id).ok_or(CoreError::UnknownCandidate)?;
            (c.source.clone(), c.app_id.clone(), c.uid.clone())
        };
        if text.trim().is_empty() {
            return Err(CoreError::EmptyReply);
        }
        let reply = NotifyReply {
            uid,
            app_id,
            text: text.to_string(),
        }
        .truncate();
        self.send_to(&source, reply.to_frame()?).await
    }

    /// Take a queued inbound reply exactly once by its stable id. Returns the
    /// addressing view and the reply text; the caller that owns the phone path
    /// acts on it (fires the notification's reply action). Single-use so a
    /// quick-reply is never fired twice.
    pub fn take_reply(&self, id: &str) -> Option<(ReplyView, String)> {
        self.shared
            .hubs
            .lock()
            .expect("hubs")
            .notify
            .take_reply_id(id)
    }

    /// Queued `notify.reply` entries awaiting delivery to their phone.
    pub fn reply_views(&self) -> Vec<ReplyView> {
        self.shared.hubs.lock().expect("hubs").notify.reply_views()
    }

    /// Install the app allowlist that gates the ANCS "open" action (M4.2).
    pub fn set_open_allowlist(&self, list: OpenAllowlist) {
        *self.shared.allowlist.lock().expect("allowlist") = list;
    }

    /// Add one allowlist rule (validated). Used by the pairing wizard.
    pub fn add_open_rule(&self, rule: tetherly_core::OpenRule) -> Result<(), CoreError> {
        self.shared.allowlist.lock().expect("allowlist").push(rule)
    }

    pub fn open_allowlist(&self) -> OpenAllowlist {
        self.shared.allowlist.lock().expect("allowlist").clone()
    }

    pub fn ancs_state(&self) -> Option<AncsState> {
        self.shared
            .ancs
            .lock()
            .expect("ancs")
            .as_ref()
            .map(|i| i.state())
    }

    /// Bring the ANCS link up: Data Source first, then Notification Source.
    pub fn ancs_connect(&self) -> Result<Option<ConnectOutcome>, CoreError> {
        let mut guard = self.shared.ancs.lock().expect("ancs");
        match guard.as_mut() {
            Some(ing) => ing.connect().map(Some),
            None => Ok(None),
        }
    }

    /// Phone disconnected: schedule the backoff reconnect.
    pub fn ancs_disconnected(&self) -> Option<u64> {
        self.shared
            .ancs
            .lock()
            .expect("ancs")
            .as_mut()
            .map(|i| i.on_disconnected())
    }

    /// Notification Source value-changed. Never writes the Control Point.
    pub async fn ancs_notification_source(&self, value: &[u8]) -> Result<(), CoreError> {
        self.ancs_feed(|i| i.on_notification_source(value)).await
    }

    /// Data Source value-changed (fragments included).
    pub async fn ancs_data_source(&self, value: &[u8]) -> Result<(), CoreError> {
        self.ancs_feed(|i| i.on_data_source(value)).await
    }

    /// Drive the serial Control Point queue. Call from the BLE task.
    pub async fn ancs_tick(&self) -> Result<(), CoreError> {
        self.ancs_feed(|i| i.tick()).await
    }

    async fn ancs_feed<F>(&self, f: F) -> Result<(), CoreError>
    where
        F: FnOnce(&mut AncsIngress) -> Result<(), CoreError>,
    {
        let events = {
            let mut guard = self.shared.ancs.lock().expect("ancs");
            let Some(ing) = guard.as_mut() else {
                return Ok(());
            };
            f(ing)?;
            ing.drain_events()
        };
        for ev in events {
            self.apply_ancs_event(ev).await;
        }
        Ok(())
    }

    async fn apply_ancs_event(&self, ev: AncsEvent) {
        let source = self.shared.ancs_source.clone();
        match ev {
            AncsEvent::Added(n) | AncsEvent::Modified(n) => {
                let actions = {
                    let list = self.shared.allowlist.lock().expect("allowlist");
                    ancs_actions(&n.app_id, &list)
                };
                let push = NotifyPush {
                    uid: n.uid.to_string(),
                    app_id: n.app_id,
                    app_name: n.app_name,
                    title: n.title,
                    body: n.body,
                    ts: self.shared.clock.unix_ms(),
                    actions,
                };
                let _ = self.ingest_local_push(source, push).await;
            }
            AncsEvent::Removed { uid } => {
                let ids = {
                    let mut hubs = self.shared.hubs.lock().expect("hubs");
                    hubs.notify.dismiss_uid(&source, &uid.to_string())
                };
                for id in ids {
                    let _ = self.shared.events.send(UiEvent::CandidateExpired { id });
                }
            }
        }
    }

    pub async fn send_to(&self, peer: &DeviceId, frame: InnerFrame) -> Result<(), CoreError> {
        let tx = {
            let map = self.shared.sessions.lock().await;
            map.get(peer).cloned()
        };
        match tx {
            Some(tx) => tx
                .send(frame)
                .map_err(|_| CoreError::Json("peer gone".into())),
            None => Err(CoreError::Json("not connected".into())),
        }
    }

    pub fn clone_handle(&self) -> Self {
        Self {
            shared: self.shared.clone(),
        }
    }

    pub fn overlay_status(&self) -> OverlayStatus {
        self.shared.overlay.lock().expect("overlay").clone()
    }

    pub fn lan_only(&self) -> bool {
        !self.overlay_status().present
    }

    pub async fn run(&self) -> anyhow::Result<()> {
        self.spawn_listeners().await?;
        if self.shared.cfg.ui_port != 0 {
            crate::uihttp::spawn(self.clone_handle(), self.shared.cfg.ui_port);
        }
        info!(
            device_id = %self.shared.identity.device_id(),
            "node running"
        );
        std::future::pending::<()>().await;
        Ok(())
    }

    pub async fn spawn_listeners(&self) -> anyhow::Result<()> {
        let addrs = if self.shared.cfg.loopback_only {
            vec![SocketAddr::from((
                [127, 0, 0, 1],
                self.shared.cfg.control_port,
            ))]
        } else {
            control_bind_addrs(self.shared.cfg.control_port)
        };
        let listeners = bind_many(&addrs).await?;
        if let Some(first) = listeners.first() {
            self.shared
                .bound_control
                .store(first.local_addr()?.port(), Ordering::SeqCst);
        }

        macro_rules! data_listeners {
            ($port:expr, $bound:expr) => {{
                let port = $port;
                let addrs = if self.shared.cfg.loopback_only {
                    vec![SocketAddr::from(([127, 0, 0, 1], port))]
                } else {
                    control_bind_addrs(port)
                };
                let ls = bind_many_optional(&addrs).await;
                if let Some(a) = ls.first().and_then(|l| l.local_addr().ok()) {
                    $bound.store(a.port(), Ordering::SeqCst);
                }
                ls
            }};
        }
        let file_listeners = data_listeners!(self.shared.cfg.file_port, self.shared.bound_file);
        let input_listeners = data_listeners!(self.shared.cfg.input_port, self.shared.bound_input);
        let screen_listeners =
            data_listeners!(self.shared.cfg.screen_port, self.shared.bound_screen);

        if self.shared.cfg.advertise {
            self.advertise_mdns();
        }
        if self.shared.cfg.overlay.enabled {
            self.spawn_overlay();
        }
        if self.shared.cfg.reconnect {
            self.spawn_reconnect();
        }

        for l in listeners {
            node_accept_loop(self.clone_handle(), l, "control", |node, s, a| async move {
                node.accept_one(s, a).await
            });
        }
        for l in file_listeners {
            node_accept_loop(self.clone_handle(), l, "file", |node, s, a| async move {
                node.accept_file(s, a).await
            });
        }
        for l in input_listeners {
            node_accept_loop(self.clone_handle(), l, "input", |node, s, a| async move {
                node.accept_input(s, a).await
            });
        }
        for l in screen_listeners {
            node_accept_loop(self.clone_handle(), l, "screen", |node, s, a| async move {
                node.accept_screen(s, a).await
            });
        }
        Ok(())
    }

    pub fn shutdown(&self) {
        let _ = self.shared.shutdown.send(true);
        if let Ok(mut sessions) = self.shared.sessions.try_lock() {
            sessions.clear();
        }
        self.shared.live.lock().expect("live").clear();
    }

    pub fn forget(&self, device_id: &DeviceId) {
        let mut store = self.shared.trust.lock().expect("trust");
        let _ = store.forget(device_id);
        drop(store);
        self.persist_trust();
        // A forgotten device must not keep its live session, its capability
        // state, or (if it held input) the remote seat. Dropping the outbound
        // sender closes the channel, which ends the session task and its TCP.
        let had_session = match self.shared.sessions.try_lock() {
            Ok(mut sessions) => sessions.remove(device_id).is_some(),
            Err(_) => false,
        };
        let had_live = self
            .shared
            .live
            .lock()
            .expect("live")
            .remove(device_id)
            .is_some();
        if *self.shared.input_seat.lock().expect("seat") == Some(CursorSeat::Remote) {
            *self.shared.input_seat.lock().expect("seat") = Some(CursorSeat::Local);
        }
        if had_session || had_live {
            self.emit(UiEvent::PeerDown {
                device_id: device_id.to_string(),
            });
        }
    }

    fn already_live_on_ip(&self, addr: SocketAddr) -> bool {
        self.shared
            .live
            .lock()
            .expect("live")
            .values()
            .any(|p| p.addr.ip() == addr.ip() && p.addr.port() == addr.port())
    }

    fn spawn_overlay(&self) {
        if !self.shared.cfg.loopback_only {
            let overlay_addrs =
                sidecar::overlay_bind_addrs(&self.shared.cfg.overlay, self.shared.cfg.control_port);
            let overlay_file =
                sidecar::overlay_bind_addrs(&self.shared.cfg.overlay, self.shared.cfg.file_port);
            let overlay_input =
                sidecar::overlay_bind_addrs(&self.shared.cfg.overlay, self.shared.cfg.input_port);
            let overlay_screen =
                sidecar::overlay_bind_addrs(&self.shared.cfg.overlay, self.shared.cfg.screen_port);
            let node = self.clone_handle();
            tokio::spawn(async move {
                for l in bind_overlay(&overlay_addrs).await {
                    node_accept_loop(
                        node.clone_handle(),
                        l,
                        "overlay control",
                        |n, s, a| async move { n.accept_one(s, a).await },
                    );
                }
                for l in bind_overlay(&overlay_file).await {
                    node_accept_loop(
                        node.clone_handle(),
                        l,
                        "overlay file",
                        |n, s, a| async move { n.accept_file(s, a).await },
                    );
                }
                for l in bind_overlay(&overlay_input).await {
                    node_accept_loop(
                        node.clone_handle(),
                        l,
                        "overlay input",
                        |n, s, a| async move { n.accept_input(s, a).await },
                    );
                }
                for l in bind_overlay(&overlay_screen).await {
                    node_accept_loop(
                        node.clone_handle(),
                        l,
                        "overlay screen",
                        |n, s, a| async move { n.accept_screen(s, a).await },
                    );
                }
            });
        }
        let node = self.clone_handle();
        tokio::spawn(async move {
            node.overlay_scan_loop().await;
        });
    }

    async fn overlay_scan_loop(&self) {
        let mut stop = self.shared.shutdown.subscribe();
        let mut last_present = false;
        loop {
            if *stop.borrow() {
                break;
            }
            let st = sidecar::discover(&self.shared.cfg.overlay).await;
            let present = st.present;
            {
                *self.shared.overlay.lock().expect("overlay") = st.clone();
            }
            if present != last_present {
                last_present = present;
                self.emit(UiEvent::Overlay {
                    present,
                    lan_only: !present,
                });
                if !present {
                    info!("overlay down; LAN only");
                } else {
                    info!(peers = st.peers.len(), "overlay peers");
                }
            }
            let live = self.live_peers();
            let live_ips: Vec<IpAddr> = live.iter().map(|p| p.addr.ip()).collect();
            let port = self.control_port();
            let mut targets: Vec<SocketAddr> = self.shared.cfg.overlay.extra_peers.clone();
            for ip in &st.peers {
                if live_ips.contains(&IpAddr::V4(*ip)) {
                    continue;
                }
                targets.push(SocketAddr::from((*ip, port)));
            }
            for addr in targets {
                if self.already_live_on_ip(addr) {
                    continue;
                }
                if let Err(e) = self.dial(addr).await {
                    debug!(%e, %addr, "overlay dial");
                }
            }
            tokio::select! {
                _ = stop.changed() => {
                    if *stop.borrow() {
                        break;
                    }
                }
                _ = tokio::time::sleep(Duration::from_millis(OVERLAY_PROBE_MS.max(5_000))) => {}
            }
        }
    }

    fn advertise_mdns(&self) {
        let id = self.shared.identity.device_id().clone();
        let fp = DeviceId::fingerprint12(self.shared.identity.id_pk());
        let name = self.shared.cfg.name.clone();
        let port = self.shared.cfg.control_port;
        let addrs = lan_ips();
        let mut stop = self.shared.shutdown.subscribe();
        tokio::task::spawn_blocking(move || {
            let d = match tetherly_net::mdns::daemon() {
                Ok(d) => d,
                Err(e) => {
                    warn!(%e, "mdns daemon");
                    return;
                }
            };
            if let Err(e) =
                tetherly_net::mdns::advertise(&d, &name, port, &id, &fp, "notify,clip,file", &addrs)
            {
                warn!(%e, "mdns advertise");
                return;
            }
            // Hold the registration for the node's lifetime, then drop the
            // daemon so the service is withdrawn instead of lingering.
            let handle = match tokio::runtime::Handle::try_current() {
                Ok(h) => h,
                Err(_) => {
                    std::thread::park();
                    return;
                }
            };
            handle.block_on(async {
                loop {
                    if *stop.borrow() {
                        break;
                    }
                    if stop.changed().await.is_err() {
                        break;
                    }
                }
            });
        });
    }

    fn spawn_reconnect(&self) {
        let node = self.clone_handle();
        tokio::spawn(async move {
            let mut attempt = 0u32;
            let mut stop = node.shared.shutdown.subscribe();
            loop {
                if *stop.borrow() {
                    break;
                }
                let trusted = node.trusted();
                let live = node.live_peers();
                let routes = crate::store::load_routes(&node.shared.cfg.data_dir);
                for peer in trusted {
                    if peer.revoked {
                        continue;
                    }
                    if live.iter().any(|l| l.device_id == peer.device_id) {
                        continue;
                    }
                    if let Some(addr) = route_addr(&routes, peer.device_id.as_str()) {
                        match node.dial(addr).await {
                            Ok(()) => attempt = 0,
                            Err(e) => {
                                debug!(%e, "reconnect");
                                attempt = attempt.saturating_add(1);
                            }
                        }
                    }
                }
                tokio::select! {
                    _ = stop.changed() => {
                        if *stop.borrow() {
                            break;
                        }
                    }
                    _ = tokio::time::sleep(backoff_delay(attempt)) => {}
                }
            }
        });
    }

    pub async fn dial(&self, addr: SocketAddr) -> Result<(), tetherly_net::NetError> {
        if self.already_live_on_ip(addr) {
            return Ok(());
        }
        let stream = TcpStream::connect(addr).await?;
        let cfg = self.session_config();
        let sess = dial_session(stream, addr, &cfg).await?;
        self.persist_trust();
        let _ = remember_route(
            &self.shared.cfg.data_dir,
            sess.peer_id.as_str(),
            &addr.to_string(),
        );
        self.attach(sess).await;
        Ok(())
    }

    async fn accept_one(
        &self,
        stream: TcpStream,
        addr: SocketAddr,
    ) -> Result<(), tetherly_net::NetError> {
        let cfg = self.session_config();
        let sess = accept_session(stream, addr, &cfg).await?;
        self.persist_trust();
        let _ = remember_route(
            &self.shared.cfg.data_dir,
            sess.peer_id.as_str(),
            &addr.to_string(),
        );
        self.attach(sess).await;
        Ok(())
    }

    async fn attach(&self, mut sess: ActiveSession) {
        let peer_id = sess.peer_id.clone();
        let name = sess.peer_hello.name.clone();
        let platform = sess.peer_hello.platform.clone();
        let addr = sess.peer_addr;
        let incoming = path_kind(addr, &self.shared.cfg.overlay.cidrs);
        {
            let live = self.shared.live.lock().expect("live");
            if let Some(cur) = live.get(&peer_id) {
                if cur.path == PathKind::Lan && incoming == PathKind::Overlay {
                    debug!(peer = %peer_id, %addr, "keep LAN; drop overlay attach");
                    return;
                }
            }
        }
        {
            let mut live = self.shared.live.lock().expect("live");
            live.insert(
                peer_id.clone(),
                LivePeer {
                    device_id: peer_id.clone(),
                    name: name.clone(),
                    platform: platform.clone(),
                    addr,
                    handshake_hash: *sess.handshake_hash(),
                    file_port: None,
                    input_port: None,
                    screen_port: None,
                    path: incoming,
                },
            );
        }
        self.emit(peer_up(&peer_id, &name, &platform));
        match incoming {
            PathKind::Lan => info!(peer = %peer_id, %addr, "path lan"),
            PathKind::Overlay => info!(peer = %peer_id, %addr, "path overlay"),
        }
        let (tx, mut rx) = mpsc::unbounded_channel::<InnerFrame>();
        {
            self.shared
                .sessions
                .lock()
                .await
                .insert(peer_id.clone(), tx.clone());
        }
        let file_port = self.file_port();
        let input_port = self.input_port();
        let screen_port = self.screen_port();
        let caps = CapsUpdate {
            caps: vec![
                "notify".into(),
                "clip".into(),
                "file".into(),
                "input".into(),
                "screen".into(),
                format!("fileport={file_port}"),
                format!("inputport={input_port}"),
                format!("screenport={screen_port}"),
            ],
        };
        if let Ok(frame) = caps.to_frame() {
            let _ = tx.send(frame);
        }
        let node = self.clone_handle();
        let mut stop = self.shared.shutdown.subscribe();
        let sess_addr = addr;
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = stop.changed() => {
                        if *stop.borrow() {
                            break;
                        }
                    }
                    outbound = rx.recv() => {
                        match outbound {
                            Some(frame) => {
                                if let Err(e) = sess.send_inner(frame).await {
                                    warn!(%e, "send");
                                    break;
                                }
                            }
                            None => break,
                        }
                    }
                    inbound = sess.recv_event() => {
                        match inbound {
                            Ok(ev) => {
                                if let Err(e) = node.handle_event(&peer_id, ev, &mut sess).await {
                                    warn!(%e, "event");
                                    break;
                                }
                            }
                            Err(e) => {
                                debug!(%e, "recv end");
                                break;
                            }
                        }
                    }
                }
            }
            let still_mine = node
                .shared
                .live
                .lock()
                .expect("live")
                .get(&peer_id)
                .map(|p| p.addr)
                == Some(sess_addr);
            if still_mine {
                node.shared.sessions.lock().await.remove(&peer_id);
                node.shared.live.lock().expect("live").remove(&peer_id);
                *node.shared.input_seat.lock().expect("seat") = None;
                node.emit(UiEvent::PeerDown {
                    device_id: peer_id.to_string(),
                });
            }
        });
    }

    async fn handle_event(
        &self,
        peer: &DeviceId,
        ev: SessionEvent,
        sess: &mut ActiveSession,
    ) -> Result<(), tetherly_net::NetError> {
        match ev {
            SessionEvent::Ping(ts) => {
                sess.send_inner(pong(ts)).await?;
            }
            SessionEvent::Pong(_) => {}
            SessionEvent::Notify(push) => {
                let _ = self.ingest_local_push(peer.clone(), push).await;
            }
            SessionEvent::Dismiss(d) => {
                let mut hubs = self.shared.hubs.lock().expect("hubs");
                hubs.notify.dismiss_wire(peer, &d);
            }
            SessionEvent::Reply(reply) => {
                let text_len = reply.text.len();
                let accepted = {
                    let mut hubs = self.shared.hubs.lock().expect("hubs");
                    hubs.notify.ingest_reply(peer, reply.clone())
                };
                if accepted {
                    // Never log the text; only its length and identity.
                    info!(
                        source = %peer,
                        app_id = %reply.app_id,
                        uid = %reply.uid,
                        text_len,
                        "notify reply queued"
                    );
                    self.emit(UiEvent::ReplyReceived {
                        reply: crate::events::ReplyViewDto::from(ReplyView {
                            id: tetherly_core::notify::reply_id(peer, &reply.app_id, &reply.uid),
                            source: peer.clone(),
                            app_id: reply.app_id.clone(),
                            uid: reply.uid.clone(),
                        }),
                    });
                }
            }
            SessionEvent::Clip(clip) => {
                self.apply_clip(peer, clip)?;
            }
            SessionEvent::FileOffer(offer) => {
                let total: u64 = offer.files.iter().map(|f| f.size).sum();
                let tid = offer.transfer_id.clone();
                let hint = {
                    let hubs = self.shared.hubs.lock().expect("hubs");
                    hubs.resume.claim(peer, &tid, total)
                };
                if let Some(off) = hint {
                    info!(transfer = %tid, offset = off, "resuming interrupted transfer");
                }
                let t = {
                    let mut hubs = self.shared.hubs.lock().expect("hubs");
                    hubs.files.offer_resumable(
                        peer.clone(),
                        offer,
                        self.shared.clock.unix_ms(),
                        hint,
                    )?
                };
                self.emit(file_offered(&t));
            }
            SessionEvent::FileAccept(d) => {
                debug!(transfer = %d.transfer_id, resume = ?d.resume_from, "peer accepted");
                self.on_peer_accepted_file(&d).await;
            }
            SessionEvent::FileReject(d) => {
                info!(transfer = %d.transfer_id, "peer rejected file");
                // Drop the buffered outbound bytes: without this the rejected
                // file's whole body stays in `pending_out` for the node's life.
                self.shared
                    .pending_out
                    .lock()
                    .expect("pending")
                    .remove(&d.transfer_id);
            }
            SessionEvent::FileDone(done) => {
                let mut hubs = self.shared.hubs.lock().expect("hubs");
                match hubs.files.mark_done(&done) {
                    Ok(_) => self.emit(UiEvent::FileDone {
                        transfer_id: done.transfer_id,
                    }),
                    Err(e) => warn!(%e, "file done"),
                }
            }
            SessionEvent::Caps(update) => {
                let file_port = parse_file_port(&update.caps);
                let input_port = parse_input_port(&update.caps);
                let screen_port = parse_screen_port(&update.caps);
                if file_port.is_some() || input_port.is_some() || screen_port.is_some() {
                    if let Some(live) = self.shared.live.lock().expect("live").get_mut(peer) {
                        if let Some(port) = file_port {
                            live.file_port = Some(port);
                        }
                        if let Some(port) = input_port {
                            live.input_port = Some(port);
                        }
                        if let Some(port) = screen_port {
                            live.screen_port = Some(port);
                        }
                    }
                }
            }
            SessionEvent::Unknown(_) => {}
        }
        Ok(())
    }

    fn apply_clip(&self, peer: &DeviceId, clip: ClipSet) -> Result<(), tetherly_net::NetError> {
        let now = self.shared.clock.unix_ms();
        let apply = {
            let mut hubs = self.shared.hubs.lock().expect("hubs");
            hubs.clip.apply_remote(peer, clip, now)?
        };
        match apply {
            ClipApply::Apply(text) => {
                self.shared.clip.set_text(&text)?;
            }
            ClipApply::Echo | ClipApply::RateLimited => {}
        }
        Ok(())
    }

    pub async fn accept_file(
        &self,
        mut stream: TcpStream,
        _addr: SocketAddr,
    ) -> Result<(), tetherly_net::NetError> {
        // Header is unauthenticated (id/total/start). It only names the
        // transfer; the token (HKDF over the Noise handshake + transfer_id)
        // gates every chunk, so a forged id simply fails to decrypt.
        let (transfer_id, total, start) = read_file_header(&mut stream).await?;
        let (accepted, source, size, resume_hint, name, expected) = {
            let hubs = self.shared.hubs.lock().expect("hubs");
            let t = hubs
                .files
                .get(&transfer_id)
                .ok_or(tetherly_net::NetError::File("unknown".into()))?;
            (
                t.status == tetherly_core::TransferStatus::Accepted,
                t.source.clone(),
                t.files.iter().map(|f| f.size).sum::<u64>(),
                t.resume_from.unwrap_or(0),
                t.files
                    .first()
                    .map(|f| f.name.clone())
                    .unwrap_or_else(|| "file.bin".into()),
                t.files
                    .first()
                    .map(|f| f.sha256.clone())
                    .unwrap_or_default(),
            )
        };
        if !accepted {
            return Err(tetherly_net::NetError::File("not accepted".into()));
        }
        if total != size || start != resume_hint {
            return Err(tetherly_net::NetError::Handshake);
        }
        // The offer's size is authoritative; also refuse an absurd declared size.
        if total > size.max(32 * 1024 * 1024) {
            return Err(tetherly_net::NetError::TooLarge);
        }
        let token = self
            .file_token_for(&source, &transfer_id)
            .ok_or(tetherly_net::NetError::File("no session token".into()))?;
        let dest_dir = self.shared.cfg.data_dir.join("inbox");
        std::fs::create_dir_all(&dest_dir)?;
        let part = dest_dir.join(format!(".{}.part", transfer_id));
        // Preload the resume prefix from the durable partial, refusing a resume
        // the local file cannot actually honor.
        let mut out = if start > 0 {
            let bytes = std::fs::read(&part).unwrap_or_default();
            let aligned =
                bytes.len() as u64 - (bytes.len() as u64 % tetherly_core::FILE_CHUNK as u64);
            if aligned < start {
                return Err(tetherly_net::NetError::Handshake);
            }
            let mut v = bytes;
            v.truncate(start as usize);
            v
        } else {
            Vec::new()
        };
        let mut written = out.len() as u64;
        // Make the on-disk partial exactly `start` bytes so appends land at the
        // right offset (a stale/un-aligned tail from a previous attempt is
        // dropped; the sender re-sends that whole chunk).
        {
            let f = std::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(false)
                .open(&part)?;
            f.set_len(written)?;
            f.sync_all()?;
        }
        let res = recv_chunks_after_header(
            &mut stream,
            &token,
            &transfer_id,
            total,
            start,
            u64::MAX,
            &mut out,
            |buf, new_len| {
                let mut f = std::fs::OpenOptions::new().append(true).open(&part)?;
                use std::io::Write;
                f.write_all(&buf[written as usize..new_len as usize])?;
                f.sync_all()?;
                written = new_len;
                Ok(())
            },
        )
        .await;
        if let Err(e) = res {
            // Persist a chunk-aligned resume marker (in memory + sidecar) so the
            // next offer resumes instead of restarting.
            self.note_resume(&source, &transfer_id, total, written);
            return Err(e);
        }
        if tetherly_core::hex_sha256(&out) != expected {
            let _ = std::fs::remove_file(&part);
            self.clear_resume(&transfer_id);
            return Err(tetherly_net::NetError::File("sha256".into()));
        }
        let path = dest_dir.join(&name);
        crate::store::atomic_write(&path, &out)?;
        let _ = std::fs::remove_file(&part);
        self.clear_resume(&transfer_id);
        let done = FileDone {
            transfer_id: transfer_id.clone(),
            sha256: tetherly_core::hex_sha256(&out),
        };
        {
            let mut hubs = self.shared.hubs.lock().expect("hubs");
            let _ = hubs.files.mark_done(&done);
        }
        let _ = self.send_to(&source, done.to_frame()?).await;
        self.emit(UiEvent::FileDone { transfer_id });
        Ok(())
    }

    /// Record a chunk-aligned resume marker in the hub and on disk (sidecar).
    fn note_resume(&self, source: &DeviceId, transfer_id: &str, total: u64, offset: u64) {
        let st = tetherly_core::ResumeState {
            transfer_id: transfer_id.to_string(),
            source: source.clone(),
            total,
            offset,
        };
        {
            let mut hubs = self.shared.hubs.lock().expect("hubs");
            hubs.resume.put(st.clone());
        }
        // Persist the chunk-aligned offset the table actually settled on.
        let aligned = self
            .shared
            .hubs
            .lock()
            .expect("hubs")
            .resume
            .get(transfer_id)
            .map(|s| s.offset)
            .unwrap_or(0);
        if aligned == 0 {
            return;
        }
        let side = self.resume_sidecar(transfer_id);
        let dto = ResumeSidecar {
            device_id: source.as_str().to_string(),
            total,
            offset: aligned,
        };
        if let Ok(bytes) = serde_json::to_vec(&dto) {
            let _ = crate::store::atomic_write(&side, &bytes);
        }
    }

    fn clear_resume(&self, transfer_id: &str) {
        {
            let mut hubs = self.shared.hubs.lock().expect("hubs");
            hubs.resume.remove(transfer_id);
        }
        let _ = std::fs::remove_file(self.resume_sidecar(transfer_id));
    }

    fn resume_sidecar(&self, transfer_id: &str) -> std::path::PathBuf {
        self.shared
            .cfg
            .data_dir
            .join("inbox")
            .join(format!(".{}.resume.json", transfer_id))
    }

    pub async fn user_accept_file(&self, transfer_id: &str) -> Result<(), CoreError> {
        let decision = {
            let mut hubs = self.shared.hubs.lock().expect("hubs");
            hubs.files
                .accept(transfer_id, self.shared.clock.unix_ms())?
        };
        let source = {
            let hubs = self.shared.hubs.lock().expect("hubs");
            hubs.files
                .get(transfer_id)
                .map(|t| t.source.clone())
                .ok_or(CoreError::UnknownTransfer)?
        };
        // Token is derived from the live session handshake hash.
        if let Some(sess_tx) = self.shared.sessions.lock().await.get(&source).cloned() {
            let _ = sess_tx.send(decision.accept_frame()?);
        }
        Ok(())
    }

    pub fn file_token_for(&self, peer: &DeviceId, transfer_id: &str) -> Option<[u8; 32]> {
        let live = self.shared.live.lock().expect("live");
        live.get(peer)
            .map(|p| tetherly_crypto::file_token(&p.handshake_hash, transfer_id.as_bytes()))
    }

    pub fn clip(&self) -> Arc<dyn Clip> {
        self.shared.clip.clone()
    }

    pub async fn user_reject_file(&self, transfer_id: &str) -> Result<(), CoreError> {
        let decision = {
            let mut hubs = self.shared.hubs.lock().expect("hubs");
            hubs.files.reject(transfer_id)?
        };
        let source = {
            let hubs = self.shared.hubs.lock().expect("hubs");
            hubs.files
                .get(transfer_id)
                .map(|t| t.source.clone())
                .ok_or(CoreError::UnknownTransfer)?
        };
        self.send_to(&source, decision.reject_frame()?).await
    }

    pub async fn send_clipboard(&self) -> Result<(), CoreError> {
        let text = self
            .shared
            .clip
            .get_text()?
            .ok_or_else(|| CoreError::Json("empty clipboard".into()))?;
        let set = {
            let mut hubs = self.shared.hubs.lock().expect("hubs");
            hubs.clip.prepare_local(&text)?
        };
        let Some(set) = set else {
            return Ok(());
        };
        let frame = set.to_frame()?;
        let peers: Vec<DeviceId> = self
            .live_peers()
            .into_iter()
            .filter(|p| is_desktop_platform(&p.platform))
            .map(|p| p.device_id)
            .collect();
        for p in peers {
            let _ = self.send_to(&p, frame.clone()).await;
        }
        Ok(())
    }

    pub async fn offer_and_send_bytes(
        &self,
        peer: &DeviceId,
        name: &str,
        data: &[u8],
        _wait_accept: bool,
    ) -> Result<String, CoreError> {
        // Content-addressed: the same bytes re-offered after an interruption
        // carry the same id, so the receiver's persisted partial can be matched
        // and resumed. Different content -> different id -> fresh transfer.
        let digest = tetherly_core::hex_sha256(data);
        let transfer_id = format!("tr_{}", &digest[..16]);
        let offer = FileOffer {
            transfer_id: transfer_id.clone(),
            files: vec![tetherly_core::FileMeta {
                name: name.into(),
                size: data.len() as u64,
                sha256: digest,
            }],
        };
        self.shared.pending_out.lock().expect("pending").insert(
            transfer_id.clone(),
            OutboundFile {
                peer: peer.clone(),
                data: data.to_vec(),
            },
        );
        self.send_to(peer, offer.to_frame()?).await?;
        let _ = _wait_accept;
        Ok(transfer_id)
    }

    async fn on_peer_accepted_file(&self, decision: &tetherly_core::FileDecision) {
        let transfer_id = decision.transfer_id.as_str();
        let outbound = self
            .shared
            .pending_out
            .lock()
            .expect("pending")
            .remove(transfer_id);
        let Some(outbound) = outbound else {
            return;
        };
        let (addr, file_port, token) = {
            let live = self.shared.live.lock().expect("live");
            let Some(peer) = live.get(&outbound.peer) else {
                warn!(transfer = %transfer_id, "accepted file but peer gone");
                return;
            };
            let token = tetherly_crypto::file_token(&peer.handshake_hash, transfer_id.as_bytes());
            (peer.addr, peer.file_port.unwrap_or(self.file_port()), token)
        };
        let start = decision.resume_from.unwrap_or(0);
        if start > outbound.data.len() as u64 {
            warn!(transfer = %transfer_id, "resume offset beyond local data; aborting");
            return;
        }
        if let Err(e) = self
            .send_file_data(
                addr,
                file_port,
                &token,
                transfer_id,
                outbound.data.len() as u64,
                start,
                &outbound.data,
            )
            .await
        {
            warn!(%e, transfer = %transfer_id, "file data send failed");
        }
    }

    /// Send file bytes from `start` (the resume offset) to the end of `data`,
    /// where `data` holds the whole file and `total` is its length on the wire.
    #[allow(clippy::too_many_arguments)]
    pub async fn send_file_data(
        &self,
        peer_addr: SocketAddr,
        file_port: u16,
        token: &[u8; 32],
        transfer_id: &str,
        total: u64,
        start: u64,
        data: &[u8],
    ) -> Result<(), tetherly_net::NetError> {
        let dest = SocketAddr::new(peer_addr.ip(), file_port);
        let mut stream = TcpStream::connect(dest).await?;
        tetherly_net::filechan::send_chunks_from(
            &mut stream,
            token,
            transfer_id,
            total,
            start,
            &data[start as usize..],
        )
        .await
    }

    pub fn control_port(&self) -> u16 {
        self.shared.bound_control.load(Ordering::SeqCst)
    }

    pub fn file_port(&self) -> u16 {
        self.shared.bound_file.load(Ordering::SeqCst)
    }

    pub fn input_port(&self) -> u16 {
        self.shared.bound_input.load(Ordering::SeqCst)
    }

    pub fn control_addr(&self) -> SocketAddr {
        SocketAddr::from(([127, 0, 0, 1], self.control_port()))
    }

    pub fn file_addr(&self) -> SocketAddr {
        SocketAddr::from(([127, 0, 0, 1], self.file_port()))
    }

    pub fn input_addr(&self) -> SocketAddr {
        SocketAddr::from(([127, 0, 0, 1], self.input_port()))
    }

    pub fn screen_port(&self) -> u16 {
        self.shared.bound_screen.load(Ordering::SeqCst)
    }

    pub fn screen_addr(&self) -> SocketAddr {
        SocketAddr::from(([127, 0, 0, 1], self.screen_port()))
    }

    pub fn input_seat(&self) -> Option<CursorSeat> {
        *self.shared.input_seat.lock().expect("seat")
    }

    /// "remote" while the logical cursor is on a peer, else "local". Encoding
    /// lives here so the UI never has to mirror the enum.
    pub fn input_seat_str(&self) -> &'static str {
        match self.input_seat() {
            Some(CursorSeat::Remote) => "remote",
            _ => "local",
        }
    }

    pub fn input_sink_snapshot(&self) -> MemorySink {
        self.shared.input_sink.lock().expect("sink").clone()
    }

    /// Open the resume-only input channel. Pairing on 45719 is refused.
    pub async fn open_input(&self, peer: &DeviceId) -> Result<(), tetherly_net::NetError> {
        let addr = {
            let live = self.shared.live.lock().expect("live");
            let p = live
                .get(peer)
                .ok_or(tetherly_net::NetError::InputRequiresTrust)?;
            let port = p.input_port.ok_or(tetherly_net::NetError::Handshake)?;
            SocketAddr::new(p.addr.ip(), port)
        };
        let stream = TcpStream::connect(addr).await?;
        let mut cfg = self.session_config_inner(true);
        cfg.pin = None;
        let sess = dial_session(stream, addr, &cfg).await?;
        self.attach_input_server(sess).await;
        Ok(())
    }

    /// True while an input control channel to a peer is attached. The UI uses
    /// this to dial the channel on demand instead of tracking sessions.
    pub async fn input_channel_open(&self) -> bool {
        self.shared.input_out.lock().await.is_some()
    }

    /// Controller: stop forwarding local input to the remote seat and release
    /// the channel. The capture hooks stay armed (they are cheap while local).
    pub async fn input_stop(&self) -> Result<(), CoreError> {
        *self.shared.input_out.lock().await = None;
        self.shared
            .input_engine
            .lock()
            .expect("engine")
            .on_peer_gone();
        *self.shared.input_seat.lock().expect("seat") = Some(CursorSeat::Local);
        Ok(())
    }

    /// Open the resume-only screen channel 45720 as the **visitor**: this node
    /// dials a trusted peer to view its screen. Pairing on 45720 is refused.
    /// Frames only start flowing after the host calls `allow()` locally and the
    /// host responds to our `Start`. Real capture is Manual-required.
    pub async fn open_screen(&self, peer: &DeviceId) -> Result<(), tetherly_net::NetError> {
        let addr = {
            let live = self.shared.live.lock().expect("live");
            let p = live
                .get(peer)
                .ok_or(tetherly_net::NetError::ScreenRequiresTrust)?;
            let port = p.screen_port.ok_or(tetherly_net::NetError::Handshake)?;
            SocketAddr::new(p.addr.ip(), port)
        };
        let stream = TcpStream::connect(addr).await?;
        let mut cfg = self.session_config_inner(true);
        cfg.pin = None;
        cfg.screen_only = true;
        let sess = dial_session(stream, addr, &cfg).await?;
        self.attach_screen_client(sess).await;
        Ok(())
    }

    /// Start the controller-side capture: low-level mouse hook (edge
    /// detection) and a keyboard hook armed only while the seat is remote.
    /// Events feed the same edge state machine `drive_input` uses, so CI and
    /// production share one path. Non-Windows: honest error, no hooks.
    pub fn spawn_input_capture(&self) -> Result<(), CoreError> {
        let (tx, rx) = std::sync::mpsc::channel::<crate::platform::RawInput>();
        let control = crate::platform::start_input_hooks(tx)?;
        let (w, h) = crate::platform::screen_size();
        self.shared
            .input_engine
            .lock()
            .expect("engine")
            .resize(w.max(1), h.max(1));
        let handle = tokio::runtime::Handle::try_current()
            .map_err(|_| CoreError::Json("input capture needs a tokio runtime".into()))?;
        let node = self.clone_handle();
        std::thread::spawn(move || {
            let mut keyboard_armed = false;
            for ev in rx {
                // A failed send means the remote input channel is not open (yet)
                // or just closed. That is transient, not fatal: keep the hook
                // alive so a later `open_input` starts delivering again, rather
                // than wedging input sharing until process restart. Events are
                // paced by real user input, so retrying cannot busy-spin.
                match ev {
                    crate::platform::RawInput::Move { x, y } => {
                        let _ = handle.block_on(node.drive_input(x, y, w, h));
                    }
                    crate::platform::RawInput::Button { mask, down } => {
                        let _ = handle.block_on(node.drive_input_button(mask, down));
                    }
                    crate::platform::RawInput::Wheel { dx, dy } => {
                        let _ = handle.block_on(node.drive_input_wheel(dx, dy));
                    }
                    crate::platform::RawInput::Key { code, down } => {
                        let _ = handle.block_on(node.drive_input_key(code, down));
                    }
                }
                // The keyboard hook records real keystrokes: arm it only for
                // the remote seat, disarm the moment the cursor comes home.
                let remote = node.input_engine_seat() == CursorSeat::Remote;
                if remote != keyboard_armed && control.set_keyboard(remote).is_ok() {
                    keyboard_armed = remote;
                }
            }
        });
        Ok(())
    }

    /// Push local cursor position toward the edge state machine. Crossing the
    /// right edge emits Enter + Move. Seq is monotonic for the life of the
    /// node. No JSON, and never onto 45717. OS hooks stay Manual-required.
    pub async fn drive_input(
        &self,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    ) -> Result<Option<tetherly_core::ScreenEdge>, CoreError> {
        let (edge, events) = {
            let mut server = self.shared.input_engine.lock().expect("engine");
            server.resize(width, height);
            let edge = server.local_move(x, y);
            (edge, server.drain())
        };
        self.send_input_events(events).await?;
        if edge.is_some() {
            *self.shared.input_seat.lock().expect("seat") = Some(CursorSeat::Remote);
        }
        Ok(edge)
    }

    pub async fn drive_input_key(&self, code: u16, down: bool) -> Result<(), CoreError> {
        let events = {
            let mut server = self.shared.input_engine.lock().expect("engine");
            if server.seat() != CursorSeat::Remote {
                return Ok(());
            }
            let seq = server.next_seq();
            server.note_seq(seq);
            vec![InputEvent::key(seq, code, down, 0)]
        };
        self.send_input_events(events).await
    }

    /// Push a physical mouse-button observation to the remote seat. No-op
    /// while the seat is local (the engine gates the same way).
    pub async fn drive_input_button(&self, mask: u8, down: bool) -> Result<(), CoreError> {
        let events = {
            let mut server = self.shared.input_engine.lock().expect("engine");
            if server.seat() != CursorSeat::Remote {
                return Ok(());
            }
            let seq = server.next_seq();
            server.note_seq(seq);
            vec![InputEvent::button(seq, mask, down)]
        };
        self.send_input_events(events).await
    }

    /// Push a physical wheel observation to the remote seat.
    pub async fn drive_input_wheel(&self, dx: i16, dy: i16) -> Result<(), CoreError> {
        let events = {
            let mut server = self.shared.input_engine.lock().expect("engine");
            if server.seat() != CursorSeat::Remote {
                return Ok(());
            }
            let seq = server.next_seq();
            server.note_seq(seq);
            vec![InputEvent::wheel(seq, dx, dy)]
        };
        self.send_input_events(events).await
    }

    pub fn input_engine_seat(&self) -> CursorSeat {
        self.shared.input_engine.lock().expect("engine").seat()
    }

    async fn send_input_events(&self, events: Vec<InputEvent>) -> Result<(), CoreError> {
        let tx = self
            .shared
            .input_out
            .lock()
            .await
            .clone()
            .ok_or_else(|| CoreError::Json("input channel closed".into()))?;
        for ev in events {
            tx.send(ev.to_frame()?)
                .map_err(|_| CoreError::Json("input peer gone".into()))?;
        }
        Ok(())
    }

    async fn accept_input(
        &self,
        stream: TcpStream,
        addr: SocketAddr,
    ) -> Result<(), tetherly_net::NetError> {
        let mut cfg = self.session_config_inner(true);
        cfg.pin = None;
        let sess = accept_session(stream, addr, &cfg).await?;
        self.attach_input_client(sess).await;
        Ok(())
    }

    /// Accept a 45720 screen connection. The peer wants to view this machine's
    /// screen. We refuse to stream until the local user clicks `allow()`.
    async fn accept_screen(
        &self,
        stream: TcpStream,
        addr: SocketAddr,
    ) -> Result<(), tetherly_net::NetError> {
        let mut cfg = self.session_config_inner(true);
        cfg.pin = None;
        cfg.screen_only = true;
        let sess = accept_session(stream, addr, &cfg).await?;
        self.attach_screen_server(sess).await;
        Ok(())
    }

    async fn attach_input_server(&self, mut sess: ActiveSession) {
        let (tx, mut rx) = mpsc::unbounded_channel::<InnerFrame>();
        *self.shared.input_out.lock().await = Some(tx);
        let mut stop = self.shared.shutdown.subscribe();
        let node = self.clone_handle();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = stop.changed() => {
                        if *stop.borrow() {
                            break;
                        }
                    }
                    outbound = rx.recv() => {
                        match outbound {
                            Some(frame) => {
                                if sess.send_inner(frame).await.is_err() {
                                    tracing::warn!("input server: peer send failed; closing");
                                    break;
                                }
                            }
                            None => break,
                        }
                    }
                    inbound = sess.recv_inner() => {
                        match inbound {
                            Ok(frame) => {
                                if let Ok(ev) = InputEvent::from_frame(&frame) {
                                    if matches!(ev.kind, tetherly_core::InputKind::Leave) {
                                        node.shared.input_engine.lock().expect("engine").on_remote_leave();
                                        *node.shared.input_seat.lock().expect("seat") = Some(CursorSeat::Local);
                                    }
                                }
                            }
                            Err(e) => {
                                tracing::warn!(%e, "input server: session recv ended; closing");
                                break;
                            }
                        }
                    }
                }
            }
            *node.shared.input_out.lock().await = None;
            node.shared
                .input_engine
                .lock()
                .expect("engine")
                .on_peer_gone();
            *node.shared.input_seat.lock().expect("seat") = Some(CursorSeat::Local);
        });
    }

    async fn attach_input_client(&self, mut sess: ActiveSession) {
        let mut client = InputClient::new(MemorySink::default());
        let mut stop = self.shared.shutdown.subscribe();
        let node = self.clone_handle();
        // Real injection rides behind the CI observer: the sink trace records
        // exactly the actions `apply` performed, so forwarding the tail keeps
        // OS injection in lockstep with protocol-level filtering.
        let real = node.shared.input_real_sink.clone();
        let mut forwarded: usize = 0;
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = stop.changed() => {
                        if *stop.borrow() {
                            break;
                        }
                    }
                    inbound = sess.recv_inner() => {
                        match inbound {
                            Ok(frame) => {
                                if let Ok(ev) = InputEvent::from_frame(&frame) {
                                    if let Ok(Some(leave)) = client.apply(ev) {
                                        let _ = sess.send_inner(match leave.to_frame() {
                                            Ok(f) => f,
                                            Err(_) => continue,
                                        }).await;
                                    }
                                    *node.shared.input_sink.lock().expect("sink") = client.sink().clone();
                                    if let Some(sink) = &real {
                                        let mut sink = sink.lock().expect("real input sink");
                                        let trace = client.sink().applied.as_slices();
                                        let total = trace.0.len() + trace.1.len();
                                        // The bounded trace (256) can wrap; min()
                                        // then skips nothing new rather than
                                        // replaying stale actions.
                                        let start = forwarded.min(total);
                                        for kind in trace.0.iter().chain(trace.1.iter()).skip(start) {
                                            match *kind {
                                                tetherly_core::InputKind::Move { x, y } => {
                                                    sink.move_abs(x, y)
                                                }
                                                tetherly_core::InputKind::Button { mask, down } => {
                                                    sink.button(mask, down)
                                                }
                                                tetherly_core::InputKind::Wheel { dx, dy } => {
                                                    sink.wheel(dx, dy)
                                                }
                                                tetherly_core::InputKind::Key { code, down, mods } => {
                                                    sink.key(code, down, mods)
                                                }
                                                _ => {}
                                            }
                                        }
                                        forwarded = total;
                                    }
                                    let seat = if client.focused() {
                                        CursorSeat::Remote
                                    } else {
                                        CursorSeat::Local
                                    };
                                    *node.shared.input_seat.lock().expect("seat") = Some(seat);
                                } else if let Err(e) = InputEvent::from_frame(&frame) {
                                    tracing::warn!(%e, "input client: bad frame");
                                }
                            }
                            Err(e) => {
                                tracing::warn!(%e, "input client: session recv ended; closing");
                                break;
                            }
                        }
                    }
                }
            }
            client.on_peer_gone();
            *node.shared.input_seat.lock().expect("seat") = Some(CursorSeat::Local);
            *node.shared.input_sink.lock().expect("sink") = client.sink().clone();
        });
    }

    /// Host side of a 45720 session: this machine is being viewed. Consent is
    /// enforced here — the peer's `Start` is refused until `screen_allow()` has
    /// run locally. Frames are pushed by the app calling `screen_send_frame()`
    /// at its own cadence (CLI loop on real hardware).
    async fn attach_screen_server(&self, mut sess: ActiveSession) {
        let (tx, mut rx) = mpsc::unbounded_channel::<InnerFrame>();
        *self.shared.screen_out.lock().await = Some(tx);
        {
            self.shared
                .screen_engine
                .lock()
                .expect("screen")
                .on_peer_connected();
        }
        let mut stop = self.shared.shutdown.subscribe();
        let node = self.clone_handle();
        // Pipe outbound frames to the session and inbound controls into the
        // engine. The session's send/recv is the wire; the pump task below
        // produces frames on the timer.
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = stop.changed() => {
                        if *stop.borrow() {
                            break;
                        }
                    }
                    outbound = rx.recv() => {
                        match outbound {
                            Some(frame) => {
                                if sess.send_inner(frame).await.is_err() {
                                    break;
                                }
                            }
                            None => break,
                        }
                    }
                    inbound = sess.recv_inner() => {
                        match inbound {
                            Ok(frame) => {
                                if let Ok(msg) = ControlMsg::from_frame(&frame) {
                                    let reply = node
                                        .shared
                                        .screen_engine
                                        .lock()
                                        .expect("screen")
                                        .on_control(msg);
                                    if let Ok(Some(reply)) = reply {
                                        if let Ok(f) = reply.to_frame() {
                                            let _ = sess.send_inner(f).await;
                                        }
                                    }
                                }
                            }
                            Err(_) => break,
                        }
                    }
                }
            }
            node.shared
                .screen_engine
                .lock()
                .expect("screen")
                .on_peer_gone();
            *node.shared.screen_out.lock().await = None;
        });
    }

    /// Visitor side of a 45720 session: this machine views a peer's screen.
    /// Reassembles `TMF1` fragments, decodes the codec, presents through the
    /// sink, and acks the sequence it accepted.
    async fn attach_screen_client(&self, mut sess: ActiveSession) {
        let (ctl_tx, mut ctl_rx) = mpsc::unbounded_channel::<ControlMsg>();
        *self.shared.screen_ctl.lock().await = Some(ctl_tx);
        let mut receiver = ScreenReceiver::new();
        let mut assembler = ScreenAssembler::new();
        let mut stop = self.shared.shutdown.subscribe();
        let node = self.clone_handle();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = stop.changed() => {
                        if *stop.borrow() {
                            break;
                        }
                    }
                    ctl = ctl_rx.recv() => {
                        match ctl {
                            Some(msg) => {
                                match msg.to_frame() {
                                    Ok(f) => {
                                        if sess.send_inner(f).await.is_err() {
                                            break;
                                        }
                                    }
                                    Err(_) => continue,
                                }
                            }
                            None => break,
                        }
                    }
                    inbound = sess.recv_inner() => {
                        match inbound {
                            Ok(frame) => {
                                match assembler.push(&frame) {
                                    Ok(Some(sf)) => {
                                        // Decompress if the sender zstd-compressed
                                        // this frame.
                                        let present_frame = if sf.compressed {
                                            match node.shared.screen_codec.decompress(
                                                &sf.bytes,
                                                sf.raw_len(),
                                            ) {
                                                Ok(raw) => ScreenFrame {
                                                    bytes: raw,
                                                    compressed: false,
                                                    ..sf
                                                },
                                                Err(e) => {
                                                    tracing::warn!(%e, seq = sf.seq, "screen decompress failed; closing viewer");
                                                    break;
                                                }
                                            }
                                        } else {
                                            sf
                                        };
                                        match receiver.apply(&present_frame) {
                                            Ok(acked) => {
                                                // The default sink is the
                                                // observer itself (shared
                                                // inner), so presenting once
                                                // updates the CI snapshot too.
                                                *node
                                                    .shared
                                                    .screen_rx_stats
                                                    .lock()
                                                    .expect("screen rx") =
                                                    receiver.stats();
                                                if let Some(seq) = acked {
                                                    let _ = node
                                                        .shared
                                                        .screen_sink
                                                        .present(&present_frame);
                                                    let ack = ControlMsg::Ack { seq };
                                                    if let Ok(f) = ack.to_frame() {
                                                        let _ = sess.send_inner(f).await;
                                                    }
                                                }
                                            }
                                            Err(e) => {
                                                tracing::warn!(%e, "screen receiver rejected frame; closing viewer");
                                                break;
                                            }
                                        }
                                    }
                                    Ok(None) => {}
                                    Err(e) => {
                                        tracing::warn!(%e, "screen assembler error; closing viewer");
                                        break;
                                    }
                                }
                            }
                            Err(e) => {
                                tracing::warn!(%e, "screen session recv ended; closing viewer");
                                break;
                            }
                        }
                    }
                }
            }
            receiver.on_peer_gone();
            *node.shared.screen_ctl.lock().await = None;
        });
    }

    /// Grab one frame from the source and push it toward the visitor. Returns
    /// `true` only when a frame was queued (i.e. while streaming). Deterministic
    /// in CI via `MemoryScreenSource`; real capture is Manual-required.
    pub async fn screen_send_frame(&self) -> Result<bool, CoreError> {
        // Grab is the expensive step (DXGI map, adb pipe read): skip it
        // entirely unless a visitor is actually streaming.
        if !self
            .shared
            .screen_engine
            .lock()
            .expect("screen")
            .state()
            .is_streaming()
        {
            return Ok(false);
        }
        // Grab can block for a long time (adb pipe reads in particular), so
        // run it on the blocking pool with a hard timeout. On timeout, call
        // `interrupt` so a wedged device stream EOFs and unwinds instead of
        // stalling the pump forever.
        let source = Arc::clone(&self.shared.screen_source);
        let grab_task = source.clone();
        let grab = tokio::task::spawn_blocking(move || grab_task.grab());
        let frame = match tokio::time::timeout(SCREEN_GRAB_TIMEOUT, grab).await {
            Ok(Ok(inner)) => inner?,
            Ok(Err(join)) => {
                return Err(CoreError::ScreenRefused(format!(
                    "screen grab task: {join}"
                )));
            }
            Err(_) => {
                source.interrupt();
                return Err(CoreError::ScreenRefused("screen grab timed out".into()));
            }
        };
        let Some(frame) = frame else {
            return Ok(false);
        };
        let outbound = {
            let mut engine = self.shared.screen_engine.lock().expect("screen");
            if engine.push(frame).is_none() {
                return Ok(false);
            }
            engine.drain()
        };
        self.shared
            .screen_interval
            .lock()
            .expect("interval")
            .record();
        self.send_screen_frames(outbound).await?;
        Ok(true)
    }

    async fn send_screen_frames(&self, frames: Vec<ScreenFrame>) -> Result<(), CoreError> {
        let tx = self
            .shared
            .screen_out
            .lock()
            .await
            .clone()
            .ok_or_else(|| CoreError::Json("screen channel closed".into()))?;
        for f in frames {
            for frag in f.to_frames()? {
                tx.send(frag)
                    .map_err(|_| CoreError::Json("screen peer gone".into()))?;
            }
        }
        Ok(())
    }

    async fn send_screen_control(&self, msg: ControlMsg) -> Result<(), CoreError> {
        let tx = self
            .shared
            .screen_ctl
            .lock()
            .await
            .clone()
            .ok_or_else(|| CoreError::Json("screen control channel closed".into()))?;
        tx.send(msg)
            .map_err(|_| CoreError::Json("screen peer gone".into()))
    }

    /// Local consent for the currently connected visitor. This is the ONLY path
    /// into `Allowed`; nothing a peer sends can reach it.
    pub fn screen_allow(&self) {
        self.shared.screen_engine.lock().expect("screen").allow();
    }

    /// Revoke local consent; any queued frames are dropped.
    pub fn screen_revoke(&self) {
        self.shared.screen_engine.lock().expect("screen").revoke();
    }

    /// Visitor: ask the host to begin sending. Refused by the host until it has
    /// allowed locally.
    pub async fn screen_start(&self) -> Result<(), CoreError> {
        self.send_screen_control(ControlMsg::Start).await
    }

    /// True while a screen control channel to a host is attached. The UI uses
    /// this to dial the screen channel on demand instead of tracking sessions.
    pub async fn screen_channel_open(&self) -> bool {
        self.shared.screen_ctl.lock().await.is_some()
    }

    /// Visitor: ask the host to stop sending.
    pub async fn screen_stop(&self) -> Result<(), CoreError> {
        self.send_screen_control(ControlMsg::Stop).await
    }

    pub fn screen_state(&self) -> ScreenState {
        self.shared.screen_engine.lock().expect("screen").state()
    }

    pub fn screen_stats(&self) -> ScreenStats {
        self.shared.screen_engine.lock().expect("screen").stats()
    }

    /// Visitor-side receiver counters. All zeros until this node views a peer.
    pub fn screen_rx_stats(&self) -> ScreenStats {
        *self.shared.screen_rx_stats.lock().expect("screen rx")
    }

    /// 95th-percentile host-side frame interval in microseconds, or `None`
    /// before two frames have shipped. Wall-clock: this is the M5.1 budget
    /// measurement on real hardware.
    pub fn screen_latency_p95_us(&self) -> Option<u64> {
        self.shared
            .screen_interval
            .lock()
            .expect("interval")
            .p95_us()
    }

    pub fn screen_sink_snapshot(&self) -> MemoryScreenSink {
        self.shared.screen_observer.lock().expect("sink").clone()
    }

    pub fn offered_files(&self) -> Vec<tetherly_core::IncomingTransfer> {
        self.shared.hubs.lock().expect("hubs").files.offered()
    }
}

fn route_addr(table: &RouteTable, device_id: &str) -> Option<SocketAddr> {
    table
        .routes
        .iter()
        .find(|r| r.device_id == device_id)
        .and_then(|r| r.addr.parse().ok())
}

fn parse_file_port(caps: &[String]) -> Option<u16> {
    parse_port_cap(caps, "fileport=")
}

fn parse_input_port(caps: &[String]) -> Option<u16> {
    parse_port_cap(caps, "inputport=")
}

fn parse_screen_port(caps: &[String]) -> Option<u16> {
    parse_port_cap(caps, "screenport=")
}

fn parse_port_cap(caps: &[String], prefix: &str) -> Option<u16> {
    caps.iter().find_map(|c| {
        c.strip_prefix(prefix)
            .and_then(|s| s.parse::<u16>().ok())
            .filter(|p| *p != 0)
    })
}

#[cfg(test)]
mod resume_scan_tests {
    use super::*;
    use tetherly_core::{DeviceId, FILE_CHUNK};

    fn temp_inbox(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "tetherly-scan-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(dir.join("inbox")).unwrap();
        dir
    }

    fn write_sidecar(dir: &std::path::Path, stem: &str, src: &str, total: u64, offset: u64) {
        let dto = serde_json::json!({ "device_id": src, "total": total, "offset": offset });
        std::fs::write(
            dir.join("inbox").join(format!(".{stem}.resume.json")),
            serde_json::to_vec(&dto).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn a_healthy_sidecar_is_loaded_and_a_torn_one_is_dropped() {
        let dir = temp_inbox("scan");
        let src = DeviceId::from_id_pk(&[7u8; 32]).to_string();
        let total = 3 * FILE_CHUNK as u64;
        // Healthy: sidecar offset 2*CHUNK, matching partial of that length.
        write_sidecar(&dir, "tr_good", &src, total, 2 * FILE_CHUNK as u64);
        std::fs::write(
            dir.join("inbox").join(".tr_good.part"),
            vec![0u8; 2 * FILE_CHUNK],
        )
        .unwrap();
        // Torn: sidecar claims 2*CHUNK but the partial is missing entirely.
        write_sidecar(&dir, "tr_torn", &src, total, 2 * FILE_CHUNK as u64);
        // Bogus: unparseable source id, and a zero/oversized offset.
        write_sidecar(
            &dir,
            "tr_badid",
            "not-a-device-id",
            total,
            FILE_CHUNK as u64,
        );
        write_sidecar(&dir, "tr_zero", &src, total, 0);
        write_sidecar(&dir, "tr_over", &src, total, total + 1);
        // Markdown dir entries that merely look like sidecars are ignored.
        std::fs::write(dir.join("inbox").join("plain.txt"), b"x").unwrap();

        let table = load_resume_table(&dir);
        let good = table.get("tr_good").expect("healthy sidecar loads");
        assert_eq!(good.offset, 2 * FILE_CHUNK as u64);
        assert!(table.get("tr_torn").is_none(), "torn sidecar is dropped");
        assert!(table.get("tr_badid").is_none());
        assert!(table.get("tr_zero").is_none());
        assert!(table.get("tr_over").is_none());
        assert_eq!(table.all().len(), 1);
        // The torn sidecar file was removed so a later transfer starts clean.
        assert!(!dir.join("inbox").join(".tr_torn.resume.json").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_inbox_dir_yields_an_empty_table() {
        let dir = std::env::temp_dir().join(format!("tetherly-scan-none-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let table = load_resume_table(&dir);
        assert_eq!(table.all().len(), 0);
    }
}
