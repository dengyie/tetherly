// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Versioned control-client contract (ADR-139).
//!
//! Every type a client sees on `/api/v1` lives here and nowhere else. The
//! daemon converts its internal reports into these DTOs; the desktop/web
//! client consumes TypeScript generated from them (see the `bindings` test,
//! which fails when `client/src/lib/api/gen/api.ts` drifts). This crate must
//! not depend on `tetherly-node` internals.

use serde::{ser::SerializeStruct, Deserialize, Serialize};

/// Major version of the `/api/v1` contract. Breaking changes bump this and
/// move to a new path prefix.
pub const API_VERSION: u32 = 1;

/// Path prefix all v1 routes live under.
pub const PREFIX: &str = "/api/v1";

/// Response header carrying [`API_VERSION`].
pub const API_HEADER: &str = "Tetherly-Api";

/// Response header carrying the daemon's package version.
pub const VERSION_HEADER: &str = "Tetherly-Version";

/// Machine-readable error codes. Clients branch on these, never on message
/// text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    BadRequest,
    Unauthorized,
    Forbidden,
    NotFound,
    MethodNotAllowed,
    Conflict,
    /// The action has to run in the user's interactive session (e.g. open a
    /// file); a SYSTEM daemon refuses it and the client performs it locally.
    NeedsInteractiveSession,
    PayloadTooLarge,
    Internal,
    Unavailable,
}

impl ErrorCode {
    pub const ALL: [ErrorCode; 10] = [
        ErrorCode::BadRequest,
        ErrorCode::Unauthorized,
        ErrorCode::Forbidden,
        ErrorCode::NotFound,
        ErrorCode::MethodNotAllowed,
        ErrorCode::Conflict,
        ErrorCode::NeedsInteractiveSession,
        ErrorCode::PayloadTooLarge,
        ErrorCode::Internal,
        ErrorCode::Unavailable,
    ];

    /// HTTP status the daemon answers with for this code.
    pub fn http_status(self) -> u16 {
        match self {
            ErrorCode::BadRequest => 400,
            ErrorCode::Unauthorized => 401,
            ErrorCode::Forbidden => 403,
            ErrorCode::NotFound => 404,
            ErrorCode::MethodNotAllowed => 405,
            ErrorCode::Conflict | ErrorCode::NeedsInteractiveSession => 409,
            ErrorCode::PayloadTooLarge => 413,
            ErrorCode::Internal => 500,
            ErrorCode::Unavailable => 503,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ApiError {
    pub code: ErrorCode,
    pub message: String,
}

impl ApiError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

/// Body of every non-2xx v1 response: `{"error":{"code":…,"message":…}}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ErrorBody {
    pub error: ApiError,
}

impl ErrorBody {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            error: ApiError::new(code, message),
        }
    }
}

/// `GET /api/v1/health` — the only unauthenticated v1 route. Lets a client
/// tell "daemon down" from "token wrong" and check API compatibility.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct Health {
    pub ok: bool,
    pub api: u32,
    pub version: String,
}

impl Health {
    pub fn new(version: impl Into<String>) -> Self {
        Self {
            ok: true,
            api: API_VERSION,
            version: version.into(),
        }
    }
}

/// Listening ports of the local node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct Ports {
    pub control: u16,
    pub file: u16,
    pub input: u16,
    pub screen: u16,
    pub ui: u16,
}

/// Peer counts shown in the overview header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct PeerCounts {
    #[cfg_attr(test, ts(type = "number"))]
    pub live: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub trusted: u64,
}

/// `GET /api/v1/system` — identity, version and capabilities of this node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct System {
    pub device_id: String,
    pub name: String,
    pub platform: String,
    pub version: String,
    pub api: u32,
    pub pid: u32,
    #[cfg_attr(test, ts(type = "number"))]
    pub uptime_secs: u64,
    pub ports: Ports,
    pub peers: PeerCounts,
    /// Wire capabilities this node advertises to peers.
    pub caps: Vec<String>,
    /// Locally enabled feature flags.
    pub features: Vec<String>,
    /// §145: no overlay sidecar is up — LAN-only operation. Drives the CLI
    /// status line and the SPA badge.
    pub lan_only: bool,
}

