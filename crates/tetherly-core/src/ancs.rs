// SPDX-License-Identifier: Apache-2.0 OR MIT
//! ANCS (Apple Notification Center Service) computer-side ingress.
//!
//! OS-free: the BLE GATT link lives behind [`AncsTransport`]. Everything that
//! can be decided from bytes alone — the Notification Source event tuple, the
//! Control Point command encoders, the Data Source fragment reassembler, the
//! subscribe/backoff state machine, uid dedup — lives here so CI can drive it
//! with [`MemoryAncsTransport`].
//!
//! Hard constraints from the spec (§9.4), enforced by construction:
//! * subscribe Data Source **before** Notification Source ([`AncsIngress::connect`]);
//! * never write Control Point on the value-changed thread — `on_notification_source`
//!   only enqueues, [`AncsIngress::tick`] writes;
//! * Control Point is serial: at most one unanswered request in flight;
//! * fragments are reassembled by byte count, never by partial UTF-8;
//! * a silently dropped first response is retried once, then abandoned;
//! * disconnect schedules a capped exponential backoff;
//! * uids are deduped, and `PreExisting` notifications are dropped so a
//!   reconnect never re-pops notifications the user already saw.

use crate::allowlist::OpenAllowlist;
use crate::device_id::DeviceId;
use crate::error::CoreError;
use crate::ports::Clock;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------- UUIDs

/// ANCS service UUID (canonical text form, as published by Apple).
pub const UUID_ANCS_SERVICE: &str = "7905F431-B5CE-4E99-A40F-4B1E122D00D0";
/// Notification Source characteristic.
pub const UUID_NOTIFICATION_SOURCE: &str = "9FBF120D-6301-42D9-8C58-25E699A21DBD";
/// Control Point characteristic.
pub const UUID_CONTROL_POINT: &str = "69D1D8F3-45E1-49A8-9821-9BBDFDAAD9D9";
/// Data Source characteristic.
pub const UUID_DATA_SOURCE: &str = "22EAC6E9-24D6-4BB5-BE44-B36ACE7C7BFB";

/// Decode the canonical text form into the 16 bytes a GATT stack wants
/// (least-significant octet first). Returns `None` for anything malformed.
pub fn uuid_bytes_le(uuid: &str) -> Option<[u8; 16]> {
    let hex: Vec<u8> = uuid
        .bytes()
        .filter(|b| *b != b'-')
        .map(|b| match b {
            b'0'..=b'9' => Ok(b - b'0'),
            b'a'..=b'f' => Ok(b - b'a' + 10),
            b'A'..=b'F' => Ok(b - b'A' + 10),
            _ => Err(()),
        })
        .collect::<Result<_, _>>()
        .ok()?;
    if hex.len() != 32 {
        return None;
    }
    let mut be = [0u8; 16];
    for (i, pair) in hex.chunks_exact(2).enumerate() {
        be[i] = (pair[0] << 4) | pair[1];
    }
    be.reverse();
    Some(be)
}

// ------------------------------------------------- Notification Source

pub const NOTIFICATION_EVENT_LEN: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventId {
    Added,
    Modified,
    Removed,
    /// Forward compatibility: unknown event ids are ignored, never guessed.
    Unknown(u8),
}

impl EventId {
    pub fn from_u8(raw: u8) -> Self {
        match raw {
            0 => Self::Added,
            1 => Self::Modified,
            2 => Self::Removed,
            other => Self::Unknown(other),
        }
    }
}

/// ANCS notification categories. Unknown values stay representable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Other,
    IncomingCall,
    MissedCall,
    Voicemail,
    Social,
    Schedule,
    Email,
    News,
    HealthAndFitness,
    BusinessAndFinance,
    Location,
    Entertainment,
    Unknown(u8),
}

impl Category {
    pub fn from_u8(raw: u8) -> Self {
        match raw {
            0 => Self::Other,
            1 => Self::IncomingCall,
            2 => Self::MissedCall,
            3 => Self::Voicemail,
            4 => Self::Social,
            5 => Self::Schedule,
            6 => Self::Email,
            7 => Self::News,
            8 => Self::HealthAndFitness,
            9 => Self::BusinessAndFinance,
            10 => Self::Location,
            11 => Self::Entertainment,
            other => Self::Unknown(other),
        }
    }
}

/// Bit flags carried by a Notification Source event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EventFlags(u8);

impl EventFlags {
    pub const SILENT: u8 = 0x01;
    pub const IMPORTANT: u8 = 0x02;
    pub const PRE_EXISTING: u8 = 0x04;
    pub const POSITIVE_ACTION: u8 = 0x08;
    pub const NEGATIVE_ACTION: u8 = 0x10;

    pub fn from_bits(bits: u8) -> Self {
        Self(bits)
    }

    pub fn bits(&self) -> u8 {
        self.0
    }

    pub fn silent(&self) -> bool {
        self.0 & Self::SILENT != 0
    }

    pub fn important(&self) -> bool {
        self.0 & Self::IMPORTANT != 0
    }

    /// Set by iOS for every notification that already existed when the
    /// connection came up. These must never be surfaced again (M4.3).
    pub fn pre_existing(&self) -> bool {
        self.0 & Self::PRE_EXISTING != 0
    }

    pub fn has_positive_action(&self) -> bool {
        self.0 & Self::POSITIVE_ACTION != 0
    }

