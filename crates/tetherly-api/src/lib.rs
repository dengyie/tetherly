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

// ---- §165a: battery/power domain ----

/// One battery measurement (local or a peer's broadcast).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct BatteryStateDto {
    /// Battery level percentage: 0..=100.
    #[cfg_attr(test, ts(type = "number"))]
    pub battery_level: u8,
    pub is_charging: bool,
    /// `PowerSource` lowercase string (`battery`/`ac`/`wireless`/`unknown`).
    pub power_source: String,
    pub battery_saver: bool,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub health_percent: Option<u8>,
    #[cfg_attr(test, ts(type = "number"))]
    pub timestamp_ms: u64,
}

/// `GET /battery/state?peer=` report (legacy `BatteryStatusReport` with the
/// `ok` envelope folded off).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct BatteryState {
    #[cfg_attr(test, ts(optional))]
    pub peer: Option<String>,
    /// Absent when the target has never broadcast a measurement.
    #[cfg_attr(test, ts(optional))]
    pub state: Option<BatteryStateDto>,
}

/// `PATCH /battery/state` body: sparse local state update.
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct BatteryStatePatch {
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub battery_level: Option<u8>,
    #[cfg_attr(test, ts(optional))]
    pub is_charging: Option<bool>,
    #[cfg_attr(test, ts(optional))]
    pub power_source: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub battery_saver: Option<bool>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub health_percent: Option<u8>,
}

/// The battery governance policy as one flat resource
/// (legacy `BatteryPolicyReport` with the `ok` envelope folded off).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct BatteryPolicyDto {
    #[cfg_attr(test, ts(type = "number"))]
    pub low_battery_threshold: u8,
    #[cfg_attr(test, ts(type = "number"))]
    pub critical_battery_threshold: u8,
    pub alert_on_low_battery: bool,
    pub auto_throttle_on_battery_saver: bool,
    pub allowed_groups: Vec<String>,
    pub allowed_tags: Vec<String>,
    /// Allowed device form factors, as the `DeviceType` snake_case strings.
    pub allowed_types: Vec<String>,
}

/// `PATCH /battery/policy` body: sparse policy fields (absent stays; the
/// `clear_filters` sweep wipes all three allow-lists in one call).
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct BatteryPolicyPatch {
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub low_battery_threshold: Option<u8>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub critical_battery_threshold: Option<u8>,
    #[cfg_attr(test, ts(optional))]
    pub alert_on_low_battery: Option<bool>,
    #[cfg_attr(test, ts(optional))]
    pub auto_throttle_on_battery_saver: Option<bool>,
    #[cfg_attr(test, ts(optional))]
    pub allowed_groups: Option<Vec<String>>,
    #[cfg_attr(test, ts(optional))]
    pub allowed_tags: Option<Vec<String>>,
    #[cfg_attr(test, ts(optional))]
    pub allowed_types: Option<Vec<String>>,
    #[cfg_attr(test, ts(optional))]
    pub clear_filters: Option<bool>,
}

// ---- §165a: media control domain ----

/// `POST /media/command` body: one playback command, single peer or
/// selector fan-out (`all`/`tag`/`group`/`device_type`).
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct MediaCommandRequest {
    #[cfg_attr(test, ts(optional))]
    pub peer: Option<String>,
    /// `play`/`pause`/`play_pause`/`stop`/`next`/`prev`/`seek`/`volume`/`toggle_mute`.
    #[cfg_attr(test, ts(optional))]
    pub command: Option<String>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub volume: Option<u8>,
    /// Seek offset for `seek` (ms; negative rewinds).
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub offset_ms: Option<i64>,
    #[cfg_attr(test, ts(optional))]
    pub target_player: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub tag: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub group: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub device_type: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub all: Option<bool>,
}

/// `POST /media/command` single-peer result (legacy `MediaCommandSendResult`
/// with the `ok` envelope folded off).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct MediaCommandReport {
    pub id: String,
    pub peer: String,
    #[cfg_attr(test, ts(optional))]
    pub peer_alias: Option<String>,
    /// Normalized `MediaCommand` wire string (e.g. `play`, `seek:-30000`).
    pub command: String,
    #[cfg_attr(test, ts(optional))]
    pub target_player: Option<String>,
    #[cfg_attr(test, ts(type = "number"))]
    pub timestamp_ms: u64,
}

/// One peer's slice of a selector fan-out dispatch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct MediaDispatchItem {
    pub peer: String,
    #[cfg_attr(test, ts(optional))]
    pub peer_alias: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub command_id: Option<String>,
    pub success: bool,
    #[cfg_attr(test, ts(optional))]
    pub error: Option<String>,
}

/// `POST /media/command` selector fan-out report (legacy
/// `MultiMediaCommandReport` with the `ok` envelope folded off).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct MediaCommandMultiReport {
    /// Selector wire string (e.g. `all`, `tag:kiosk`, `type:phone`).
    pub selector: String,
    pub command: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub target_count: usize,
    #[cfg_attr(test, ts(type = "number"))]
    pub success_count: usize,
    #[cfg_attr(test, ts(type = "number"))]
    pub skipped_offline_count: usize,
    #[cfg_attr(test, ts(type = "number"))]
    pub failed_count: usize,
    pub dispatches: Vec<MediaDispatchItem>,
}

/// One media playback state snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct MediaStateDto {
    pub player_id: String,
    /// `PlaybackState` lowercase string (`stopped`/`playing`/`paused`).
    pub state: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub position_ms: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub duration_ms: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub volume_percent: u8,
    pub muted: bool,
    pub can_play: bool,
    pub can_pause: bool,
    pub can_go_next: bool,
    pub can_go_previous: bool,
    pub can_seek: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub timestamp_ms: u64,
}

/// `GET /media/state?peer=` report (legacy `MediaStatusReport` with the
/// `ok` envelope folded off).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct MediaState {
    #[cfg_attr(test, ts(optional))]
    pub peer: Option<String>,
    /// Absent when nothing has been broadcast yet.
    #[cfg_attr(test, ts(optional))]
    pub state: Option<MediaStateDto>,
}

/// `PATCH /media/state` body: sparse local playback state update.
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct MediaStatePatch {
    #[cfg_attr(test, ts(optional))]
    pub player_id: Option<String>,
    /// `PlaybackState` lowercase string.
    #[cfg_attr(test, ts(optional))]
    pub state: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub title: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub artist: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub album: Option<String>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub position_ms: Option<u64>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub duration_ms: Option<u64>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub volume_percent: Option<u8>,
    #[cfg_attr(test, ts(optional))]
    pub muted: Option<bool>,
    #[cfg_attr(test, ts(optional))]
    pub can_play: Option<bool>,
    #[cfg_attr(test, ts(optional))]
    pub can_pause: Option<bool>,
    #[cfg_attr(test, ts(optional))]
    pub can_go_next: Option<bool>,
    #[cfg_attr(test, ts(optional))]
    pub can_go_previous: Option<bool>,
    #[cfg_attr(test, ts(optional))]
    pub can_seek: Option<bool>,
}

/// `PATCH /media/state` report: the sanitized state after the update.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct MediaStateUpdateReport {
    pub state: MediaStateDto,
}

/// The media control policy as one flat resource
/// (legacy `MediaPolicyReport` with the `ok` envelope folded off).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct MediaPolicyDto {
    /// `MediaControlMode` snake_case string (`allow`/`prompt_only`/`disabled`).
    pub mode: String,
    pub allowed_groups: Vec<String>,
    pub allowed_tags: Vec<String>,
    /// Allowed device form factors, as the `DeviceType` snake_case strings.
    pub allowed_types: Vec<String>,
    #[cfg_attr(test, ts(type = "number"))]
    pub max_volume_limit: u8,
    pub allow_seek: bool,
}

/// `PATCH /media/policy` body: sparse policy fields (absent stays; the
/// `clear_filters` sweep wipes all three allow-lists in one call).
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct MediaPolicyPatch {
    #[cfg_attr(test, ts(optional))]
    pub mode: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub allowed_groups: Option<Vec<String>>,
    #[cfg_attr(test, ts(optional))]
    pub allowed_tags: Option<Vec<String>>,
    #[cfg_attr(test, ts(optional))]
    pub allowed_types: Option<Vec<String>>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub max_volume_limit: Option<u8>,
    #[cfg_attr(test, ts(optional))]
    pub allow_seek: Option<bool>,
    #[cfg_attr(test, ts(optional))]
    pub clear_filters: Option<bool>,
}

// ---- §165b: input tunnel domain ----

/// One active input-tunnel session (legacy `ActiveInputTunnelSession`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ActiveInputSessionDto {
    pub session_id: String,
    pub peer_id: String,
    /// `InputTunnelRole` snake_case string (`controller`/`target`).
    pub role: String,
    #[cfg_attr(test, ts(optional))]
    pub requested_edge: Option<String>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub entry_x: Option<i32>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub entry_y: Option<i32>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub capture_width: Option<u32>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub capture_height: Option<u32>,
    #[cfg_attr(test, ts(type = "number"))]
    pub pre_arm_epoch: u64,
}

/// `GET /input` report — input sharing status (legacy `InputStatusReport`
/// with the `ok` envelope folded off).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct InputStatus {
    pub channel_open: bool,
    /// `CursorSeat` snake_case string (`local`/`remote`).
    pub seat: String,
    #[cfg_attr(test, ts(optional))]
    pub active_peer: Option<String>,
    #[cfg_attr(test, ts(type = "number"))]
    pub applied: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub dropped: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub rx_buttons: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub rx_wheel: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub rx_keys: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub tx_buttons: u64,
}

/// One input-engine decision ring entry (legacy `InputTraceEntry`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct InputTraceEntryDto {
    #[cfg_attr(test, ts(type = "number"))]
    pub ts_ms: u64,
    pub kind: String,
    pub detail: String,
}

/// `GET /input/trace` report (legacy `InputTraceReport` folded).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct InputTrace {
    pub seat: String,
    pub channel_open: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub capacity: usize,
    pub entries: Vec<InputTraceEntryDto>,
}

/// `POST /input/start` + `POST /input/stop` control report
/// (legacy `InputControlReport` with the `ok` envelope folded off).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct InputControl {
    pub action: String,
    pub seat: String,
    pub channel_open: bool,
    #[cfg_attr(test, ts(optional))]
    pub peer: Option<String>,
    #[cfg_attr(test, ts(type = "number"))]
    pub timestamp_ms: u64,
}

/// `GET /input/tunnel` report (legacy `InputTunnelStatusReport` folded).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct InputTunnelStatus {
    pub active: bool,
    #[cfg_attr(test, ts(optional))]
    pub session: Option<ActiveInputSessionDto>,
    pub policy: InputPolicyDto,
    pub accessibility_trusted: bool,
    pub capture_active: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub timestamp_ms: u64,
}

/// `POST /input/tunnel/attach` + `POST /input/tunnel/release` decision
/// report (legacy `InputTunnelDecisionReport` folded).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct InputTunnelDecision {
    pub action: String,
    #[cfg_attr(test, ts(optional))]
    pub session_id: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub peer_id: Option<String>,
    /// `InputTunnelRole` snake_case string.
    #[cfg_attr(test, ts(optional))]
    pub role: Option<String>,
    pub message: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub timestamp_ms: u64,
}

/// The input-tunnel governance policy as one flat resource
/// (legacy `InputTunnelPolicy` with enums folded to strings).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct InputPolicyDto {
    pub enabled: bool,
    pub allow_inbound_control: bool,
    pub allow_keyboard: bool,
    pub allow_mouse: bool,
    pub hotkey_release_enabled: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub idle_timeout_secs: u32,
    pub allowed_groups: Vec<String>,
    pub allowed_tags: Vec<String>,
    /// `DeviceType` snake_case strings.
    pub allowed_types: Vec<String>,
    #[cfg_attr(test, ts(optional))]
    pub keymap: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub edge: Option<String>,
    #[cfg_attr(test, ts(type = "number"))]
    pub wheel_speed: f64,
}

/// `PATCH /input/policy` body: sparse policy fields (absent stays; the
/// `clear_filters` sweep wipes all three allow-lists in one call).
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct InputPolicyPatch {
    #[cfg_attr(test, ts(optional))]
    pub enabled: Option<bool>,
    #[cfg_attr(test, ts(optional))]
    pub allow_inbound_control: Option<bool>,
    #[cfg_attr(test, ts(optional))]
    pub allow_keyboard: Option<bool>,
    #[cfg_attr(test, ts(optional))]
    pub allow_mouse: Option<bool>,
    #[cfg_attr(test, ts(optional))]
    pub hotkey_release_enabled: Option<bool>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub idle_timeout_secs: Option<u32>,
    #[cfg_attr(test, ts(optional))]
    pub allowed_groups: Option<Vec<String>>,
    #[cfg_attr(test, ts(optional))]
    pub allowed_tags: Option<Vec<String>>,
    #[cfg_attr(test, ts(optional))]
    pub allowed_types: Option<Vec<String>>,
    #[cfg_attr(test, ts(optional))]
    pub keymap: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub edge: Option<String>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub wheel_speed: Option<f64>,
    #[cfg_attr(test, ts(optional))]
    pub clear_filters: Option<bool>,
}

/// `POST /input/tunnel/attach` body.
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct InputAttachRequest {
    #[cfg_attr(test, ts(optional))]
    pub peer: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub edge: Option<String>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub x: Option<i32>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub y: Option<i32>,
}

/// `POST /input/tunnel/release` body.
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct InputReleaseRequest {
    #[cfg_attr(test, ts(optional))]
    pub reason: Option<String>,
}

/// `POST /input/start` body.
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct InputStartRequest {
    #[cfg_attr(test, ts(optional))]
    pub peer: Option<String>,
}

// ---- §165b: handoff domain ----

/// One inbound handoff inbox entry (legacy `HandoffInboxEntryDto`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct HandoffInboxEntryDto {
    pub id: String,
    pub from_peer: String,
    #[cfg_attr(test, ts(optional))]
    pub from_alias: Option<String>,
    pub url: String,
    pub title: String,
    pub auto_open_requested: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub received_at_ms: u64,
}