/// How a live device is reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum LinkPath {
    Lan,
    Overlay,
}

/// One device this node knows about: trusted, live, discovered or blocked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct Device {
    pub id: String,
    /// Best display name: live name, else discovered name, else alias, else id.
    pub name: String,
    pub alias: Option<String>,
    pub platform: Option<String>,
    pub device_type: String,
    pub online: bool,
    pub trusted: bool,
    pub blocked: bool,
    pub path: Option<LinkPath>,
    pub addr: Option<String>,
    #[cfg_attr(test, ts(type = "number | null"))]
    pub rtt_ms: Option<u64>,
    #[cfg_attr(test, ts(type = "number | null"))]
    pub last_seen_ms: Option<u64>,
    pub caps: Vec<String>,
    pub features: Vec<String>,
    pub tags: Vec<String>,
    /// PeerProfile group (§144): the CLI peers inventory and SPA group-by
    /// view; `None` when the device has no profile or no group set.
    pub group: Option<String>,
}

/// Pairing state. The PIN itself is never pushed on the event stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct Pairing {
    pub pin_active: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub pin_expires_in_secs: u64,
}

/// Who holds the cursor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct Input {
    pub seat: String,
    pub channel_open: bool,
    pub peer: Option<String>,
}

/// Full client-visible state; the first frame of every event stream.
/// Sections are added as resources land; clients ignore unknown fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct Snapshot {
    pub system: System,
    pub devices: Vec<Device>,
    pub pairing: Pairing,
    pub input: Input,
}

/// Event-stream topics (`?topics=a,b`). Also the resource an
/// [`Event::Invalidated`] tells the client to refetch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum Topic {
    Devices,
    Pairing,
    Input,
    System,
    Layout,
    Transfers,
    Inbox,
    Activity,
    Clipboard,
    Settings,
}

impl Topic {
    pub const ALL: [Topic; 10] = [
        Topic::Devices,
        Topic::Pairing,
        Topic::Input,
        Topic::System,
        Topic::Layout,
        Topic::Transfers,
        Topic::Inbox,
        Topic::Activity,
        Topic::Clipboard,
        Topic::Settings,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Topic::Devices => "devices",
            Topic::Pairing => "pairing",
            Topic::Input => "input",
            Topic::System => "system",
            Topic::Layout => "layout",
            Topic::Transfers => "transfers",
            Topic::Inbox => "inbox",
            Topic::Activity => "activity",
            Topic::Clipboard => "clipboard",
            Topic::Settings => "settings",
        }
    }

    pub fn parse(s: &str) -> Option<Topic> {
        Topic::ALL.into_iter().find(|t| t.as_str() == s)
    }

    /// Parse a comma-separated list; `Err` carries the first unknown name.
    pub fn parse_list(s: &str) -> Result<Vec<Topic>, String> {
        s.split(',')
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(|t| Topic::parse(t).ok_or_else(|| t.to_string()))
            .collect()
    }
}

/// One event on `/api/v1/events`. State-bearing events carry the resource's
/// full current value (clients replace, never merge); `invalidated` asks the
/// client to refetch a resource that has no pushed form yet.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(tag = "type")]
pub enum Event {
    #[serde(rename = "snapshot")]
    Snapshot { snapshot: Snapshot },
    #[serde(rename = "device.updated")]
    DeviceUpdated { device: Device },
    #[serde(rename = "device.removed")]
    DeviceRemoved { id: String },
    #[serde(rename = "pairing.changed")]
    PairingChanged { pairing: Pairing },
    #[serde(rename = "input.changed")]
    InputChanged { input: Input },
    #[serde(rename = "system.changed")]
    SystemChanged { system: System },
    #[serde(rename = "transfers.changed")]
    TransfersChanged { transfers: Transfers },
    #[serde(rename = "inbox.changed")]
    InboxChanged { inbox: Inbox },
    #[serde(rename = "layout.changed")]
    LayoutChanged { layout: Layout },
    #[serde(rename = "settings.changed")]
    SettingsChanged { settings: Settings },
    #[serde(rename = "activity.changed")]
    ActivityChanged { activity: Activity },
    #[serde(rename = "invalidated")]
    Invalidated { topic: Topic },
}