    pub fn has_negative_action(&self) -> bool {
        self.0 & Self::NEGATIVE_ACTION != 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotificationEvent {
    pub event_id: EventId,
    pub flags: EventFlags,
    pub category: Category,
    pub category_count: u8,
    pub uid: u32,
}

impl NotificationEvent {
    /// Parse an 8-byte Notification Source value. Trailing bytes (which iOS does
    /// not send) are ignored rather than treated as a different event.
    pub fn parse(value: &[u8]) -> Result<Self, CoreError> {
        if value.len() < NOTIFICATION_EVENT_LEN {
            return Err(CoreError::AncsFrame);
        }
        let uid = u32::from_le_bytes(value[4..8].try_into().map_err(|_| CoreError::AncsFrame)?);
        Ok(Self {
            event_id: EventId::from_u8(value[0]),
            flags: EventFlags::from_bits(value[1]),
            category: Category::from_u8(value[2]),
            category_count: value[3],
            uid,
        })
    }
}

// ---------------------------------------------------- Control Point

pub const CP_GET_NOTIFICATION_ATTRIBUTES: u8 = 0;
pub const CP_GET_APP_ATTRIBUTES: u8 = 1;
pub const CP_PERFORM_NOTIFICATION_ACTION: u8 = 2;

pub const ATTR_APP_IDENTIFIER: u8 = 0;
pub const ATTR_TITLE: u8 = 1;
pub const ATTR_SUBTITLE: u8 = 2;
pub const ATTR_MESSAGE: u8 = 3;
pub const ATTR_MESSAGE_SIZE: u8 = 4;
pub const ATTR_DATE: u8 = 5;
pub const ATTR_POSITIVE_ACTION_LABEL: u8 = 6;
pub const ATTR_NEGATIVE_ACTION_LABEL: u8 = 7;

pub const APP_ATTR_DISPLAY_NAME: u8 = 0;

pub const ACTION_POSITIVE: u8 = 0;
pub const ACTION_NEGATIVE: u8 = 1;

/// Attributes requested for every notification. `MESSAGE_SIZE` lets us tell a
/// truncated body from a short one without guessing.
pub const NOTIF_ATTR_REQUEST: [u8; 4] = [
    ATTR_APP_IDENTIFIER,
    ATTR_TITLE,
    ATTR_MESSAGE,
    ATTR_MESSAGE_SIZE,
];

/// Longest string attribute we ask iOS for, in bytes.
pub const ATTR_MAX_LEN: u16 = 512;

/// Attributes that carry a caller-supplied length in the request.
pub fn attr_takes_length(attr: u8) -> bool {
    matches!(attr, ATTR_TITLE | ATTR_SUBTITLE | ATTR_MESSAGE)
}

/// `GetNotificationAttributes` for one uid. Length-bearing attributes get a
/// 2-byte maximum immediately after their id, per the ANCS spec.
pub fn encode_get_notification_attributes(uid: u32, attrs: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(7 + attrs.len() * 3);
    out.push(CP_GET_NOTIFICATION_ATTRIBUTES);
    out.extend_from_slice(&uid.to_le_bytes());
    for &attr in attrs {
        out.push(attr);
        if attr_takes_length(attr) {
            out.extend_from_slice(&ATTR_MAX_LEN.to_le_bytes());
        }
    }
    out
}

/// `GetAppAttributes` for one bundle id (NUL-terminated on the wire).
pub fn encode_get_app_attributes(app_id: &str, attrs: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + app_id.len() + attrs.len());
    out.push(CP_GET_APP_ATTRIBUTES);
    out.extend_from_slice(app_id.as_bytes());
    out.push(0);
    out.extend_from_slice(attrs);
    out
}

/// `PerformNotificationAction` — dismiss or invoke the positive action.
pub fn encode_perform_action(uid: u32, action: u8) -> Vec<u8> {
    let mut out = Vec::with_capacity(6);
    out.push(CP_PERFORM_NOTIFICATION_ACTION);
    out.extend_from_slice(&uid.to_le_bytes());
    out.push(action);
    out
}

// ---------------------------------------------------- Data Source

/// Upper bound on buffered Data Source bytes. A response we cannot parse within
/// this budget is dropped instead of growing without bound.
pub const DATA_BUF_MAX: usize = 8 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DataItem {
    NotificationAttr {
        uid: u32,
        attr: u8,
        value: Vec<u8>,
    },
    AppAttr {
        app_id: String,
        attr: u8,
        value: Vec<u8>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Partial {
    Notification { uid: u32 },
    App { app_id: String },
}

/// Incremental Data Source parser. Fragments are appended as raw bytes and only
/// whole attribute tuples are ever handed out, so a UTF-8 value split across
/// two GATT notifications is never decoded early.
#[derive(Debug, Default)]
pub struct DataAssembler {
    buf: VecDeque<u8>,
    partial: Option<Partial>,
}

impl DataAssembler {
    pub fn push(&mut self, chunk: &[u8]) -> Result<(), CoreError> {
        if self.buf.len() + chunk.len() > DATA_BUF_MAX {
            self.reset();
            return Err(CoreError::AncsFrame);
        }
        self.buf.extend(chunk.iter().copied());
        Ok(())
    }

    pub fn pending(&self) -> usize {
        self.buf.len()
    }

    pub fn reset(&mut self) {
        self.buf.clear();
        self.partial = None;
    }

    /// Mark the current transaction finished. ANCS Data Source responses carry
    /// no terminator, so the boundary comes from the serial Control Point: once
    /// the request that produced these bytes is satisfied, the next byte is a
    /// new command. Leftover bytes can only be a protocol violation.
    pub fn end_response(&mut self) {
        if !self.buf.is_empty() {
            tracing::debug!(
                leftover = self.buf.len(),
                "ancs: dropping bytes after a completed response"
            );
        }
        self.reset();
    }

    fn take(&mut self, n: usize) -> Vec<u8> {
        (0..n).filter_map(|_| self.buf.pop_front()).collect()
    }

    /// Next complete attribute tuple, or `None` while more bytes are needed.
    /// Consumed bytes are removed; a partially received tuple stays buffered.
    pub fn next_item(&mut self) -> Result<Option<DataItem>, CoreError> {
        if self.partial.is_none() {
            let Some(&cmd) = self.buf.front() else {
                return Ok(None);
            };
            match cmd {
                CP_GET_NOTIFICATION_ATTRIBUTES => {
                    if self.buf.len() < 5 {
                        return Ok(None);
                    }
                    self.take(1);
                    let raw = self.take(4);
                    let uid = u32::from_le_bytes(raw.try_into().map_err(|_| CoreError::AncsFrame)?);
                    self.partial = Some(Partial::Notification { uid });
                }
                CP_GET_APP_ATTRIBUTES => {
                    // `nul` indexes the buffer including the command byte, so the
                    // identifier itself is `nul - 1` bytes long.
                    let Some(nul) = self.buf.iter().position(|b| *b == 0) else {
                        if self.buf.len() > DATA_BUF_MAX / 2 {
                            self.reset();
                            return Err(CoreError::AncsFrame);
                        }
                        return Ok(None);
                    };
                    if nul > 256 {
                        self.reset();
                        return Err(CoreError::AncsFrame);
                    }
                    self.take(1);
                    let raw = self.take(nul - 1);
                    self.take(1);
                    self.partial = Some(Partial::App {
                        app_id: String::from_utf8_lossy(&raw).into_owned(),
                    });
                }
                other => {
                    // An unknown response id cannot be skipped safely; drop the
                    // buffer rather than mis-parse everything behind it.
                    tracing::debug!(cmd = other, "ancs: unknown data source response");
                    self.reset();
                    return Err(CoreError::AncsFrame);
                }
            }
        }

        if self.buf.len() < 3 {
            return Ok(None);
        }
        let head = self.take(3);
        let attr = head[0];
        let len = u16::from_le_bytes([head[1], head[2]]) as usize;
        if self.buf.len() < len {
            // Put the tuple header back; the value is still in flight.
            for b in head.iter().rev() {
                self.buf.push_front(*b);
            }
            return Ok(None);
        }
        let value = self.take(len);
        match self.partial.clone() {
            Some(Partial::Notification { uid }) => {
                Ok(Some(DataItem::NotificationAttr { uid, attr, value }))
            }
            Some(Partial::App { app_id }) => Ok(Some(DataItem::AppAttr {
                app_id,
                attr,
                value,
            })),
            None => Err(CoreError::AncsFrame),
        }
    }
}

// ------------------------------------------------------- transport port

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AncsCharacteristic {
    DataSource,
    NotificationSource,
}

/// The BLE side of ANCS. Implemented for real by `windows`/CoreBluetooth/BlueZ
/// in the platform layer; [`MemoryAncsTransport`] covers CI.
pub trait AncsTransport: Send + Sync {
    fn subscribe_data_source(&self) -> Result<(), CoreError>;
    fn subscribe_notification_source(&self) -> Result<(), CoreError>;
    /// One Control Point write. The ingress guarantees at most one outstanding
    /// request, so this is never called concurrently.
    fn write_control_point(&self, bytes: &[u8]) -> Result<(), CoreError>;
}

#[derive(Debug, Default)]
struct MemoryAncsState {
    subscribes: Vec<AncsCharacteristic>,
    writes: Vec<Vec<u8>>,
    fail_subscribe: bool,
    fail_write: bool,
}

/// Records every subscribe and Control Point write. Never touches hardware.
#[derive(Debug, Default)]
pub struct MemoryAncsTransport {
    inner: Mutex<MemoryAncsState>,
}

impl MemoryAncsTransport {
    pub fn subscribes(&self) -> Vec<AncsCharacteristic> {
        self.inner
            .lock()
            .expect("ancs transport")
            .subscribes
            .clone()
    }

    pub fn writes(&self) -> Vec<Vec<u8>> {
        self.inner.lock().expect("ancs transport").writes.clone()
    }

    pub fn write_count(&self) -> usize {
        self.inner.lock().expect("ancs transport").writes.len()
    }

    pub fn set_fail_subscribe(&self, fail: bool) {
        self.inner.lock().expect("ancs transport").fail_subscribe = fail;
    }

    pub fn set_fail_write(&self, fail: bool) {
        self.inner.lock().expect("ancs transport").fail_write = fail;
    }

    pub fn clear(&self) {
        let mut g = self.inner.lock().expect("ancs transport");
        g.subscribes.clear();
        g.writes.clear();
    }
}

impl AncsTransport for MemoryAncsTransport {
    fn subscribe_data_source(&self) -> Result<(), CoreError> {
        let mut g = self.inner.lock().expect("ancs transport");
        if g.fail_subscribe {
            return Err(CoreError::AncsSubscribe);
        }
        g.subscribes.push(AncsCharacteristic::DataSource);
        Ok(())
    }

    fn subscribe_notification_source(&self) -> Result<(), CoreError> {
        let mut g = self.inner.lock().expect("ancs transport");
        if g.fail_subscribe {
            return Err(CoreError::AncsSubscribe);
        }
        g.subscribes.push(AncsCharacteristic::NotificationSource);
        Ok(())
    }

    fn write_control_point(&self, bytes: &[u8]) -> Result<(), CoreError> {
        let mut g = self.inner.lock().expect("ancs transport");
        if g.fail_write {
            return Err(CoreError::AncsWrite);
        }
        g.writes.push(bytes.to_vec());
        Ok(())
    }
}

// ------------------------------------------------------------- timing

/// How long we wait for a Data Source response before retrying it once.
pub const CP_RESPONSE_TIMEOUT_MS: u64 = 1_500;
/// How long a notification may wait for its app display name before we emit it
/// with whatever we already know.
pub const APP_NAME_WAIT_MS: u64 = 400;
/// Final deadline for assembling one notification's attributes.
pub const NOTIF_ASSEMBLE_DEADLINE_MS: u64 = 3_000;
/// How long a uid stays deduped. Matches the notify hub's window.
pub const UID_SEEN_WINDOW_MS: u64 = 10 * 60 * 1000;
/// Hard cap on remembered uids.
pub const UID_SEEN_MAX: usize = 4_096;

pub const ANCS_BACKOFF_BASE_MS: u64 = 500;
pub const ANCS_BACKOFF_MAX_MS: u64 = 30_000;
pub const ANCS_BACKOFF_SHIFT_CAP: u32 = 6;

/// Capped exponential backoff between BLE reconnect attempts.
pub fn ancs_backoff_delay(attempt: u32) -> u64 {
    let step = ANCS_BACKOFF_BASE_MS
        .checked_shl(attempt.min(ANCS_BACKOFF_SHIFT_CAP))
        .unwrap_or(ANCS_BACKOFF_MAX_MS);
    step.min(ANCS_BACKOFF_MAX_MS)
}

// ------------------------------------------------------------- ingress

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AncsState {
    Idle,
    Ready,
    Backoff { retry_at_ms: u64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectOutcome {
    Subscribed,
    BackingOff { retry_at_ms: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AncsNotification {
    pub uid: u32,
    pub app_id: String,
    pub app_name: String,
    pub title: String,
    pub body: String,
    pub category: Category,
    pub flags: EventFlags,
    /// The body was longer than what iOS returned, so an OTP may be cut off.
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AncsEvent {
    Added(AncsNotification),
    Modified(AncsNotification),
    Removed { uid: u32 },
}

/// Desktop actions offered for an ANCS notification. `open` appears only when
/// the app is in the local allowlist — a url carried by the notification is
/// never consulted, and never opened (M4.2).
pub fn ancs_actions(app_id: &str, allowlist: &OpenAllowlist) -> Vec<String> {
    let mut actions = vec!["copy".to_string(), "dismiss".to_string()];
    if allowlist.url_for(app_id).is_some() {
        actions.push("open".to_string());
    }
    actions
}

/// Stable synthetic id for the phone behind an ANCS link. ANCS is not a
/// Tetherly session, so the peer has no `id_pk`; we derive one from the BLE
/// peripheral identity instead of inventing a random id per connect.
pub fn ancs_source_id(peripheral: &str) -> DeviceId {
    let mut h = Sha256::new();
    h.update(b"tetherly-ancs-v1");
    h.update(peripheral.as_bytes());
    let digest: [u8; 32] = h.finalize().into();
    DeviceId::from_id_pk(&digest)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CpKind {
    NotificationAttrs,
    AppAttrs,
}

struct CpRequest {
    kind: CpKind,
    bytes: Vec<u8>,
    uid: u32,
    app_id: String,
}

struct InFlight {
    kind: CpKind,
    bytes: Vec<u8>,
    uid: u32,
    app_id: String,
    deadline_ms: u64,
    retried: bool,
}

struct PendingNotif {
    event_id: EventId,
    category: Category,
    flags: EventFlags,
    app_id: String,
    title: Option<String>,
    body: Option<String>,
    message_size: Option<u32>,
    received: usize,
    deadline_ms: u64,
}

struct Held {
    event_id: EventId,
    notif: AncsNotification,
    app_id: String,
    deadline_ms: u64,
}

/// ANCS session state machine. All time comes from the injected [`Clock`], so
/// retry/backoff/deadline behaviour is deterministic under test.
pub struct AncsIngress {
    transport: Arc<dyn AncsTransport>,
    clock: Arc<dyn Clock>,
    state: AncsState,
    backoff_attempt: u32,
    queue: VecDeque<CpRequest>,
    in_flight: Option<InFlight>,
    assembler: DataAssembler,
    pending: HashMap<u32, PendingNotif>,
    held: Vec<Held>,
    app_names: HashMap<String, String>,
    app_req: Option<String>,
    seen: HashMap<u32, u64>,
    events: VecDeque<AncsEvent>,
}

impl AncsIngress {
    pub fn new(transport: Arc<dyn AncsTransport>, clock: Arc<dyn Clock>) -> Self {
        Self {
            transport,
            clock,
            state: AncsState::Idle,
            backoff_attempt: 0,
            queue: VecDeque::new(),
            in_flight: None,
            assembler: DataAssembler::default(),
            pending: HashMap::new(),
            held: Vec::new(),
            app_names: HashMap::new(),
            app_req: None,
            seen: HashMap::new(),
            events: VecDeque::new(),
        }
    }

    pub fn state(&self) -> AncsState {
        self.state
    }

    pub fn queued(&self) -> usize {
        self.queue.len()
    }

    pub fn pending_notifications(&self) -> usize {
        self.pending.len()
    }

    /// Subscribe Data Source first, then Notification Source (§9.4). Returns
    /// `BackingOff` while a previous disconnect's backoff has not elapsed, so a
    /// hot reconnect loop cannot hammer the phone.
    pub fn connect(&mut self) -> Result<ConnectOutcome, CoreError> {
        let now = self.clock.unix_ms();
        if let AncsState::Backoff { retry_at_ms } = self.state {
            if now < retry_at_ms {
                return Ok(ConnectOutcome::BackingOff { retry_at_ms });
            }
        }
        self.transport.subscribe_data_source()?;
        self.transport.subscribe_notification_source()?;
        self.state = AncsState::Ready;
        self.backoff_attempt = 0;
        self.assembler.reset();
        self.in_flight = None;
        self.queue.clear();
        self.pending.clear();
        self.held.clear();
        self.app_req = None;
        Ok(ConnectOutcome::Subscribed)
    }

    /// Peripheral went away. Clears per-connection work but keeps the uid set and
    /// the app-name cache, so the reconnect neither re-pops old notifications
    /// nor loses learned names. Returns the next permitted attempt time.
    pub fn on_disconnected(&mut self) -> u64 {
        let now = self.clock.unix_ms();
        let retry_at = now.saturating_add(ancs_backoff_delay(self.backoff_attempt));
        self.backoff_attempt = self.backoff_attempt.saturating_add(1);
        self.state = AncsState::Backoff {
            retry_at_ms: retry_at,
        };
        self.in_flight = None;
        self.queue.clear();
        self.pending.clear();
        self.held.clear();
        self.app_req = None;
        self.assembler.reset();
        retry_at
    }

    /// Value-changed callback for the Notification Source. Enqueues work only —
    /// writing the Control Point here would break the serial guarantee.
    pub fn on_notification_source(&mut self, value: &[u8]) -> Result<(), CoreError> {
        if self.state != AncsState::Ready {
            return Ok(());
        }
        let ev = NotificationEvent::parse(value)?;
        match ev.event_id {
            EventId::Removed => {
                self.pending.remove(&ev.uid);
                self.events.push_back(AncsEvent::Removed { uid: ev.uid });
            }
            EventId::Added | EventId::Modified => {
                if ev.flags.pre_existing() {
                    return Ok(());
                }
                if self.is_seen(ev.uid) {
                    return Ok(());
                }
                self.mark_seen(ev.uid);
                let now = self.clock.unix_ms();
                self.pending.insert(
                    ev.uid,
                    PendingNotif {
                        event_id: ev.event_id,
                        category: ev.category,
                        flags: ev.flags,
                        app_id: String::new(),
                        title: None,
                        body: None,
                        message_size: None,
                        received: 0,
                        deadline_ms: now.saturating_add(NOTIF_ASSEMBLE_DEADLINE_MS),
                    },
                );
                self.queue.push_back(CpRequest {
                    kind: CpKind::NotificationAttrs,
                    bytes: encode_get_notification_attributes(ev.uid, &NOTIF_ATTR_REQUEST),
                    uid: ev.uid,
                    app_id: String::new(),
                });
            }
            EventId::Unknown(_) => {}
        }
        Ok(())
    }

    /// Value-changed callback for the Data Source. Fragments are accumulated by
    /// byte count; only whole attribute tuples are consumed.
    pub fn on_data_source(&mut self, value: &[u8]) -> Result<(), CoreError> {
        if self.state != AncsState::Ready {
            return Ok(());
        }
        self.assembler.push(value)?;
        let mut progressed = false;
        loop {
            match self.assembler.next_item()? {
                None => break,
                Some(DataItem::NotificationAttr { uid, attr, value }) => {
                    self.absorb_notification_attr(uid, attr, value);
                    progressed = true;
                }
                Some(DataItem::AppAttr {
                    app_id,
                    attr,
                    value,
                }) => {
                    self.absorb_app_attr(&app_id, attr, value);
                    progressed = true;
                }
            }
        }
        if progressed {
            // A slow trickle of fragments is progress, not silence: push the
            // retry deadline out so we never re-request mid-response.
            let now = self.clock.unix_ms();
            if let Some(f) = self.in_flight.as_mut() {
                f.deadline_ms = now.saturating_add(CP_RESPONSE_TIMEOUT_MS);
            }
        }
        Ok(())
    }

    /// Time-driven maintenance. Call from the BLE task, never from a
    /// value-changed callback. Sends at most one Control Point request.
    pub fn tick(&mut self) -> Result<(), CoreError> {
        if self.state != AncsState::Ready {
            return Ok(());
        }
        let now = self.clock.unix_ms();
        self.finalize_expired_pending(now);
        self.release_expired_held(now);

        let stalled = self
            .in_flight
            .as_ref()
            .filter(|f| now >= f.deadline_ms)
            .map(|f| (f.retried, f.uid, f.bytes.clone()));
        if let Some((retried, uid, bytes)) = stalled {
            if retried {
                tracing::debug!(uid, "ancs: giving up on control point request");
                self.in_flight = None;
                self.assembler.end_response();
            } else {
                self.transport.write_control_point(&bytes)?;
                tracing::debug!(uid, "ancs: retrying first attribute response");
                if let Some(f) = self.in_flight.as_mut() {
                    f.retried = true;
                    f.deadline_ms = now.saturating_add(CP_RESPONSE_TIMEOUT_MS);
                }
            }
        }

        if self.in_flight.is_none() {
            if let Some(req) = self.queue.pop_front() {
                self.transport.write_control_point(&req.bytes)?;
                self.in_flight = Some(InFlight {
                    kind: req.kind,
                    bytes: req.bytes,
                    uid: req.uid,
                    app_id: req.app_id,
                    deadline_ms: now.saturating_add(CP_RESPONSE_TIMEOUT_MS),
                    retried: false,
                });
            }
        }
        Ok(())
    }

    pub fn drain_events(&mut self) -> Vec<AncsEvent> {
        self.events.drain(..).collect()
    }

    fn is_seen(&self, uid: u32) -> bool {
        self.seen.contains_key(&uid)
    }

    fn mark_seen(&mut self, uid: u32) {
        let now = self.clock.unix_ms();
        if self.seen.len() >= UID_SEEN_MAX {
            self.seen
                .retain(|_, ts| now.saturating_sub(*ts) <= UID_SEEN_WINDOW_MS);
            if self.seen.len() >= UID_SEEN_MAX {
                self.seen.clear();
            }
        }
        self.seen.insert(uid, now);
    }

    fn absorb_notification_attr(&mut self, uid: u32, attr: u8, value: Vec<u8>) {
        let Some(p) = self.pending.get_mut(&uid) else {
            return;
        };
        p.received += 1;
        match attr {
            ATTR_APP_IDENTIFIER => p.app_id = String::from_utf8_lossy(&value).into_owned(),
            ATTR_TITLE => p.title = Some(String::from_utf8_lossy(&value).into_owned()),
            ATTR_MESSAGE => p.body = Some(String::from_utf8_lossy(&value).into_owned()),
            ATTR_MESSAGE_SIZE if value.len() == 4 => {
                p.message_size = Some(u32::from_le_bytes(
                    value.as_slice().try_into().unwrap_or([0; 4]),
                ));
            }
            _ => {}
        }
        if p.received >= NOTIF_ATTR_REQUEST.len() {
            self.complete_notification(uid);
        }
    }

    fn absorb_app_attr(&mut self, app_id: &str, attr: u8, value: Vec<u8>) {
        if attr != APP_ATTR_DISPLAY_NAME {
            return;
        }
        let name = String::from_utf8_lossy(&value).into_owned();
        self.app_names.insert(app_id.to_string(), name);
        if self.app_req.as_deref() == Some(app_id) {
            self.app_req = None;
        }
        if let Some(f) = self.in_flight.as_ref() {
            if f.kind == CpKind::AppAttrs && f.app_id == app_id {
                self.in_flight = None;
                self.assembler.end_response();
            }
        }
        let now = self.clock.unix_ms();
        let (ready, rest): (Vec<Held>, Vec<Held>) = std::mem::take(&mut self.held)
            .into_iter()
            .partition(|h| h.app_id == app_id);
        self.held = rest;
        for mut h in ready {
            h.notif.app_name = self.app_names.get(app_id).cloned().unwrap_or_default();
            self.emit(h.event_id, h.notif);
        }
        self.release_expired_held(now);
    }

    fn complete_notification(&mut self, uid: u32) {
        let Some(p) = self.pending.remove(&uid) else {
            return;
        };
        if let Some(f) = self.in_flight.as_ref() {
            if f.kind == CpKind::NotificationAttrs && f.uid == uid {
                self.in_flight = None;
            }
        }
        self.assembler.end_response();
        self.finish(uid, p);
    }

    fn finish(&mut self, uid: u32, p: PendingNotif) {
        let body = p.body.unwrap_or_default();
        let truncated = p
            .message_size
            .map(|size| (size as usize) > body.len())
            .unwrap_or(false);
        let cached_name = self.app_names.get(&p.app_id).cloned().unwrap_or_default();
        let notif = AncsNotification {
            uid,
            app_id: p.app_id.clone(),
            app_name: cached_name,
            title: p.title.unwrap_or_default(),
            body,
            category: p.category,
            flags: p.flags,
            truncated,
        };

        let needs_name = !p.app_id.is_empty()
            && !self.app_names.contains_key(&p.app_id)
            && self.app_req.as_deref() != Some(p.app_id.as_str());
        if !needs_name {
            self.emit(p.event_id, notif);
            return;
        }

        // Ask for the display name and hold this notification briefly so the
        // first popup already carries the app name.
        let now = self.clock.unix_ms();
        self.app_req = Some(p.app_id.clone());
        self.queue.push_back(CpRequest {
            kind: CpKind::AppAttrs,
            bytes: encode_get_app_attributes(&p.app_id, &[APP_ATTR_DISPLAY_NAME]),
            uid,
            app_id: p.app_id.clone(),
        });
        self.held.push(Held {
            event_id: p.event_id,
            notif,
            app_id: p.app_id,
            deadline_ms: now.saturating_add(APP_NAME_WAIT_MS),
        });
    }

    fn release_expired_held(&mut self, now: u64) {
        if self.held.is_empty() {
            return;
        }
        let held = std::mem::take(&mut self.held);
        let mut keep = Vec::new();
        for h in held {
            if now >= h.deadline_ms {
                self.emit(h.event_id, h.notif);
            } else {
                keep.push(h);
            }
        }
        self.held = keep;
    }

    fn finalize_expired_pending(&mut self, now: u64) {
        if self.pending.is_empty() {
            return;
        }
        let due: Vec<u32> = self
            .pending
            .iter()
            .filter(|(_, p)| now >= p.deadline_ms)
            .map(|(uid, _)| *uid)
            .collect();
        for uid in due {
            self.complete_notification(uid);
        }
    }

    fn emit(&mut self, event_id: EventId, notif: AncsNotification) {
        match event_id {
            EventId::Modified => self.events.push_back(AncsEvent::Modified(notif)),
            _ => self.events.push_back(AncsEvent::Added(notif)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::ManualClock;

    fn event(id: u8, flags: u8, category: u8, uid: u32) -> Vec<u8> {
        let mut v = vec![id, flags, category, 1];
        v.extend_from_slice(&uid.to_le_bytes());
        v
    }

    fn notif_attr(uid: u32, attr: u8, value: &[u8]) -> Vec<u8> {
        let mut v = vec![CP_GET_NOTIFICATION_ATTRIBUTES];
        v.extend_from_slice(&uid.to_le_bytes());
        v.push(attr);
        v.extend_from_slice(&(value.len() as u16).to_le_bytes());
        v.extend_from_slice(value);
        v
    }

    fn full_response(uid: u32, app: &str, title: &str, body: &str) -> Vec<u8> {
        let mut v = notif_attr(uid, ATTR_APP_IDENTIFIER, app.as_bytes());
        v.extend(
            notif_attr(uid, ATTR_TITLE, title.as_bytes())
                .into_iter()
                .skip(5),
        );
        v.extend(
            notif_attr(uid, ATTR_MESSAGE, body.as_bytes())
                .into_iter()
                .skip(5),
        );
        v.extend(
            notif_attr(uid, ATTR_MESSAGE_SIZE, &(body.len() as u32).to_le_bytes())
                .into_iter()
                .skip(5),
        );
        v
    }

    fn app_response(app: &str, name: &str) -> Vec<u8> {
        let mut v = vec![CP_GET_APP_ATTRIBUTES];
        v.extend_from_slice(app.as_bytes());
        v.push(0);
        v.push(APP_ATTR_DISPLAY_NAME);
        v.extend_from_slice(&(name.len() as u16).to_le_bytes());
        v.extend_from_slice(name.as_bytes());
        v
    }

    fn ingress() -> (AncsIngress, Arc<MemoryAncsTransport>, Arc<ManualClock>) {
        let transport = Arc::new(MemoryAncsTransport::default());
        let clock = Arc::new(ManualClock::new(1_000));
        let ing = AncsIngress::new(transport.clone(), clock.clone());
        (ing, transport, clock)
    }

    /// Advance past the app-name wait and run one maintenance pass.
    fn release(ing: &mut AncsIngress, clock: &ManualClock) {
        clock.set(clock.unix_ms() + APP_NAME_WAIT_MS);
        ing.tick().unwrap();
    }

    #[test]
    fn uuids_decode_little_endian() {
        let bytes = uuid_bytes_le(UUID_NOTIFICATION_SOURCE).unwrap();
        assert_eq!(bytes[0], 0xBD);
        assert_eq!(bytes[15], 0x9F);
        assert!(uuid_bytes_le("nope").is_none());
        assert!(uuid_bytes_le("7905F431-B5CE-4E99-A40F-4B1E122D00D").is_none());
    }

    #[test]
    fn notification_event_parses_and_flags() {
        let ev = NotificationEvent::parse(&event(0, 0x04, 6, 0x0102_0304)).unwrap();
        assert_eq!(ev.event_id, EventId::Added);
        assert!(ev.flags.pre_existing());
        assert!(!ev.flags.silent());
        assert_eq!(ev.category, Category::Email);
        assert_eq!(ev.uid, 0x0102_0304);
        assert!(NotificationEvent::parse(&[0, 0, 0]).is_err());
        assert_eq!(EventId::from_u8(9), EventId::Unknown(9));
        assert_eq!(Category::from_u8(200), Category::Unknown(200));
    }

    #[test]
    fn control_point_encoders_are_exact() {
        let req = encode_get_notification_attributes(7, &NOTIF_ATTR_REQUEST);
        assert_eq!(req, vec![0, 7, 0, 0, 0, 0, 1, 0, 2, 3, 0, 2, 4]);
        let app = encode_get_app_attributes("com.apple.MobileSMS", &[APP_ATTR_DISPLAY_NAME]);
        assert_eq!(app[0], CP_GET_APP_ATTRIBUTES);
        assert_eq!(app[app.len() - 2..], [0, 0]);
        assert_eq!(
            encode_perform_action(7, ACTION_NEGATIVE),
            vec![2, 7, 0, 0, 0, 1]
        );
    }

    #[test]
    fn data_assembler_reassembles_by_bytes() {
        let mut a = DataAssembler::default();
        let full = full_response(5, "com.apple.MobileSMS", "网易", "验证码 868740");
        // Split mid-way through a multi-byte UTF-8 value.
        let (head, tail) = full.split_at(20);
        a.push(head).unwrap();
        assert!(
            a.next_item().unwrap().is_none(),
            "a split value must never be handed out"
        );
        a.push(tail).unwrap();
        let mut got = Vec::new();
        while let Some(item) = a.next_item().unwrap() {
            got.push(item);
        }
        assert_eq!(got.len(), 4);
        match &got[0] {
            DataItem::NotificationAttr { uid, attr, value } => {
                assert_eq!(*uid, 5);
                assert_eq!(*attr, ATTR_APP_IDENTIFIER);
                assert_eq!(
                    String::from_utf8(value.clone()).unwrap(),
                    "com.apple.MobileSMS"
                );
            }
            other => panic!("unexpected {other:?}"),
        }
        match &got[3] {
            DataItem::NotificationAttr { attr, value, .. } => {
                assert_eq!(*attr, ATTR_MESSAGE_SIZE);
                assert_eq!(value.len(), 4);
            }
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(a.pending(), 0);
    }

    #[test]
    fn data_assembler_app_response() {
        let mut a = DataAssembler::default();
        a.push(&app_response("com.apple.MobileSMS", "信息"))
            .unwrap();
        match a.next_item().unwrap().unwrap() {
            DataItem::AppAttr {
                app_id,
                attr,
                value,
            } => {
                assert_eq!(app_id, "com.apple.MobileSMS");
                assert_eq!(attr, APP_ATTR_DISPLAY_NAME);
                assert_eq!(String::from_utf8(value).unwrap(), "信息");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn data_assembler_rejects_unknown_command_and_overlong() {
        let mut a = DataAssembler::default();
        a.push(&[9, 1, 2]).unwrap();
        assert!(a.next_item().is_err());
        assert_eq!(a.pending(), 0);

        let mut b = DataAssembler::default();
        assert!(b.push(&vec![0u8; DATA_BUF_MAX + 1]).is_err());
        assert_eq!(b.pending(), 0);
    }

    #[test]
    fn connect_subscribes_data_source_first() {
        let (mut ing, t, _) = ingress();
        assert_eq!(ing.connect().unwrap(), ConnectOutcome::Subscribed);
        assert_eq!(
            t.subscribes(),
            vec![
                AncsCharacteristic::DataSource,
                AncsCharacteristic::NotificationSource
            ]
        );
        assert_eq!(ing.state(), AncsState::Ready);
    }

    #[test]
    fn notification_source_never_writes_on_the_callback_thread() {
        let (mut ing, t, _) = ingress();
        ing.connect().unwrap();
        ing.on_notification_source(&event(0, 0, 6, 42)).unwrap();
        assert_eq!(t.write_count(), 0, "value-changed must not write");
        assert_eq!(ing.queued(), 1);
        ing.tick().unwrap();
        assert_eq!(t.write_count(), 1);
    }

    #[test]
    fn control_point_is_serial_and_retries_silence_once() {
        let (mut ing, t, clock) = ingress();
        ing.connect().unwrap();
        ing.on_notification_source(&event(0, 0, 6, 1)).unwrap();
        ing.on_notification_source(&event(0, 0, 6, 2)).unwrap();
        ing.tick().unwrap();
        assert_eq!(t.write_count(), 1, "one outstanding request");

        clock.set(1_000 + CP_RESPONSE_TIMEOUT_MS);
        ing.tick().unwrap();
        assert_eq!(t.write_count(), 2, "silent first response retried once");
        clock.set(1_000 + CP_RESPONSE_TIMEOUT_MS * 2);
        ing.tick().unwrap();
        assert_eq!(t.write_count(), 3, "next queued request proceeds");
        let writes = t.writes();
        assert_eq!(writes[0], writes[1]);
        assert_ne!(writes[1], writes[2]);
    }

    #[test]
    fn full_flow_yields_app_name_and_otp_body() {
        let (mut ing, t, clock) = ingress();
        ing.connect().unwrap();
        ing.on_notification_source(&event(0, 0x02, 6, 42)).unwrap();
        ing.tick().unwrap();
        ing.on_data_source(&full_response(
            42,
            "com.apple.MobileSMS",
            "网易",
            "验证码 868740",
        ))
        .unwrap();
        assert!(ing.drain_events().is_empty(), "held until app name");
        ing.tick().unwrap();
        assert_eq!(t.write_count(), 2, "app attributes requested next");
        ing.on_data_source(&app_response("com.apple.MobileSMS", "信息"))
            .unwrap();
        let events = ing.drain_events();
        match &events[..] {
            [AncsEvent::Added(n)] => {
                assert_eq!(n.app_id, "com.apple.MobileSMS");
                assert_eq!(n.app_name, "信息");
                assert_eq!(n.title, "网易");
                assert_eq!(n.body, "验证码 868740");
                assert!(!n.truncated);
            }
            other => panic!("unexpected {other:?}"),
        }
        // Second notification from the same app skips the app-attribute round trip.
        clock.set(clock.unix_ms() + 1);
        ing.on_notification_source(&event(0, 0, 6, 43)).unwrap();
        ing.tick().unwrap();
        ing.on_data_source(&full_response(43, "com.apple.MobileSMS", "网易", "hello"))
            .unwrap();
        match &ing.drain_events()[..] {
            [AncsEvent::Added(n)] => assert_eq!(n.app_name, "信息"),
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(t.write_count(), 3);
    }

    #[test]
    fn held_notification_is_released_when_app_name_never_arrives() {
        let (mut ing, _t, clock) = ingress();
        ing.connect().unwrap();
        ing.on_notification_source(&event(0, 0, 6, 7)).unwrap();
        ing.tick().unwrap();
        ing.on_data_source(&full_response(7, "com.unknown.app", "t", "b"))
            .unwrap();
        assert!(ing.drain_events().is_empty());
        clock.set(1_000 + APP_NAME_WAIT_MS);
        ing.tick().unwrap();
        match &ing.drain_events()[..] {
            [AncsEvent::Added(n)] => {
                assert_eq!(n.app_id, "com.unknown.app");
                assert_eq!(n.app_name, "");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn pre_existing_is_dropped_after_reconnect() {
        let (mut ing, t, clock) = ingress();
        ing.connect().unwrap();
        ing.on_notification_source(&event(0, 0, 6, 100)).unwrap();
        ing.tick().unwrap();
        ing.on_data_source(&full_response(100, "a.b", "t", "b"))
            .unwrap();
        ing.tick().unwrap();
        ing.on_data_source(&app_response("a.b", "App")).unwrap();
        assert_eq!(ing.drain_events().len(), 1);

        // Bluetooth off, then on again: iOS replays everything as PreExisting.
        ing.on_disconnected();
        clock.set(clock.unix_ms() + ancs_backoff_delay(0));
        assert_eq!(ing.connect().unwrap(), ConnectOutcome::Subscribed);
        t.clear();
        ing.on_notification_source(&event(0, 0x04, 6, 100)).unwrap();
        ing.on_notification_source(&event(0, 0x04, 6, 101)).unwrap();
        assert!(ing.drain_events().is_empty());
        ing.tick().unwrap();
        assert_eq!(
            t.write_count(),
            0,
            "no attribute fetch for old notifications"
        );
        // A genuinely new notification still flows.
        ing.on_notification_source(&event(0, 0, 6, 102)).unwrap();
        ing.tick().unwrap();
        assert_eq!(t.write_count(), 1);
    }

    #[test]
    fn duplicate_uid_is_ignored_and_modified_does_not_re_pop() {
        let (mut ing, t, clock) = ingress();
        ing.connect().unwrap();
        ing.on_notification_source(&event(0, 0, 6, 5)).unwrap();
        ing.tick().unwrap();
        ing.on_data_source(&full_response(5, "a.b", "t", "b"))
            .unwrap();
        release(&mut ing, &clock);
        assert_eq!(ing.drain_events().len(), 1);
        let before = t.write_count();
        ing.on_notification_source(&event(0, 0, 6, 5)).unwrap();
        ing.on_notification_source(&event(1, 0, 6, 5)).unwrap();
        ing.tick().unwrap();
        assert_eq!(t.write_count(), before);
        assert!(ing.drain_events().is_empty());
    }

    #[test]
    fn removed_event_is_forwarded() {
        let (mut ing, _t, _) = ingress();
        ing.connect().unwrap();
        ing.on_notification_source(&event(2, 0, 6, 9)).unwrap();
        assert_eq!(ing.drain_events(), vec![AncsEvent::Removed { uid: 9 }]);
    }

    #[test]
    fn events_before_connect_are_ignored() {
        let (mut ing, t, _) = ingress();
        ing.on_notification_source(&event(0, 0, 6, 1)).unwrap();
        ing.on_data_source(&full_response(1, "a.b", "t", "b"))
            .unwrap();
        ing.tick().unwrap();
        assert_eq!(t.write_count(), 0);
        assert!(ing.drain_events().is_empty());
    }

    #[test]
    fn backoff_grows_then_caps() {
        assert_eq!(ancs_backoff_delay(0), 500);
        assert_eq!(ancs_backoff_delay(1), 1_000);
        assert_eq!(ancs_backoff_delay(6), 30_000);
        assert_eq!(ancs_backoff_delay(50), 30_000);
    }

    #[test]
    fn reconnect_inside_backoff_is_refused() {
        let (mut ing, t, clock) = ingress();
        ing.connect().unwrap();
        let retry_at = ing.on_disconnected();
        assert_eq!(
            ing.state(),
            AncsState::Backoff {
                retry_at_ms: retry_at
            }
        );
        t.clear();
        assert_eq!(
            ing.connect().unwrap(),
            ConnectOutcome::BackingOff {
                retry_at_ms: retry_at
            }
        );
        assert_eq!(t.subscribes().len(), 0);
        clock.set(retry_at);
        assert_eq!(ing.connect().unwrap(), ConnectOutcome::Subscribed);
        assert_eq!(t.subscribes().len(), 2);
    }

    #[test]
    fn assemble_deadline_finalizes_partial_attributes() {
        let (mut ing, _t, clock) = ingress();
        ing.connect().unwrap();
        ing.on_notification_source(&event(0, 0, 6, 3)).unwrap();
        ing.tick().unwrap();
        ing.on_data_source(&notif_attr(3, ATTR_APP_IDENTIFIER, b"a.b"))
            .unwrap();
        clock.set(1_000 + NOTIF_ASSEMBLE_DEADLINE_MS);
        ing.tick().unwrap();
        release(&mut ing, &clock);
        match &ing.drain_events()[..] {
            [AncsEvent::Added(n)] => {
                assert_eq!(n.app_id, "a.b");
                assert_eq!(n.title, "");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn truncated_body_is_flagged() {
        let (mut ing, _t, clock) = ingress();
        ing.connect().unwrap();
        ing.on_notification_source(&event(0, 0, 6, 11)).unwrap();
        ing.tick().unwrap();
        let mut v = notif_attr(11, ATTR_APP_IDENTIFIER, b"a.b");
        v.extend(notif_attr(11, ATTR_TITLE, b"t").into_iter().skip(5));
        v.extend(notif_attr(11, ATTR_MESSAGE, b"short").into_iter().skip(5));
        v.extend(
            notif_attr(11, ATTR_MESSAGE_SIZE, &(900u32).to_le_bytes())
                .into_iter()
                .skip(5),
        );
        ing.on_data_source(&v).unwrap();
        release(&mut ing, &clock);
        match &ing.drain_events()[..] {
            [AncsEvent::Added(n)] => assert!(n.truncated),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn subscribe_failure_propagates() {
        let (mut ing, t, _) = ingress();
        t.set_fail_subscribe(true);
        assert_eq!(ing.connect(), Err(CoreError::AncsSubscribe));
        assert_eq!(ing.state(), AncsState::Idle);
    }

    #[test]
    fn actions_gate_open_on_the_allowlist() {
        let list = OpenAllowlist::from_rules(vec![crate::allowlist::OpenRule {
            app_id: "com.apple.MobileSMS".into(),
            url: "weixin://".into(),
        }])
        .unwrap();
        assert_eq!(
            ancs_actions("com.apple.MobileSMS", &list),
            vec!["copy", "dismiss", "open"]
        );
        assert_eq!(
            ancs_actions("com.apple.MobileSMS.evil", &list),
            vec!["copy", "dismiss"]
        );
        assert_eq!(
            ancs_actions("x", &OpenAllowlist::default()),
            vec!["copy", "dismiss"]
        );
    }

    #[test]
    fn source_id_is_stable_per_peripheral() {
        assert_eq!(ancs_source_id("abc"), ancs_source_id("abc"));
        assert_ne!(ancs_source_id("abc"), ancs_source_id("abd"));
    }
}
