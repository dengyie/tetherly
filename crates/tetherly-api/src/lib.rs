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
    /// Why the device is blocked (§151): the CLI blocked inventory and SPA
    /// blocklist; `None` when the device is not blocked or was blocked
    /// without a reason.
    #[cfg_attr(test, ts(optional))]
    pub block_reason: Option<String>,
}

/// Pairing state. The PIN itself is never pushed on the event stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct Pairing {
    pub pin_active: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub pin_expires_in_secs: u64,
}

/// §154: `POST /pairing/dial` body — dial a peer by direct address or a
/// discovered name/id, optionally priming the pairing PIN first. Exactly one
/// of `addr`/`peer` must be set; the empty-request shape is a handler 400.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct DialRequest {
    pub addr: Option<String>,
    pub peer: Option<String>,
    pub pin: Option<String>,
}

/// §154: dial result — mirrored from the legacy `/api/dial` reply shape
/// (`{ok, addr}`) so CLI consumers are unchanged; `addr` is the resolved
/// socket the session actually dialed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct DialReport {
    pub ok: bool,
    pub addr: String,
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

/// §152: dynamic heartbeat keepalive configuration, mirrored from the
/// runtime `HeartbeatConfig` (interval/deadline in seconds). Lives on
/// `GET /link/heartbeat`; write via `HeartbeatPatch`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct HeartbeatConfig {
    pub interval_secs: u64,
    pub deadline_secs: u64,
}

/// §152: `PATCH /link/heartbeat` — every field optional; omitted fields
/// stay untouched.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct HeartbeatPatch {
    #[cfg_attr(test, ts(optional))]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interval_secs: Option<u64>,
    #[cfg_attr(test, ts(optional))]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline_secs: Option<u64>,
}

impl HeartbeatPatch {
    /// `true` when the patch sets no field.
    pub fn is_empty(&self) -> bool {
        self.interval_secs.is_none() && self.deadline_secs.is_none()
    }
}

/// §152: sliding-window link quality profile for one peer connection,
/// mirrored from the runtime `PeerLinkQualityDto` field names so the CLI
/// table printer and JSON consumers stay byte-compatible.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct LinkQualityPeer {
    pub device_id: String,
    pub name: String,
    pub addr: String,
    pub is_online: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub last_seen_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub rtt_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub min_rtt_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub max_rtt_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub avg_rtt_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub jitter_ms: Option<f64>,
    pub pings_sent: u64,
    pub pongs_recv: u64,
    pub packet_loss_rate: f64,
    pub grade: String,
    pub samples_count: u64,
    pub connected_uptime_secs: u64,
}

/// §152: the link resource — `GET /api/v1/link` snapshot with the
/// heartbeat config and per-peer quality rows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct LinkQuality {
    pub heartbeat: HeartbeatConfig,
    pub total_peers: u64,
    pub online_peers: u64,
    pub links: Vec<LinkQualityPeer>,
}

/// §152: `POST /devices/{id}/probe` body — an on-demand multi-ping over
/// the Noise session.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct LinkProbeRequest {
    #[cfg_attr(test, ts(optional))]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<u32>,
    #[cfg_attr(test, ts(optional))]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interval_ms: Option<u64>,
}

/// §152: probe result — mirrored from the runtime `LinkProbeReport` field
/// names (device_id alias for `peer`; samples inlined as `samples_ms`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct LinkProbeReport {
    pub device_id: String,
    pub count: u64,
    pub sent_count: u64,
    pub recv_count: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub min_rtt_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub max_rtt_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub avg_rtt_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub jitter_ms: Option<f64>,
    pub packet_loss_rate: f64,
    pub samples_ms: Vec<u64>,
    pub grade: String,
}

/// §152: single ping result — mirrored from the runtime `PeerPingReport`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct DevicePingReport {
    pub device_id: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub rtt_ms: u64,
    pub path: String,
    pub addr: String,
}

/// §153: per-channel traffic counters — mirrored from the runtime
/// `ChannelTrafficDto` (telemetry.rs) field-for-field.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ChannelTraffic {
    #[cfg_attr(test, ts(type = "number"))]
    pub tx_bytes: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub rx_bytes: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub tx_frames: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub rx_frames: u64,
}

/// §153: protocol message counters — mirrored from the runtime
/// `ProtocolMessagesDto` field-for-field.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ProtocolMessages {
    #[cfg_attr(test, ts(type = "number"))]
    pub pings_sent: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub pings_recv: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub pongs_sent: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub pongs_recv: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub notifies_sent: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub notifies_recv: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub clips_sent: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub clips_recv: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub file_offers_sent: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub file_offers_recv: u64,
}

/// §153: `GET /diagnostics/metrics` — unified traffic + protocol telemetry.
/// Field names mirror the legacy `/api/metrics` `TrafficReport` so the CLI
/// printer needs no reshaping; a `PATCH` reset folds into the same GET
/// readback (reset counters all zero, §151 fold pattern).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct TrafficTelemetry {
    #[cfg_attr(test, ts(type = "number"))]
    pub uptime_secs: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub total_tx_bytes: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub total_rx_bytes: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub total_tx_frames: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub total_rx_frames: u64,
    pub control: ChannelTraffic,
    pub file: ChannelTraffic,
    pub input: ChannelTraffic,
    pub screen: ChannelTraffic,
    pub messages: ProtocolMessages,
    #[cfg_attr(test, ts(type = "number"))]
    pub sessions_established: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub sessions_closed: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub handshakes_failed: u64,
}

/// `POST /inbox/open`: reveal one inbox file with the OS opener.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct InboxOpenRequest {
    pub name: String,
}

/// §162a: `POST /inbox/{name}/open` report — the revealed file (or the inbox
/// directory when `name` is null) and where it lives on disk.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct InboxOpenReport {
    #[cfg_attr(test, ts(optional))]
    pub name: Option<String>,
    pub path: String,
}

// ---- §162b: file domain (offers / send / outbox / storage / resume) ----

/// One pending incoming file offer (§4.2 `GET /offers` item). Field names
/// mirror the legacy `/api/file/offers` DTO so CLI printers need no reshaping.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct FileOffer {
    pub transfer_id: String,
    pub filename: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub total_bytes: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub resume_from: u64,
    pub source: String,
    pub peer: String,
    #[cfg_attr(test, ts(optional))]
    pub peer_alias: Option<String>,
    #[cfg_attr(test, ts(type = "number"))]
    pub size: u64,
    pub sha256: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub timestamp_ms: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub offered_ms: u64,
}

/// The pending-offers resource: count, the auto-accept master state and the
/// offer list (optionally filtered by `id`/`peer` query).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct FileOffers {
    #[cfg_attr(test, ts(type = "number"))]
    pub count: u64,
    pub auto_accept_trusted: bool,
    pub offers: Vec<FileOffer>,
}

/// `POST /offers/accept` | `POST /offers/reject` body: one offer by id, or
/// every pending offer from `peer` with `all`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct FileDecisionRequest {
    #[cfg_attr(test, ts(optional))]
    pub id: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub all: Option<bool>,
    #[cfg_attr(test, ts(optional))]
    pub peer: Option<String>,
}