impl Event {
    /// Wire name: the `type` tag and the SSE `event:` field.
    pub fn name(&self) -> &'static str {
        match self {
            Event::Snapshot { .. } => "snapshot",
            Event::DeviceUpdated { .. } => "device.updated",
            Event::DeviceRemoved { .. } => "device.removed",
            Event::PairingChanged { .. } => "pairing.changed",
            Event::InputChanged { .. } => "input.changed",
            Event::SystemChanged { .. } => "system.changed",
            Event::TransfersChanged { .. } => "transfers.changed",
            Event::InboxChanged { .. } => "inbox.changed",
            Event::LayoutChanged { .. } => "layout.changed",
            Event::SettingsChanged { .. } => "settings.changed",
            Event::ActivityChanged { .. } => "activity.changed",
            Event::Invalidated { .. } => "invalidated",
        }
    }

    /// Topic used for `?topics=` filtering; `None` (snapshot) always passes.
    pub fn topic(&self) -> Option<Topic> {
        match self {
            Event::Snapshot { .. } => None,
            Event::DeviceUpdated { .. } | Event::DeviceRemoved { .. } => Some(Topic::Devices),
            Event::PairingChanged { .. } => Some(Topic::Pairing),
            Event::InputChanged { .. } => Some(Topic::Input),
            Event::SystemChanged { .. } => Some(Topic::System),
            Event::TransfersChanged { .. } => Some(Topic::Transfers),
            Event::InboxChanged { .. } => Some(Topic::Inbox),
            Event::LayoutChanged { .. } => Some(Topic::Layout),
            Event::SettingsChanged { .. } => Some(Topic::Settings),
            Event::ActivityChanged { .. } => Some(Topic::Activity),
            Event::Invalidated { topic } => Some(*topic),
        }
    }
}

/// SSE `data:` payload: `{"seq":…,"type":…,"data":<bare payload>}`. `seq` is
/// also the SSE `id:` and strictly increases within one daemon run; a run
/// starts above any earlier run's range, so a stale `Last-Event-ID` resyncs
/// via snapshot. `data` is uniform across every event name so consumers do a
/// single dumb unwrap instead of per-variant field lookup.
impl EventEnvelope {
    fn tagged(&self) -> Result<serde_json::Value, serde_json::Error> {
        serde_json::to_value(&self.event)
    }
}

impl Serialize for EventEnvelope {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let event = self.tagged().map_err(serde::ser::Error::custom)?;
        // The tagged event is `{type, <payload fields>}`; strip `type` and the
        // rest becomes the uniform bare `data` payload.
        let mut obj = event;
        let ty = match obj {
            serde_json::Value::Object(ref mut map) => map.remove("type"),
            _ => None,
        }
        .unwrap_or(serde_json::Value::Null);
        let mut st = s.serialize_struct("EventEnvelope", 3)?;
        st.serialize_field("seq", &self.seq)?;
        st.serialize_field("type", &ty)?;
        st.serialize_field("data", &obj)?;
        st.end()
    }
}

