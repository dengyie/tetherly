// SPDX-License-Identifier: Apache-2.0 OR MIT
//! LAN + optional EasyTier sidecar overlay. Overlay is unicast only; no mDNS on TUN.

use crate::events::{file_offered, peer_up, CandidateViewDto, UiEvent};
use crate::lan::{bind_many, bind_overlay, control_bind_addrs, lan_ips};
use crate::platform::{insert_foreground, MemoryClip, SystemClip};
use crate::sidecar::{self, OverlayConfig, OverlayStatus};
use crate::store::{load_or_create_identity, load_trust, remember_route, save_trust, RouteTable};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tetherly_core::frame::{
    pong, CapsUpdate, ClipSet, FileDone, FileOffer, InnerFrame, NotifyPush,
};
use tetherly_core::pin::{format_pin, generate_pin};
use tetherly_core::ports::{Clip, Clock, Rng, SystemClock};
use tetherly_core::session::LockoutTable;
use tetherly_core::session::{backoff_delay, PIN_TTL};
use tetherly_core::{
    ancs_actions, ancs_source_id, hex_lower, is_desktop_platform, path_kind, AncsEvent,
    AncsIngress, AncsState, AncsTransport, ClipApply, ClipHub, ConnectOutcome, ControlMsg,
    CoreError, CursorSeat, DeviceId, FileHub, IngestOutcome, InputClient, InputEvent, InputServer,
    MemoryScreenSink, MemorySink, MemoryTrustStore, NotifyHub, OpenAllowlist, PathKind,
    ScreenFrame, ScreenReceiver, ScreenSender, ScreenSink, ScreenSource, ScreenState, ScreenStats,
    TcpLimiter, TrustStore, CLIPBOARD_OTP_CLEAR_MS, OVERLAY_PROBE_MS,
};
use tetherly_crypto::Identity;
use tetherly_net::dispatch::SessionEvent;
use tetherly_net::filechan::send_bytes as send_file_bytes;
use tetherly_net::session::{
    accept_session, dial_session, ActiveSession, SessionConfig, CONTROL_PORT, FILE_PORT,
    HANDSHAKE_TIMEOUT, INPUT_PORT, SCREEN_PORT,
};
use tokio::net::TcpStream;
use tokio::sync::{broadcast, mpsc, watch, Mutex as AsyncMutex};
use tracing::{debug, info, warn};

struct OsRng;

impl Rng for OsRng {
    fn fill_bytes(&self, dest: &mut [u8]) -> Result<(), CoreError> {
        getrandom::getrandom(dest).map_err(|_| CoreError::Rng)
    }
}

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
        }
    }
}

fn hostname() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "tetherly".into())
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
}

struct OutboundFile {
    peer: DeviceId,
    data: Vec<u8>,
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
    screen_sink: Mutex<MemoryScreenSink>,
    /// Visitor-side control channel. None until a visitor attaches.
    screen_ctl: AsyncMutex<Option<mpsc::UnboundedSender<ControlMsg>>>,
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

impl Node {
    pub fn start(cfg: NodeConfig) -> Result<Self, crate::store::NodeStoreError> {
        Self::start_with_clock(cfg, Arc::new(SystemClock))
    }