/// One accept/reject outcome (single decision inside the batch report).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct FileDecision {
    pub accepted: bool,
    pub action: String,
    pub transfer_id: String,
    pub peer: String,
    pub filename: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub total_bytes: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub resume_from: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub timestamp_ms: u64,
}

/// The decision report: single decision when `id` was given, batch counts +
/// per-offer outcomes when `all`/`peer` was given. Both shapes fold here —
/// `decisions` stays empty on the single path.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct FileDecisionReport {
    #[cfg_attr(test, ts(optional))]
    pub accepted: Option<bool>,
    #[cfg_attr(test, ts(optional))]
    pub action: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub transfer_id: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub peer: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub filename: Option<String>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub total_bytes: Option<u64>,
    #[cfg_attr(test, ts(type = "number"))]
    pub accepted_count: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub rejected_count: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub total_evaluated: u64,
    pub decisions: Vec<FileDecision>,
}

/// One target outcome of a multi-peer file dispatch (`PeerFileSendItem`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct FileDispatchItem {
    pub peer: String,
    #[cfg_attr(test, ts(optional))]
    pub peer_alias: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub transfer_id: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub outbox_id: Option<String>,
    pub success: bool,
    #[cfg_attr(test, ts(optional))]
    pub error: Option<String>,
}

/// `POST /transfers` report for a single-peer send: the content-addressed
/// transfer id plus the resolved target (legacy `/api/file/send` shape).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct FileSendReport {
    pub transfer_id: String,
    pub name: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub size: u64,
    pub peer: String,
    #[cfg_attr(test, ts(optional))]
    pub sha256: Option<String>,
}

/// `POST /transfers/multi` report: one file fanned out to a selector
/// (legacy `/api/file/send-multi` shape, same counts as messages dispatch).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct FileDispatchReport {
    pub selector: String,
    pub filename: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub size: u64,
    pub sha256: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub target_count: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub success_count: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub skipped_offline_count: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub failed_count: u64,
    pub dispatches: Vec<FileDispatchItem>,
}

/// One per-file outcome inside a batch send.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct FileBatchItem {
    #[cfg_attr(test, ts(optional))]
    pub transfer_id: Option<String>,
    pub name: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub size: u64,
    #[cfg_attr(test, ts(optional))]
    pub sha256: Option<String>,
    pub status: String,
    #[cfg_attr(test, ts(optional))]
    pub error: Option<String>,
}

/// `POST /transfers/batch` report: many files to one peer.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct FileBatchReport {
    pub batch_id: String,
    pub peer: String,
    #[cfg_attr(test, ts(optional))]
    pub peer_alias: Option<String>,
    #[cfg_attr(test, ts(type = "number"))]
    pub total_files: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub successful_files: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub failed_files: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub total_bytes: u64,
    pub items: Vec<FileBatchItem>,
}

/// One sent-file record in the outbox ring (`OutboxEntry`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct OutboxEntryDto {
    pub id: String,
    pub transfer_id: String,
    pub peer: String,
    #[cfg_attr(test, ts(optional))]
    pub peer_alias: Option<String>,
    pub filename: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub size: u64,
    pub sha256: String,
    #[cfg_attr(test, ts(optional))]
    pub source_path: Option<String>,
    /// `pending | sent | failed | cancelled` (lowercase).
    pub status: String,
    #[cfg_attr(test, ts(optional))]
    pub error: Option<String>,
    #[cfg_attr(test, ts(type = "number"))]
    pub started_ms: u64,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub completed_ms: Option<u64>,
    #[cfg_attr(test, ts(type = "number"))]
    pub speed_bps: u64,
}

/// The outbox resource: ring total plus the queried (filtered/capped) page.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct Outbox {
    #[cfg_attr(test, ts(type = "number"))]
    pub total: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub returned: u64,
    pub entries: Vec<OutboxEntryDto>,
}

/// `DELETE /outbox` report: how many records the purge dropped.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct OutboxClearReport {
    #[cfg_attr(test, ts(type = "number"))]
    pub cleared_count: u64,
    #[cfg_attr(test, ts(optional))]
    pub target_peer: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub target_status: Option<String>,
}

/// `POST /outbox/{id}/resend` report: the re-offered transfer.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct OutboxResendReport {
    pub original_id: String,
    pub new_transfer_id: String,
}

/// One resumable transfer marker (memory entry or disk sidecar).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ResumeMarker {
    pub transfer_id: String,
    pub source: String,
    #[cfg_attr(test, ts(optional))]
    pub source_alias: Option<String>,
    #[cfg_attr(test, ts(type = "number"))]
    pub total_bytes: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub offset_bytes: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub progress_percent: f64,
    pub chunk_aligned: bool,
    pub part_file_exists: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub part_file_bytes: u64,
}

/// The resume-markers resource: every resumable inbound transfer.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ResumeMarkers {
    #[cfg_attr(test, ts(type = "number"))]
    pub count: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub total_partial_bytes: u64,
    pub markers: Vec<ResumeMarker>,
}

/// The storage resource: data/inbox directories and disk consumption.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct Storage {
    pub data_dir: String,
    pub inbox_dir: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub inbox_total_bytes: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub inbox_file_count: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub partial_bytes: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub partial_count: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub orphaned_partial_count: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub orphaned_partial_bytes: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub meta_bytes: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub resume_markers_count: u64,
}

/// `POST /storage/clean` report: freed space and what got deleted.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct StorageCleanReport {
    #[cfg_attr(test, ts(type = "number"))]
    pub freed_bytes: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub deleted_files_count: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub cleared_resume_entries: u64,
}

/// `POST /storage/clean` body: `orphaned_only` defaults to true when absent.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct StorageCleanBody {
    #[cfg_attr(test, ts(optional))]
    pub orphaned_only: Option<bool>,
}

/// `POST /transfers` body: one file to one peer — a `path` on the daemon
/// filesystem or inline `data_base64` (legacy `/api/file/send` shape).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct FileSendRequest {
    #[cfg_attr(test, ts(optional))]
    pub path: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub data_base64: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub name: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub peer: Option<String>,
}

/// `POST /transfers/multi` body: one file fanned out to a target selector
/// (`peer`/`tag`/`group`/`type`/`all` — exactly one axis, legacy parity).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct FileSendMultiRequest {
    #[cfg_attr(test, ts(optional))]
    pub path: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub data_base64: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub name: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub peer: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub tag: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub group: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub device_type: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub all: Option<bool>,
}

/// One file inside `POST /transfers/batch`: a daemon `path` or inline bytes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct FileBatchEntry {
    #[cfg_attr(test, ts(optional))]
    pub path: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub name: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub data_base64: Option<String>,
}

/// `POST /transfers/batch` body: many files to one peer.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct FileBatchRequest {
    #[cfg_attr(test, ts(optional))]
    pub peer: Option<String>,
    #[serde(default)]
    pub paths: Vec<String>,
    #[serde(default)]
    pub items: Vec<FileBatchEntry>,
}