impl<'de> Deserialize<'de> for EventEnvelope {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(serde::Deserialize)]
        struct Raw {
            seq: u64,
            #[serde(rename = "type")]
            ty: String,
            data: serde_json::Value,
        }
        let raw = Raw::deserialize(d)?;
        let mut obj = match raw.data {
            serde_json::Value::Null => serde_json::Value::Object(Default::default()),
            v @ serde_json::Value::Object(_) => v,
            other => serde_json::json!({ "value": other }),
        };
        let Some(map) = obj.as_object_mut() else {
            return Err(serde::de::Error::custom("data must be an object"));
        };
        map.insert("type".into(), raw.ty.into());
        let event = serde_json::from_value(obj).map_err(serde::de::Error::custom)?;
        Ok(Self {
            seq: raw.seq,
            event,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct EventEnvelope {
    #[cfg_attr(test, ts(type = "number"))]
    pub seq: u64,
    // Serialized by hand (see the impl above): the wire carries the bare
    // payload, not the nested event enum.
    #[cfg_attr(test, ts(type = "unknown"))]
    pub event: Event,
}

/// `PATCH /devices/{id}`: only the fields present change.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct DevicePatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub alias: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub tags: Option<Vec<String>>,
}

impl DevicePatch {
    pub fn is_empty(&self) -> bool {
        self.alias.is_none() && self.tags.is_none()
    }
}

/// `POST /devices/{id}/connect`: optional explicit `ip:port` to dial.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct ConnectRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub addr: Option<String>,
}

/// `POST /devices/{id}/block`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct BlockRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub reason: Option<String>,
}

/// The local pairing PIN (`GET`/`POST /pairing/pin`). Fetched on demand,
/// never pushed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct PinGrant {
    pub pin: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub expires_in_secs: u64,
}

/// Hits on one frozen `/api/*` route, by caller class (ADR-139 §9).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct LegacyRouteHits {
    pub method: String,
    pub route: String,
    /// `cli` | `browser` | `android` | `curl` | `other`.
    pub caller: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub count: u64,
}

/// `GET /diagnostics`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct Diagnostics {
    /// Current event-stream sequence number.
    #[cfg_attr(test, ts(type = "number"))]
    pub event_seq: u64,
    pub legacy_routes: Vec<LegacyRouteHits>,
}

/// `GET /diagnostics/logs?tail=&level=&grep=`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct LogTail {
    pub lines: Vec<String>,
}

/// Active log filter (`PATCH /diagnostics/log-level` response).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct LogLevel {
    pub filter: String,
    pub baseline: String,
    #[cfg_attr(test, ts(type = "number | null"))]
    pub revert_at_ms: Option<u64>,
}

/// `PATCH /diagnostics/log-level`: set `filter` (optionally auto-reverting)
/// or `reset` to the baseline.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct LogLevelRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub filter: Option<String>,
    #[serde(default)]
    #[cfg_attr(test, ts(as = "Option<bool>", optional))]
    pub reset: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub revert_after_secs: Option<u64>,
}

// ---- M2c: transfers / inbox / layout / settings ----

/// Direction of one file transfer relative to this node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum TransferDirection {
    Inbound,
    Outbound,
}

/// Lifecycle phase of one file transfer (mirrors the daemon's report).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum TransferPhase {
    Transferring,
    Verifying,
    Completed,
    Failed,
}

/// One file transfer, active or terminal (§4.2 `GET /transfers`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct Transfer {
    pub transfer_id: String,
    /// Peer device id (contract device id string, not the raw pubkey id).
    pub peer: String,
    pub direction: TransferDirection,
    pub filename: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub total: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub transferred: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub speed_bps: u64,
    pub phase: TransferPhase,
    pub error: Option<String>,
    #[cfg_attr(test, ts(type = "number"))]
    pub started_ms: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub updated_ms: u64,
}

/// The transfers resource: counts plus the full list (active and terminal).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct Transfers {
    #[cfg_attr(test, ts(type = "number"))]
    pub active_count: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub terminal_count: u64,
    pub transfers: Vec<Transfer>,
    /// §145: session aggregate that rode on the legacy /api/state — moved
    /// onto the transfers resource it belongs to.
    pub stats: TransferStats,
}

