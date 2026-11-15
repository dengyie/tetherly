// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Versioned control-client contract (ADR-139).
//!
//! Every type a client sees on `/api/v1` lives here and nowhere else. The
//! daemon converts its internal reports into these DTOs; the desktop/web
//! client consumes TypeScript generated from them (see the `bindings` test,
//! which fails when `client/src/lib/api/gen/api.ts` drifts). This crate must
//! not depend on `tetherly-node` internals.

use serde::{Deserialize, Serialize};

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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
            Event::Invalidated { topic } => Some(*topic),
        }
    }
}

/// SSE `data:` payload: `{"seq":…,"type":…,…}`. `seq` is also the SSE `id:`
/// and strictly increases within one daemon run; a run starts above any
/// earlier run's range, so a stale `Last-Event-ID` resyncs via snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct EventEnvelope {
    #[cfg_attr(test, ts(type = "number"))]
    pub seq: u64,
    #[serde(flatten)]
    pub event: Event,
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