/// The auto-accept policy resource: master toggle plus scoped allow-lists
/// (mirrors the runtime `AutoAcceptPolicy` field-for-field).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct AutoAcceptPolicyDto {
    pub enabled: bool,
    pub allowed_groups: Vec<String>,
    pub allowed_tags: Vec<String>,
    pub allowed_types: Vec<String>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub max_file_size: Option<u64>,
    /// Session count of offers accepted automatically (legacy report extra).
    #[cfg_attr(test, ts(type = "number"))]
    pub auto_accepted_count: u64,
}

/// `PATCH /policy/auto-accept` body: policy fields plus the master toggle;
/// omitted fields stay untouched (`Patch` distinguishes JSON `null` from
/// absent — see the profiles-domain `Patch<T>` notes).
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct AutoAcceptPolicyPatch {
    #[cfg_attr(test, ts(optional))]
    pub enabled: Option<bool>,
    #[cfg_attr(test, ts(optional))]
    pub allowed_groups: Option<Vec<String>>,
    #[cfg_attr(test, ts(optional))]
    pub allowed_tags: Option<Vec<String>>,
    #[cfg_attr(test, ts(optional))]
    pub allowed_types: Option<Vec<String>>,
    /// JSON number sets the cap; JSON `null` lifts it; absent leaves it.
    #[cfg_attr(test, ts(type = "number | null"))]
    #[serde(default)]
    pub max_file_size: Patch<u64>,
}

// ---- §156a: profiles domain ----

/// One peer metadata profile (mirrors the runtime `PeerProfile` field-for-field
/// so the CLI printers need no reshaping).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct PeerProfileDto {
    pub device_id: String,
    pub device_type: String,
    pub tags: Vec<String>,
    pub group: Option<String>,
    pub notes: Option<String>,
    #[cfg_attr(test, ts(type = "number"))]
    pub updated_ms: u64,
}

/// `GET /profiles` — the full profile table, optionally filtered.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct PeerProfiles {
    pub total: usize,
    pub profiles: Vec<PeerProfileDto>,
}

/// `PATCH /profiles/{id}` — the updatable profile fields.
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct PeerProfilePatch {
    #[cfg_attr(test, ts(optional))]
    pub device_type: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub tags: Option<Vec<String>>,
    /// JSON string sets the group; JSON `null` clears it; absent leaves it
    /// untouched (`Patch` distinguishes null from absent — a bare
    /// `Option<Option<T>>` collapses both to "untouched" because serde's
    /// outer `visit_none` fires before the inner type sees the null).
    #[cfg_attr(test, ts(type = "string | null"))]
    #[serde(default)]
    pub group: Patch<String>,
    /// Same three-state semantics as `group`.
    #[cfg_attr(test, ts(type = "string | null"))]
    #[serde(default)]
    pub notes: Patch<String>,
}

impl PeerProfilePatch {
    /// `true` when the patch sets no field.
    pub fn is_empty(&self) -> bool {
        self.device_type.is_none()
            && self.tags.is_none()
            && matches!(self.group, Patch::Leave)
            && matches!(self.notes, Patch::Leave)
    }
}

/// Three-state patch field: an absent key is `Leave` (untouched, via
/// `#[serde(default)]`), a JSON `null` is `Clear`, a JSON value is `Set`.
/// The custom `Deserialize` is required because serde's outer `Option`
/// collapses an explicit null into the same `visit_none` path as absence.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Patch<T> {
    #[default]
    Leave,
    Set(T),
    Clear,
}

impl<T> Patch<T> {
    /// The set value, if any.
    pub fn set(&self) -> Option<&T> {
        match self {
            Patch::Set(v) => Some(v),
            Patch::Leave | Patch::Clear => None,
        }
    }
    /// `true` when this is an explicit JSON `null` (clear request).
    pub fn is_clear(&self) -> bool {
        matches!(self, Patch::Clear)
    }
}

impl<'de, T: serde::Deserialize<'de>> serde::Deserialize<'de> for Patch<T> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        match serde_json::Value::deserialize(deserializer)? {
            serde_json::Value::Null => Ok(Patch::Clear),
            other => T::deserialize(other)
                .map(Patch::Set)
                .map_err(serde::de::Error::custom),
        }
    }
}

/// `PATCH /profiles/{id}` readback: the resolved device and its post-patch
/// profile (§151 fold pattern — the legacy `ok`/`message` flags fold away).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct PeerProfileUpdated {
    pub device_id: String,
    pub profile: PeerProfileDto,
}

/// `PATCH /caps` request — capability feature flags to enable/disable.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct CapsUpdate {
    #[serde(default)]
    pub enable: Vec<String>,
    #[serde(default)]
    pub disable: Vec<String>,
}

/// `GET /caps` and `PATCH /caps` readback — local node capability report
/// (mirrors the runtime `LocalCapsReport`; the legacy `ok` flag folds away).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct LocalCaps {
    pub device_id: String,
    pub caps: Vec<String>,
    pub features: Vec<String>,
    #[cfg_attr(test, ts(type = "number"))]
    pub file_port: u16,
    #[cfg_attr(test, ts(type = "number"))]
    pub input_port: u16,
    #[cfg_attr(test, ts(type = "number"))]
    pub screen_port: u16,
}

/// Capability profile for one peer (mirrors the runtime `PeerCapsDto`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct PeerCaps {
    pub device_id: String,
    pub name: String,
    pub alias: Option<String>,
    pub caps: Vec<String>,
    pub features: Vec<String>,
    #[cfg_attr(test, ts(type = "number | null"))]
    pub file_port: Option<u16>,
    #[cfg_attr(test, ts(type = "number | null"))]
    pub input_port: Option<u16>,
    #[cfg_attr(test, ts(type = "number | null"))]
    pub screen_port: Option<u16>,
    pub online: bool,
}

// ---- §156b: topology domain ----

/// One edge neighbor of the spatial display topology (mirrors the core
/// `EdgeNeighbor`; edge enums folded to lowercase strings per §4.2 style).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct TopologyEdgeNeighbor {
    pub peer_id: String,
    /// Edge on the peer's display the local edge connects to: `left | right | top | bottom`.
    pub peer_edge: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub offset: i32,
}

/// The topology resource (§4.2 `GET /topology`): local display rectangle,
/// one neighbor per configured edge, and the crossing config knobs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct Topology {
    pub display: DisplayRect,
    /// Keyed by local edge name (`left | right | top | bottom`).
    pub neighbors: Vec<TopologyEdgeNeighbor>,
    /// Local edge the neighbor entry sits on.
    #[cfg_attr(test, ts(type = "string"))]
    pub neighbor_edges: Vec<String>,
    pub config: CrossingConfig,
    #[cfg_attr(test, ts(type = "number"))]
    pub peer_count: u64,
}

/// A peer's advertised display topology summary.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct PeerTopologyDto {
    pub peer_id: String,
    pub peer_alias: Option<String>,
    pub display: DisplayRect,
    pub config: CrossingConfig,
    #[cfg_attr(test, ts(type = "number"))]
    pub timestamp_ms: u64,
    /// Number of edges the peer has linked (count of its neighbors).
    #[cfg_attr(test, ts(type = "number"))]
    pub neighbors_count: u64,
}

/// `GET /topology/peers` — all known peer display topologies.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct PeerTopologies {
    #[cfg_attr(test, ts(type = "number"))]
    pub count: u64,
    pub peers: Vec<PeerTopologyDto>,
}