/// `GET /handoff/inbox` report (legacy `HandoffInboxReport` folded).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct HandoffInbox {
    pub count: usize,
    pub entries: Vec<HandoffInboxEntryDto>,
}

/// `POST /handoff/send` single-peer result (legacy `HandoffSendResult`
/// with the `ok` envelope folded off).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct HandoffSendReport {
    pub id: String,
    pub peer: String,
    #[cfg_attr(test, ts(optional))]
    pub peer_alias: Option<String>,
    pub url: String,
    pub title: String,
    pub auto_open_requested: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub timestamp_ms: u64,
}

/// One peer's slice of a handoff fan-out dispatch (legacy `PeerHandoffSendItem`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct HandoffDispatchItem {
    pub peer: String,
    #[cfg_attr(test, ts(optional))]
    pub peer_alias: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub id: Option<String>,
    pub success: bool,
    #[cfg_attr(test, ts(optional))]
    pub error: Option<String>,
}

/// `POST /handoff/send` selector fan-out report (legacy
/// `MultiHandoffSendReport` with the `ok` envelope folded off).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct HandoffSendMultiReport {
    /// Selector wire string (e.g. `all`, `tag:kiosk`, `type:phone`).
    pub selector: String,
    pub url: String,
    pub title: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub target_count: usize,
    #[cfg_attr(test, ts(type = "number"))]
    pub success_count: usize,
    #[cfg_attr(test, ts(type = "number"))]
    pub skipped_offline_count: usize,
    #[cfg_attr(test, ts(type = "number"))]
    pub failed_count: usize,
    pub dispatches: Vec<HandoffDispatchItem>,
}

/// `POST /handoff/inbox/open` + `POST /handoff/inbox/dismiss` report
/// (legacy `HandoffInboxActionReport` folded).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct HandoffInboxActionReport {
    /// `"open"` or `"dismiss"`.
    pub action: String,
    pub id: String,
    pub remaining: usize,
    #[cfg_attr(test, ts(type = "number"))]
    pub timestamp_ms: u64,
}

/// `POST /handoff/inbox/clear` report (legacy `HandoffInboxClearReport` folded).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct HandoffInboxClearReport {
    pub cleared_count: usize,
    #[cfg_attr(test, ts(type = "number"))]
    pub timestamp_ms: u64,
}

/// The handoff policy as one flat resource (legacy `HandoffPolicy`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct HandoffPolicyDto {
    /// Handoff admission action (`HandoffPolicyAction` snake_case string).
    pub action: String,
    pub allowed_groups: Vec<String>,
    pub allowed_tags: Vec<String>,
    /// `DeviceType` snake_case strings.
    pub allowed_types: Vec<String>,
    #[cfg_attr(test, ts(type = "number"))]
    pub max_inbox_entries: usize,
}

/// `PATCH /handoff/policy` body: sparse policy fields.
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct HandoffPolicyPatch {
    #[cfg_attr(test, ts(optional))]
    pub action: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub allowed_groups: Option<Vec<String>>,
    #[cfg_attr(test, ts(optional))]
    pub allowed_tags: Option<Vec<String>>,
    #[cfg_attr(test, ts(optional))]
    pub allowed_types: Option<Vec<String>>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub max_inbox_entries: Option<usize>,
    #[cfg_attr(test, ts(optional))]
    pub clear_filters: Option<bool>,
}

/// `POST /handoff/send` body: one link, single peer or selector fan-out.
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct HandoffSendRequest {
    #[cfg_attr(test, ts(optional))]
    pub peer: Option<String>,
    pub url: String,
    #[cfg_attr(test, ts(optional))]
    pub title: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub auto_open: Option<bool>,
    #[cfg_attr(test, ts(optional))]
    pub tag: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub group: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub device_type: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub all: Option<bool>,
}

/// `POST /handoff/inbox/open` + `.../dismiss` body.
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct HandoffInboxActionRequest {
    #[cfg_attr(test, ts(optional))]
    pub id: Option<String>,
}

// ---- §165b: focus domain ----

/// One focus-state frame (legacy `FocusStateFrame` with the mode folded).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct FocusStateDto {
    pub enabled: bool,
    /// `FocusMode` snake_case string (`dnd`/`work`/`meeting`/`sleep`).
    pub mode: String,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub expires_ms: Option<u64>,
    #[cfg_attr(test, ts(optional))]
    pub reason: Option<String>,
    #[cfg_attr(test, ts(type = "number"))]
    pub timestamp_ms: u64,
}

/// One peer's focus report (legacy `PeerFocusReport` folded).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct PeerFocusDto {
    pub peer: String,
    #[cfg_attr(test, ts(optional))]
    pub peer_alias: Option<String>,
    pub state: FocusStateDto,
    pub active: bool,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub remaining_secs: Option<u64>,
}

/// `GET /focus/peers` report (legacy `PeersFocusReport` folded).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct FocusPeers {
    pub count: usize,
    pub peers: Vec<PeerFocusDto>,
}

/// `GET /focus/state` report: local focus state (legacy `LocalFocusReport` folded).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct FocusState {
    pub state: FocusStateDto,
    pub active: bool,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub remaining_secs: Option<u64>,
}

/// `POST /focus/state` report (legacy `FocusSetReport` with the `ok`
/// envelope folded off).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct FocusSetReport {
    pub enabled: bool,
    /// `FocusMode` snake_case string.
    pub mode: String,
    pub active: bool,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub expires_ms: Option<u64>,
    #[cfg_attr(test, ts(optional))]
    pub reason: Option<String>,
    #[cfg_attr(test, ts(type = "number"))]
    pub broadcast_count: usize,
    pub message: String,
}

/// `POST /focus/sync` report (legacy `FocusSyncReport` folded).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct FocusSyncReport {
    pub requested: usize,
    pub responded: usize,
    pub peers: Vec<PeerFocusDto>,
    #[cfg_attr(test, ts(type = "number"))]
    pub timestamp_ms: u64,
}

/// The focus / quiet-hours policy as one flat resource (legacy `FocusPolicy`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct FocusPolicyDto {
    pub enabled: bool,
    /// `FocusMode` snake_case string.
    pub default_mode: String,
    pub mirror_inbound_focus: bool,
    pub allow_emergency_breakthrough: bool,
    pub quiet_hours_enabled: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub quiet_hours_start_minute: u16,
    #[cfg_attr(test, ts(type = "number"))]
    pub quiet_hours_end_minute: u16,
    pub allowed_groups: Vec<String>,
    pub allowed_tags: Vec<String>,
    /// `DeviceType` snake_case strings.
    pub allowed_types: Vec<String>,
}

/// `PATCH /focus/policy` body: sparse policy fields.
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct FocusPolicyPatch {
    #[cfg_attr(test, ts(optional))]
    pub enabled: Option<bool>,
    #[cfg_attr(test, ts(optional))]
    pub default_mode: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub mirror_inbound_focus: Option<bool>,
    #[cfg_attr(test, ts(optional))]
    pub allow_emergency_breakthrough: Option<bool>,
    #[cfg_attr(test, ts(optional))]
    pub quiet_hours_enabled: Option<bool>,
    /// `HH:MM` shorthand folded into minutes on apply.
    #[cfg_attr(test, ts(optional))]
    pub quiet_hours_start: Option<String>,
    /// `HH:MM` shorthand folded into minutes on apply.
    #[cfg_attr(test, ts(optional))]
    pub quiet_hours_end: Option<String>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub quiet_hours_start_minute: Option<u16>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub quiet_hours_end_minute: Option<u16>,
    #[cfg_attr(test, ts(optional))]
    pub allowed_groups: Option<Vec<String>>,
    #[cfg_attr(test, ts(optional))]
    pub allowed_tags: Option<Vec<String>>,
    #[cfg_attr(test, ts(optional))]
    pub allowed_types: Option<Vec<String>>,
    #[cfg_attr(test, ts(optional))]
    pub clear_filters: Option<bool>,
}

/// `POST /focus/state` body.
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct FocusSetRequest {
    #[cfg_attr(test, ts(optional))]
    pub enabled: Option<bool>,
    #[cfg_attr(test, ts(optional))]
    pub mode: Option<String>,
    /// Duration in minutes (`duration`/`duration_minutes` aliases folded).
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub duration_mins: Option<u32>,
    #[cfg_attr(test, ts(optional))]
    pub reason: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub broadcast: Option<bool>,
}

// ---- §165b: camera domain ----

/// One active camera streaming session (legacy `ActiveCameraSession`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ActiveCameraSessionDto {
    pub session_id: String,
    pub peer_id: String,
    /// `CameraRole` snake_case string (`viewer`/`lens`).
    pub role: String,
    pub lens_id: String,
    pub width: u32,
    pub height: u32,
    #[cfg_attr(test, ts(type = "number"))]
    pub target_fps: u32,
    #[cfg_attr(test, ts(type = "number"))]
    pub current_fps: u32,
    /// `CameraCodec` snake_case string.
    pub codec: String,
    pub flashlight_on: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub zoom_ratio: f32,
    #[cfg_attr(test, ts(type = "number"))]
    pub frames_sent: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub frames_received: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub bytes_transferred: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub started_at_ms: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub last_activity_ms: u64,
}

/// `GET /camera` report (legacy `CameraStatusReport` folded).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct CameraStatus {
    pub active: bool,
    #[cfg_attr(test, ts(optional))]
    pub session: Option<ActiveCameraSessionDto>,
    pub policy: CameraPolicyDto,
    #[cfg_attr(test, ts(type = "number"))]
    pub timestamp_ms: u64,
}

/// `POST /camera/attach` + `POST /camera/detach` decision report
/// (legacy `CameraDecisionReport` folded).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct CameraDecision {
    pub action: String,
    #[cfg_attr(test, ts(optional))]
    pub session_id: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub peer_id: Option<String>,
    /// `CameraRole` snake_case string.
    #[cfg_attr(test, ts(optional))]
    pub role: Option<String>,
    pub message: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub timestamp_ms: u64,
}

/// The camera policy as one flat resource (legacy `CameraPolicy`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct CameraPolicyDto {
    pub enabled: bool,
    pub allow_inbound_stream: bool,
    pub allow_remote_capture: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub max_resolution_width: u32,
    #[cfg_attr(test, ts(type = "number"))]
    pub max_resolution_height: u32,
    #[cfg_attr(test, ts(type = "number"))]
    pub max_fps: u32,
    pub enforce_privacy_shutter_indicator: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub idle_timeout_secs: u32,
    pub allowed_groups: Vec<String>,
    pub allowed_tags: Vec<String>,
    /// `DeviceType` snake_case strings.
    pub allowed_types: Vec<String>,
}

/// `PATCH /camera/policy` body: sparse policy fields.
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct CameraPolicyPatch {
    #[cfg_attr(test, ts(optional))]
    pub enabled: Option<bool>,
    #[cfg_attr(test, ts(optional))]
    pub allow_inbound_stream: Option<bool>,
    #[cfg_attr(test, ts(optional))]
    pub allow_remote_capture: Option<bool>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub max_resolution_width: Option<u32>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub max_resolution_height: Option<u32>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub max_fps: Option<u32>,
    #[cfg_attr(test, ts(optional))]
    pub enforce_privacy_shutter_indicator: Option<bool>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub idle_timeout_secs: Option<u32>,
    #[cfg_attr(test, ts(optional))]
    pub allowed_groups: Option<Vec<String>>,
    #[cfg_attr(test, ts(optional))]
    pub allowed_tags: Option<Vec<String>>,
    #[cfg_attr(test, ts(optional))]
    pub allowed_types: Option<Vec<String>>,
    #[cfg_attr(test, ts(optional))]
    pub clear_filters: Option<bool>,
}

/// `POST /camera/attach` body.
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct CameraAttachRequest {
    #[cfg_attr(test, ts(optional))]
    pub peer: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub lens_id: Option<String>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub width: Option<u32>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub height: Option<u32>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub fps: Option<u32>,
    #[cfg_attr(test, ts(optional))]
    pub codec: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub flashlight: Option<bool>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub zoom: Option<f32>,
}

/// `POST /camera/detach` body.
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct CameraDetachRequest {
    #[cfg_attr(test, ts(optional))]
    pub reason: Option<String>,
}

// ---- §165b: mic domain ----

/// One active microphone session (legacy `ActiveMicrophoneSession`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ActiveMicSessionDto {
    pub session_id: String,
    pub peer_id: String,
    /// `MicRole` snake_case string (`listener`/`source`).
    pub role: String,
    /// `MicMode` snake_case string (`stream`/`intercom`).
    pub mode: String,
    pub mic_id: String,
    /// `MicCodec` snake_case string.
    pub codec: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub sample_rate: u32,
    #[cfg_attr(test, ts(type = "number"))]
    pub channels: u32,
    #[cfg_attr(test, ts(type = "number"))]
    pub gain: f32,
    pub muted: bool,
    pub noise_suppression: bool,
    pub echo_cancellation: bool,
    pub auto_gain: bool,
    pub privacy_indicator_active: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub frames_sent: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub frames_received: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub bytes_transferred: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub started_at_ms: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub last_activity_ms: u64,
}

/// `GET /mic` report (legacy `MicrophoneStatusReport` folded).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct MicStatus {
    pub active: bool,
    #[cfg_attr(test, ts(optional))]
    pub session: Option<ActiveMicSessionDto>,
    pub policy: MicPolicyDto,
    #[cfg_attr(test, ts(type = "number"))]
    pub timestamp_ms: u64,
}

/// `POST /mic/attach` + `POST /mic/detach` decision report
/// (legacy `MicrophoneDecisionReport` folded).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct MicDecision {
    pub action: String,
    #[cfg_attr(test, ts(optional))]
    pub session_id: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub peer_id: Option<String>,
    /// `MicRole` snake_case string.
    #[cfg_attr(test, ts(optional))]
    pub role: Option<String>,
    pub message: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub timestamp_ms: u64,
}