/// §145: session aggregate transfer metrics across inbound and outbound
/// files (legacy `/api/state` `transfer_stats` shape, field-for-field).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct TransferStats {
    #[cfg_attr(test, ts(type = "number"))]
    pub bytes_sent: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub bytes_received: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub completed_count: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub failed_count: u64,
}

/// §145: `GET /diagnostics/sys` — process and host telemetry (§4.2 puts
/// 内存/流量 under diagnostics). Field names mirror the legacy `/api/sys`
/// report so the CLI's human printer needs no reshaping.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct SysTelemetry {
    pub ok: bool,
    pub node_name: String,
    pub device_id: String,
    pub process: SysProcess,
    pub host: SysHost,
    #[cfg_attr(test, ts(type = "number"))]
    pub timestamp_ms: u64,
}

/// §145: one process sample (legacy `/api/sys` `process` object).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct SysProcess {
    #[cfg_attr(test, ts(type = "number"))]
    pub pid: u32,
    #[cfg_attr(test, ts(type = "number"))]
    pub uptime_secs: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub rss_bytes: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub virt_bytes: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub thread_count: u32,
    #[cfg_attr(test, ts(type = "number | null"))]
    pub open_fds: Option<u32>,
    #[cfg_attr(test, ts(type = "number"))]
    pub cpu_time_ms: u64,
    pub memory_rss_formatted: String,
    pub memory_virt_formatted: String,
}

/// §145: host hardware/OS profile (legacy `/api/sys` `host` object).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct SysHost {
    pub os: String,
    pub arch: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub num_cpus: usize,
    #[cfg_attr(test, ts(type = "number | null"))]
    pub total_memory_bytes: Option<u64>,
    pub total_memory_formatted: Option<String>,
}

/// One file waiting in the incoming inbox.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct InboxFile {
    pub name: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub size: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub modified_at_secs: u64,
    pub is_safe: bool,
}

/// The inbox resource (§4.2 `GET /inbox`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct Inbox {
    #[cfg_attr(test, ts(type = "number"))]
    pub count: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub total_bytes: u64,
    pub files: Vec<InboxFile>,
}

/// One edge of the local display and the peer that lives across it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct LayoutEdge {
    /// Edge of the local display: `left | right | top | bottom`.
    pub edge: String,
    pub peer: Option<String>,
}

/// The layout resource (§4.2 `GET /layout`): the local display rectangle,
/// one entry per configured edge, and the crossing config knobs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct Layout {
    pub display: DisplayRect,
    pub edges: Vec<LayoutEdge>,
    pub config: CrossingConfig,
    pub policy: LayoutPolicy,
}

/// Local display rectangle: virtual-desktop resolution and scale.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct DisplayRect {
    pub width: u32,
    pub height: u32,
    pub scale_factor: f32,
}

/// Crossing knobs for edge switching, straight from the daemon topology.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct CrossingConfig {
    /// Push resistance in px before the seat crosses.
    #[cfg_attr(test, ts(type = "number"))]
    pub resistance_px: u32,
    /// Dwell time in ms at the edge before a slow push crosses.
    #[cfg_attr(test, ts(type = "number"))]
    pub delay_ms: u32,
    /// Dead-zone size in px around corners where crossing is locked.
    #[cfg_attr(test, ts(type = "number"))]
    pub corner_lock_size_px: u32,
    /// Whether the seat wraps from the last edge back to the first.
    pub wrap_around: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub redundancy_band_px: u32,
    #[cfg_attr(test, ts(type = "number"))]
    pub slow_dwell_ms: u32,
    #[cfg_attr(test, ts(type = "number"))]
    pub fast_push_delta_px: u32,
    #[cfg_attr(test, ts(type = "number"))]
    pub entry_return_grace_ms: u32,
    #[cfg_attr(test, ts(type = "number"))]
    pub remote_speed_percent: u32,
}

/// Topology policy: who may attach and how links form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct LayoutPolicy {
    pub enabled: bool,
    pub allow_inbound_control: bool,
    pub auto_link_paired_peers: bool,
}