/// `POST /topology/links` request: attach a peer to a local display edge.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Default)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct TopologyLinkRequest {
    /// Local edge: `left | right | top | bottom`.
    pub edge: String,
    /// Peer target (id, name, or alias).
    pub peer: String,
    /// Peer's edge that faces us; defaults to the geometric opposite.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peer_edge: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<i32>,
}

/// Report returned when a link or unlink lands (§4.2 style).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct TopologyLinkReport {
    /// Local edge affected.
    pub edge: String,
    pub peer_id: String,
    pub peer_edge: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub offset: i32,
}

/// `POST /topology/unlinks` request: detach a local display edge.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Default)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct TopologyUnlinkRequest {
    pub edge: String,
}

/// `POST /topology/unlinks` report: the edge that was detached.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct TopologyUnlinkReport {
    pub edge: String,
}

/// `PATCH /topology/config` report: the effective display rect and crossing
/// config after the sparse patch applied.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct TopologyConfigReport {
    pub display: DisplayRect,
    pub config: CrossingConfig,
}

/// `PATCH /topology/config` request: sparse crossing-geometry update.
/// Absent keys keep their current values (§4.2 sparse PATCH semantics).
#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct TopologyConfigPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resistance_px: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delay_ms: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub corner_lock_size_px: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wrap_around: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub redundancy_band_px: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slow_dwell_ms: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fast_push_delta_px: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry_return_grace_ms: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_speed_percent: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale_factor: Option<f32>,
}

/// Topology governance policy (mirrors the store's `TopologyPolicy`; device
/// types folded to lowercase strings).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct TopologyPolicy {
    pub enabled: bool,
    pub allow_inbound_control: bool,
    pub auto_link_paired_peers: bool,
    pub allowed_groups: Vec<String>,
    pub allowed_tags: Vec<String>,
    pub allowed_types: Vec<String>,
}

/// `PATCH /topology/policy` request: sparse policy update.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Default)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct TopologyPolicyPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_inbound_control: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_link_paired_peers: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_groups: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_tags: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_types: Option<Vec<String>>,
    /// When true, clears the allowed_groups/tags/types filter lists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clear_filters: Option<bool>,
}

/// `POST /topology/sync` report: sync requests and broadcasts dispatched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct TopologySyncReport {
    #[cfg_attr(test, ts(type = "number"))]
    pub sync_requests_sent: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub broadcasts_sent: u64,
}

// ---- §156b: routes / network domain ----

/// One cached physical addressing route (mirrors the runtime `RouteEntryDto`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct RouteEntryDto {
    pub device_id: String,
    pub addr: String,
    pub alias: Option<String>,
    pub name: Option<String>,
    pub online: bool,
    pub trusted: bool,
}

/// `GET /routes` — all cached physical addressing routes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct Routes {
    #[cfg_attr(test, ts(type = "number"))]
    pub count: u64,
    pub routes: Vec<RouteEntryDto>,
}

/// `POST /routes` request: pin a peer to a physical socket address.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Default)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct RouteSetRequest {
    /// Peer target (id, name, or alias).
    pub peer: String,
    pub addr: String,
}

/// Report returned after a route add/remove (mirrors `RouteMutateReport`;
/// `ok` folded away, `action` kept for the CLI printer).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct RouteMutateReport {
    pub device_id: String,
    pub addr: Option<String>,
    /// `added | removed`.
    pub action: String,
}

/// One network interface probe row (mirrors the runtime `IfaceReportDto`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct IfaceReport {
    pub iface: String,
    pub ip: String,
    pub listen: bool,
    pub reason: Option<String>,
}

/// EasyTier overlay sidecar status (mirrors the runtime report).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct OverlayStatus {
    pub enabled: bool,
    pub present: bool,
    pub source: String,
    pub local_ips: Vec<String>,
    #[cfg_attr(test, ts(type = "number"))]
    pub peer_count: u64,
    pub peers: Vec<String>,
}

/// `GET /net` — network interfaces and overlay status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct Network {
    pub interfaces: Vec<IfaceReport>,
    pub overlay: OverlayStatus,
}

/// `POST /net/refresh` report — rescan + overlay probe results.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct NetworkRefreshReport {
    #[cfg_attr(test, ts(type = "number"))]
    pub ifaces_scanned: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub ifaces_listening: u64,
    pub overlay_present: bool,
    pub overlay_source: String,
    pub overlay_local_ips: Vec<String>,
    #[cfg_attr(test, ts(type = "number"))]
    pub overlay_peers_count: u64,
}

// ---- §160: discovery / daemon lifecycle domain ----

/// One peer discovered on the LAN via mDNS beacons (mirrors the runtime
/// `DiscoveredPeerDto`; the legacy `ok` envelope folds away).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct DiscoveredPeer {
    pub device_id: String,
    pub name: String,
    pub addrs: Vec<String>,
    #[cfg_attr(test, ts(type = "number"))]
    pub port: u16,
    pub fp: Option<String>,
    pub caps: Vec<String>,
    pub is_trusted: bool,
    pub is_online: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub last_seen_ms: u64,
}

/// `GET /discovery` — all peers discovered on the local network.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct Discovery {
    #[cfg_attr(test, ts(type = "number"))]
    pub count: u64,
    pub peers: Vec<DiscoveredPeer>,
}

/// `POST /discovery/rescan` report: a fresh mDNS sweep was started.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct DiscoveryRescanReport {
    pub message: String,
}

/// `POST /system/restart` request: optional replacement binary and a delay
/// before the restart fires.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct DaemonRestartRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replacement_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(type = "number | null"))]
    pub delay_ms: Option<u64>,
}

/// `POST /system/restart` report (legacy `ok`/`message` folded; the action
/// string keeps the legacy value).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct DaemonRestartReport {
    pub action: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub old_pid: u32,
    #[cfg_attr(test, ts(type = "number"))]
    pub new_pid: u32,
    #[cfg_attr(test, ts(type = "number"))]
    pub timestamp_ms: u64,
}

/// `POST /system/shutdown` report: the shutdown was scheduled.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct DaemonShutdownReport {
    pub status: String,
}

// ---- §161: notify / reply / OTP domain ----

/// `POST /notify` request: push a desktop notification text to a peer.
/// Without `peer`, the single connected peer is used (legacy ambiguity rules).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct NotifySendRequest {
    pub title: String,
    pub body: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peer: Option<String>,
}

/// `POST /notify` report: the peer the notification was delivered to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct NotifySendReport {
    pub peer: String,
}

/// One notification candidate (legacy `CandidateViewDto` shape).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct NotifyCandidate {
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

/// `GET /notify/candidates` — active candidates (count from the array).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct NotifyCandidates {
    #[cfg_attr(test, ts(type = "number"))]
    pub count: u64,
    pub candidates: Vec<NotifyCandidate>,
}

/// One queued inbound reply (no text — the no-content rule).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct NotifyReplyDto {
    pub id: String,
    pub source: String,
    pub app_id: String,
    pub uid: String,
}

/// `GET /notify/replies` — queued inbound replies (count from the array).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct NotifyReplies {
    #[cfg_attr(test, ts(type = "number"))]
    pub count: u64,
    pub replies: Vec<NotifyReplyDto>,
}

/// `POST /notify/otp` report: the extracted OTP for a candidate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct NotifyOtpReport {
    pub id: String,
    pub otp: String,
}