/// The microphone policy as one flat resource (legacy `MicrophonePolicy`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct MicPolicyDto {
    pub enabled: bool,
    pub allow_inbound_stream: bool,
    pub allow_remote_capture: bool,
    pub allow_intercom: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub max_sample_rate: u32,
    #[cfg_attr(test, ts(type = "number"))]
    pub max_session_duration_secs: u32,
    pub enforce_privacy_indicator: bool,
    pub default_muted: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub idle_timeout_secs: u32,
    pub allowed_groups: Vec<String>,
    pub allowed_tags: Vec<String>,
    /// `DeviceType` snake_case strings.
    pub allowed_types: Vec<String>,
}

/// `PATCH /mic/policy` body: sparse policy fields.
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct MicPolicyPatch {
    #[cfg_attr(test, ts(optional))]
    pub enabled: Option<bool>,
    #[cfg_attr(test, ts(optional))]
    pub allow_inbound_stream: Option<bool>,
    #[cfg_attr(test, ts(optional))]
    pub allow_remote_capture: Option<bool>,
    #[cfg_attr(test, ts(optional))]
    pub allow_intercom: Option<bool>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub max_sample_rate: Option<u32>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub max_session_duration_secs: Option<u32>,
    #[cfg_attr(test, ts(optional))]
    pub enforce_privacy_indicator: Option<bool>,
    #[cfg_attr(test, ts(optional))]
    pub default_muted: Option<bool>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub idle_timeout_secs: Option<u32>,
    #[cfg_attr(test, ts(optional))]
    pub allowed_groups: Option<Vec<String>>,
    #[cfg_attr(test, ts(optional))]
    pub allowed_tags: Option<Vec<String>>,
    #[cfg_attr(test, ts(optional))]
    pub allowed_types: Option<Vec<String>>,
    #[cfg_attr(test, ts(optional))]
    pub clear_filters: Option<bool>,
}

/// `POST /mic/attach` body.
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct MicAttachRequest {
    #[cfg_attr(test, ts(optional))]
    pub peer: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub mic_id: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub mode: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub codec: Option<String>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub sample_rate: Option<u32>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub channels: Option<u32>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub gain: Option<f32>,
    #[cfg_attr(test, ts(optional))]
    pub muted: Option<bool>,
    #[cfg_attr(test, ts(optional))]
    pub noise_suppression: Option<bool>,
    #[cfg_attr(test, ts(optional))]
    pub echo_cancellation: Option<bool>,
    #[cfg_attr(test, ts(optional))]
    pub auto_gain: Option<bool>,
}

/// `POST /mic/detach` body.
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct MicDetachRequest {
    #[cfg_attr(test, ts(optional))]
    pub reason: Option<String>,
}

// ---- §165b: audio routing domain ----

/// Spatial position (x/y/z metres).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct SpatialPositionDto {
    #[cfg_attr(test, ts(type = "number"))]
    pub x: f32,
    #[cfg_attr(test, ts(type = "number"))]
    pub y: f32,
    #[cfg_attr(test, ts(type = "number"))]
    pub z: f32,
}

/// One active audio-routing session (legacy `ActiveAudioRoutingSession`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ActiveAudioSessionDto {
    pub session_id: String,
    pub peer_id: String,
    /// `AudioRoutingRole` snake_case string (`speaker`/`source`).
    pub role: String,
    /// `AudioChannelLayout` snake_case string (`mono`/`stereo`).
    pub layout: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub sample_rate: u32,
    #[cfg_attr(test, ts(type = "number"))]
    pub channels: u8,
    /// `AudioSampleFormat` snake_case string.
    pub format: String,
    /// `AudioStreamCodec` snake_case string.
    pub codec: String,
    #[cfg_attr(test, ts(optional))]
    pub spatial_pos: Option<SpatialPositionDto>,
    #[cfg_attr(test, ts(type = "number"))]
    pub volume_gain: f32,
    #[cfg_attr(test, ts(type = "number"))]
    pub chunks_sent: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub chunks_received: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub bytes_transferred: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub started_at_ms: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub last_activity_ms: u64,
}

/// `GET /audio` report (legacy `AudioRoutingStatusReport` folded).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct AudioStatus {
    pub active: bool,
    #[cfg_attr(test, ts(optional))]
    pub session: Option<ActiveAudioSessionDto>,
    pub policy: AudioPolicyDto,
    #[cfg_attr(test, ts(type = "number"))]
    pub timestamp_ms: u64,
}

/// `POST /audio/attach` + `POST /audio/detach` decision report
/// (legacy `AudioRoutingDecisionReport` folded).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct AudioDecision {
    pub action: String,
    #[cfg_attr(test, ts(optional))]
    pub session_id: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub peer_id: Option<String>,
    /// `AudioRoutingRole` snake_case string.
    #[cfg_attr(test, ts(optional))]
    pub role: Option<String>,
    pub message: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub timestamp_ms: u64,
}

/// The audio-routing policy as one flat resource (legacy `AudioRoutingPolicy`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct AudioPolicyDto {
    pub enabled: bool,
    pub allow_inbound_sink: bool,
    pub allow_outbound_routing: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub max_volume_limit: u8,
    pub allow_spatial_rendering: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub idle_timeout_secs: u32,
    pub allowed_groups: Vec<String>,
    pub allowed_tags: Vec<String>,
    /// `DeviceType` snake_case strings.
    pub allowed_types: Vec<String>,
}

/// `PATCH /audio/policy` body: sparse policy fields.
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct AudioPolicyPatch {
    #[cfg_attr(test, ts(optional))]
    pub enabled: Option<bool>,
    #[cfg_attr(test, ts(optional))]
    pub allow_inbound_sink: Option<bool>,
    #[cfg_attr(test, ts(optional))]
    pub allow_outbound_routing: Option<bool>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub max_volume_limit: Option<u8>,
    #[cfg_attr(test, ts(optional))]
    pub allow_spatial_rendering: Option<bool>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub idle_timeout_secs: Option<u32>,
    #[cfg_attr(test, ts(optional))]
    pub allowed_groups: Option<Vec<String>>,
    #[cfg_attr(test, ts(optional))]
    pub allowed_tags: Option<Vec<String>>,
    #[cfg_attr(test, ts(optional))]
    pub allowed_types: Option<Vec<String>>,
    #[cfg_attr(test, ts(optional))]
    pub clear_filters: Option<bool>,
}

/// `POST /audio/attach` body.
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct AudioAttachRequest {
    #[cfg_attr(test, ts(optional))]
    pub peer: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub layout: Option<String>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub sample_rate: Option<u32>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub channels: Option<u8>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub spatial_x: Option<f32>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub spatial_y: Option<f32>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub spatial_z: Option<f32>,
    #[cfg_attr(test, ts(optional))]
    pub spatial_position: Option<SpatialPositionDto>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub volume_gain: Option<f32>,
}

/// `POST /audio/detach` body.
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct AudioDetachRequest {
    #[cfg_attr(test, ts(optional))]
    pub reason: Option<String>,
}

// ---- §165b: locate domain ----

/// One active acoustic locate ring (legacy `ActiveLocateRing`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ActiveLocateRingDto {
    pub request_id: String,
    pub initiator_peer: String,
    pub initiator_alias: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub duration_secs: u32,
    /// `LocatePattern` snake_case string (`pulse`/`sos`/`chime`).
    pub pattern: String,
    pub override_silent: bool,
    #[cfg_attr(test, ts(optional))]
    pub message: Option<String>,
    #[cfg_attr(test, ts(type = "number"))]
    pub started_at_ms: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub expires_at_ms: u64,
}

/// `GET /locate/status` report (legacy `LocateStatusReport` folded).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct LocateStatus {
    pub is_ringing: bool,
    #[cfg_attr(test, ts(optional))]
    pub active_ring: Option<ActiveLocateRingDto>,
    pub policy: LocatePolicyDto,
    pub message: String,
}

/// `POST /locate/ring` report (legacy `LocateRingReport` folded).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct LocateRingReport {
    pub request_id: String,
    pub target_peer: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub duration_secs: u32,
    /// `LocatePattern` snake_case string.
    pub pattern: String,
    pub override_silent: bool,
    pub message: String,
}

/// `POST /locate/stop` report (legacy `LocateStopReport` folded).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct LocateStopReport {
    #[cfg_attr(test, ts(optional))]
    pub request_id: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub target_peer: Option<String>,
    pub reason: String,
    pub message: String,
}

/// The locator policy as one flat resource (legacy `LocatorPolicy`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct LocatePolicyDto {
    pub enabled: bool,
    pub allow_override_silent: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub max_duration_secs: u32,
    pub allowed_groups: Vec<String>,
    pub allowed_tags: Vec<String>,
    /// `DeviceType` snake_case strings.
    pub allowed_types: Vec<String>,
}

/// `PATCH /locate/policy` body: sparse policy fields.
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct LocatePolicyPatch {
    #[cfg_attr(test, ts(optional))]
    pub enabled: Option<bool>,
    #[cfg_attr(test, ts(optional))]
    pub allow_override_silent: Option<bool>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub max_duration_secs: Option<u32>,
    #[cfg_attr(test, ts(optional))]
    pub allowed_groups: Option<Vec<String>>,
    #[cfg_attr(test, ts(optional))]
    pub allowed_tags: Option<Vec<String>>,
    #[cfg_attr(test, ts(optional))]
    pub allowed_types: Option<Vec<String>>,
    #[cfg_attr(test, ts(optional))]
    pub clear_filters: Option<bool>,
}

/// `POST /locate/ring` body.
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct LocateRingRequest {
    #[cfg_attr(test, ts(optional))]
    pub peer: Option<String>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub duration_secs: Option<u32>,
    #[cfg_attr(test, ts(optional))]
    pub pattern: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub override_silent: Option<bool>,
    #[cfg_attr(test, ts(optional))]
    pub message: Option<String>,
}

/// `POST /locate/stop` body.
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct LocateStopRequest {
    #[cfg_attr(test, ts(optional))]
    pub peer: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub request_id: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub reason: Option<String>,
}

// ---- §165b: virtual display domain ----

/// Display geometry and refresh rate (legacy core `VirtualDisplayMode`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct DisplayModeDto {
    #[cfg_attr(test, ts(type = "number"))]
    pub width: u32,
    #[cfg_attr(test, ts(type = "number"))]
    pub height: u32,
    #[cfg_attr(test, ts(type = "number"))]
    pub refresh_rate: u32,
    #[cfg_attr(test, ts(type = "number"))]
    pub scale_factor: f32,
    /// `VirtualDisplayPixelFormat` lowercase string (`bgra8`/`rgba8`).
    pub pixel_format: String,
}

/// One active virtual-display session (legacy `ActiveVirtualDisplaySession`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ActiveDisplaySessionDto {
    pub session_id: String,
    pub peer_id: String,
    /// `VirtualDisplayRole` snake_case string (`viewer`/`host`).
    pub role: String,
    /// `DisplayEdge` snake_case string.
    pub assigned_edge: String,
    /// Display geometry as advertised by the host peer.
    pub mode: DisplayModeDto,
    /// `VirtualDisplayCodec` snake_case string.
    pub codec: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub target_fps: u32,
    #[cfg_attr(test, ts(type = "number"))]
    pub current_fps: u32,
    #[cfg_attr(test, ts(type = "number"))]
    pub rtt_us: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub dropped_frames: u32,
    #[cfg_attr(test, ts(type = "number"))]
    pub started_at_ms: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub last_activity_ms: u64,
}

/// `GET /display/virtual` report (legacy `VirtualDisplayStatusReport` folded).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct DisplayVirtualStatus {
    pub active: bool,
    #[cfg_attr(test, ts(optional))]
    pub session: Option<ActiveDisplaySessionDto>,
    pub policy: DisplayPolicyDto,
    #[cfg_attr(test, ts(type = "number"))]
    pub timestamp_ms: u64,
}

/// `POST /display/virtual/attach` + `.../detach` decision report
/// (legacy `VirtualDisplayDecisionReport` folded).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct DisplayDecision {
    pub action: String,
    #[cfg_attr(test, ts(optional))]
    pub session_id: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub peer_id: Option<String>,
    /// `VirtualDisplayRole` snake_case string.
    #[cfg_attr(test, ts(optional))]
    pub role: Option<String>,
    pub message: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub timestamp_ms: u64,
}

/// The virtual-display policy as one flat resource (legacy `VirtualDisplayPolicy`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct DisplayPolicyDto {
    pub enabled: bool,
    pub allow_hosting: bool,
    pub allow_remote_extend: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub max_resolution_width: u32,
    #[cfg_attr(test, ts(type = "number"))]
    pub max_resolution_height: u32,
    #[cfg_attr(test, ts(type = "number"))]
    pub max_fps: u32,
    #[cfg_attr(test, ts(type = "number"))]
    pub idle_timeout_secs: u32,
    pub allowed_groups: Vec<String>,
    pub allowed_tags: Vec<String>,
    /// `DeviceType` snake_case strings.
    pub allowed_types: Vec<String>,
}

/// `PATCH /display/virtual/policy` body: sparse policy fields.
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct DisplayPolicyPatch {
    #[cfg_attr(test, ts(optional))]
    pub enabled: Option<bool>,
    #[cfg_attr(test, ts(optional))]
    pub allow_hosting: Option<bool>,
    #[cfg_attr(test, ts(optional))]
    pub allow_remote_extend: Option<bool>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub max_resolution_width: Option<u32>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub max_resolution_height: Option<u32>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub max_fps: Option<u32>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub idle_timeout_secs: Option<u32>,
    #[cfg_attr(test, ts(optional))]
    pub allowed_groups: Option<Vec<String>>,
    #[cfg_attr(test, ts(optional))]
    pub allowed_tags: Option<Vec<String>>,
    #[cfg_attr(test, ts(optional))]
    pub allowed_types: Option<Vec<String>>,
    #[cfg_attr(test, ts(optional))]
    pub clear_filters: Option<bool>,
}