    pub fn start_with_clock(
        cfg: NodeConfig,
        clock: Arc<dyn Clock>,
    ) -> Result<Self, crate::store::NodeStoreError> {
        let identity = load_or_create_identity(&cfg.data_dir)?;
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
        let screen_source = cfg
            .screen_source
            .clone()
            .unwrap_or_else(|| Arc::new(crate::screen::SystemSource));
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
            input_engine: Mutex::new(InputServer::new(1, 1)),
            screen_out: AsyncMutex::new(None),
            screen_engine: Mutex::new(ScreenSender::new()),
            screen_source,
            screen_sink: Mutex::new(MemoryScreenSink::new()),
            screen_ctl: AsyncMutex::new(None),
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
        let control_port = self.shared.cfg.control_port;
        let file_port = self.shared.cfg.file_port;
        let addrs = if self.shared.cfg.loopback_only {
            vec![SocketAddr::from(([127, 0, 0, 1], control_port))]
        } else {
            control_bind_addrs(control_port)
        };
        let listeners = bind_many(&addrs).await?;
        if let Some(first) = listeners.first() {
            self.shared
                .bound_control
                .store(first.local_addr()?.port(), Ordering::SeqCst);
        }
        let file_addrs = if self.shared.cfg.loopback_only {
            vec![SocketAddr::from(([127, 0, 0, 1], file_port))]
        } else {
            control_bind_addrs(file_port)
        };
        let file_listeners = bind_many(&file_addrs).await.unwrap_or_default();
        if let Some(first) = file_listeners.first() {
            if let Ok(addr) = first.local_addr() {
                self.shared.bound_file.store(addr.port(), Ordering::SeqCst);
            }
        }
        let input_port = self.shared.cfg.input_port;
        let input_addrs = if self.shared.cfg.loopback_only {
            vec![SocketAddr::from(([127, 0, 0, 1], input_port))]
        } else {
            control_bind_addrs(input_port)
        };
        let input_listeners = bind_many(&input_addrs).await.unwrap_or_default();
        if let Some(first) = input_listeners.first() {
            if let Ok(addr) = first.local_addr() {
                self.shared.bound_input.store(addr.port(), Ordering::SeqCst);
            }
        }
        let screen_port = self.shared.cfg.screen_port;
        let screen_addrs = if self.shared.cfg.loopback_only {
            vec![SocketAddr::from(([127, 0, 0, 1], screen_port))]
        } else {
            control_bind_addrs(screen_port)
        };
        let screen_listeners = bind_many(&screen_addrs).await.unwrap_or_default();
        if let Some(first) = screen_listeners.first() {
            if let Ok(addr) = first.local_addr() {
                self.shared
                    .bound_screen
                    .store(addr.port(), Ordering::SeqCst);
            }
        }

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
            let node = self.clone_handle();
            let mut stop = self.shared.shutdown.subscribe();
            tokio::spawn(async move {
                loop {
                    tokio::select! {
                        _ = stop.changed() => {
                            if *stop.borrow() {
                                break;
                            }
                        }
                        accepted = l.accept() => {
                            match accepted {
                                Ok((stream, addr)) => {
                                    let node = node.clone_handle();
                                    tokio::spawn(async move {
                                        if let Err(e) = node.accept_one(stream, addr).await {
                                            warn!(%e, %addr, "accept session failed");
                                        }
                                    });
                                }
                                Err(e) => {
                                    warn!(%e, "accept");
                                    tokio::time::sleep(Duration::from_millis(200)).await;
                                }
                            }
                        }
                    }
                }
            });
        }
        for l in file_listeners {
            let node = self.clone_handle();
            let mut stop = self.shared.shutdown.subscribe();
            tokio::spawn(async move {
                loop {
                    tokio::select! {
                        _ = stop.changed() => {
                            if *stop.borrow() {
                                break;
                            }
                        }
                        accepted = l.accept() => {
                            match accepted {
                                Ok((stream, addr)) => {
                                    let node = node.clone_handle();
                                    tokio::spawn(async move {
                                        if let Err(e) = node.accept_file(stream, addr).await {
                                            warn!(%e, "file channel");
                                        }
                                    });
                                }
                                Err(_) => {
                                    tokio::time::sleep(Duration::from_millis(200)).await;
                                }
                            }
                        }
                    }
                }
            });
        }
        for l in input_listeners {
            let node = self.clone_handle();
            let mut stop = self.shared.shutdown.subscribe();
            tokio::spawn(async move {
                loop {
                    tokio::select! {
                        _ = stop.changed() => {
                            if *stop.borrow() {
                                break;
                            }
                        }
                        accepted = l.accept() => {
                            match accepted {
                                Ok((stream, addr)) => {
                                    let node = node.clone_handle();
                                    tokio::spawn(async move {
                                        if let Err(e) = node.accept_input(stream, addr).await {
                                            debug!(%e, "input channel");
                                        }
                                    });
                                }
                                Err(_) => {
                                    tokio::time::sleep(Duration::from_millis(200)).await;
                                }
                            }
                        }
                    }
                }
            });
        }
        for l in screen_listeners {
            let node = self.clone_handle();
            let mut stop = self.shared.shutdown.subscribe();
            tokio::spawn(async move {
                loop {
                    tokio::select! {
                        _ = stop.changed() => {
                            if *stop.borrow() {
                                break;
                            }
                        }
                        accepted = l.accept() => {
                            match accepted {
                                Ok((stream, addr)) => {
                                    let node = node.clone_handle();
                                    tokio::spawn(async move {
                                        if let Err(e) = node.accept_screen(stream, addr).await {
                                            debug!(%e, "screen channel");
                                        }
                                    });
                                }
                                Err(_) => {
                                    tokio::time::sleep(Duration::from_millis(200)).await;
                                }
                            }
                        }
                    }
                }
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
                    let node = node.clone_handle();
                    let mut stop = node.shared.shutdown.subscribe();
                    tokio::spawn(async move {
                        loop {
                            tokio::select! {
                                _ = stop.changed() => {
                                    if *stop.borrow() {
                                        break;
                                    }
                                }
                                accepted = l.accept() => {
                                    match accepted {
                                        Ok((stream, addr)) => {
                                            let node = node.clone_handle();
                                            tokio::spawn(async move {
                                                if let Err(e) = node.accept_one(stream, addr).await {
                                                    warn!(%e, %addr, "overlay accept failed");
                                                }
                                            });
                                        }
                                        Err(e) => {
                                            debug!(%e, "overlay accept");
                                            tokio::time::sleep(Duration::from_millis(200)).await;
                                        }
                                    }
                                }
                            }
                        }
                    });
                }
                for l in bind_overlay(&overlay_file).await {
                    let node = node.clone_handle();
                    let mut stop = node.shared.shutdown.subscribe();
                    tokio::spawn(async move {
                        loop {
                            tokio::select! {
                                _ = stop.changed() => {
                                    if *stop.borrow() {
                                        break;
                                    }
                                }
                                accepted = l.accept() => {
                                    if let Ok((stream, addr)) = accepted {
                                        let node = node.clone_handle();
                                        tokio::spawn(async move {
                                            let _ = node.accept_file(stream, addr).await;
                                        });
                                    }
                                }
                            }
                        }
                    });
                }
                for l in bind_overlay(&overlay_input).await {
                    let node = node.clone_handle();
                    let mut stop = node.shared.shutdown.subscribe();
                    tokio::spawn(async move {
                        loop {
                            tokio::select! {
                                _ = stop.changed() => {
                                    if *stop.borrow() {
                                        break;
                                    }
                                }
                                accepted = l.accept() => {
                                    if let Ok((stream, addr)) = accepted {
                                        let node = node.clone_handle();
                                        tokio::spawn(async move {
                                            let _ = node.accept_input(stream, addr).await;
                                        });
                                    }
                                }
                            }
                        }
                    });
                }
                for l in bind_overlay(&overlay_screen).await {
                    let node = node.clone_handle();
                    let mut stop = node.shared.shutdown.subscribe();
                    tokio::spawn(async move {
                        loop {
                            tokio::select! {
                                _ = stop.changed() => {
                                    if *stop.borrow() {
                                        break;
                                    }
                                }
                                accepted = l.accept() => {
                                    if let Ok((stream, addr)) = accepted {
                                        let node = node.clone_handle();
                                        tokio::spawn(async move {
                                            let _ = node.accept_screen(stream, addr).await;
                                        });
                                    }
                                }
                            }
                        }
                    });
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
        tokio::task::spawn_blocking(move || match tetherly_net::mdns::daemon() {
            Ok(d) => {
                if let Err(e) = tetherly_net::mdns::advertise(
                    &d,
                    &name,
                    port,
                    &id,
                    &fp,
                    "notify,clip,file",
                    &addrs,
                ) {
                    warn!(%e, "mdns advertise");
                } else {
                    std::thread::park();
                }
            }
            Err(e) => warn!(%e, "mdns daemon"),
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
            SessionEvent::Clip(clip) => {
                self.apply_clip(peer, clip)?;
            }
            SessionEvent::FileOffer(offer) => {
                let t = {
                    let mut hubs = self.shared.hubs.lock().expect("hubs");
                    hubs.files
                        .offer(peer.clone(), offer, self.shared.clock.unix_ms())?
                };
                self.emit(file_offered(&t));
            }
            SessionEvent::FileAccept(d) => {
                debug!(transfer = %d.transfer_id, "peer accepted");
                self.on_peer_accepted_file(&d.transfer_id).await;
            }
            SessionEvent::FileReject(d) => {
                info!(transfer = %d.transfer_id, "peer rejected file");
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
            SessionEvent::ReplyStub | SessionEvent::Unknown(_) => {}
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
        // First 4 bytes of file channel after accept: we don't know transfer
        // until magic+id. Peek via recv_file_bytes needs token+id. Handshake:
        // peer writes magic|id|size|chunks. We read id first the same way.
        use tokio::io::AsyncReadExt;
        let mut magic = [0u8; 4];
        stream.read_exact(&mut magic).await?;
        if magic != tetherly_net::filechan::FILE_MAGIC {
            return Err(tetherly_net::NetError::Handshake);
        }
        let id_bytes = tetherly_net::codec::read_len_prefixed(&mut stream).await?;
        let transfer_id =
            String::from_utf8(id_bytes).map_err(|_| tetherly_net::NetError::Handshake)?;
        let accepted = {
            let hubs = self.shared.hubs.lock().expect("hubs");
            hubs.files.is_accepted(&transfer_id)
        };
        if !accepted {
            return Err(tetherly_net::NetError::File("not accepted".into()));
        }
        let (source, size) = {
            let hubs = self.shared.hubs.lock().expect("hubs");
            let t = hubs
                .files
                .get(&transfer_id)
                .ok_or(tetherly_net::NetError::File("unknown".into()))?;
            (
                t.source.clone(),
                t.files.iter().map(|f| f.size).sum::<u64>(),
            )
        };
        let token = self
            .file_token_for(&source, &transfer_id)
            .ok_or(tetherly_net::NetError::File("no session token".into()))?;
        let size_bytes = tetherly_net::codec::read_len_prefixed(&mut stream).await?;
        if size_bytes.len() != 8 {
            return Err(tetherly_net::NetError::Handshake);
        }
        let declared = u64::from_be_bytes(
            size_bytes
                .try_into()
                .map_err(|_| tetherly_net::NetError::Handshake)?,
        );
        if declared > size.max(32 * 1024 * 1024) {
            return Err(tetherly_net::NetError::TooLarge);
        }
        let mut data = Vec::new();
        let mut counter = 1u64;
        while (data.len() as u64) < declared {
            let ct = tetherly_net::codec::read_len_prefixed(&mut stream).await?;
            let nonce = tetherly_crypto::counter_nonce(counter);
            let pt = tetherly_crypto::aead_open(&token, &nonce, transfer_id.as_bytes(), &ct)?;
            data.extend_from_slice(&pt);
            counter += 1;
        }
        let digest = tetherly_core::hex_sha256(&data);
        let expected = {
            let hubs = self.shared.hubs.lock().expect("hubs");
            hubs.files
                .get(&transfer_id)
                .and_then(|t| t.files.first().map(|f| f.sha256.clone()))
                .unwrap_or_default()
        };
        if digest != expected {
            return Err(tetherly_net::NetError::File("sha256".into()));
        }
        let dest_dir = self.shared.cfg.data_dir.join("inbox");
        std::fs::create_dir_all(&dest_dir)?;
        let name = {
            let hubs = self.shared.hubs.lock().expect("hubs");
            hubs.files
                .get(&transfer_id)
                .and_then(|t| t.files.first().map(|f| f.name.clone()))
                .unwrap_or_else(|| "file.bin".into())
        };
        let path = dest_dir.join(name);
        crate::store::atomic_write(&path, &data)?;
        let done = FileDone {
            transfer_id: transfer_id.clone(),
            sha256: digest,
        };
        {
            let mut hubs = self.shared.hubs.lock().expect("hubs");
            let _ = hubs.files.mark_done(&done);
        }
        let _ = self.send_to(&source, done.to_frame()?).await;
        self.emit(UiEvent::FileDone { transfer_id });
        Ok(())
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
        let mut nonce = [0u8; 8];
        getrandom::getrandom(&mut nonce).map_err(|_| CoreError::Rng)?;
        let transfer_id = format!("tr_{}", hex_lower(&nonce));
        let offer = FileOffer {
            transfer_id: transfer_id.clone(),
            files: vec![tetherly_core::FileMeta {
                name: name.into(),
                size: data.len() as u64,
                sha256: tetherly_core::hex_sha256(data),
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

    async fn on_peer_accepted_file(&self, transfer_id: &str) {
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
        if let Err(e) = self
            .send_file_data(addr, file_port, &token, transfer_id, &outbound.data)
            .await
        {
            warn!(%e, transfer = %transfer_id, "file data send failed");
        }
    }

    pub async fn send_file_data(
        &self,
        peer_addr: SocketAddr,
        file_port: u16,
        token: &[u8; 32],
        transfer_id: &str,
        data: &[u8],
    ) -> Result<[u8; 32], tetherly_net::NetError> {
        let dest = SocketAddr::new(peer_addr.ip(), file_port);
        let mut stream = TcpStream::connect(dest).await?;
        send_file_bytes(&mut stream, token, transfer_id, data).await
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

    /// Push local pixels onto the open 45719 session. Crossing the right edge
    /// emits Enter + Move. Seq is monotonic for the life of the node. No JSON,
    /// and never onto 45717. OS hooks stay Manual-required.
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
                            Err(_) => break,
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
                                    let seat = if client.focused() {
                                        CursorSeat::Remote
                                    } else {
                                        CursorSeat::Local
                                    };
                                    *node.shared.input_seat.lock().expect("seat") = Some(seat);
                                }
                            }
                            Err(_) => break,
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
    /// run locally. Frames are pumped by `screen_send_frame()`.
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
    /// Receives frames, presents them to the in-memory sink (real present is
    /// Manual-required), and acks the sequence it accepted.
    async fn attach_screen_client(&self, mut sess: ActiveSession) {
        let (ctl_tx, mut ctl_rx) = mpsc::unbounded_channel::<ControlMsg>();
        *self.shared.screen_ctl.lock().await = Some(ctl_tx);
        let mut receiver = ScreenReceiver::new();
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
                                if let Ok(sf) = ScreenFrame::from_frame(&frame) {
                                    match receiver.apply(&sf) {
                                        Ok(Some(seq)) => {
                                            if let Ok(sink) = node.shared.screen_sink.lock() {
                                                let _ = sink.present(&sf);
                                            }
                                            let ack = ControlMsg::Ack { seq };
                                            if let Ok(f) = ack.to_frame() {
                                                let _ = sess.send_inner(f).await;
                                            }
                                        }
                                        Ok(None) => {}
                                        Err(_) => break,
                                    }
                                }
                            }
                            Err(_) => break,
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
        let frame = self.shared.screen_source.grab()?;
        let outbound = {
            let mut engine = self.shared.screen_engine.lock().expect("screen");
            if engine.push(frame).is_none() {
                return Ok(false);
            }
            engine.drain()
        };
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
            tx.send(f.to_frame()?)
                .map_err(|_| CoreError::Json("screen peer gone".into()))?;
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

    pub fn screen_sink_snapshot(&self) -> MemoryScreenSink {
        self.shared.screen_sink.lock().expect("sink").clone()
    }

    pub fn data_dir(&self) -> &std::path::Path {
        &self.shared.cfg.data_dir
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