/// Report for copy / insert / reply actions: the action completed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct NotifyActionReport {
    pub ok: bool,
}

/// `POST /notify/dismissals` request: dismiss by id, by filter, or all.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct NotifyDismissRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(type = "boolean | null"))]
    pub all: Option<bool>,
}

/// `POST /notify/dismissals` report: either the single-candidate shape or the
/// batch shape, depending on the request (unlike legacy both fold into one
/// struct; only the relevant fields are populated).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct NotifyDismissReport {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(type = "boolean | null"))]
    pub notified_remote: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(type = "number | null"))]
    pub count: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ids: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(type = "number | null"))]
    pub notified_remote_count: Option<u64>,
}

/// `POST /notify/open` report: the URL (or scheme target) that was opened.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct NotifyOpenReport {
    pub url: String,
}

/// Request carrying just a candidate id (OTP get / copy / insert / reply
/// take all share this shape).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct NotifyIdRequest {
    pub id: String,
}

/// `POST /replies` request: the quick reply text typed for a candidate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct NotifyReplySendRequest {
    pub id: String,
    pub text: String,
}

/// `POST /notify/replies/take` request: hand a queued reply out exactly once.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct NotifyReplyTakeRequest {
    pub id: String,
}

/// `POST /notify/replies/take` report: the reply text (only release point).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct NotifyReplyTakeReport {
    pub id: String,
    pub uid: String,
    pub app_id: String,
    pub text: String,
}

/// Banner configuration (mirrors `NotifyBannerConfig`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct NotifyConfigDto {
    pub banners_enabled: bool,
    pub dnd_mode: NotifyDndMode,
    pub sound_enabled: bool,
    pub mask_otp: bool,
    pub allow_otp_in_dnd: bool,
}

/// DND mode (mirrors the daemon's `DndMode`, snake_case on the wire).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum NotifyDndMode {
    Off,
    Silent,
    All,
}

/// `PATCH /notify/config` request: any subset of the banner switches.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct NotifyConfigPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(type = "boolean | null"))]
    pub banners_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dnd_mode: Option<NotifyDndMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(type = "boolean | null"))]
    pub sound_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(type = "boolean | null"))]
    pub mask_otp: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(type = "boolean | null"))]
    pub allow_otp_in_dnd: Option<bool>,
}

/// Banner dispatch/suppression counters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct NotifyConfigTotals {
    #[cfg_attr(test, ts(type = "number"))]
    pub dispatched: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub suppressed: u64,
}

/// `GET /notify/config` / `PATCH /notify/config` report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct NotifyConfigReportDto {
    pub config: NotifyConfigDto,
    pub totals: NotifyConfigTotals,
}

/// `POST /notify/test` request: optional title/body override.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct NotifyTestRequestDto {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
}

/// `POST /notify/test` report (legacy `ok` folded — always true on 200).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct NotifyTestReportDto {
    pub sent: bool,
    pub suppressed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suppression_reason: Option<String>,
    pub message: String,
    pub title: String,
}

// ---- §161: peer message domain ----

/// One direct peer message (legacy `PeerMessage` shape; direction reuses
/// the shared `MessageDirection` enum).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct MessageEntry {
    pub id: String,
    pub peer: String,
    pub direction: MessageDirection,
    pub text: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub timestamp_ms: u64,
    pub read: bool,
}

/// `POST /messages` request: send a direct message. Multi-target dispatch
/// (tag/group/device_type/all) reuses the same body; the response then
/// becomes a `MessageDispatchReport` instead of a `MessageSendReport`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct MessageSendRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    /// Device form-factor selector (`desktop`/`laptop`/`phone`/`tablet`/
    /// `server`/`cloud`/`other`), matching the daemon's `DeviceType`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(type = "boolean | null"))]
    pub all: Option<bool>,
    pub text: String,
}

/// `POST /messages` single-target report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct MessageSendReport {
    pub message: MessageEntry,
}

/// One per-target dispatch outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct MessageDispatchItem {
    pub peer: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peer_alias: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    pub success: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// `POST /messages` multi-target report (legacy `selector` label folded into
/// `selector` string form, e.g. `peer:tdev_…` / `tag:lab` / `all_online`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct MessageDispatchReport {
    pub selector: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub target_count: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub success_count: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub skipped_offline_count: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub failed_count: u64,
    pub dispatches: Vec<MessageDispatchItem>,
}

/// `GET /messages` query parameters (all optional).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct MessageQuery {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(type = "number | null"))]
    pub limit: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(type = "boolean | null"))]
    pub unread_only: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(type = "boolean | null"))]
    pub mark_read: Option<bool>,
}

/// `GET /messages` report: filtered history (counts from the array where the
/// CLI needs them, but total/unread are engine-side state so they stay).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct MessageHistory {
    #[cfg_attr(test, ts(type = "number"))]
    pub total: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub returned: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub unread: u64,
    pub messages: Vec<MessageEntry>,
}

/// `DELETE /messages` request: clear all, or just one peer's history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct MessageClearRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peer: Option<String>,
}

/// `DELETE /messages` report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct MessageClearReport {
    pub target: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub cleared_count: u64,
    pub message: String,
}

// ---- §163: clipboard sync domain ----

/// The clipboard auto-sync policy + sync statistics in one flat resource
/// (legacy `ClipboardPolicyReport` with the `ok` envelope folded off).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ClipPolicyDto {
    /// Sync mode, as the snake_case `ClipSyncMode` string.
    pub mode: String,
    pub sensitive_guard: bool,
    pub allowed_groups: Vec<String>,
    pub allowed_tags: Vec<String>,
    /// Allowed device form factors, as the `DeviceType` snake_case strings.
    pub allowed_types: Vec<String>,
    #[cfg_attr(test, ts(type = "number"))]
    pub max_payload_bytes: u64,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub auto_clear_timeout_secs: Option<u64>,
    #[cfg_attr(test, ts(type = "number"))]
    pub auto_synced_out_count: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub auto_synced_in_count: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub sensitive_blocked_count: u64,
}

/// `PATCH /clip/policy` body: sparse policy fields (absent stays; the
/// `clear_filters` sweep wipes all three allow-lists in one call).
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct ClipPolicyPatch {
    #[cfg_attr(test, ts(optional))]
    pub mode: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub sensitive_guard: Option<bool>,
    #[cfg_attr(test, ts(optional))]
    pub allowed_groups: Option<Vec<String>>,
    #[cfg_attr(test, ts(optional))]
    pub allowed_tags: Option<Vec<String>>,
    #[cfg_attr(test, ts(optional))]
    pub allowed_types: Option<Vec<String>>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub max_payload_bytes: Option<u64>,
    /// JSON `0` also clears the countdown (legacy parity).
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub auto_clear_timeout_secs: Option<u64>,
    #[cfg_attr(test, ts(optional))]
    pub clear_filters: Option<bool>,
}

/// `POST /clip/auto-sync/poll` report: one manual poll tick of the
/// policy engine, with the count of peers the sweep dispatched to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ClipPollReport {
    #[cfg_attr(test, ts(type = "number"))]
    pub dispatched_peers: u64,
    pub policy: ClipPolicyDto,
}