/// `POST /display/virtual/attach` body.
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct DisplayAttachRequest {
    #[cfg_attr(test, ts(optional))]
    pub peer: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub edge: Option<String>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub width: Option<u32>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub height: Option<u32>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub fps: Option<u32>,
}

/// `POST /display/virtual/detach` body.
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct DisplayDetachRequest {
    #[cfg_attr(test, ts(optional))]
    pub reason: Option<String>,
}

// ---- §167: screen streaming domain ----

/// Screen v1 contract: `ScreenStatus` (legacy `ScreenStatusReport` folded).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ScreenStatus {
    pub state: String,
    pub channel_open: bool,
    pub allowed: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub sent: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub received: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub dropped: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub refused: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub bytes: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub rx_bytes: u64,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub p95_frame_interval_us: Option<u64>,
    #[cfg_attr(test, ts(optional))]
    pub active_peer: Option<String>,
    #[cfg_attr(test, ts(type = "number"))]
    pub acked: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub in_flight: u64,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub rtt_latest_us: Option<u64>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub rtt_p95_us: Option<u64>,
    pub is_congested: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub keyframes_requested: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub gaps: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub bitrate_bps: u32,
    #[cfg_attr(test, ts(type = "number"))]
    pub fps_target: u32,
    #[cfg_attr(test, ts(type = "number"))]
    pub scale_pct: u32,
    pub profile: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub qp_min: u8,
    #[cfg_attr(test, ts(type = "number"))]
    pub qp_max: u8,
    pub matrix_mode: String,
    pub network_health: String,
    pub playout_mode: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub playout_delay_ms: f64,
    #[cfg_attr(test, ts(type = "number"))]
    pub playout_jitter_ms: f64,
    #[cfg_attr(test, ts(type = "number"))]
    pub playout_buffered: usize,
    #[cfg_attr(test, ts(type = "number"))]
    pub playout_dropped_late: u64,
    pub cursor_mode: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub cursor_x: i32,
    #[cfg_attr(test, ts(type = "number"))]
    pub cursor_y: i32,
    pub cursor_visible: bool,
    pub cursor_shape: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub cursor_updates_sent: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub cursor_updates_received: u64,
    pub squelch_mode: String,
    pub is_squelched: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub squelched_frames: u64,
    pub fec_mode: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub fec_redundancy_pct: u8,
    #[cfg_attr(test, ts(type = "number"))]
    pub fec_recovered_shards: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub fec_unrecoverable_blocks: u64,
    pub pacer_mode: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub pacer_packets_paced: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub pacer_bursts_smoothed: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub pacer_avg_delay_us: u64,
    pub trendline_mode: String,
    pub trendline_state: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub trendline_slope_us: f64,
    #[cfg_attr(test, ts(type = "number"))]
    pub trendline_overuse_count: u64,
    pub harq_mode: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub harq_nacks_sent: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub harq_nacks_received: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub harq_shards_retransmitted: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub harq_recovered_frames: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub harq_retransmit_timeouts: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub harq_buffer_occupancy: usize,
    pub twcc_mode: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub twcc_feedbacks_sent: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub twcc_feedbacks_received: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub twcc_delivery_rate_bps: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub twcc_delay_gradient_us: i64,
    #[cfg_attr(test, ts(type = "number"))]
    pub twcc_loss_ratio: f32,
    pub probe_mode: String,
    pub probe_state: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub probes_triggered: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub probes_completed: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub probes_failed: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub probe_target_bps: u32,
    #[cfg_attr(test, ts(type = "number"))]
    pub probe_measured_bps: u64,
    pub sync_mode: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub sync_beacons_sent: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub sync_beacons_received: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub sync_skew_us: i64,
    #[cfg_attr(test, ts(type = "number"))]
    pub sync_clock_drift_ppm: f64,
    #[cfg_attr(test, ts(type = "number"))]
    pub sync_adjustments_applied: u64,
    pub in_sync: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub active_display_id: u8,
    #[cfg_attr(test, ts(type = "number"))]
    pub display_count: usize,
    #[cfg_attr(test, ts(type = "number"))]
    pub display_switches_count: u64,
    pub privacy_enabled: bool,
    pub privacy_input_locked: bool,
    pub privacy_blanking_method: String,
    pub latency_mode: String,
    pub latency_bottleneck: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub latency_total_e2e_us: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub latency_p95_e2e_us: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub latency_frames_profiled: u64,
    pub tuning_mode: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub tuning_actions_total: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub tuning_recoveries: u64,
    pub tuning_last_stage: String,
    pub tuning_last_action: String,
    pub recording_active: bool,
    pub peer_recording: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub recording_frames: u64,
    pub playback_active: bool,
    pub playback_state: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub playback_frame: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub snapshot_count: u64,
    pub watermark_enabled: bool,
    pub watermark_enforced: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub watermark_frames: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub annotations_count: usize,
    pub laser_active: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub annotations_total: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub laser_updates_total: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub whiteboard_undo_count: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub whiteboard_redo_count: u64,
    pub whiteboard_read_only: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub whiteboard_max_items: u16,
    pub roi_active: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub roi_x: u16,
    #[cfg_attr(test, ts(type = "number"))]
    pub roi_y: u16,
    #[cfg_attr(test, ts(type = "number"))]
    pub roi_w: u16,
    #[cfg_attr(test, ts(type = "number"))]
    pub roi_h: u16,
    #[cfg_attr(test, ts(type = "number"))]
    pub roi_zoom_percent: u16,
    #[cfg_attr(test, ts(type = "number"))]
    pub roi_updates: u64,
    pub color_mode: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub color_threshold: u8,
    #[cfg_attr(test, ts(type = "number"))]
    pub color_updates: u64,
    pub delta_enabled: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub delta_keyframe_interval: u16,
    #[cfg_attr(test, ts(type = "number"))]
    pub delta_frames_sent: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub delta_keyframes_sent: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub delta_frames_received: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub delta_keyframes_received: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub delta_saved_bytes: u64,
    pub metric_enabled: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub metric_sample_interval: u8,
    #[cfg_attr(test, ts(type = "number"))]
    pub metric_evaluated_frames: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub metric_psnr_db: f64,
    #[cfg_attr(test, ts(type = "number"))]
    pub metric_ssim: f64,
    pub metric_quality_warning: bool,
    pub qoe_enabled: bool,
    pub qoe_mode: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub qoe_target_psnr_db: u8,
    #[cfg_attr(test, ts(type = "number"))]
    pub qoe_target_ssim_pct: u8,
    #[cfg_attr(test, ts(type = "number"))]
    pub qoe_actions_total: u64,
    pub qoe_last_action: String,
    pub pir_enabled: bool,
    pub pir_mode: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub pir_bands: u8,
    #[cfg_attr(test, ts(type = "number"))]
    pub pir_sweeps_completed: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub pir_slices_sent: u64,
    pub ltr_enabled: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub ltr_auto_interval: u16,
    #[cfg_attr(test, ts(type = "number"))]
    pub ltr_current_seq: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub ltr_anchors_established: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub ltr_recoveries_completed: u64,
    pub svc_enabled: bool,
    pub svc_mode: String,
    pub svc_drop_t1: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub svc_t0_sent: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub svc_t1_sent: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub svc_t1_dropped: u64,
    pub motion_enabled: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub motion_max_displacement: u16,
    #[cfg_attr(test, ts(type = "number"))]
    pub motion_frames_sent: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub motion_fallback_frames_sent: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub motion_saved_bytes: u64,
    pub tile_enabled: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub tile_size: u16,
    #[cfg_attr(test, ts(type = "number"))]
    pub tile_frames_sent: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub dirty_tiles_sent: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub skipped_tiles_count: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub tile_saved_bytes: u64,
    pub tile_cache_enabled: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub tile_cache_max_entries: u16,
    #[cfg_attr(test, ts(type = "number"))]
    pub tile_cache_entries: u16,
    #[cfg_attr(test, ts(type = "number"))]
    pub tile_cache_hits: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub tile_cache_misses: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub tile_cache_hit_ratio_pct: f32,
    #[cfg_attr(test, ts(type = "number"))]
    pub tile_cache_saved_bytes: u64,
    pub focus_enabled: bool,
    pub focus_mode: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub focus_radius: u16,
    #[cfg_attr(test, ts(type = "number"))]
    pub focus_peripheral_skip_ratio: u8,
    #[cfg_attr(test, ts(type = "number"))]
    pub focus_box_x: u16,
    #[cfg_attr(test, ts(type = "number"))]
    pub focus_box_y: u16,
    #[cfg_attr(test, ts(type = "number"))]
    pub focus_box_w: u16,
    #[cfg_attr(test, ts(type = "number"))]
    pub focus_box_h: u16,
    #[cfg_attr(test, ts(type = "number"))]
    pub focus_tiles_sent: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub peripheral_tiles_sent: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub peripheral_tiles_skipped: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub focus_saved_bandwidth_bytes: u64,
    pub tile_palette_enabled: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub tile_palette_max_colors: u8,
    pub tile_palette_enable_rle: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub tile_palette_solid_tiles: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub tile_palette_bilevel_tiles: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub tile_palette_palette_tiles: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub tile_palette_truecolor_tiles: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub tile_palette_saved_bytes: u64,
    pub tile_predict_enabled: bool,
    pub tile_predict_filter_mode: String,
    pub tile_predict_enable_rct: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub tile_predict_tiles_sent: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub tile_predict_saved_bytes: u64,
    pub tile_present_enabled: bool,
    pub tile_present_vsync_mode: String,
    pub tile_present_double_buffer: bool,
    pub tile_present_tear_prevention: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub tile_present_committed_transactions: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub tile_present_swapped_frames: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub tile_present_torn_frames_prevented: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub tile_present_avg_swap_latency_us: u64,
    pub tile_nack_enabled: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub tile_nack_timeout_ms: u16,
    #[cfg_attr(test, ts(type = "number"))]
    pub tile_nack_max_retries: u8,
    #[cfg_attr(test, ts(type = "number"))]
    pub tile_nack_requests_sent: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub tile_nack_tiles_retransmitted: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub tile_nack_degraded_commits: u64,
    pub tile_saliency_enabled: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub tile_saliency_high_threshold: u8,
    #[cfg_attr(test, ts(type = "number"))]
    pub tile_saliency_low_threshold: u8,
    pub tile_saliency_drop_background_on_congestion: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub tile_saliency_critical_tiles: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub tile_saliency_normal_tiles: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub tile_saliency_background_tiles: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub tile_saliency_background_skipped: u64,
    pub tile_lod_enabled: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub tile_lod_max_lod: u8,
    pub tile_lod_auto_lod_on_congestion: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub tile_lod0_tiles: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub tile_lod1_tiles: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub tile_lod2_tiles: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub tile_lod_bytes_saved: u64,
    pub tile_batch_enabled: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub tile_batch_target_chunk_bytes: u16,
    #[cfg_attr(test, ts(type = "number"))]
    pub tile_batch_pacing_interval_us: u16,
    #[cfg_attr(test, ts(type = "number"))]
    pub tile_batches_sent: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub tile_chunks_sent: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub tile_batch_avg_chunk_bytes: u32,
    #[cfg_attr(test, ts(type = "number"))]
    pub tile_batch_pacing_pauses: u64,
    pub tile_motion_enabled: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub tile_motion_max_displacement: u16,
    #[cfg_attr(test, ts(type = "number"))]
    pub tile_motion_min_match_ratio: u8,
    #[cfg_attr(test, ts(type = "number"))]
    pub tile_motion_tiles_sent: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub tile_motion_fallback_tiles_sent: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub tile_motion_bytes_saved: u64,
}

/// Screen v1 contract: `ScreenControl` (legacy `ScreenControlReport` folded).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ScreenControl {
    pub action: String,
    pub state: String,
    pub allowed: bool,
    #[cfg_attr(test, ts(optional))]
    pub peer: Option<String>,
    #[cfg_attr(test, ts(type = "number"))]
    pub timestamp_ms: u64,
}

/// Screen v1 contract: `ScreenDisplays` (legacy `ScreenDisplayReport` folded).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ScreenDisplays {
    #[cfg_attr(test, ts(type = "number"))]
    pub active_display_id: u8,
    pub displays: Vec<ScreenDisplayInfoDto>,
    #[cfg_attr(test, ts(type = "number"))]
    pub switches_count: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub last_switch_us: u64,
}

/// Screen v1 contract: `ScreenDisplayInfoDto` (legacy `ScreenDisplayInfo` folded).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ScreenDisplayInfoDto {
    #[cfg_attr(test, ts(type = "number"))]
    pub display_id: u8,
    pub title: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub width: u32,
    #[cfg_attr(test, ts(type = "number"))]
    pub height: u32,
    #[cfg_attr(test, ts(type = "number"))]
    pub scale_factor_pct: u32,
    pub is_primary: bool,
    pub is_virtual: bool,
}

/// Screen v1 contract: `ScreenPrivacy` (legacy `ScreenPrivacyReport` folded).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ScreenPrivacy {
    pub enabled: bool,
    pub lock_input: bool,
    pub blanking_method: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub last_change_us: u64,
}

/// Screen v1 contract: `ScreenRecording` (legacy `ScreenRecordingReport` folded).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ScreenRecording {
    pub active: bool,
    pub path: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub frames_recorded: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub bytes_recorded: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub started_at_us: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub duration_us: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub last_frame_seq: u64,
    pub peer_recording: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub width: u32,
    #[cfg_attr(test, ts(type = "number"))]
    pub height: u32,
}