/// The settings resource (§4.2 `GET /settings`): grouped read-only view of
/// the daemon config and policies. Writes land as `PATCH /settings` later.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct Settings {
    pub general: SettingsGeneral,
    pub transfer: SettingsTransfer,
}

/// Who renders the desktop tray icon (§6.3 tray takeover). `daemon` is the
/// built-in tray; `client` is the Tauri shell's tray. When `client` is set
/// the daemon hides its built-in tray and reclaims it if the shell's SSE
/// link stays silent for `TRAY_TAKEOVER_RECLAIM_MS`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum TrayOwner {
    #[default]
    Daemon,
    Client,
}

/// How long a claimed (`tray_owner: client`) tray may go without a shell
/// keepalive before the daemon restores the built-in tray (§6.3: 10 s).
pub const TRAY_TAKEOVER_RECLAIM_MS: u64 = 10_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct SettingsGeneral {
    pub name: String,
    pub advertise: bool,
    pub reconnect: bool,
    pub loopback_only: bool,
    pub overlay_enabled: bool,
    /// §6.3 tray takeover state; read-only on this resource (write via the
    /// `tray_owner` field of `SettingsPatch`).
    #[serde(default)]
    pub tray_owner: TrayOwner,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct SettingsTransfer {
    /// Master toggle of the granular auto-accept policy.
    pub auto_accept: bool,
}

// ---- M2d: write requests (transfers / inbox / settings) ----

/// `PATCH /settings`: every field optional; omitted fields stay untouched.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct SettingsPatch {
    #[cfg_attr(test, ts(optional))]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[cfg_attr(test, ts(optional))]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub advertise: Option<bool>,
    #[cfg_attr(test, ts(optional))]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reconnect: Option<bool>,
    #[cfg_attr(test, ts(optional))]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_accept: Option<bool>,
    /// §6.3 tray takeover: `client` hides the built-in tray (the shell owns
    /// the tray surface and must keep SSE + keepalive alive); `daemon`
    /// restores it.
    #[cfg_attr(test, ts(optional))]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tray_owner: Option<TrayOwner>,
}

impl SettingsPatch {
    /// `true` when the patch sets no field.
    pub fn is_empty(&self) -> bool {
        self.name.is_none()
            && self.advertise.is_none()
            && self.reconnect.is_none()
            && self.auto_accept.is_none()
            && self.tray_owner.is_none()
    }
}

/// `POST /inbox/open`: reveal one inbox file with the OS opener.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct InboxOpenRequest {
    pub name: String,
}

// ---- M2e: activity feed ----

/// Direction of a peer direct message relative to this node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum MessageDirection {
    Inbound,
    Outbound,
}

/// Lifecycle status of a resource handoff entry (mirrors the daemon's).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum HandoffStatus {
    Pending,
    Opened,
    Dismissed,
    Rejected,
}

/// One notification candidate (ANCS banner, possibly carrying an OTP).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ActivityCandidate {
    pub id: String,
    pub source: String,
    pub app_id: String,
    pub app_name: String,
    pub uid: String,
    pub title: String,
    pub has_otp: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub created_ms: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub expires_ms: u64,
    pub actions: Vec<String>,
}

/// One quick reply waiting to be picked up (no text — the no-content rule).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ActivityReply {
    pub id: String,
    pub source: String,
    pub app_id: String,
    pub uid: String,
}

/// One peer direct message (text included; both directions).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ActivityMessage {
    pub id: String,
    pub peer: String,
    pub direction: MessageDirection,
    pub text: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub timestamp_ms: u64,
    pub read: bool,
}

/// One resource handoff entry (URL handed over from a peer).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ActivityHandoff {
    pub id: String,
    pub sender_id: String,
    pub sender_alias: String,
    pub url: String,
    pub title: String,
    pub status: HandoffStatus,
    #[cfg_attr(test, ts(type = "number"))]
    pub received_ms: u64,
    #[cfg_attr(test, ts(type = "number | null"))]
    pub opened_ms: Option<u64>,
}