/// Clipboard state snapshot without leaking plaintext.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ClipStatus {
    pub has_text: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub bytes: u64,
    #[cfg_attr(test, ts(optional))]
    pub hash: Option<String>,
    #[cfg_attr(test, ts(type = "number"))]
    pub last_seq: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub inbound_count: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub outbound_count: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub history_count: u64,
    /// False on hosts with no display server (headless Linux).
    pub clipboard_available: bool,
    #[cfg_attr(test, ts(optional))]
    pub reason: Option<String>,
}

/// One clipboard sync event in the bounded history (no plaintext).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ClipHistoryEntry {
    #[cfg_attr(test, ts(type = "number"))]
    pub seq: u64,
    pub source: String,
    /// `"inbound"` or `"outbound"`.
    pub direction: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub bytes: u64,
    pub hash: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub timestamp_ms: u64,
}

/// `GET /clip/history` report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ClipHistory {
    #[cfg_attr(test, ts(type = "number"))]
    pub total: usize,
    pub entries: Vec<ClipHistoryEntry>,
}

/// `DELETE /clip` report (clear history + optionally the OS clipboard).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ClipClearReport {
    #[cfg_attr(test, ts(type = "number"))]
    pub cleared_history_count: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub timestamp_ms: u64,
    pub clipboard_available: bool,
    #[cfg_attr(test, ts(optional))]
    pub reason: Option<String>,
}

/// `GET /clip` report: the raw clipboard text (the one plaintext readback).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ClipContent {
    pub has_text: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub bytes: u64,
    #[cfg_attr(test, ts(optional))]
    pub hash: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub text: Option<String>,
    pub clipboard_available: bool,
    /// Present only when the OS clipboard port is absent.
    #[cfg_attr(test, ts(optional))]
    pub reason: Option<String>,
}

/// `POST /clip/sync` body: explicit text, or empty to push the current
/// local clipboard; `peer` narrows the single-live-peer implication.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct ClipSyncRequest {
    #[cfg_attr(test, ts(optional))]
    pub peer: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub text: Option<String>,
}

/// `POST /clip/sync` report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ClipSyncReport {
    #[cfg_attr(test, ts(type = "number"))]
    pub bytes: u64,
    pub hash: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub sent_peers: u64,
}

// ---- §163: audit trail domain ----

/// One structured audit entry, zero-plaintext.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct AuditEntryDto {
    #[cfg_attr(test, ts(type = "number"))]
    pub id: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub timestamp_ms: u64,
    /// `AuditCategory` snake_case string.
    pub category: String,
    /// `AuditSeverity` snake_case string.
    pub severity: String,
    pub action: String,
    #[cfg_attr(test, ts(optional))]
    pub target: Option<String>,
    pub summary: String,
    pub success: bool,
}

/// `GET /audit` report: the filtered audit trail.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct AuditTrail {
    #[cfg_attr(test, ts(type = "number"))]
    pub total: usize,
    #[cfg_attr(test, ts(type = "number"))]
    pub returned: usize,
    #[cfg_attr(test, ts(type = "number"))]
    pub capacity: usize,
    pub entries: Vec<AuditEntryDto>,
}

/// `GET /audit` query body variant (mirror of the query params).
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct AuditQuery {
    #[cfg_attr(test, ts(optional))]
    pub category: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub severity: Option<String>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub limit: Option<u64>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub since: Option<u64>,
}

/// `DELETE /audit` report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct AuditClearReport {
    #[cfg_attr(test, ts(type = "number"))]
    pub cleared_count: u64,
    pub message: String,
}

// ---- §164: remote action domain ----

/// One locally registered quick-action definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct LocalActionDto {
    pub id: String,
    pub name: String,
    #[cfg_attr(test, ts(optional))]
    pub description: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub icon: Option<String>,
    pub command: String,
    pub args: Vec<String>,
    #[cfg_attr(test, ts(type = "number"))]
    pub timeout_secs: u32,
    pub enabled: bool,
}

/// `GET /action/local` report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct LocalActions {
    #[cfg_attr(test, ts(type = "number"))]
    pub count: usize,
    pub actions: Vec<LocalActionDto>,
}

/// One peer-advertised action (metadata only, no command details).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct RemoteActionDto {
    pub id: String,
    pub name: String,
    #[cfg_attr(test, ts(optional))]
    pub description: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub icon: Option<String>,
}

/// `GET /action/remote` report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct RemoteActions {
    pub peer: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub count: usize,
    pub actions: Vec<RemoteActionDto>,
}

/// `POST /action/local` body: register or replace a local action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct LocalActionRequest {
    pub id: String,
    pub name: String,
    #[cfg_attr(test, ts(optional))]
    pub description: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub icon: Option<String>,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub timeout_secs: Option<u32>,
    #[cfg_attr(test, ts(optional))]
    pub enabled: Option<bool>,
}

/// `POST /action/remote/refresh` body: ask a connected peer to re-advertise.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct RemoteActionRefreshRequest {
    #[cfg_attr(test, ts(optional))]
    pub peer: Option<String>,
}

/// `POST /action/trigger` body: trigger one remote action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct ActionTriggerRequest {
    pub peer: String,
    pub action_id: String,
}

/// `POST /action/trigger` report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ActionTriggerReport {
    pub trigger_id: String,
    pub action_id: String,
    pub target_peer: String,
    pub status: String,
}

/// One recorded remote-action execution result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ActionHistoryEntry {
    pub trigger_id: String,
    pub action_id: String,
    pub success: bool,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub exit_code: Option<i32>,
    #[cfg_attr(test, ts(optional))]
    pub output_summary: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub error_message: Option<String>,
    #[cfg_attr(test, ts(type = "number"))]
    pub duration_ms: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub timestamp_ms: u64,
}

/// `GET /action/history` report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ActionHistory {
    #[cfg_attr(test, ts(type = "number"))]
    pub count: usize,
    pub results: Vec<ActionHistoryEntry>,
}

/// The inbound remote-action governance policy as one flat resource
/// (legacy `ActionPolicyReport` with the `ok` envelope folded off).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ActionPolicyDto {
    /// Access mode, as the snake_case `ActionAccessMode` string.
    pub mode: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub max_timeout_secs: u32,
    pub allowed_groups: Vec<String>,
    pub allowed_tags: Vec<String>,
    /// Allowed device form factors, as the `DeviceType` snake_case strings.
    pub allowed_types: Vec<String>,
    pub expose_action_list: bool,
}

/// `PATCH /action/policy` body: sparse policy fields (absent stays; the
/// `clear_filters` sweep wipes all three allow-lists in one call).
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct ActionPolicyPatch {
    #[cfg_attr(test, ts(optional))]
    pub mode: Option<String>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub max_timeout_secs: Option<u32>,
    #[cfg_attr(test, ts(optional))]
    pub expose_action_list: Option<bool>,
    #[cfg_attr(test, ts(optional))]
    pub allowed_groups: Option<Vec<String>>,
    #[cfg_attr(test, ts(optional))]
    pub allowed_tags: Option<Vec<String>>,
    #[cfg_attr(test, ts(optional))]
    pub allowed_types: Option<Vec<String>>,
    #[cfg_attr(test, ts(optional))]
    pub clear_filters: Option<bool>,
}

// ---- §164: link benchmark domain ----