/// Screen v1 contract: `ScreenRecordingReplay` (legacy `ScreenRecordingReplayReport` folded).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ScreenRecordingReplay {
    pub path: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub width: u32,
    #[cfg_attr(test, ts(type = "number"))]
    pub height: u32,
    #[cfg_attr(test, ts(type = "number"))]
    pub format_tag: u8,
    #[cfg_attr(test, ts(type = "number"))]
    pub codec_tag: u8,
    #[cfg_attr(test, ts(type = "number"))]
    pub started_at_us: u64,
    pub records: Vec<ScreenReplayRecordDto>,
    #[cfg_attr(test, ts(type = "number"))]
    pub records_returned: usize,
    pub truncated: bool,
}

/// Screen v1 contract: `ScreenReplayRecordDto` (legacy `ScreenRecordingReplayRecord` folded).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ScreenReplayRecordDto {
    #[cfg_attr(test, ts(type = "number"))]
    pub seq: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub pts_us: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub len: usize,
}

/// Screen v1 contract: `ScreenPlayback` (legacy `ScreenPlaybackReport` folded).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ScreenPlayback {
    pub state: String,
    pub path: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub speed: f32,
    pub loop_enabled: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub current_frame: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub total_frames: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub elapsed_ms: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub duration_ms: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub width: u16,
    #[cfg_attr(test, ts(type = "number"))]
    pub height: u16,
    pub format: String,
    pub codec: String,
}

/// Screen v1 contract: `ScreenSnapshot` (legacy `ScreenSnapshotReport` folded).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ScreenSnapshot {
    pub source: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub frame_seq: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub pts_us: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub width: u32,
    #[cfg_attr(test, ts(type = "number"))]
    pub height: u32,
    pub format: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub bytes_len: usize,
    pub privacy_blanked: bool,
    pub watermarked: bool,
    #[cfg_attr(test, ts(optional))]
    pub path: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub data_base64: Option<String>,
}

/// Screen v1 contract: `ScreenPir` (legacy `ScreenPirReport` folded).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ScreenPir {
    pub mode: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub bands: u8,
    #[cfg_attr(test, ts(type = "number"))]
    pub sweeps_completed: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub slices_sent: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub current_band: u8,
    pub active_sweep: bool,
}

/// Screen v1 contract: `ScreenLtr` (legacy `ScreenLtrReport` folded).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ScreenLtr {
    pub enabled: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub auto_interval: u16,
    #[cfg_attr(test, ts(type = "number"))]
    pub current_ltr_seq: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub anchors_established: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub recoveries_requested: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub recoveries_completed: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub saved_burst_bytes: u64,
}

/// Screen v1 contract: `ScreenSvc` (legacy `ScreenSvcReport` folded).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ScreenSvc {
    pub enabled: bool,
    pub mode: String,
    pub drop_t1: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub t0_frames_sent: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub t1_frames_sent: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub t1_frames_dropped: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub saved_bandwidth_bytes: u64,
}

/// Screen v1 contract: `ScreenCursor` (legacy `ScreenCursorReport` folded).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ScreenCursor {
    pub mode: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub x: i32,
    #[cfg_attr(test, ts(type = "number"))]
    pub y: i32,
    pub visible: bool,
    pub shape: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub updates_sent: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub updates_received: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub updates_dropped_dedup: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub updates_dropped_rate: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub timestamp_ms: u64,
}

/// Screen v1 contract: `ScreenTrendline` (legacy `ScreenTrendlineReport` folded).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ScreenTrendline {
    pub mode: String,
    pub state: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub samples_count: usize,
    #[cfg_attr(test, ts(type = "number"))]
    pub current_slope_us_per_frame: f64,
    #[cfg_attr(test, ts(type = "number"))]
    pub threshold_us_per_frame: f64,
    #[cfg_attr(test, ts(type = "number"))]
    pub overuse_count: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub underuse_count: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub accumulated_delay_us: i64,
    #[cfg_attr(test, ts(type = "number"))]
    pub timestamp_ms: u64,
}

/// Screen v1 contract: `ScreenProbe` (legacy `ScreenProbeReport` folded).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ScreenProbe {
    pub mode: String,
    pub state: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub probes_triggered: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub probes_completed: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub probes_failed: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub last_target_bps: u32,
    #[cfg_attr(test, ts(type = "number"))]
    pub last_measured_bps: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub last_rtt_us: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub last_loss_ratio: f32,
    #[cfg_attr(test, ts(type = "number"))]
    pub last_delay_gradient_us: i64,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub active_cluster_id: Option<u8>,
    #[cfg_attr(test, ts(type = "number"))]
    pub timestamp_ms: u64,
}

/// Screen v1 contract: `ScreenSquelch` (legacy `ScreenSquelchReport` folded).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ScreenSquelch {
    pub mode: String,
    pub is_squelched: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub frames_evaluated: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub frames_passed: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub frames_squelched: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub keepalive_frames: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub last_dirty_ratio: f32,
    #[cfg_attr(test, ts(type = "number"))]
    pub consecutive_static_frames: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub timestamp_ms: u64,
}

/// Screen v1 contract: `ScreenFec` (legacy `ScreenFecReport` folded).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ScreenFec {
    pub mode: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub redundancy_pct: u8,
    #[cfg_attr(test, ts(type = "number"))]
    pub blocks_encoded: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub data_shards_sent: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub parity_shards_sent: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub blocks_decoded: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub recovered_shards: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub unrecoverable_blocks: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub timestamp_ms: u64,
}

/// Screen v1 contract: `ScreenPacer` (legacy `ScreenPacerReport` folded).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ScreenPacer {
    pub mode: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub packets_paced: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub bursts_smoothed: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub avg_delay_us: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub last_delay_us: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub timestamp_ms: u64,
}

/// Screen v1 contract: `ScreenHarq` (legacy `ScreenHarqReport` folded).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ScreenHarq {
    pub mode: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub nacks_sent: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub nacks_received: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub shards_retransmitted: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub recovered_frames: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub retransmit_timeouts: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub buffer_occupancy: usize,
    #[cfg_attr(test, ts(type = "number"))]
    pub buffer_capacity: usize,
    #[cfg_attr(test, ts(type = "number"))]
    pub timestamp_ms: u64,
}

/// Screen v1 contract: `ScreenTwcc` (legacy `ScreenTwccReport` folded).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ScreenTwcc {
    pub mode: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub feedbacks_sent: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub feedbacks_received: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub packets_tracked: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub packets_acknowledged: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub packets_lost: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub loss_ratio: f32,
    #[cfg_attr(test, ts(type = "number"))]
    pub delivery_rate_bps: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub delay_gradient_us: i64,
    #[cfg_attr(test, ts(type = "number"))]
    pub smoothed_rtt_us: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub timestamp_ms: u64,
}

/// Screen v1 contract: `ScreenSync` (legacy `ScreenAudioSyncReport` folded).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ScreenSync {
    pub mode: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub beacons_sent: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub beacons_received: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub current_skew_us: i64,
    #[cfg_attr(test, ts(type = "number"))]
    pub clock_drift_ppm: f64,
    #[cfg_attr(test, ts(type = "number"))]
    pub adjustments_applied: u64,
    pub in_sync: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub timestamp_ms: u64,
}

/// Screen v1 contract: `ScreenLatency` (legacy `ScreenLatencyReport` folded).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ScreenLatency {
    pub mode: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub frames_profiled: u64,
    pub bottleneck: String,
    pub current: ScreenLatencyBreakdownDto,
    #[cfg_attr(test, ts(type = "number"))]
    pub avg_e2e_us: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub p50_e2e_us: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub p90_e2e_us: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub p95_e2e_us: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub p99_e2e_us: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub min_e2e_us: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub max_e2e_us: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub avg_capture_us: u32,
    #[cfg_attr(test, ts(type = "number"))]
    pub avg_encode_us: u32,
    #[cfg_attr(test, ts(type = "number"))]
    pub avg_pacer_us: u32,
    #[cfg_attr(test, ts(type = "number"))]
    pub avg_network_us: u32,
    #[cfg_attr(test, ts(type = "number"))]
    pub avg_jitter_us: u32,
    #[cfg_attr(test, ts(type = "number"))]
    pub avg_decode_us: u32,
    #[cfg_attr(test, ts(type = "number"))]
    pub avg_render_us: u32,
}

/// Screen v1 contract: `ScreenLatencyBreakdownDto` (legacy `ScreenLatencyBreakdown` folded).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ScreenLatencyBreakdownDto {
    #[cfg_attr(test, ts(type = "number"))]
    pub frame_seq: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub capture_us: u32,
    #[cfg_attr(test, ts(type = "number"))]
    pub encode_us: u32,
    #[cfg_attr(test, ts(type = "number"))]
    pub pacer_us: u32,
    #[cfg_attr(test, ts(type = "number"))]
    pub network_us: u32,
    #[cfg_attr(test, ts(type = "number"))]
    pub jitter_us: u32,
    #[cfg_attr(test, ts(type = "number"))]
    pub decode_us: u32,
    #[cfg_attr(test, ts(type = "number"))]
    pub render_us: u32,
    #[cfg_attr(test, ts(type = "number"))]
    pub total_e2e_us: u64,
}

/// Screen v1 contract: `ScreenTuning` (legacy `ScreenTuningReport` folded).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ScreenTuning {
    pub mode: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub actions_applied: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub recoveries_applied: u64,
    pub last_stage: String,
    pub last_action: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub bottleneck_streak: u32,
    #[cfg_attr(test, ts(type = "number"))]
    pub balanced_streak: u32,
    #[cfg_attr(test, ts(optional))]
    pub last_decision: Option<ScreenTuningDecisionDto>,
}

/// Screen v1 contract: `ScreenTuningDecisionDto` (legacy `ScreenTuningDecision` folded).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ScreenTuningDecisionDto {
    #[cfg_attr(test, ts(type = "number"))]
    pub frame_seq: u64,
    pub stage: String,
    pub action: String,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub new_bitrate_bps: Option<u32>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub new_fps: Option<u32>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub new_scale_pct: Option<u32>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub new_qp_max: Option<u8>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub new_playout_delay_us: Option<u32>,
    pub recovered: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub at_us: u64,
}

/// Screen v1 contract: `ScreenPlayout` (legacy `ScreenPlayoutReport` folded).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ScreenPlayout {
    pub mode: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub target_delay_ms: f64,
    #[cfg_attr(test, ts(type = "number"))]
    pub current_jitter_ms: f64,
    #[cfg_attr(test, ts(type = "number"))]
    pub target_delay_us: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub current_jitter_us: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub buffered_frames: usize,
    #[cfg_attr(test, ts(type = "number"))]
    pub played_frames: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub dropped_late_frames: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub dropped_overflow_frames: u64,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub custom_delay_us: Option<u64>,
    #[cfg_attr(test, ts(type = "number"))]
    pub timestamp_ms: u64,
}

/// Screen v1 contract: `ScreenMatrix` (legacy `ScreenMatrixReport` folded).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ScreenMatrix {
    pub mode: String,
    pub health: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub bitrate_bps: u32,
    #[cfg_attr(test, ts(type = "number"))]
    pub fps_target: u32,
    #[cfg_attr(test, ts(type = "number"))]
    pub scale_pct: u32,
    pub profile: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub qp_min: u8,
    #[cfg_attr(test, ts(type = "number"))]
    pub qp_max: u8,
    pub auto_fps: bool,
    pub auto_scale: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub consecutive_healthy_acks: u32,
    #[cfg_attr(test, ts(type = "number"))]
    pub consecutive_drops: u32,
    #[cfg_attr(test, ts(type = "number"))]
    pub in_flight: u64,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub rtt_us: Option<u64>,
    #[cfg_attr(test, ts(type = "number"))]
    pub timestamp_ms: u64,
}

/// Screen v1 contract: `ScreenProfile` (legacy `ScreenProfileReport` folded).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ScreenProfile {
    pub profile: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub min_qp: u8,
    #[cfg_attr(test, ts(type = "number"))]
    pub max_qp: u8,
    pub is_custom_qp: bool,
    pub auto_profile: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub timestamp_ms: u64,
}

/// Screen v1 contract: `ScreenScale` (legacy `ScreenScaleReport` folded).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ScreenScale {
    #[cfg_attr(test, ts(type = "number"))]
    pub scale_pct: u32,
    #[cfg_attr(test, ts(type = "number"))]
    pub min_scale_pct: u32,
    #[cfg_attr(test, ts(type = "number"))]
    pub max_scale_pct: u32,
    pub auto_profile: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub timestamp_ms: u64,
}

/// Screen v1 contract: `ScreenFps` (legacy `ScreenFpsReport` folded).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ScreenFps {
    #[cfg_attr(test, ts(type = "number"))]
    pub fps: u32,
    #[cfg_attr(test, ts(type = "number"))]
    pub min_fps: u32,
    #[cfg_attr(test, ts(type = "number"))]
    pub max_fps: u32,
    #[cfg_attr(test, ts(type = "number"))]
    pub interval_us: u64,
    pub auto_profile: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub timestamp_ms: u64,
}

/// Screen v1 contract: `ScreenBitrate` (legacy `ScreenBitrateReport` folded).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ScreenBitrate {
    #[cfg_attr(test, ts(type = "number"))]
    pub bitrate_bps: u32,
    #[cfg_attr(test, ts(type = "number"))]
    pub bitrate_kbps: u32,
    #[cfg_attr(test, ts(type = "number"))]
    pub min_bps: u32,
    #[cfg_attr(test, ts(type = "number"))]
    pub max_bps: u32,
    #[cfg_attr(test, ts(type = "number"))]
    pub timestamp_ms: u64,
}

/// Screen v1 contract: `ScreenColor` (legacy `ScreenColorReport` folded).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ScreenColor {
    pub mode: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub threshold: u8,
    pub is_active: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub update_count: u64,
}

/// Screen v1 contract: `ScreenDelta` (legacy `ScreenDeltaReport` folded).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ScreenDelta {
    pub enabled: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub keyframe_interval: u16,
    #[cfg_attr(test, ts(type = "number"))]
    pub delta_frames_sent: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub keyframes_sent: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub delta_frames_received: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub keyframes_received: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub saved_bytes: u64,
}