/// The activity resource (§4.2 `GET /activity`): the four pending/recent
/// feeds the SPA activity page renders, in one full-state event.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct Activity {
    pub candidates: Vec<ActivityCandidate>,
    pub replies: Vec<ActivityReply>,
    pub messages: Vec<ActivityMessage>,
    pub handoffs: Vec<ActivityHandoff>,
}

#[cfg(test)]
mod bindings {
    //! TypeScript drift check. Regenerate with
    //! `TETHERLY_API_BLESS=1 cargo test -p tetherly-api bindings`.
    use super::*;
    use ts_rs::{Config, TS};

    fn decl<T: TS>(cfg: &Config) -> String {
        format!("export {}\n", T::decl(cfg))
    }

    fn render() -> String {
        let cfg = Config::new();
        let mut out = String::from(
            "// GENERATED by crates/tetherly-api (ts-rs). Do not edit.\n\
             // Regenerate: TETHERLY_API_BLESS=1 cargo test -p tetherly-api bindings\n\n",
        );
        out.push_str(&format!("export const API_VERSION = {API_VERSION};\n"));
        out.push_str(&format!("export const API_PREFIX = \"{PREFIX}\";\n\n"));
        for d in [
            decl::<ErrorCode>(&cfg),
            decl::<ApiError>(&cfg),
            decl::<ErrorBody>(&cfg),
            decl::<Health>(&cfg),
            decl::<Ports>(&cfg),
            decl::<PeerCounts>(&cfg),
            decl::<System>(&cfg),
            decl::<LinkPath>(&cfg),
            decl::<Device>(&cfg),
            decl::<Pairing>(&cfg),
            decl::<Input>(&cfg),
            decl::<Snapshot>(&cfg),
            decl::<Topic>(&cfg),
            decl::<Event>(&cfg),
            decl::<EventEnvelope>(&cfg),
            decl::<DevicePatch>(&cfg),
            decl::<ConnectRequest>(&cfg),
            decl::<BlockRequest>(&cfg),
            decl::<PinGrant>(&cfg),
            decl::<LegacyRouteHits>(&cfg),
            decl::<Diagnostics>(&cfg),
            decl::<LogTail>(&cfg),
            decl::<LogLevel>(&cfg),
            decl::<LogLevelRequest>(&cfg),
            decl::<TransferDirection>(&cfg),
            decl::<TransferPhase>(&cfg),
            decl::<Transfer>(&cfg),
            decl::<Transfers>(&cfg),
            decl::<InboxFile>(&cfg),
            decl::<Inbox>(&cfg),
            decl::<LayoutEdge>(&cfg),
            decl::<Layout>(&cfg),
            decl::<DisplayRect>(&cfg),
            decl::<CrossingConfig>(&cfg),
            decl::<LayoutPolicy>(&cfg),
            decl::<Settings>(&cfg),
            decl::<SettingsGeneral>(&cfg),
            decl::<SettingsTransfer>(&cfg),
            decl::<SettingsPatch>(&cfg),
            decl::<InboxOpenRequest>(&cfg),
            decl::<MessageDirection>(&cfg),
            decl::<HandoffStatus>(&cfg),
            decl::<ActivityCandidate>(&cfg),
            decl::<ActivityReply>(&cfg),
            decl::<ActivityMessage>(&cfg),
            decl::<ActivityHandoff>(&cfg),
            decl::<Activity>(&cfg),
        ] {
            out.push_str(&d);
            out.push('\n');
        }
        out
    }

    #[test]
    fn typescript_bindings_are_current() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../client/src/lib/api/gen/api.ts");
        let want = render();
        if std::env::var_os("TETHERLY_API_BLESS").is_some() {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, &want).unwrap();
            return;
        }
        let have = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(
            have == want,
            "{} is stale; run `TETHERLY_API_BLESS=1 cargo test -p tetherly-api bindings`",
            path.display()
        );
    }
}