/// One active benchmark session's live metadata.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ActiveBenchSessionDto {
    pub session_id: String,
    pub peer_id: String,
    /// `BenchRole` lowercase string (`initiator`/`responder`).
    pub role: String,
    /// `BenchDirection` lowercase string (`push`/`pull`).
    pub direction: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub duration_secs: u32,
    #[cfg_attr(test, ts(type = "number"))]
    pub chunk_size: u32,
    #[cfg_attr(test, ts(type = "number"))]
    pub chunks_processed: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub bytes_transferred: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub started_at_ms: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub last_activity_ms: u64,
}

/// The final/last benchmark measurement frame.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct BenchReportDto {
    pub session_id: String,
    /// `BenchDirection` lowercase string (`push`/`pull`).
    pub direction: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub duration_ms: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub bytes_transferred: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub chunks: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub throughput_mbps: f64,
    #[cfg_attr(test, ts(type = "number"))]
    pub avg_chunk_latency_us: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub jitter_us: u64,
    #[cfg_attr(test, ts(optional))]
    pub stopped_early: Option<bool>,
    #[cfg_attr(test, ts(type = "number"))]
    pub timestamp_ms: u64,
}

/// `GET /bench` report: engine status + policy + last measurement
/// (legacy `BenchStatusReport` with the `ok` envelope folded off).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct BenchStatus {
    pub active: bool,
    #[cfg_attr(test, ts(optional))]
    pub session: Option<ActiveBenchSessionDto>,
    pub policy: BenchPolicyDto,
    #[cfg_attr(test, ts(optional))]
    pub last_report: Option<BenchReportDto>,
    #[cfg_attr(test, ts(type = "number"))]
    pub timestamp_ms: u64,
}

/// `POST /bench/start` and `POST /bench/stop` decision report
/// (legacy `BenchDecisionReport` with the `ok` envelope folded off).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct BenchDecision {
    /// `"start"` or `"stop"`.
    pub action: String,
    #[cfg_attr(test, ts(optional))]
    pub session_id: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub peer_id: Option<String>,
    pub message: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub timestamp_ms: u64,
}

/// `POST /bench/start` body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct BenchStartRequest {
    pub peer: String,
    /// `push` (default) or `pull`.
    #[serde(default = "default_bench_direction")]
    pub direction: String,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub duration_secs: Option<u32>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub chunk_size: Option<u32>,
}

fn default_bench_direction() -> String {
    "push".to_string()
}

/// The link benchmark governance policy as one flat resource
/// (legacy `BenchPolicyReport` with the `ok` envelope folded off).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct BenchPolicyDto {
    pub enabled: bool,
    pub allow_inbound_bench: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub max_duration_secs: u32,
    /// Soft outbound pacing ceiling (Mbps).
    #[cfg_attr(test, ts(type = "number"))]
    pub max_bandwidth_mbps: u32,
    pub allowed_groups: Vec<String>,
    pub allowed_tags: Vec<String>,
    /// Allowed device form factors, as the `DeviceType` snake_case strings.
    pub allowed_types: Vec<String>,
}

/// `PATCH /bench/policy` body: sparse policy fields (absent stays; the
/// `clear_filters` sweep wipes all three allow-lists in one call).
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct BenchPolicyPatch {
    #[cfg_attr(test, ts(optional))]
    pub enabled: Option<bool>,
    #[cfg_attr(test, ts(optional))]
    pub allow_inbound_bench: Option<bool>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub max_duration_secs: Option<u32>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub max_bandwidth_mbps: Option<u32>,
    #[cfg_attr(test, ts(optional))]
    pub allowed_groups: Option<Vec<String>>,
    #[cfg_attr(test, ts(optional))]
    pub allowed_tags: Option<Vec<String>>,
    #[cfg_attr(test, ts(optional))]
    pub allowed_types: Option<Vec<String>>,
    #[cfg_attr(test, ts(optional))]
    pub clear_filters: Option<bool>,
}

/// Recommended per-channel streaming tiers for one peer link (ADR-073..076).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct StreamingRecommendationDto {
    /// Receiver-side measured throughput the tiers were derived from (Mbps).
    #[cfg_attr(test, ts(type = "number"))]
    pub measured_mbps: f64,
    /// Human-readable tier band (flagship / high / standard / basic / minimal).
    pub tier: String,
    pub display: DisplayTierDto,
    pub camera: CameraTierDto,
    pub audio: AudioTierDto,
    pub mic: MicTierDto,
}

/// Recommended virtual display mode (ADR-073).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct DisplayTierDto {
    #[cfg_attr(test, ts(type = "number"))]
    pub width: u32,
    #[cfg_attr(test, ts(type = "number"))]
    pub height: u32,
    #[cfg_attr(test, ts(type = "number"))]
    pub fps: u32,
}

/// Recommended virtual camera capture profile (ADR-075).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct CameraTierDto {
    #[cfg_attr(test, ts(type = "number"))]
    pub width: u32,
    #[cfg_attr(test, ts(type = "number"))]
    pub height: u32,
    #[cfg_attr(test, ts(type = "number"))]
    pub fps: u32,
    /// `CameraCodec` string.
    pub codec: String,
}

/// Recommended spatial audio routing profile (ADR-074).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct AudioTierDto {
    #[cfg_attr(test, ts(type = "number"))]
    pub sample_rate: u32,
    #[cfg_attr(test, ts(type = "number"))]
    pub channels: u32,
    /// `AudioStreamCodec` string.
    pub codec: String,
}

/// Recommended virtual microphone profile (ADR-076).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct MicTierDto {
    #[cfg_attr(test, ts(type = "number"))]
    pub sample_rate: u32,
    #[cfg_attr(test, ts(type = "number"))]
    pub channels: u32,
    /// `MicCodec` string.
    pub codec: String,
}

/// `GET /recommendation?peer=` report: one peer's measured link + tiers
/// (legacy `StreamingRecommendationReport` with the `ok` envelope folded off).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct RecommendationDto {
    pub peer_id: String,
    pub peer_alias: String,
    /// False when no measurement exists yet (conservative fallback tiers).
    pub has_profile: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub measured_mbps: f64,
    /// `BenchDirection` lowercase string (`push`/`pull`).
    pub direction: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub measured_at_ms: u64,
    pub recommendation: StreamingRecommendationDto,
    #[cfg_attr(test, ts(type = "number"))]
    pub timestamp_ms: u64,
}

/// One peer's recommendation summary line for the aggregate status view.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct RecommendationSummaryDto {
    pub peer_id: String,
    pub peer_alias: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub measured_mbps: f64,
    /// `BenchDirection` lowercase string (`push`/`pull`).
    pub direction: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub measured_at_ms: u64,
    pub tier: String,
}