/// Screen v1 contract: `ScreenQualityMetric` (legacy `ScreenQualityMetricReport` folded).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ScreenQualityMetric {
    pub enabled: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub sample_interval: u8,
    #[cfg_attr(test, ts(type = "number"))]
    pub evaluated_frames: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub avg_mse: f64,
    #[cfg_attr(test, ts(type = "number"))]
    pub avg_psnr_db: f64,
    #[cfg_attr(test, ts(type = "number"))]
    pub avg_ssim: f64,
    #[cfg_attr(test, ts(type = "number"))]
    pub last_psnr_db: f64,
    #[cfg_attr(test, ts(type = "number"))]
    pub last_ssim: f64,
    pub quality_warning: bool,
}

/// Screen v1 contract: `ScreenQoe` (legacy `ScreenQoeReport` folded).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ScreenQoe {
    pub mode: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub target_psnr_db: u8,
    #[cfg_attr(test, ts(type = "number"))]
    pub target_ssim_pct: u8,
    #[cfg_attr(test, ts(type = "number"))]
    pub actions_total: u64,
    pub last_action: String,
    pub last_action_reason: String,
    pub in_cooldown: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub active_boosts: u32,
}

/// Screen v1 contract: `ScreenMotion` (legacy `ScreenMotionReport` folded).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ScreenMotion {
    pub enabled: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub max_displacement: u16,
    #[cfg_attr(test, ts(type = "number"))]
    pub motion_frames_sent: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub fallback_frames_sent: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub saved_bandwidth_bytes: u64,
}

/// Screen v1 contract: `ScreenRoi` (legacy `ScreenRoiReport` folded).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ScreenRoi {
    pub active: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub x: u16,
    #[cfg_attr(test, ts(type = "number"))]
    pub y: u16,
    #[cfg_attr(test, ts(type = "number"))]
    pub w: u16,
    #[cfg_attr(test, ts(type = "number"))]
    pub h: u16,
    #[cfg_attr(test, ts(type = "number"))]
    pub zoom_percent: u16,
    #[cfg_attr(test, ts(type = "number"))]
    pub source_w: u16,
    #[cfg_attr(test, ts(type = "number"))]
    pub source_h: u16,
    #[cfg_attr(test, ts(type = "number"))]
    pub effective_w: u16,
    #[cfg_attr(test, ts(type = "number"))]
    pub effective_h: u16,
    #[cfg_attr(test, ts(type = "number"))]
    pub update_count: u64,
}

/// Screen v1 contract: `ScreenWatermark` (legacy `ScreenWatermarkReport` folded).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ScreenWatermark {
    pub enabled: bool,
    pub host_enforced: bool,
    pub text_template: String,
    pub rendered_text: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub opacity: u8,
    #[cfg_attr(test, ts(type = "number"))]
    pub opacity_pct: u8,
    pub color_rgb: [u8; 3],
    pub color_hex: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub scale: u8,
    pub pattern: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub spacing_x: u16,
    #[cfg_attr(test, ts(type = "number"))]
    pub spacing_y: u16,
    #[cfg_attr(test, ts(type = "number"))]
    pub frames_watermarked: u64,
}

/// Screen v1 contract: `ScreenAnnotations` (legacy `ScreenAnnotationReport` folded).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ScreenAnnotations {
    #[cfg_attr(test, ts(type = "number"))]
    pub annotations_count: usize,
    pub annotations: Vec<AnnotationItemDto>,
    pub laser_active: bool,
    #[cfg_attr(test, ts(optional))]
    pub laser: Option<LaserPointerDto>,
    #[cfg_attr(test, ts(type = "number"))]
    pub total_annotations_added: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub laser_updates_total: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub timestamp_ms: u64,
    pub whiteboard_policy: WhiteboardPolicyDto,
    #[cfg_attr(test, ts(type = "number"))]
    pub redo_count: usize,
}

/// Screen v1 contract: `LaserPointerDto` (legacy `LaserPointerState` folded).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct LaserPointerDto {
    #[cfg_attr(test, ts(type = "number"))]
    pub x: u16,
    #[cfg_attr(test, ts(type = "number"))]
    pub y: u16,
    pub active: bool,
    pub color_rgb: [u8; 3],
    #[cfg_attr(test, ts(type = "number"))]
    pub radius: u8,
}

/// Screen v1 contract: `WhiteboardPolicyDto` (legacy `WhiteboardPolicy` folded).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct WhiteboardPolicyDto {
    pub read_only: bool,
    pub allow_peer_clear: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub max_items: u16,
}

/// Screen v1 contract: `WhiteboardSnapshotDto` (legacy `WhiteboardSnapshot` folded).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct WhiteboardSnapshotDto {
    #[cfg_attr(test, ts(type = "number"))]
    pub version: u32,
    #[cfg_attr(test, ts(type = "number"))]
    pub created_at_ms: u64,
    pub session_id: String,
    pub active_items: Vec<AnnotationItemDto>,
    #[cfg_attr(test, ts(type = "number"))]
    pub total_added_counter: u64,
}

/// Screen v1 contract: `ScreenTile` (legacy `ScreenTileReport` folded).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ScreenTile {
    pub enabled: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub tile_size: u16,
    #[cfg_attr(test, ts(type = "number"))]
    pub grid_frames_sent: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub dirty_tiles_sent: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub skipped_tiles_count: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub saved_bandwidth_bytes: u64,
}

/// Screen v1 contract: `ScreenTileCache` (legacy `ScreenTileCacheReport` folded).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ScreenTileCache {
    pub enabled: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub max_entries: u16,
    #[cfg_attr(test, ts(type = "number"))]
    pub current_entries: u16,
    #[cfg_attr(test, ts(type = "number"))]
    pub cache_hits: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub cache_misses: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub hit_ratio_pct: f32,
    #[cfg_attr(test, ts(type = "number"))]
    pub saved_bandwidth_bytes: u64,
}

/// Screen v1 contract: `ScreenFocus` (legacy `ScreenFocusReport` folded).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ScreenFocus {
    pub enabled: bool,
    pub mode: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub radius: u16,
    #[cfg_attr(test, ts(type = "number"))]
    pub peripheral_skip_ratio: u8,
    #[cfg_attr(test, ts(type = "number"))]
    pub box_x: u16,
    #[cfg_attr(test, ts(type = "number"))]
    pub box_y: u16,
    #[cfg_attr(test, ts(type = "number"))]
    pub box_w: u16,
    #[cfg_attr(test, ts(type = "number"))]
    pub box_h: u16,
    #[cfg_attr(test, ts(type = "number"))]
    pub focus_tiles_sent: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub peripheral_tiles_sent: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub peripheral_tiles_skipped: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub saved_bandwidth_bytes: u64,
}

/// Screen v1 contract: `ScreenTilePalette` (legacy `ScreenTilePaletteReport` folded).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ScreenTilePalette {
    pub enabled: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub max_colors: u8,
    pub enable_rle: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub solid_tiles_count: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub bilevel_tiles_count: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub palette_tiles_count: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub truecolor_tiles_count: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub saved_bandwidth_bytes: u64,
}

/// Screen v1 contract: `ScreenTilePredict` (legacy `ScreenTilePredictReport` folded).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ScreenTilePredict {
    pub enabled: bool,
    pub filter_mode: String,
    pub enable_rct: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub predict_tiles_count: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub saved_bandwidth_bytes: u64,
}

/// Screen v1 contract: `ScreenTilePresent` (legacy `ScreenTilePresentReport` folded).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ScreenTilePresent {
    pub enabled: bool,
    pub vsync_mode: String,
    pub double_buffer: bool,
    pub tear_prevention: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub committed_transactions: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub swapped_frames: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub torn_frames_prevented: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub avg_swap_latency_us: u64,
}

/// Screen v1 contract: `ScreenTileNack` (legacy `ScreenTileNackReport` folded).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ScreenTileNack {
    pub enabled: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub timeout_ms: u16,
    #[cfg_attr(test, ts(type = "number"))]
    pub max_retries: u8,
    #[cfg_attr(test, ts(type = "number"))]
    pub nack_requests_sent: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub tiles_retransmitted: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub degraded_commits: u64,
}

/// Screen v1 contract: `ScreenTileSaliency` (legacy `ScreenTileSaliencyReport` folded).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ScreenTileSaliency {
    pub enabled: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub high_threshold: u8,
    #[cfg_attr(test, ts(type = "number"))]
    pub low_threshold: u8,
    pub drop_background_on_congestion: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub critical_tiles_sent: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub normal_tiles_sent: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub background_tiles_sent: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub background_tiles_skipped: u64,
}

/// Screen v1 contract: `ScreenTileLod` (legacy `ScreenTileLodReport` folded).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ScreenTileLod {
    pub enabled: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub max_lod: u8,
    pub auto_lod_on_congestion: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub lod0_tiles_sent: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub lod1_tiles_sent: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub lod2_tiles_sent: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub bytes_saved: u64,
}

/// Screen v1 contract: `ScreenTileBatch` (legacy `ScreenTileBatchReport` folded).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ScreenTileBatch {
    pub enabled: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub target_chunk_bytes: u16,
    #[cfg_attr(test, ts(type = "number"))]
    pub pacing_interval_us: u16,
    #[cfg_attr(test, ts(type = "number"))]
    pub batches_sent: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub chunks_sent: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub avg_chunk_bytes: u32,
    #[cfg_attr(test, ts(type = "number"))]
    pub pacing_pauses: u64,
}

/// Screen v1 contract: `ScreenTileMotion` (legacy `ScreenTileMotionReport` folded).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ScreenTileMotion {
    pub enabled: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub max_displacement: u16,
    #[cfg_attr(test, ts(type = "number"))]
    pub min_match_ratio: u8,
    #[cfg_attr(test, ts(type = "number"))]
    pub motion_tiles_sent: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub fallback_tiles_sent: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub bytes_saved: u64,
}

/// Screen v1 contract: `AnnotationItemDto` (legacy core `AnnotationItem`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct AnnotationItemDto {
    #[cfg_attr(test, ts(type = "number"))]
    pub id: u32,
    pub author: String,
    pub kind: AnnotationKindDto,
    pub color_rgb: [u8; 3],
    #[cfg_attr(test, ts(type = "number"))]
    pub thickness: u8,
    #[cfg_attr(test, ts(type = "number"))]
    pub alpha: u8,
    #[cfg_attr(test, ts(type = "number"))]
    pub ttl_ms: u32,
    #[cfg_attr(test, ts(type = "number"))]
    pub created_at_ms: u64,
}

/// Screen v1 contract: `AnnotationKindDto` (legacy core `AnnotationKind`, tagged mirror).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AnnotationKindDto {
    Stroke {
        points: Vec<(u16, u16)>,
    },
    Rect {
        x: u16,
        y: u16,
        w: u16,
        h: u16,
    },
    Arrow {
        from: (u16, u16),
        to: (u16, u16),
    },
    Text {
        x: u16,
        y: u16,
        text: String,
        scale: u8,
    },
}

/// `POST /screen/annotation` report (§167).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ScreenAnnotationAdded {
    #[cfg_attr(test, ts(type = "number"))]
    pub id: u32,
    pub report: ScreenAnnotations,
}

/// `POST /screen/whiteboard/undo` report (§167).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ScreenWhiteboardUndoReport {
    #[cfg_attr(test, ts(optional))]
    pub undone: Option<AnnotationItemDto>,
    pub report: ScreenAnnotations,
}

/// `POST /screen/whiteboard/redo` report (§167).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ScreenWhiteboardRedoReport {
    #[cfg_attr(test, ts(optional))]
    pub redone: Option<AnnotationItemDto>,
    pub report: ScreenAnnotations,
}

/// `GET /screen/whiteboard/export` report (§167).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ScreenWhiteboardExportReport {
    pub export: WhiteboardSnapshotDto,
    pub report: ScreenAnnotations,
}

/// `POST /screen/whiteboard/import` report (§167).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ScreenWhiteboardImportReport {
    #[cfg_attr(test, ts(type = "number"))]
    pub imported_count: usize,
    pub report: ScreenAnnotations,
}

/// `GET|PATCH /screen/whiteboard/policy` report (§167).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ScreenWhiteboardPolicyView {
    pub policy: WhiteboardPolicyDto,
}

/// `GET|POST /screen/laser` report (§167, legacy ad-hoc object named).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ScreenLaserReport {
    pub laser_active: bool,
    #[cfg_attr(test, ts(optional))]
    pub laser: Option<LaserPointerDto>,
    #[cfg_attr(test, ts(type = "number"))]
    pub laser_updates_total: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub timestamp_ms: u64,
}

/// Screen v1 contract: `ScreenStartRequest` (sparse, deny_unknown_fields).
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct ScreenStartRequest {
    #[cfg_attr(test, ts(optional))]
    pub peer: Option<String>,
}

/// Screen v1 contract: `ScreenSwitchRequest` (sparse, deny_unknown_fields).
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct ScreenSwitchRequest {
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub display_id: Option<u8>,
}

/// Screen v1 contract: `ScreenPrivacyPatch` (sparse, deny_unknown_fields).
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct ScreenPrivacyPatch {
    #[cfg_attr(test, ts(optional))]
    pub enabled: Option<bool>,
    #[cfg_attr(test, ts(optional))]
    pub lock_input: Option<bool>,
}

/// Screen v1 contract: `ScreenRecordingStartRequest` (sparse, deny_unknown_fields).
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct ScreenRecordingStartRequest {
    #[cfg_attr(test, ts(optional))]
    pub path: Option<String>,
}

/// Screen v1 contract: `ScreenPlaybackStartRequest` (sparse, deny_unknown_fields).
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct ScreenPlaybackStartRequest {
    #[cfg_attr(test, ts(optional))]
    pub path: Option<String>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub speed: Option<f32>,
    #[cfg_attr(test, ts(optional))]
    #[serde(rename = "loop")]
    pub loop_enabled: Option<bool>,
}

/// Screen v1 contract: `ScreenRecordingSnapshotRequest` (sparse, deny_unknown_fields).
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct ScreenRecordingSnapshotRequest {
    #[cfg_attr(test, ts(optional))]
    pub path: Option<String>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub frame: Option<u64>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub pts: Option<u64>,
    #[cfg_attr(test, ts(optional))]
    pub out: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub format: Option<String>,
}

/// Screen v1 contract: `ScreenCursorPatch` (sparse, deny_unknown_fields).
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct ScreenCursorPatch {
    #[cfg_attr(test, ts(optional))]
    pub mode: Option<String>,
}

/// Screen v1 contract: `ScreenTrendlinePatch` (sparse, deny_unknown_fields).
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct ScreenTrendlinePatch {
    #[cfg_attr(test, ts(optional))]
    pub mode: Option<String>,
}

/// Screen v1 contract: `ScreenProbePatch` (sparse, deny_unknown_fields).
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct ScreenProbePatch {
    #[cfg_attr(test, ts(optional))]
    pub mode: Option<String>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub target_bps: Option<u32>,
    #[cfg_attr(test, ts(optional))]
    pub trigger: Option<bool>,
}

/// Screen v1 contract: `ScreenSquelchPatch` (sparse, deny_unknown_fields).
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct ScreenSquelchPatch {
    #[cfg_attr(test, ts(optional))]
    pub mode: Option<String>,
}

/// Screen v1 contract: `ScreenFecPatch` (sparse, deny_unknown_fields).
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct ScreenFecPatch {
    #[cfg_attr(test, ts(optional))]
    pub mode: Option<String>,
}

/// Screen v1 contract: `ScreenPacerPatch` (sparse, deny_unknown_fields).
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct ScreenPacerPatch {
    #[cfg_attr(test, ts(optional))]
    pub mode: Option<String>,
}

/// Screen v1 contract: `ScreenHarqPatch` (sparse, deny_unknown_fields).
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct ScreenHarqPatch {
    #[cfg_attr(test, ts(optional))]
    pub mode: Option<String>,
}

/// Screen v1 contract: `ScreenTwccPatch` (sparse, deny_unknown_fields).
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct ScreenTwccPatch {
    #[cfg_attr(test, ts(optional))]
    pub mode: Option<String>,
}

/// Screen v1 contract: `ScreenSyncPatch` (sparse, deny_unknown_fields).
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct ScreenSyncPatch {
    #[cfg_attr(test, ts(optional))]
    pub mode: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub trigger: Option<bool>,
}

/// Screen v1 contract: `ScreenLatencyPatch` (sparse, deny_unknown_fields).
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct ScreenLatencyPatch {
    #[cfg_attr(test, ts(optional))]
    pub mode: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub reset: Option<bool>,
}

/// Screen v1 contract: `ScreenTuningPatch` (sparse, deny_unknown_fields).
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct ScreenTuningPatch {
    #[cfg_attr(test, ts(optional))]
    pub mode: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub reset: Option<bool>,
}

/// Screen v1 contract: `ScreenPlayoutPatch` (sparse, deny_unknown_fields).
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct ScreenPlayoutPatch {
    #[cfg_attr(test, ts(optional))]
    pub mode: Option<String>,
}

/// Screen v1 contract: `ScreenMatrixModePatch` (sparse, deny_unknown_fields).
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct ScreenMatrixModePatch {
    #[cfg_attr(test, ts(optional))]
    pub mode: Option<String>,
}

/// Screen v1 contract: `ScreenProfilePatch` (sparse, deny_unknown_fields).
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct ScreenProfilePatch {
    #[cfg_attr(test, ts(optional))]
    pub profile: Option<String>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub min_qp: Option<u8>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub max_qp: Option<u8>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub qp: Option<u8>,
}

/// Screen v1 contract: `ScreenScalePatch` (sparse, deny_unknown_fields).
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct ScreenScalePatch {
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub scale_pct: Option<u32>,
}

/// Screen v1 contract: `ScreenFpsPatch` (sparse, deny_unknown_fields).
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct ScreenFpsPatch {
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub fps: Option<u32>,
}

/// Screen v1 contract: `ScreenBitratePatch` (sparse, deny_unknown_fields).
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct ScreenBitratePatch {
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub kbps: Option<u32>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub bps: Option<u32>,
}

/// Screen v1 contract: `ScreenColorPatch` (sparse, deny_unknown_fields).
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct ScreenColorPatch {
    #[cfg_attr(test, ts(optional))]
    pub mode: Option<String>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub threshold: Option<u8>,
}

/// Screen v1 contract: `ScreenDeltaPatch` (sparse, deny_unknown_fields).
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct ScreenDeltaPatch {
    #[cfg_attr(test, ts(optional))]
    pub enabled: Option<bool>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub keyframe_interval: Option<u16>,
}

/// Screen v1 contract: `ScreenQualityMetricPatch` (sparse, deny_unknown_fields).
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct ScreenQualityMetricPatch {
    #[cfg_attr(test, ts(optional))]
    pub enabled: Option<bool>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub sample_interval: Option<u8>,
}

/// Screen v1 contract: `ScreenQoePatch` (sparse, deny_unknown_fields).
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct ScreenQoePatch {
    #[cfg_attr(test, ts(optional))]
    pub mode: Option<String>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub target_psnr_db: Option<u8>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub target_ssim_pct: Option<u8>,
}

/// Screen v1 contract: `ScreenPirPatch` (sparse, deny_unknown_fields).
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct ScreenPirPatch {
    #[cfg_attr(test, ts(optional))]
    pub mode: Option<String>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub bands: Option<u8>,
}

/// Screen v1 contract: `ScreenMotionPatch` (sparse, deny_unknown_fields).
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct ScreenMotionPatch {
    #[cfg_attr(test, ts(optional))]
    pub enabled: Option<bool>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub max_displacement: Option<u16>,
}

/// Screen v1 contract: `ScreenRoiPatch` (sparse, deny_unknown_fields).
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct ScreenRoiPatch {
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub x: Option<u16>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub y: Option<u16>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub w: Option<u16>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub h: Option<u16>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub zoom_percent: Option<u16>,
}

/// Screen v1 contract: `ScreenLtrPatch` (sparse, deny_unknown_fields).
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct ScreenLtrPatch {
    #[cfg_attr(test, ts(optional))]
    pub enabled: Option<bool>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub auto_interval: Option<u16>,
}

/// Screen v1 contract: `ScreenSvcPatch` (sparse, deny_unknown_fields).
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct ScreenSvcPatch {
    #[cfg_attr(test, ts(optional))]
    pub mode: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub drop_t1: Option<bool>,
}

/// Screen v1 contract: `ScreenSvcDropT1Request` (sparse, deny_unknown_fields).
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct ScreenSvcDropT1Request {
    #[cfg_attr(test, ts(optional))]
    pub drop_t1: Option<bool>,
}

/// Screen v1 contract: `ScreenWatermarkPatch` (sparse, deny_unknown_fields).
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct ScreenWatermarkPatch {
    #[cfg_attr(test, ts(optional))]
    pub enabled: Option<bool>,
    #[cfg_attr(test, ts(optional))]
    pub host_enforced: Option<bool>,
    #[cfg_attr(test, ts(optional))]
    pub text: Option<String>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub opacity: Option<u8>,
    #[cfg_attr(test, ts(optional))]
    pub color: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub color_rgb: Option<[u8; 3]>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub scale: Option<u8>,
    #[cfg_attr(test, ts(optional))]
    pub pattern: Option<String>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub spacing_x: Option<u16>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub spacing_y: Option<u16>,
}

/// Screen v1 contract: `ScreenWatermarkEnableRequest` (sparse, deny_unknown_fields).
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct ScreenWatermarkEnableRequest {
    #[cfg_attr(test, ts(optional))]
    pub host_enforced: Option<bool>,
}

/// Screen v1 contract: `ScreenLaserPatch` (sparse, deny_unknown_fields).
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct ScreenLaserPatch {
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub x: Option<u16>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub y: Option<u16>,
    #[cfg_attr(test, ts(optional))]
    pub active: Option<bool>,
    #[cfg_attr(test, ts(optional))]
    pub color: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub color_rgb: Option<[u8; 3]>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub radius: Option<u8>,
}

/// Screen v1 contract: `ScreenAnnotationCreateRequest` (sparse, deny_unknown_fields).
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct ScreenAnnotationCreateRequest {
    #[cfg_attr(test, ts(optional))]
    pub author: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub kind: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub points: Option<Vec<(u16, u16)>>,
    #[cfg_attr(test, ts(optional))]
    pub points_str: Option<String>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub x: Option<u16>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub y: Option<u16>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub w: Option<u16>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub h: Option<u16>,
    #[cfg_attr(test, ts(optional))]
    pub from: Option<(u16, u16)>,
    #[cfg_attr(test, ts(optional))]
    pub to: Option<(u16, u16)>,
    /// `"x,y"` string form (legacy alias).
    #[cfg_attr(test, ts(optional))]
    pub from_str: Option<String>,
    /// `"x,y"` string form (legacy alias).
    #[cfg_attr(test, ts(optional))]
    pub to_str: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub text: Option<String>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub scale: Option<u8>,
    #[cfg_attr(test, ts(optional))]
    pub color: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub color_rgb: Option<[u8; 3]>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub thickness: Option<u8>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub alpha: Option<u8>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub ttl_ms: Option<u32>,
}

/// Screen v1 contract: `ScreenAuthorRequest` (sparse, deny_unknown_fields).
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct ScreenAuthorRequest {
    #[cfg_attr(test, ts(optional))]
    pub author: Option<String>,
}

/// Screen v1 contract: `ScreenWhiteboardImportRequest` (sparse, deny_unknown_fields).
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct ScreenWhiteboardImportRequest {
    #[cfg_attr(test, ts(optional))]
    pub snapshot: Option<WhiteboardSnapshotDto>,
    #[cfg_attr(test, ts(optional))]
    pub items: Option<Vec<AnnotationItemDto>>,
    #[cfg_attr(test, ts(optional))]
    pub file: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub replace: Option<bool>,
}

/// Screen v1 contract: `ScreenWhiteboardPolicyPatch` (sparse, deny_unknown_fields).
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct ScreenWhiteboardPolicyPatch {
    #[cfg_attr(test, ts(optional))]
    pub read_only: Option<bool>,
    #[cfg_attr(test, ts(optional))]
    pub allow_peer_clear: Option<bool>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub max_items: Option<u16>,
}

/// Screen v1 contract: `ScreenTilePatch` (sparse, deny_unknown_fields).
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct ScreenTilePatch {
    #[cfg_attr(test, ts(optional))]
    pub enabled: Option<bool>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub tile_size: Option<u16>,
}

/// Screen v1 contract: `ScreenTileCachePatch` (sparse, deny_unknown_fields).
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct ScreenTileCachePatch {
    #[cfg_attr(test, ts(optional))]
    pub enabled: Option<bool>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub max_entries: Option<u16>,
}

/// Screen v1 contract: `ScreenFocusPatch` (sparse, deny_unknown_fields).
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct ScreenFocusPatch {
    #[cfg_attr(test, ts(optional))]
    pub enabled: Option<bool>,
    #[cfg_attr(test, ts(optional))]
    pub mode: Option<String>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub radius: Option<u16>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub peripheral_skip_ratio: Option<u8>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub box_x: Option<u16>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub box_y: Option<u16>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub box_w: Option<u16>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub box_h: Option<u16>,
}

/// Screen v1 contract: `ScreenTilePalettePatch` (sparse, deny_unknown_fields).
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct ScreenTilePalettePatch {
    #[cfg_attr(test, ts(optional))]
    pub enabled: Option<bool>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub max_colors: Option<u8>,
    #[cfg_attr(test, ts(optional))]
    pub enable_rle: Option<bool>,
}

/// Screen v1 contract: `ScreenTilePredictPatch` (sparse, deny_unknown_fields).
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct ScreenTilePredictPatch {
    #[cfg_attr(test, ts(optional))]
    pub enabled: Option<bool>,
    #[cfg_attr(test, ts(optional))]
    pub filter_mode: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub enable_rct: Option<bool>,
}

/// Screen v1 contract: `ScreenTilePresentPatch` (sparse, deny_unknown_fields).
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct ScreenTilePresentPatch {
    #[cfg_attr(test, ts(optional))]
    pub enabled: Option<bool>,
    #[cfg_attr(test, ts(optional))]
    pub vsync_mode: Option<String>,
    #[cfg_attr(test, ts(optional))]
    pub double_buffer: Option<bool>,
    #[cfg_attr(test, ts(optional))]
    pub tear_prevention: Option<bool>,
}

/// Screen v1 contract: `ScreenTileNackPatch` (sparse, deny_unknown_fields).
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct ScreenTileNackPatch {
    #[cfg_attr(test, ts(optional))]
    pub enabled: Option<bool>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub timeout_ms: Option<u16>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub max_retries: Option<u8>,
}

/// Screen v1 contract: `ScreenTileSaliencyPatch` (sparse, deny_unknown_fields).
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct ScreenTileSaliencyPatch {
    #[cfg_attr(test, ts(optional))]
    pub enabled: Option<bool>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub high_threshold: Option<u8>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub low_threshold: Option<u8>,
    #[cfg_attr(test, ts(optional))]
    pub drop_background_on_congestion: Option<bool>,
}

/// Screen v1 contract: `ScreenTileLodPatch` (sparse, deny_unknown_fields).
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct ScreenTileLodPatch {
    #[cfg_attr(test, ts(optional))]
    pub enabled: Option<bool>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub max_lod: Option<u8>,
    #[cfg_attr(test, ts(optional))]
    pub auto_lod_on_congestion: Option<bool>,
}

/// Screen v1 contract: `ScreenTileBatchPatch` (sparse, deny_unknown_fields).
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct ScreenTileBatchPatch {
    #[cfg_attr(test, ts(optional))]
    pub enabled: Option<bool>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub target_chunk_bytes: Option<u16>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub pacing_interval_us: Option<u16>,
}