/// `GET /recommendation` report: aggregate across all measured peers
/// (legacy `RecommendationStatusReport` with the `ok` envelope folded off).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct RecommendationStatus {
    pub peers: Vec<RecommendationSummaryDto>,
    #[cfg_attr(test, ts(type = "number"))]
    pub timestamp_ms: u64,
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
            decl::<DialRequest>(&cfg),
            decl::<DialReport>(&cfg),
            decl::<ChannelTraffic>(&cfg),
            decl::<ProtocolMessages>(&cfg),
            decl::<TrafficTelemetry>(&cfg),
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
            decl::<InboxOpenReport>(&cfg),
            decl::<FileOffer>(&cfg),
            decl::<FileOffers>(&cfg),
            decl::<FileDecisionRequest>(&cfg),
            decl::<FileDecision>(&cfg),
            decl::<FileDecisionReport>(&cfg),
            decl::<FileDispatchItem>(&cfg),
            decl::<FileSendReport>(&cfg),
            decl::<FileDispatchReport>(&cfg),
            decl::<FileBatchItem>(&cfg),
            decl::<FileBatchReport>(&cfg),
            decl::<OutboxEntryDto>(&cfg),
            decl::<Outbox>(&cfg),
            decl::<OutboxClearReport>(&cfg),
            decl::<OutboxResendReport>(&cfg),
            decl::<ResumeMarker>(&cfg),
            decl::<ResumeMarkers>(&cfg),
            decl::<Storage>(&cfg),
            decl::<StorageCleanReport>(&cfg),
            decl::<AutoAcceptPolicyDto>(&cfg),
            decl::<AutoAcceptPolicyPatch>(&cfg),
            decl::<StorageCleanBody>(&cfg),
            decl::<FileSendRequest>(&cfg),
            decl::<FileSendMultiRequest>(&cfg),
            decl::<FileBatchEntry>(&cfg),
            decl::<FileBatchRequest>(&cfg),
            decl::<PeerProfileDto>(&cfg),
            decl::<PeerProfiles>(&cfg),
            decl::<PeerProfilePatch>(&cfg),
            decl::<PeerProfileUpdated>(&cfg),
            decl::<CapsUpdate>(&cfg),
            decl::<LocalCaps>(&cfg),
            decl::<PeerCaps>(&cfg),
            decl::<TopologyEdgeNeighbor>(&cfg),
            decl::<Topology>(&cfg),
            decl::<PeerTopologyDto>(&cfg),
            decl::<PeerTopologies>(&cfg),
            decl::<TopologyLinkRequest>(&cfg),
            decl::<TopologyLinkReport>(&cfg),
            decl::<TopologyUnlinkRequest>(&cfg),
            decl::<TopologyUnlinkReport>(&cfg),
            decl::<TopologyConfigPatch>(&cfg),
            decl::<TopologyConfigReport>(&cfg),
            decl::<TopologyPolicy>(&cfg),
            decl::<TopologyPolicyPatch>(&cfg),
            decl::<TopologySyncReport>(&cfg),
            decl::<RouteEntryDto>(&cfg),
            decl::<Routes>(&cfg),
            decl::<RouteSetRequest>(&cfg),
            decl::<RouteMutateReport>(&cfg),
            decl::<IfaceReport>(&cfg),
            decl::<OverlayStatus>(&cfg),
            decl::<Network>(&cfg),
            decl::<NetworkRefreshReport>(&cfg),
            decl::<DiscoveredPeer>(&cfg),
            decl::<Discovery>(&cfg),
            decl::<DiscoveryRescanReport>(&cfg),
            decl::<DaemonRestartRequest>(&cfg),
            decl::<DaemonRestartReport>(&cfg),
            decl::<DaemonShutdownReport>(&cfg),
            decl::<NotifySendRequest>(&cfg),
            decl::<NotifySendReport>(&cfg),
            decl::<NotifyCandidate>(&cfg),
            decl::<NotifyCandidates>(&cfg),
            decl::<NotifyReplyDto>(&cfg),
            decl::<NotifyReplies>(&cfg),
            decl::<NotifyOtpReport>(&cfg),
            decl::<NotifyActionReport>(&cfg),
            decl::<NotifyIdRequest>(&cfg),
            decl::<NotifyReplySendRequest>(&cfg),
            decl::<NotifyDismissRequest>(&cfg),
            decl::<NotifyDismissReport>(&cfg),
            decl::<NotifyOpenReport>(&cfg),
            decl::<NotifyReplyTakeRequest>(&cfg),
            decl::<NotifyReplyTakeReport>(&cfg),
            decl::<NotifyDndMode>(&cfg),
            decl::<NotifyConfigDto>(&cfg),
            decl::<NotifyConfigPatch>(&cfg),
            decl::<NotifyConfigTotals>(&cfg),
            decl::<NotifyConfigReportDto>(&cfg),
            decl::<NotifyTestRequestDto>(&cfg),
            decl::<NotifyTestReportDto>(&cfg),
            decl::<MessageEntry>(&cfg),
            decl::<MessageSendRequest>(&cfg),
            decl::<MessageSendReport>(&cfg),
            decl::<MessageDispatchItem>(&cfg),
            decl::<MessageDispatchReport>(&cfg),
            decl::<MessageQuery>(&cfg),
            decl::<MessageHistory>(&cfg),
            decl::<MessageClearRequest>(&cfg),
            decl::<MessageClearReport>(&cfg),
            decl::<MessageDirection>(&cfg),
            decl::<HandoffStatus>(&cfg),
            decl::<ActivityCandidate>(&cfg),
            decl::<ActivityReply>(&cfg),
            decl::<ActivityMessage>(&cfg),
            decl::<ActivityHandoff>(&cfg),
            decl::<Activity>(&cfg),
            decl::<ClipPolicyDto>(&cfg),
            decl::<ClipPolicyPatch>(&cfg),
            decl::<ClipPollReport>(&cfg),
            decl::<ClipStatus>(&cfg),
            decl::<ClipHistoryEntry>(&cfg),
            decl::<ClipHistory>(&cfg),
            decl::<ClipClearReport>(&cfg),
            decl::<ClipContent>(&cfg),
            decl::<ClipSyncRequest>(&cfg),
            decl::<ClipSyncReport>(&cfg),
            decl::<AuditEntryDto>(&cfg),
            decl::<AuditTrail>(&cfg),
            decl::<AuditQuery>(&cfg),
            decl::<AuditClearReport>(&cfg),
            decl::<LocalActionDto>(&cfg),
            decl::<LocalActions>(&cfg),
            decl::<RemoteActionDto>(&cfg),
            decl::<RemoteActions>(&cfg),
            decl::<LocalActionRequest>(&cfg),
            decl::<RemoteActionRefreshRequest>(&cfg),
            decl::<ActionTriggerRequest>(&cfg),
            decl::<ActionTriggerReport>(&cfg),
            decl::<ActionHistoryEntry>(&cfg),
            decl::<ActionHistory>(&cfg),
            decl::<ActionPolicyDto>(&cfg),
            decl::<ActionPolicyPatch>(&cfg),
            decl::<ActiveBenchSessionDto>(&cfg),
            decl::<BenchReportDto>(&cfg),
            decl::<BenchStatus>(&cfg),
            decl::<BenchDecision>(&cfg),
            decl::<BenchStartRequest>(&cfg),
            decl::<BenchPolicyDto>(&cfg),
            decl::<BenchPolicyPatch>(&cfg),
            decl::<StreamingRecommendationDto>(&cfg),
            decl::<DisplayTierDto>(&cfg),
            decl::<CameraTierDto>(&cfg),
            decl::<AudioTierDto>(&cfg),
            decl::<MicTierDto>(&cfg),
            decl::<RecommendationDto>(&cfg),
            decl::<RecommendationSummaryDto>(&cfg),
            decl::<RecommendationStatus>(&cfg),
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