/// Screen v1 contract: `ScreenTileMotionPatch` (sparse, deny_unknown_fields).
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct ScreenTileMotionPatch {
    #[cfg_attr(test, ts(optional))]
    pub enabled: Option<bool>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub max_displacement: Option<u16>,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub min_match_ratio: Option<u8>,
}

// ---- §165a: ANCS domain ----

/// One ANCS open-rule allowlist entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct AncsRule {
    pub app_id: String,
    pub url: String,
}

/// `GET /ancs` report (legacy `AncsReport` with the `ok` envelope folded off).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct AncsStatus {
    pub attached: bool,
    pub state: String,
    #[cfg_attr(test, ts(optional))]
    pub peripheral: Option<String>,
    /// Next reconnect attempt (absent when not scheduled).
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub retry_at_ms: Option<u64>,
    pub rules: Vec<AncsRule>,
    pub allowlist: Vec<String>,
}

/// `POST /ancs/connect` report (legacy `AncsConnectReport` folded).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct AncsConnectReport {
    pub state: String,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub retry_at_ms: Option<u64>,
    pub message: String,
}

/// `POST /ancs/disconnect` report (legacy `AncsDisconnectReport` folded).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct AncsDisconnectReport {
    pub state: String,
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub retry_at_ms: Option<u64>,
    pub message: String,
}

/// `POST /ancs/allow` body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct AncsAllowRequest {
    pub app_id: String,
    #[serde(default = "default_ancs_url")]
    pub url: String,
}

fn default_ancs_url() -> String {
    String::new()
}

/// `POST /ancs/allow` + `DELETE /ancs/allow` report
/// (legacy `AncsRuleReport` with the `ok` envelope folded off).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct AncsRuleReport {
    /// `"add"` or `"remove"`.
    pub action: String,
    #[cfg_attr(test, ts(optional))]
    pub rule: Option<AncsRule>,
    #[cfg_attr(test, ts(type = "number"))]
    pub total_rules: usize,
    #[cfg_attr(test, ts(type = "number"))]
    pub timestamp_ms: u64,
}

/// `POST /ancs/rules/clear` report (legacy `AncsClearRulesReport` folded).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct AncsClearRulesReport {
    #[cfg_attr(test, ts(type = "number"))]
    pub cleared_count: usize,
    #[cfg_attr(test, ts(type = "number"))]
    pub total_rules: usize,
    #[cfg_attr(test, ts(type = "number"))]
    pub timestamp_ms: u64,
}

/// `DELETE /ancs/allow` body variant (query `app_id` preferred).
#[derive(Debug, Clone, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct AncsRemoveRequest {
    #[cfg_attr(test, ts(optional))]
    pub app_id: Option<String>,
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
            decl::<BatteryStateDto>(&cfg),
            decl::<BatteryState>(&cfg),
            decl::<BatteryStatePatch>(&cfg),
            decl::<BatteryPolicyDto>(&cfg),
            decl::<BatteryPolicyPatch>(&cfg),
            decl::<MediaCommandRequest>(&cfg),
            decl::<MediaCommandReport>(&cfg),
            decl::<MediaDispatchItem>(&cfg),
            decl::<MediaCommandMultiReport>(&cfg),
            decl::<MediaStateDto>(&cfg),
            decl::<MediaState>(&cfg),
            decl::<MediaStatePatch>(&cfg),
            decl::<MediaStateUpdateReport>(&cfg),
            decl::<MediaPolicyDto>(&cfg),
            decl::<MediaPolicyPatch>(&cfg),
            decl::<AncsRule>(&cfg),
            decl::<AncsStatus>(&cfg),
            decl::<AncsConnectReport>(&cfg),
            decl::<AncsDisconnectReport>(&cfg),
            decl::<AncsAllowRequest>(&cfg),
            decl::<AncsRuleReport>(&cfg),
            decl::<AncsClearRulesReport>(&cfg),
            decl::<AncsRemoveRequest>(&cfg),
            decl::<ActiveInputSessionDto>(&cfg),
            decl::<InputStatus>(&cfg),
            decl::<InputTraceEntryDto>(&cfg),
            decl::<InputTrace>(&cfg),
            decl::<InputControl>(&cfg),
            decl::<InputTunnelStatus>(&cfg),
            decl::<InputTunnelDecision>(&cfg),
            decl::<InputPolicyDto>(&cfg),
            decl::<InputPolicyPatch>(&cfg),
            decl::<InputAttachRequest>(&cfg),
            decl::<InputReleaseRequest>(&cfg),
            decl::<InputStartRequest>(&cfg),
            decl::<HandoffInboxEntryDto>(&cfg),
            decl::<HandoffInbox>(&cfg),
            decl::<HandoffSendReport>(&cfg),
            decl::<HandoffDispatchItem>(&cfg),
            decl::<HandoffSendMultiReport>(&cfg),
            decl::<HandoffInboxActionReport>(&cfg),
            decl::<HandoffInboxClearReport>(&cfg),
            decl::<HandoffPolicyDto>(&cfg),
            decl::<HandoffPolicyPatch>(&cfg),
            decl::<HandoffSendRequest>(&cfg),
            decl::<HandoffInboxActionRequest>(&cfg),
            decl::<FocusStateDto>(&cfg),
            decl::<PeerFocusDto>(&cfg),
            decl::<FocusPeers>(&cfg),
            decl::<FocusState>(&cfg),
            decl::<FocusSetReport>(&cfg),
            decl::<FocusSyncReport>(&cfg),
            decl::<FocusPolicyDto>(&cfg),
            decl::<FocusPolicyPatch>(&cfg),
            decl::<FocusSetRequest>(&cfg),
            decl::<ActiveCameraSessionDto>(&cfg),
            decl::<CameraStatus>(&cfg),
            decl::<CameraDecision>(&cfg),
            decl::<CameraPolicyDto>(&cfg),
            decl::<CameraPolicyPatch>(&cfg),
            decl::<CameraAttachRequest>(&cfg),
            decl::<CameraDetachRequest>(&cfg),
            decl::<ActiveMicSessionDto>(&cfg),
            decl::<MicStatus>(&cfg),
            decl::<MicDecision>(&cfg),
            decl::<MicPolicyDto>(&cfg),
            decl::<MicPolicyPatch>(&cfg),
            decl::<MicAttachRequest>(&cfg),
            decl::<MicDetachRequest>(&cfg),
            decl::<ScreenStatus>(&cfg),
            decl::<ScreenControl>(&cfg),
            decl::<ScreenDisplays>(&cfg),
            decl::<ScreenDisplayInfoDto>(&cfg),
            decl::<ScreenPrivacy>(&cfg),
            decl::<ScreenRecording>(&cfg),
            decl::<ScreenRecordingReplay>(&cfg),
            decl::<ScreenReplayRecordDto>(&cfg),
            decl::<ScreenPlayback>(&cfg),
            decl::<ScreenSnapshot>(&cfg),
            decl::<ScreenPir>(&cfg),
            decl::<ScreenLtr>(&cfg),
            decl::<ScreenSvc>(&cfg),
            decl::<ScreenCursor>(&cfg),
            decl::<ScreenTrendline>(&cfg),
            decl::<ScreenProbe>(&cfg),
            decl::<ScreenSquelch>(&cfg),
            decl::<ScreenFec>(&cfg),
            decl::<ScreenPacer>(&cfg),
            decl::<ScreenHarq>(&cfg),
            decl::<ScreenTwcc>(&cfg),
            decl::<ScreenSync>(&cfg),
            decl::<ScreenLatency>(&cfg),
            decl::<ScreenLatencyBreakdownDto>(&cfg),
            decl::<ScreenTuning>(&cfg),
            decl::<ScreenTuningDecisionDto>(&cfg),
            decl::<ScreenPlayout>(&cfg),
            decl::<ScreenMatrix>(&cfg),
            decl::<ScreenProfile>(&cfg),
            decl::<ScreenScale>(&cfg),
            decl::<ScreenFps>(&cfg),
            decl::<ScreenBitrate>(&cfg),
            decl::<ScreenColor>(&cfg),
            decl::<ScreenDelta>(&cfg),
            decl::<ScreenQualityMetric>(&cfg),
            decl::<ScreenQoe>(&cfg),
            decl::<ScreenMotion>(&cfg),
            decl::<ScreenRoi>(&cfg),
            decl::<ScreenWatermark>(&cfg),
            decl::<ScreenAnnotations>(&cfg),
            decl::<LaserPointerDto>(&cfg),
            decl::<WhiteboardPolicyDto>(&cfg),
            decl::<WhiteboardSnapshotDto>(&cfg),
            decl::<ScreenTile>(&cfg),
            decl::<ScreenTileCache>(&cfg),
            decl::<ScreenFocus>(&cfg),
            decl::<ScreenTilePalette>(&cfg),
            decl::<ScreenTilePredict>(&cfg),
            decl::<ScreenTilePresent>(&cfg),
            decl::<ScreenTileNack>(&cfg),
            decl::<ScreenTileSaliency>(&cfg),
            decl::<ScreenTileLod>(&cfg),
            decl::<ScreenTileBatch>(&cfg),
            decl::<ScreenTileMotion>(&cfg),
            decl::<ScreenAnnotationAdded>(&cfg),
            decl::<ScreenWhiteboardUndoReport>(&cfg),
            decl::<ScreenWhiteboardRedoReport>(&cfg),
            decl::<ScreenWhiteboardExportReport>(&cfg),
            decl::<ScreenWhiteboardImportReport>(&cfg),
            decl::<ScreenWhiteboardPolicyView>(&cfg),
            decl::<ScreenLaserReport>(&cfg),
            decl::<ScreenStartRequest>(&cfg),
            decl::<ScreenSwitchRequest>(&cfg),
            decl::<ScreenPrivacyPatch>(&cfg),
            decl::<ScreenRecordingStartRequest>(&cfg),
            decl::<ScreenPlaybackStartRequest>(&cfg),
            decl::<ScreenRecordingSnapshotRequest>(&cfg),
            decl::<ScreenCursorPatch>(&cfg),
            decl::<ScreenTrendlinePatch>(&cfg),
            decl::<ScreenProbePatch>(&cfg),
            decl::<ScreenSquelchPatch>(&cfg),
            decl::<ScreenFecPatch>(&cfg),
            decl::<ScreenPacerPatch>(&cfg),
            decl::<ScreenHarqPatch>(&cfg),
            decl::<ScreenTwccPatch>(&cfg),
            decl::<ScreenSyncPatch>(&cfg),
            decl::<ScreenLatencyPatch>(&cfg),
            decl::<ScreenTuningPatch>(&cfg),
            decl::<ScreenPlayoutPatch>(&cfg),
            decl::<ScreenMatrixModePatch>(&cfg),
            decl::<ScreenProfilePatch>(&cfg),
            decl::<ScreenScalePatch>(&cfg),
            decl::<ScreenFpsPatch>(&cfg),
            decl::<ScreenBitratePatch>(&cfg),
            decl::<ScreenColorPatch>(&cfg),
            decl::<ScreenDeltaPatch>(&cfg),
            decl::<ScreenQualityMetricPatch>(&cfg),
            decl::<ScreenQoePatch>(&cfg),
            decl::<ScreenPirPatch>(&cfg),
            decl::<ScreenMotionPatch>(&cfg),
            decl::<ScreenRoiPatch>(&cfg),
            decl::<ScreenLtrPatch>(&cfg),
            decl::<ScreenSvcPatch>(&cfg),
            decl::<ScreenSvcDropT1Request>(&cfg),
            decl::<ScreenWatermarkPatch>(&cfg),
            decl::<ScreenWatermarkEnableRequest>(&cfg),
            decl::<ScreenLaserPatch>(&cfg),
            decl::<ScreenAnnotationCreateRequest>(&cfg),
            decl::<ScreenAuthorRequest>(&cfg),
            decl::<ScreenWhiteboardImportRequest>(&cfg),
            decl::<ScreenWhiteboardPolicyPatch>(&cfg),
            decl::<ScreenTilePatch>(&cfg),
            decl::<ScreenTileCachePatch>(&cfg),
            decl::<ScreenFocusPatch>(&cfg),
            decl::<ScreenTilePalettePatch>(&cfg),
            decl::<ScreenTilePredictPatch>(&cfg),
            decl::<ScreenTilePresentPatch>(&cfg),
            decl::<ScreenTileNackPatch>(&cfg),
            decl::<ScreenTileSaliencyPatch>(&cfg),
            decl::<ScreenTileLodPatch>(&cfg),
            decl::<ScreenTileBatchPatch>(&cfg),
            decl::<ScreenTileMotionPatch>(&cfg),
            decl::<AnnotationItemDto>(&cfg),
            decl::<AnnotationKindDto>(&cfg),
            decl::<SpatialPositionDto>(&cfg),
            decl::<ActiveAudioSessionDto>(&cfg),
            decl::<AudioStatus>(&cfg),
            decl::<AudioDecision>(&cfg),
            decl::<AudioPolicyDto>(&cfg),
            decl::<AudioPolicyPatch>(&cfg),
            decl::<AudioAttachRequest>(&cfg),
            decl::<AudioDetachRequest>(&cfg),
            decl::<ActiveLocateRingDto>(&cfg),
            decl::<LocateStatus>(&cfg),
            decl::<LocateRingReport>(&cfg),
            decl::<LocateStopReport>(&cfg),
            decl::<LocatePolicyDto>(&cfg),
            decl::<LocatePolicyPatch>(&cfg),
            decl::<LocateRingRequest>(&cfg),
            decl::<LocateStopRequest>(&cfg),
            decl::<ActiveDisplaySessionDto>(&cfg),
            decl::<DisplayVirtualStatus>(&cfg),
            decl::<DisplayDecision>(&cfg),
            decl::<DisplayPolicyDto>(&cfg),
            decl::<DisplayPolicyPatch>(&cfg),
            decl::<DisplayAttachRequest>(&cfg),
            decl::<DisplayDetachRequest>(&cfg),
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
