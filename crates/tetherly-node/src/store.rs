// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Identity and trust persistence. Temp file + rename. Unix 0600; Windows
//! lives under %LOCALAPPDATA% (user ACL). Never logs secret bytes.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
pub use tetherly_core::battery::{BatteryStateFrame, BatteryValidator, PowerSource};
pub use tetherly_core::focus::{FocusMode, FocusStateFrame, FocusSyncReqFrame, FocusValidator};
pub use tetherly_core::media::{MediaCommand, MediaControlMode, MediaStateFrame, PlaybackState};
pub use tetherly_core::topology::{
    DisplayEdge, DisplayRect, EdgeCrossingConfig, EdgeNeighbor, TopologyStateFrame,
    TopologySyncReqFrame, TopologyValidator,
};
use tetherly_core::{ClipSyncMode, MemoryTrustStore, TrustFile};
pub use tetherly_core::{HandoffAction, HandoffEntry, HandoffStatus};
use tetherly_crypto::Identity;
use tracing::info;

pub const IDENTITY_FILE: &str = "identity.bin";
pub const TRUST_FILE: &str = "trusted.json";
pub const ROUTES_FILE: &str = "routes.json";
pub const BLOCKED_FILE: &str = "blocked.json";
pub const PROFILES_FILE: &str = "profiles.json";
pub const AUTO_ACCEPT_POLICY_FILE: &str = "auto_accept.json";
pub const CLIPBOARD_POLICY_FILE: &str = "clipboard_policy.json";
pub const HANDOFF_POLICY_FILE: &str = "handoff_policy.json";
pub const HANDOFF_INBOX_FILE: &str = "handoff_inbox.json";
pub const MEDIA_POLICY_FILE: &str = "media_policy.json";
pub const BATTERY_POLICY_FILE: &str = "battery_policy.json";
pub const LOCATOR_POLICY_FILE: &str = "locator_policy.json";
pub const ACTIONS_FILE: &str = "actions.json";
pub const ACTION_POLICY_FILE: &str = "action_policy.json";
pub const FOCUS_POLICY_FILE: &str = "focus_policy.json";
pub const TOPOLOGY_LAYOUT_FILE: &str = "topology_layout.json";
pub const TOPOLOGY_POLICY_FILE: &str = "topology_policy.json";
pub const INPUT_POLICY_FILE: &str = "input_policy.json";
pub const VIRTUAL_DISPLAY_POLICY_FILE: &str = "virtual_display_policy.json";
pub const AUDIO_ROUTING_POLICY_FILE: &str = "audio_routing_policy.json";
pub const CAMERA_POLICY_FILE: &str = "camera_policy.json";
pub const MICROPHONE_POLICY_FILE: &str = "microphone_policy.json";
pub const DEFAULT_MAX_HANDOFF_INBOX_ENTRIES: usize = 100;

pub fn default_data_dir() -> PathBuf {
    if let Ok(p) = std::env::var("TETHERLY_DATA_DIR") {
        return PathBuf::from(p);
    }
    if let Some(base) = std::env::var_os("LOCALAPPDATA") {
        return PathBuf::from(base).join("tetherly");
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home).join(".local/share/tetherly");
    }
    PathBuf::from(".tetherly")
}

pub fn ensure_dir(dir: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dir)?;
    restrict_user_only(dir);
    Ok(())
}

fn restrict_user_only(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = fs::metadata(path) {
            let mut p = meta.permissions();
            p.set_mode(0o700);
            let _ = fs::set_permissions(path, p);
        }
    }
    let _ = path;
}

fn restrict_file(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = fs::metadata(path) {
            let mut p = meta.permissions();
            p.set_mode(0o600);
            let _ = fs::set_permissions(path, p);
        }
    }
    let _ = path;
}

pub fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        ensure_dir(parent)?;
    }
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp = path.with_extension(format!("tmp-{}-{nonce}", std::process::id()));
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    restrict_file(path);
    Ok(())
}

pub fn load_or_create_identity(dir: &Path) -> Result<Identity, NodeStoreError> {
    ensure_dir(dir)?;
    let path = dir.join(IDENTITY_FILE);
    if path.exists() {
        let bytes = fs::read(&path)?;
        let id = Identity::from_bytes(&bytes).map_err(|_| NodeStoreError::Corrupt)?;
        info!(device_id = %id.device_id(), "loaded identity");
        return Ok(id);
    }
    let id = Identity::generate().map_err(|_| NodeStoreError::Rng)?;
    atomic_write(&path, &id.to_bytes())?;
    info!(device_id = %id.device_id(), "created identity");
    Ok(id)
}

pub fn load_trust(dir: &Path) -> Result<MemoryTrustStore, NodeStoreError> {
    let path = dir.join(TRUST_FILE);
    let mut store = MemoryTrustStore::default();
    if !path.exists() {
        return Ok(store);
    }
    let bytes = fs::read(&path)?;
    let file: TrustFile = serde_json::from_slice(&bytes)?;
    store.load(file.into_peers()?);
    Ok(store)
}

pub fn save_trust(dir: &Path, store: &MemoryTrustStore) -> Result<(), NodeStoreError> {
    let file = TrustFile::from_peers(&store.all());
    let bytes = serde_json::to_vec_pretty(&file)?;
    atomic_write(&dir.join(TRUST_FILE), &bytes)?;
    Ok(())
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct RouteTable {
    pub routes: Vec<Route>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Route {
    pub device_id: String,
    pub addr: String,
}

pub fn load_routes(dir: &Path) -> RouteTable {
    let path = dir.join(ROUTES_FILE);
    fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn save_routes(dir: &Path, table: &RouteTable) -> Result<(), NodeStoreError> {
    atomic_write(&dir.join(ROUTES_FILE), &serde_json::to_vec_pretty(table)?)?;
    Ok(())
}

pub fn remember_route(dir: &Path, device_id: &str, addr: &str) -> Result<bool, NodeStoreError> {
    let mut table = load_routes(dir);
    let is_update = if let Some(r) = table.routes.iter_mut().find(|r| r.device_id == device_id) {
        r.addr = addr.to_string();
        true
    } else {
        table.routes.push(Route {
            device_id: device_id.to_string(),
            addr: addr.to_string(),
        });
        false
    };
    save_routes(dir, &table)?;
    Ok(is_update)
}

pub fn forget_route(dir: &Path, device_id: &str) -> Result<bool, NodeStoreError> {
    let mut table = load_routes(dir);
    let initial_len = table.routes.len();
    table.routes.retain(|r| r.device_id != device_id);
    let removed = table.routes.len() < initial_len;
    if removed {
        save_routes(dir, &table)?;
    }
    Ok(removed)
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct BlockedPeer {
    pub device_id: String,
    pub reason: Option<String>,
    pub blocked_at: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default, PartialEq, Eq)]
pub struct BlockedTable {
    pub peers: Vec<BlockedPeer>,
}

pub fn load_blocked(dir: &Path) -> BlockedTable {
    let path = dir.join(BLOCKED_FILE);
    fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn save_blocked(dir: &Path, table: &BlockedTable) -> Result<(), NodeStoreError> {
    atomic_write(&dir.join(BLOCKED_FILE), &serde_json::to_vec_pretty(table)?)?;
    Ok(())
}

pub fn add_blocked_peer(
    dir: &Path,
    device_id: &str,
    reason: Option<&str>,
    now_ms: u64,
) -> Result<bool, NodeStoreError> {
    let mut table = load_blocked(dir);
    let is_update = if let Some(p) = table.peers.iter_mut().find(|p| p.device_id == device_id) {
        p.reason = reason.map(ToString::to_string);
        p.blocked_at = now_ms;
        true
    } else {
        table.peers.push(BlockedPeer {
            device_id: device_id.to_string(),
            reason: reason.map(ToString::to_string),
            blocked_at: now_ms,
        });
        false
    };
    save_blocked(dir, &table)?;
    Ok(is_update)
}

pub fn remove_blocked_peer(dir: &Path, device_id: &str) -> Result<bool, NodeStoreError> {
    let mut table = load_blocked(dir);
    let initial_len = table.peers.len();
    table.peers.retain(|p| p.device_id != device_id);
    let removed = table.peers.len() < initial_len;
    if removed {
        save_blocked(dir, &table)?;
    }
    Ok(removed)
}

pub fn is_peer_blocked(dir: &Path, device_id: &str) -> bool {
    let table = load_blocked(dir);
    table.peers.iter().any(|p| p.device_id == device_id)
}

/// Device classification / form-factor enumeration.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize, Default,
)]
#[serde(rename_all = "snake_case")]
pub enum DeviceType {
    Desktop,
    Laptop,
    Phone,
    Tablet,
    Server,
    Cloud,
    #[default]
    Other,
}

impl DeviceType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Desktop => "desktop",
            Self::Laptop => "laptop",
            Self::Phone => "phone",
            Self::Tablet => "tablet",
            Self::Server => "server",
            Self::Cloud => "cloud",
            Self::Other => "other",
        }
    }

    pub fn icon(&self) -> &'static str {
        match self {
            Self::Desktop => "💻",
            Self::Laptop => "💻",
            Self::Phone => "📱",
            Self::Tablet => "📱",
            Self::Server => "🖥️",
            Self::Cloud => "☁️",
            Self::Other => "📟",
        }
    }
}

impl std::str::FromStr for DeviceType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "desktop" => Ok(Self::Desktop),
            "laptop" => Ok(Self::Laptop),
            "phone" | "mobile" => Ok(Self::Phone),
            "tablet" | "pad" => Ok(Self::Tablet),
            "server" | "nas" => Ok(Self::Server),
            "cloud" | "vps" => Ok(Self::Cloud),
            "other" | "" => Ok(Self::Other),
            unknown => Err(format!(
                "invalid device type '{unknown}': expected desktop, laptop, phone, tablet, server, cloud, or other"
            )),
        }
    }
}

/// Rich metadata profile for a mesh peer device.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PeerProfile {
    pub device_id: String,
    #[serde(default)]
    pub device_type: DeviceType,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    pub updated_ms: u64,
}

/// Persistent table containing all known peer profiles.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default, PartialEq, Eq)]
pub struct ProfileTable {
    #[serde(default)]
    pub profiles: Vec<PeerProfile>,
}

pub fn load_profiles(dir: &Path) -> ProfileTable {
    let path = dir.join(PROFILES_FILE);
    fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn save_profiles(dir: &Path, table: &ProfileTable) -> Result<(), NodeStoreError> {
    ensure_dir(dir)?;
    atomic_write(&dir.join(PROFILES_FILE), &serde_json::to_vec_pretty(table)?)?;
    Ok(())
}

/// Routing destination selector for targeted dispatch (single peer, tag multicast, group multicast, device-type fan-out, or all online peers).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetSelector {
    /// Direct single peer target by device ID, hex prefix, or alias.
    Peer(String),
    /// Multicast to all peers matching a specific tag (e.g., "lab", "backup").
    Tag(String),
    /// Multicast to all peers belonging to an organizational group (e.g., "office", "homelab").
    Group(String),
    /// Fan-out to all peers matching a device form-factor type (e.g., Server, Desktop).
    Type(DeviceType),
    /// Broadcast to all currently connected/online live peers.
    AllOnline,
}

impl TargetSelector {
    pub fn as_label(&self) -> String {
        match self {
            Self::Peer(p) => format!("peer:{p}"),
            Self::Tag(t) => format!("tag:{t}"),
            Self::Group(g) => format!("group:{g}"),
            Self::Type(dt) => format!("type:{}", dt.as_str()),
            Self::AllOnline => "all_online".to_string(),
        }
    }

    pub fn from_query(
        peer: Option<&str>,
        tag: Option<&str>,
        group: Option<&str>,
        dev_type: Option<&str>,
        all: bool,
    ) -> Result<Self, String> {
        if all {
            return Ok(Self::AllOnline);
        }
        if let Some(t) = tag.filter(|s| !s.trim().is_empty()) {
            return Ok(Self::Tag(t.trim().to_string()));
        }
        if let Some(g) = group.filter(|s| !s.trim().is_empty()) {
            return Ok(Self::Group(g.trim().to_string()));
        }
        if let Some(dt_str) = dev_type.filter(|s| !s.trim().is_empty()) {
            let dt = dt_str.parse::<DeviceType>()?;
            return Ok(Self::Type(dt));
        }
        if let Some(p) = peer.filter(|s| !s.trim().is_empty()) {
            return Ok(Self::Peer(p.trim().to_string()));
        }
        Err("missing target destination: specify peer, tag, group, type, or all".to_string())
    }
}

pub const AUTO_ACCEPT_FILE: &str = "auto_accept.json";

/// Granular scoped auto-accept policy for inbound file transfer offers.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct AutoAcceptPolicy {
    /// Global master toggle for auto-accepting file transfers.
    pub enabled: bool,
    /// List of organizational group names whose members are permitted to auto-accept without interactive confirmation.
    #[serde(default)]
    pub allowed_groups: Vec<String>,
    /// List of peer profile tags whose members are permitted to auto-accept.
    #[serde(default)]
    pub allowed_tags: Vec<String>,
    /// List of device form factors allowed to auto-accept.
    #[serde(default)]
    pub allowed_types: Vec<DeviceType>,
    /// Optional maximum file size in bytes eligible for automatic acceptance (None = no size limit up to FILE_MAX).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_file_size: Option<u64>,
}

impl AutoAcceptPolicy {
    /// Evaluates whether an incoming file offer from a peer matching `profile` and of size `file_size`
    /// should be automatically accepted according to policy rules.
    pub fn is_eligible(&self, profile: &PeerProfile, file_size: u64) -> bool {
        if !self.enabled {
            return false;
        }
        if let Some(max_sz) = self.max_file_size {
            if file_size > max_sz {
                return false;
            }
        }
        // If no specific group/tag/type filters are configured, policy applies to all trusted peers
        if self.allowed_groups.is_empty()
            && self.allowed_tags.is_empty()
            && self.allowed_types.is_empty()
        {
            return true;
        }

        // Match group
        if let Some(ref grp) = profile.group {
            let grp_lower = grp.to_ascii_lowercase();
            if self
                .allowed_groups
                .iter()
                .any(|g| g.to_ascii_lowercase() == grp_lower)
            {
                return true;
            }
        }

        // Match tags
        for t in &profile.tags {
            let t_lower = t.to_ascii_lowercase();
            if self
                .allowed_tags
                .iter()
                .any(|rule_tag| rule_tag.to_ascii_lowercase() == t_lower)
            {
                return true;
            }
        }

        // Match device type
        if self.allowed_types.contains(&profile.device_type) {
            return true;
        }

        false
    }
}

pub fn load_auto_accept_policy(dir: &Path) -> AutoAcceptPolicy {
    let path = dir.join(AUTO_ACCEPT_FILE);
    fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn save_auto_accept_policy(
    dir: &Path,
    policy: &AutoAcceptPolicy,
) -> Result<(), NodeStoreError> {
    ensure_dir(dir)?;
    atomic_write(
        &dir.join(AUTO_ACCEPT_FILE),
        &serde_json::to_vec_pretty(policy)?,
    )?;
    Ok(())
}

pub const DEFAULT_MAX_CLIP_AUTO_SYNC_BYTES: usize = 65536; // 64 KiB

fn default_true() -> bool {
    true
}

fn default_max_clip_auto_sync_bytes() -> usize {
    DEFAULT_MAX_CLIP_AUTO_SYNC_BYTES
}

/// Granular scoped auto-sync policy for cross-device clipboard distribution.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct ClipboardPolicy {
    /// Operating synchronization mode (Manual, BidirectionalAuto, SendOnly, ReceiveOnly).
    #[serde(default)]
    pub mode: ClipSyncMode,
    /// Whether the Zero-Plaintext sensitive leak guard is enabled (rejecting private keys, API tokens, JWTs, etc.).
    #[serde(default = "default_true")]
    pub sensitive_guard: bool,
    /// List of organizational group names whose members are permitted for automatic sync (empty = all trusted).
    #[serde(default)]
    pub allowed_groups: Vec<String>,
    /// List of peer profile tags whose members are permitted for automatic sync (empty = all trusted).
    #[serde(default)]
    pub allowed_tags: Vec<String>,
    /// List of device form factors allowed for automatic sync (empty = all trusted).
    #[serde(default)]
    pub allowed_types: Vec<DeviceType>,
    /// Maximum payload size in bytes permitted for automatic synchronization (default: 64 KiB).
    #[serde(default = "default_max_clip_auto_sync_bytes")]
    pub max_payload_bytes: usize,
    /// Optional auto-clear countdown in seconds for local clipboard after receipt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_clear_timeout_secs: Option<u64>,
}

impl Default for ClipboardPolicy {
    fn default() -> Self {
        Self {
            mode: ClipSyncMode::Manual,
            sensitive_guard: true,
            allowed_groups: Vec::new(),
            allowed_tags: Vec::new(),
            allowed_types: Vec::new(),
            max_payload_bytes: DEFAULT_MAX_CLIP_AUTO_SYNC_BYTES,
            auto_clear_timeout_secs: None,
        }
    }
}

impl ClipboardPolicy {
    /// Evaluates whether an outbound or inbound clipboard synchronization is permitted for the given peer profile.
    pub fn is_peer_eligible(&self, profile: &PeerProfile) -> bool {
        // If no specific group/tag/type filters are configured, policy applies to all trusted peers
        if self.allowed_groups.is_empty()
            && self.allowed_tags.is_empty()
            && self.allowed_types.is_empty()
        {
            return true;
        }

        // Match group
        if let Some(ref grp) = profile.group {
            let grp_lower = grp.to_ascii_lowercase();
            if self
                .allowed_groups
                .iter()
                .any(|g| g.to_ascii_lowercase() == grp_lower)
            {
                return true;
            }
        }

        // Match tags
        for t in &profile.tags {
            let t_lower = t.to_ascii_lowercase();
            if self
                .allowed_tags
                .iter()
                .any(|rule_tag| rule_tag.to_ascii_lowercase() == t_lower)
            {
                return true;
            }
        }

        // Match device type
        if self.allowed_types.contains(&profile.device_type) {
            return true;
        }

        false
    }
}

pub fn load_clipboard_policy(dir: &Path) -> ClipboardPolicy {
    let path = dir.join(CLIPBOARD_POLICY_FILE);
    fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn save_clipboard_policy(dir: &Path, policy: &ClipboardPolicy) -> Result<(), NodeStoreError> {
    ensure_dir(dir)?;
    atomic_write(
        &dir.join(CLIPBOARD_POLICY_FILE),
        &serde_json::to_vec_pretty(policy)?,
    )?;
    Ok(())
}

fn default_handoff_action() -> HandoffAction {
    HandoffAction::PromptOnly
}

fn default_max_handoff_inbox_entries() -> usize {
    DEFAULT_MAX_HANDOFF_INBOX_ENTRIES
}

/// Node policy governing inbound resource and URL handoff execution and profile scoping.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HandoffPolicy {
    #[serde(default = "default_handoff_action")]
    pub action: HandoffAction,
    #[serde(default)]
    pub allowed_groups: Vec<String>,
    #[serde(default)]
    pub allowed_tags: Vec<String>,
    #[serde(default)]
    pub allowed_types: Vec<DeviceType>,
    #[serde(default = "default_max_handoff_inbox_entries")]
    pub max_inbox_entries: usize,
}

impl Default for HandoffPolicy {
    fn default() -> Self {
        Self {
            action: HandoffAction::PromptOnly,
            allowed_groups: Vec::new(),
            allowed_tags: Vec::new(),
            allowed_types: Vec::new(),
            max_inbox_entries: DEFAULT_MAX_HANDOFF_INBOX_ENTRIES,
        }
    }
}

impl HandoffPolicy {
    /// Checks whether an inbound handoff is permitted to automatically open based on peer profile.
    pub fn is_peer_eligible_for_auto_open(&self, profile: &PeerProfile) -> bool {
        if !self.action.allows_auto_open() {
            return false;
        }

        // If no filter is configured, all trusted peers with AutoOpen policy are eligible
        if self.allowed_groups.is_empty()
            && self.allowed_tags.is_empty()
            && self.allowed_types.is_empty()
        {
            return true;
        }

        // Match group
        if let Some(ref grp) = profile.group {
            let grp_lower = grp.to_ascii_lowercase();
            if self
                .allowed_groups
                .iter()
                .any(|g| g.to_ascii_lowercase() == grp_lower)
            {
                return true;
            }
        }

        // Match tags
        for t in &profile.tags {
            let t_lower = t.to_ascii_lowercase();
            if self
                .allowed_tags
                .iter()
                .any(|rule_tag| rule_tag.to_ascii_lowercase() == t_lower)
            {
                return true;
            }
        }

        // Match device type
        if self.allowed_types.contains(&profile.device_type) {
            return true;
        }

        false
    }
}

pub fn load_handoff_policy(dir: &Path) -> HandoffPolicy {
    let path = dir.join(HANDOFF_POLICY_FILE);
    fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn save_handoff_policy(dir: &Path, policy: &HandoffPolicy) -> Result<(), NodeStoreError> {
    ensure_dir(dir)?;
    atomic_write(
        &dir.join(HANDOFF_POLICY_FILE),
        &serde_json::to_vec_pretty(policy)?,
    )?;
    Ok(())
}

/// In-memory and persisted handoff inbox table.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct HandoffInboxTable {
    #[serde(default)]
    pub entries: Vec<HandoffEntry>,
}

impl HandoffInboxTable {
    pub fn add(&mut self, entry: HandoffEntry, max_entries: usize) {
        // Prepend new entry
        self.entries.insert(0, entry);
        if max_entries > 0 && self.entries.len() > max_entries {
            self.entries.truncate(max_entries);
        }
    }

    pub fn find(&self, id: &str) -> Option<&HandoffEntry> {
        self.entries.iter().find(|e| e.id == id)
    }

    pub fn find_mut(&mut self, id: &str) -> Option<&mut HandoffEntry> {
        self.entries.iter_mut().find(|e| e.id == id)
    }

    pub fn remove(&mut self, id: &str) -> Option<HandoffEntry> {
        if let Some(pos) = self.entries.iter().position(|e| e.id == id) {
            Some(self.entries.remove(pos))
        } else {
            None
        }
    }

    pub fn clear(&mut self) -> usize {
        let count = self.entries.len();
        self.entries.clear();
        count
    }
}

pub fn load_handoff_inbox(dir: &Path) -> HandoffInboxTable {
    let path = dir.join(HANDOFF_INBOX_FILE);
    fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn save_handoff_inbox(dir: &Path, table: &HandoffInboxTable) -> Result<(), NodeStoreError> {
    ensure_dir(dir)?;
    atomic_write(
        &dir.join(HANDOFF_INBOX_FILE),
        &serde_json::to_vec_pretty(table)?,
    )?;
    Ok(())
}

fn default_media_control_mode() -> MediaControlMode {
    MediaControlMode::Allow
}

fn default_max_volume_limit() -> u8 {
    100
}

/// Node policy governing inbound media control commands and profile scoping.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MediaControlPolicy {
    #[serde(default = "default_media_control_mode")]
    pub mode: MediaControlMode,
    #[serde(default)]
    pub allowed_groups: Vec<String>,
    #[serde(default)]
    pub allowed_tags: Vec<String>,
    #[serde(default)]
    pub allowed_types: Vec<DeviceType>,
    #[serde(default = "default_max_volume_limit")]
    pub max_volume_limit: u8,
    #[serde(default = "default_true")]
    pub allow_seek: bool,
}

impl Default for MediaControlPolicy {
    fn default() -> Self {
        Self {
            mode: MediaControlMode::Allow,
            allowed_groups: Vec::new(),
            allowed_tags: Vec::new(),
            allowed_types: Vec::new(),
            max_volume_limit: 100,
            allow_seek: true,
        }
    }
}

impl MediaControlPolicy {
    /// Checks whether an inbound media command is eligible to be processed based on peer profile.
    pub fn is_peer_eligible(&self, profile: &PeerProfile) -> bool {
        if self.mode == MediaControlMode::Disabled {
            return false;
        }

        // If no filter is configured, all trusted peers are eligible
        if self.allowed_groups.is_empty()
            && self.allowed_tags.is_empty()
            && self.allowed_types.is_empty()
        {
            return true;
        }

        // Match group
        if let Some(ref grp) = profile.group {
            let grp_lower = grp.to_ascii_lowercase();
            if self
                .allowed_groups
                .iter()
                .any(|g| g.to_ascii_lowercase() == grp_lower)
            {
                return true;
            }
        }

        // Match tags
        for t in &profile.tags {
            let t_lower = t.to_ascii_lowercase();
            if self
                .allowed_tags
                .iter()
                .any(|at| at.to_ascii_lowercase() == t_lower)
            {
                return true;
            }
        }

        // Match device type
        if self.allowed_types.contains(&profile.device_type) {
            return true;
        }

        false
    }

    /// Clamps command volume to policy limit.
    pub fn clamp_volume(&self, requested_vol: u8) -> u8 {
        requested_vol.min(self.max_volume_limit).min(100)
    }
}

pub fn load_media_policy(dir: &Path) -> MediaControlPolicy {
    let path = dir.join(MEDIA_POLICY_FILE);
    fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn save_media_policy(dir: &Path, policy: &MediaControlPolicy) -> Result<(), NodeStoreError> {
    ensure_dir(dir)?;
    atomic_write(
        &dir.join(MEDIA_POLICY_FILE),
        &serde_json::to_vec_pretty(policy)?,
    )?;
    Ok(())
}

fn default_low_battery_threshold() -> u8 {
    20
}

fn default_critical_battery_threshold() -> u8 {
    10
}

/// Node policy governing cross-device battery monitoring and low-power alerting.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BatteryPolicy {
    #[serde(default = "default_low_battery_threshold")]
    pub low_battery_threshold: u8,
    #[serde(default = "default_critical_battery_threshold")]
    pub critical_battery_threshold: u8,
    #[serde(default = "default_true")]
    pub alert_on_low_battery: bool,
    #[serde(default)]
    pub auto_throttle_on_battery_saver: bool,
    #[serde(default)]
    pub allowed_groups: Vec<String>,
    #[serde(default)]
    pub allowed_tags: Vec<String>,
    #[serde(default)]
    pub allowed_types: Vec<DeviceType>,
}

impl Default for BatteryPolicy {
    fn default() -> Self {
        Self {
            low_battery_threshold: 20,
            critical_battery_threshold: 10,
            alert_on_low_battery: true,
            auto_throttle_on_battery_saver: false,
            allowed_groups: Vec::new(),
            allowed_tags: Vec::new(),
            allowed_types: Vec::new(),
        }
    }
}

impl BatteryPolicy {
    /// Checks whether an inbound battery update is eligible to be processed/alerted based on peer profile.
    pub fn is_peer_eligible(&self, profile: &PeerProfile) -> bool {
        // If no filter is configured, all trusted peers are eligible
        if self.allowed_groups.is_empty()
            && self.allowed_tags.is_empty()
            && self.allowed_types.is_empty()
        {
            return true;
        }

        if let Some(group) = &profile.group {
            if self.allowed_groups.iter().any(|g| g == group) {
                return true;
            }
        }

        if profile.tags.iter().any(|t| self.allowed_tags.contains(t)) {
            return true;
        }

        if self.allowed_types.contains(&profile.device_type) {
            return true;
        }

        false
    }

    /// Sanitizes thresholds to valid ranges.
    pub fn sanitize(&mut self) {
        self.low_battery_threshold = self.low_battery_threshold.min(100);
        self.critical_battery_threshold = self
            .critical_battery_threshold
            .min(self.low_battery_threshold);
    }
}

pub fn load_battery_policy(dir: &Path) -> BatteryPolicy {
    let path = dir.join(BATTERY_POLICY_FILE);
    fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn save_battery_policy(dir: &Path, policy: &BatteryPolicy) -> Result<(), NodeStoreError> {
    ensure_dir(dir)?;
    atomic_write(
        &dir.join(BATTERY_POLICY_FILE),
        &serde_json::to_vec_pretty(policy)?,
    )?;
    Ok(())
}

fn default_locator_max_duration() -> u32 {
    60
}

/// Node policy governing cross-device device locator and acoustic ringing.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LocatorPolicy {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub allow_override_silent: bool,
    #[serde(default = "default_locator_max_duration")]
    pub max_duration_secs: u32,
    #[serde(default)]
    pub allowed_groups: Vec<String>,
    #[serde(default)]
    pub allowed_tags: Vec<String>,
    #[serde(default)]
    pub allowed_types: Vec<DeviceType>,
}

impl Default for LocatorPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            allow_override_silent: true,
            max_duration_secs: 60,
            allowed_groups: Vec::new(),
            allowed_tags: Vec::new(),
            allowed_types: Vec::new(),
        }
    }
}

impl LocatorPolicy {
    /// Checks whether an inbound locate/ring request is eligible based on peer profile.
    pub fn is_peer_eligible(&self, profile: &PeerProfile) -> bool {
        if !self.enabled {
            return false;
        }

        // If no filter is configured, all trusted peers are eligible
        if self.allowed_groups.is_empty()
            && self.allowed_tags.is_empty()
            && self.allowed_types.is_empty()
        {
            return true;
        }

        if let Some(group) = &profile.group {
            if self.allowed_groups.iter().any(|g| g == group) {
                return true;
            }
        }

        if profile.tags.iter().any(|t| self.allowed_tags.contains(t)) {
            return true;
        }

        if self.allowed_types.contains(&profile.device_type) {
            return true;
        }

        false
    }

    /// Sanitizes duration to valid limits (1..=60).
    pub fn sanitize(&mut self) {
        self.max_duration_secs = self.max_duration_secs.clamp(1, 60);
    }
}

pub fn load_locator_policy(dir: &Path) -> LocatorPolicy {
    let path = dir.join(LOCATOR_POLICY_FILE);
    fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn save_locator_policy(dir: &Path, policy: &LocatorPolicy) -> Result<(), NodeStoreError> {
    ensure_dir(dir)?;
    atomic_write(
        &dir.join(LOCATOR_POLICY_FILE),
        &serde_json::to_vec_pretty(policy)?,
    )?;
    Ok(())
}

fn default_action_timeout() -> u32 {
    10
}

/// Local definition of an executable action.
///
/// Under the ADR-008 security invariant, the actual command string and arguments
/// are strictly private to this node's local storage and are NEVER transmitted over the wire.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LocalActionDefinition {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default = "default_action_timeout")]
    pub timeout_secs: u32,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

impl LocalActionDefinition {
    pub fn sanitize(&mut self) {
        self.id = tetherly_core::ActionValidator::clean_identifier(
            &self.id,
            tetherly_core::MAX_ACTION_ID_LEN,
        );
        self.name = tetherly_core::ActionValidator::sanitize_text(
            &self.name,
            tetherly_core::MAX_ACTION_NAME_LEN,
        );
        if let Some(desc) = &self.description {
            self.description = Some(tetherly_core::ActionValidator::sanitize_text(
                desc,
                tetherly_core::MAX_ACTION_DESC_LEN,
            ));
        }
        if let Some(icon) = &self.icon {
            self.icon = Some(tetherly_core::ActionValidator::sanitize_text(
                icon,
                tetherly_core::MAX_ACTION_ICON_LEN,
            ));
        }
        self.timeout_secs = tetherly_core::ActionValidator::clamp_timeout(self.timeout_secs);
    }

    /// Converts this local definition into a safe public metadata ActionItem to advertise to peers.
    pub fn to_item(&self) -> tetherly_core::ActionItem {
        tetherly_core::ActionItem {
            id: self.id.clone(),
            name: self.name.clone(),
            description: self.description.clone(),
            icon: self.icon.clone(),
        }
    }
}

/// Mode governing inbound remote action trigger requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ActionAccessMode {
    #[default]
    Allow,
    PromptOnly,
    Disabled,
}

impl ActionAccessMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::PromptOnly => "prompt_only",
            Self::Disabled => "disabled",
        }
    }
}

impl std::fmt::Display for ActionAccessMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for ActionAccessMode {
    type Err = tetherly_core::CoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "allow" | "enabled" | "yes" => Ok(Self::Allow),
            "prompt_only" | "prompt" | "ask" => Ok(Self::PromptOnly),
            "disabled" | "deny" | "off" | "no" => Ok(Self::Disabled),
            other => Err(tetherly_core::CoreError::InvalidPayload(format!(
                "invalid action access mode: '{other}', expected allow, prompt_only, or disabled"
            ))),
        }
    }
}

/// Inbound access control and scoping policy for predefined quick actions.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActionPolicy {
    #[serde(default)]
    pub mode: ActionAccessMode,
    #[serde(default = "default_action_timeout")]
    pub max_timeout_secs: u32,
    #[serde(default)]
    pub allowed_groups: Vec<String>,
    #[serde(default)]
    pub allowed_tags: Vec<String>,
    #[serde(default)]
    pub allowed_types: Vec<DeviceType>,
    #[serde(default = "default_true")]
    pub expose_action_list: bool,
}

impl Default for ActionPolicy {
    fn default() -> Self {
        Self {
            mode: ActionAccessMode::Allow,
            max_timeout_secs: 10,
            allowed_groups: Vec::new(),
            allowed_tags: Vec::new(),
            allowed_types: Vec::new(),
            expose_action_list: true,
        }
    }
}

impl ActionPolicy {
    /// Evaluates if an inbound action request or list query from a peer is eligible.
    pub fn is_peer_eligible(&self, profile: &PeerProfile) -> bool {
        if self.mode == ActionAccessMode::Disabled {
            return false;
        }

        if self.allowed_groups.is_empty()
            && self.allowed_tags.is_empty()
            && self.allowed_types.is_empty()
        {
            return true;
        }

        if let Some(group) = &profile.group {
            if self.allowed_groups.iter().any(|g| g == group) {
                return true;
            }
        }

        if profile.tags.iter().any(|t| self.allowed_tags.contains(t)) {
            return true;
        }

        if self.allowed_types.contains(&profile.device_type) {
            return true;
        }

        false
    }

    pub fn sanitize(&mut self) {
        self.max_timeout_secs =
            tetherly_core::ActionValidator::clamp_timeout(self.max_timeout_secs);
    }
}

pub fn load_local_actions(dir: &Path) -> Vec<LocalActionDefinition> {
    let path = dir.join(ACTIONS_FILE);
    fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn save_local_actions(
    dir: &Path,
    actions: &[LocalActionDefinition],
) -> Result<(), NodeStoreError> {
    ensure_dir(dir)?;
    atomic_write(
        &dir.join(ACTIONS_FILE),
        &serde_json::to_vec_pretty(actions)?,
    )?;
    Ok(())
}

pub fn load_action_policy(dir: &Path) -> ActionPolicy {
    let path = dir.join(ACTION_POLICY_FILE);
    fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn save_action_policy(dir: &Path, policy: &ActionPolicy) -> Result<(), NodeStoreError> {
    ensure_dir(dir)?;
    atomic_write(
        &dir.join(ACTION_POLICY_FILE),
        &serde_json::to_vec_pretty(policy)?,
    )?;
    Ok(())
}

fn default_quiet_hours_start() -> u16 {
    1380 // 23:00 (23 * 60)
}

fn default_quiet_hours_end() -> u16 {
    420 // 07:00 (7 * 60)
}

/// Node policy governing cross-device focus mode synchronization, quiet hours, and urgency breakthroughs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FocusPolicy {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub default_mode: FocusMode,
    #[serde(default)]
    pub mirror_inbound_focus: bool,
    #[serde(default = "default_true")]
    pub allow_emergency_breakthrough: bool,
    #[serde(default)]
    pub quiet_hours_enabled: bool,
    #[serde(default = "default_quiet_hours_start")]
    pub quiet_hours_start_minute: u16,
    #[serde(default = "default_quiet_hours_end")]
    pub quiet_hours_end_minute: u16,
    #[serde(default)]
    pub allowed_groups: Vec<String>,
    #[serde(default)]
    pub allowed_tags: Vec<String>,
    #[serde(default)]
    pub allowed_types: Vec<DeviceType>,
}

impl Default for FocusPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            default_mode: FocusMode::Dnd,
            mirror_inbound_focus: false,
            allow_emergency_breakthrough: true,
            quiet_hours_enabled: false,
            quiet_hours_start_minute: default_quiet_hours_start(),
            quiet_hours_end_minute: default_quiet_hours_end(),
            allowed_groups: Vec::new(),
            allowed_tags: Vec::new(),
            allowed_types: Vec::new(),
        }
    }
}

impl FocusPolicy {
    /// Evaluates if an inbound focus broadcast from a peer is eligible for mirroring/tracking.
    pub fn is_peer_eligible(&self, profile: &PeerProfile) -> bool {
        if !self.enabled {
            return false;
        }

        if self.allowed_groups.is_empty()
            && self.allowed_tags.is_empty()
            && self.allowed_types.is_empty()
        {
            return true;
        }

        if let Some(group) = &profile.group {
            if self.allowed_groups.iter().any(|g| g == group) {
                return true;
            }
        }

        if profile.tags.iter().any(|t| self.allowed_tags.contains(t)) {
            return true;
        }

        if self.allowed_types.contains(&profile.device_type) {
            return true;
        }

        false
    }

    /// Evaluates whether the given minute-of-day (0..=1439) falls within configured quiet hours.
    /// Handles both daytime intervals (e.g., 09:00 to 17:00, start < end) and overnight wrap-around (e.g., 23:00 to 07:00, start > end).
    pub fn is_quiet_hours_active(&self, minute_of_day: u16) -> bool {
        if !self.quiet_hours_enabled {
            return false;
        }
        let start = self.quiet_hours_start_minute % 1440;
        let end = self.quiet_hours_end_minute % 1440;
        let current = minute_of_day % 1440;

        if start == end {
            // Equal start and end when enabled means active all 24 hours
            true
        } else if start < end {
            // Normal range within same calendar day
            current >= start && current < end
        } else {
            // Crosses midnight (e.g., 23:00 to 07:00: >= 23:00 OR < 07:00)
            current >= start || current < end
        }
    }

    pub fn sanitize(&mut self) {
        self.quiet_hours_start_minute %= 1440;
        self.quiet_hours_end_minute %= 1440;
    }
}

pub fn load_focus_policy(dir: &Path) -> FocusPolicy {
    let path = dir.join(FOCUS_POLICY_FILE);
    fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn save_focus_policy(dir: &Path, policy: &FocusPolicy) -> Result<(), NodeStoreError> {
    ensure_dir(dir)?;
    atomic_write(
        &dir.join(FOCUS_POLICY_FILE),
        &serde_json::to_vec_pretty(policy)?,
    )?;
    Ok(())
}

/// Local screen layout and multi-display edge neighbor arrangement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct TopologyLayout {
    #[serde(default)]
    pub display: DisplayRect,
    #[serde(default)]
    pub neighbors: HashMap<DisplayEdge, EdgeNeighbor>,
    #[serde(default)]
    pub config: EdgeCrossingConfig,
}

pub fn load_topology_layout(dir: &Path) -> TopologyLayout {
    let path = dir.join(TOPOLOGY_LAYOUT_FILE);
    fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn save_topology_layout(dir: &Path, layout: &TopologyLayout) -> Result<(), NodeStoreError> {
    ensure_dir(dir)?;
    atomic_write(
        &dir.join(TOPOLOGY_LAYOUT_FILE),
        &serde_json::to_vec_pretty(layout)?,
    )?;
    Ok(())
}

/// Node policy governing cross-device spatial multi-display topology and cursor crossing.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TopologyPolicy {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub allow_inbound_control: bool,
    #[serde(default = "default_true")]
    pub auto_link_paired_peers: bool,
    #[serde(default)]
    pub allowed_groups: Vec<String>,
    #[serde(default)]
    pub allowed_tags: Vec<String>,
    #[serde(default)]
    pub allowed_types: Vec<DeviceType>,
}

impl Default for TopologyPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            allow_inbound_control: true,
            auto_link_paired_peers: true,
            allowed_groups: Vec::new(),
            allowed_tags: Vec::new(),
            allowed_types: Vec::new(),
        }
    }
}

impl TopologyPolicy {
    /// Checks whether an inbound topology request or cursor crossing is eligible based on peer profile.
    pub fn is_peer_eligible(&self, profile: &PeerProfile) -> bool {
        if !self.enabled {
            return false;
        }

        if self.allowed_groups.is_empty()
            && self.allowed_tags.is_empty()
            && self.allowed_types.is_empty()
        {
            return true;
        }

        if let Some(group) = &profile.group {
            if self.allowed_groups.iter().any(|g| g == group) {
                return true;
            }
        }

        if profile.tags.iter().any(|t| self.allowed_tags.contains(t)) {
            return true;
        }

        if self.allowed_types.contains(&profile.device_type) {
            return true;
        }

        false
    }

    pub fn sanitize(&mut self) {
        self.allowed_groups.retain(|g| !g.trim().is_empty());
        self.allowed_groups.truncate(32);
        self.allowed_tags.retain(|t| !t.trim().is_empty());
        self.allowed_tags.truncate(32);
        self.allowed_types.truncate(16);
    }
}

pub fn load_topology_policy(dir: &Path) -> TopologyPolicy {
    let path = dir.join(TOPOLOGY_POLICY_FILE);
    fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn save_topology_policy(dir: &Path, policy: &TopologyPolicy) -> Result<(), NodeStoreError> {
    ensure_dir(dir)?;
    atomic_write(
        &dir.join(TOPOLOGY_POLICY_FILE),
        &serde_json::to_vec_pretty(policy)?,
    )?;
    Ok(())
}

/// Security and admission governance policy for cross-device virtual peripheral / input tunneling.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InputTunnelPolicy {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub allow_inbound_control: bool,
    #[serde(default = "default_true")]
    pub allow_keyboard: bool,
    #[serde(default = "default_true")]
    pub allow_mouse: bool,
    #[serde(default = "default_true")]
    pub hotkey_release_enabled: bool,
    #[serde(default = "default_input_tunnel_idle_timeout")]
    pub idle_timeout_secs: u32,
    #[serde(default)]
    pub allowed_groups: Vec<String>,
    #[serde(default)]
    pub allowed_tags: Vec<String>,
    #[serde(default)]
    pub allowed_types: Vec<DeviceType>,
}

fn default_input_tunnel_idle_timeout() -> u32 {
    300
}

impl Default for InputTunnelPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            allow_inbound_control: true,
            allow_keyboard: true,
            allow_mouse: true,
            hotkey_release_enabled: true,
            idle_timeout_secs: default_input_tunnel_idle_timeout(),
            allowed_groups: Vec::new(),
            allowed_tags: Vec::new(),
            allowed_types: Vec::new(),
        }
    }
}

impl InputTunnelPolicy {
    pub fn is_peer_eligible(&self, profile: &PeerProfile) -> bool {
        if !self.enabled || !self.allow_inbound_control {
            return false;
        }

        if self.allowed_groups.is_empty()
            && self.allowed_tags.is_empty()
            && self.allowed_types.is_empty()
        {
            return true;
        }

        if let Some(group) = &profile.group {
            if self.allowed_groups.iter().any(|g| g == group) {
                return true;
            }
        }

        if profile.tags.iter().any(|t| self.allowed_tags.contains(t)) {
            return true;
        }

        if self.allowed_types.contains(&profile.device_type) {
            return true;
        }

        false
    }

    pub fn sanitize(&mut self) {
        self.idle_timeout_secs = self.idle_timeout_secs.clamp(10, 3600);
        self.allowed_groups.retain(|g| !g.trim().is_empty());
        self.allowed_groups.truncate(32);
        self.allowed_tags.retain(|t| !t.trim().is_empty());
        self.allowed_tags.truncate(32);
        self.allowed_types.truncate(16);
    }
}

pub fn load_input_policy(dir: &Path) -> InputTunnelPolicy {
    let path = dir.join(INPUT_POLICY_FILE);
    fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn save_input_policy(dir: &Path, policy: &InputTunnelPolicy) -> Result<(), NodeStoreError> {
    ensure_dir(dir)?;
    atomic_write(
        &dir.join(INPUT_POLICY_FILE),
        &serde_json::to_vec_pretty(policy)?,
    )?;
    Ok(())
}

/// Governance policy for virtual secondary display hosting and client extension.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VirtualDisplayPolicy {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub allow_hosting: bool,
    #[serde(default = "default_true")]
    pub allow_remote_extend: bool,
    #[serde(default = "default_vdisp_max_width")]
    pub max_resolution_width: u32,
    #[serde(default = "default_vdisp_max_height")]
    pub max_resolution_height: u32,
    #[serde(default = "default_vdisp_max_fps")]
    pub max_fps: u32,
    #[serde(default = "default_vdisp_idle_timeout")]
    pub idle_timeout_secs: u32,
    #[serde(default)]
    pub allowed_groups: Vec<String>,
    #[serde(default)]
    pub allowed_tags: Vec<String>,
    #[serde(default)]
    pub allowed_types: Vec<DeviceType>,
}

fn default_vdisp_max_width() -> u32 {
    3840
}

fn default_vdisp_max_height() -> u32 {
    2160
}

fn default_vdisp_max_fps() -> u32 {
    60
}

fn default_vdisp_idle_timeout() -> u32 {
    300
}

impl Default for VirtualDisplayPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            allow_hosting: true,
            allow_remote_extend: true,
            max_resolution_width: default_vdisp_max_width(),
            max_resolution_height: default_vdisp_max_height(),
            max_fps: default_vdisp_max_fps(),
            idle_timeout_secs: default_vdisp_idle_timeout(),
            allowed_groups: Vec::new(),
            allowed_tags: Vec::new(),
            allowed_types: Vec::new(),
        }
    }
}

impl VirtualDisplayPolicy {
    pub fn is_peer_eligible(&self, profile: &PeerProfile, as_host: bool) -> bool {
        if !self.enabled {
            return false;
        }
        if as_host && !self.allow_hosting {
            return false;
        }
        if !as_host && !self.allow_remote_extend {
            return false;
        }

        if self.allowed_groups.is_empty()
            && self.allowed_tags.is_empty()
            && self.allowed_types.is_empty()
        {
            return true;
        }

        if let Some(group) = &profile.group {
            if self.allowed_groups.iter().any(|g| g == group) {
                return true;
            }
        }

        if profile.tags.iter().any(|t| self.allowed_tags.contains(t)) {
            return true;
        }

        if self.allowed_types.contains(&profile.device_type) {
            return true;
        }

        false
    }

    pub fn sanitize(&mut self) {
        self.max_resolution_width = self.max_resolution_width.clamp(320, 7680);
        self.max_resolution_height = self.max_resolution_height.clamp(240, 4320);
        self.max_fps = self.max_fps.clamp(10, 120);
        self.idle_timeout_secs = self.idle_timeout_secs.clamp(10, 3600);
        self.allowed_groups.retain(|g| !g.trim().is_empty());
        self.allowed_groups.truncate(32);
        self.allowed_tags.retain(|t| !t.trim().is_empty());
        self.allowed_tags.truncate(32);
        self.allowed_types.truncate(16);
    }
}

pub fn load_virtual_display_policy(dir: &Path) -> VirtualDisplayPolicy {
    let path = dir.join(VIRTUAL_DISPLAY_POLICY_FILE);
    fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn save_virtual_display_policy(
    dir: &Path,
    policy: &VirtualDisplayPolicy,
) -> Result<(), NodeStoreError> {
    ensure_dir(dir)?;
    atomic_write(
        &dir.join(VIRTUAL_DISPLAY_POLICY_FILE),
        &serde_json::to_vec_pretty(policy)?,
    )?;
    Ok(())
}

fn default_audio_max_volume() -> u8 {
    100
}

fn default_audio_idle_timeout() -> u32 {
    300
}

/// Governance policy for cross-device spatial audio routing and sink playback.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AudioRoutingPolicy {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub allow_inbound_sink: bool,
    #[serde(default = "default_true")]
    pub allow_outbound_routing: bool,
    #[serde(default = "default_audio_max_volume")]
    pub max_volume_limit: u8,
    #[serde(default = "default_true")]
    pub allow_spatial_rendering: bool,
    #[serde(default = "default_audio_idle_timeout")]
    pub idle_timeout_secs: u32,
    #[serde(default)]
    pub allowed_groups: Vec<String>,
    #[serde(default)]
    pub allowed_tags: Vec<String>,
    #[serde(default)]
    pub allowed_types: Vec<DeviceType>,
}

impl Default for AudioRoutingPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            allow_inbound_sink: true,
            allow_outbound_routing: true,
            max_volume_limit: default_audio_max_volume(),
            allow_spatial_rendering: true,
            idle_timeout_secs: default_audio_idle_timeout(),
            allowed_groups: Vec::new(),
            allowed_tags: Vec::new(),
            allowed_types: Vec::new(),
        }
    }
}

impl AudioRoutingPolicy {
    pub fn is_peer_eligible(&self, profile: &PeerProfile, as_sink: bool) -> bool {
        if !self.enabled {
            return false;
        }
        if as_sink && !self.allow_inbound_sink {
            return false;
        }
        if !as_sink && !self.allow_outbound_routing {
            return false;
        }

        if self.allowed_groups.is_empty()
            && self.allowed_tags.is_empty()
            && self.allowed_types.is_empty()
        {
            return true;
        }

        if let Some(group) = &profile.group {
            if self.allowed_groups.iter().any(|g| g == group) {
                return true;
            }
        }

        if profile.tags.iter().any(|t| self.allowed_tags.contains(t)) {
            return true;
        }

        if self.allowed_types.contains(&profile.device_type) {
            return true;
        }

        false
    }

    pub fn sanitize(&mut self) {
        self.max_volume_limit = self.max_volume_limit.min(100);
        self.idle_timeout_secs = self.idle_timeout_secs.clamp(10, 3600);
        self.allowed_groups.retain(|g| !g.trim().is_empty());
        self.allowed_groups.truncate(32);
        self.allowed_tags.retain(|t| !t.trim().is_empty());
        self.allowed_tags.truncate(32);
        self.allowed_types.truncate(16);
    }
}

pub fn load_audio_routing_policy(dir: &Path) -> AudioRoutingPolicy {
    let path = dir.join(AUDIO_ROUTING_POLICY_FILE);
    fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn save_audio_routing_policy(
    dir: &Path,
    policy: &AudioRoutingPolicy,
) -> Result<(), NodeStoreError> {
    ensure_dir(dir)?;
    atomic_write(
        &dir.join(AUDIO_ROUTING_POLICY_FILE),
        &serde_json::to_vec_pretty(policy)?,
    )?;
    Ok(())
}

fn default_camera_max_width() -> u32 {
    3840
}

fn default_camera_max_height() -> u32 {
    2160
}

fn default_camera_max_fps() -> u32 {
    60
}

fn default_camera_idle_timeout() -> u32 {
    300
}

/// Governance policy for cross-device virtual camera streaming and remote lens capture.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CameraPolicy {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub allow_inbound_stream: bool,
    #[serde(default = "default_true")]
    pub allow_remote_capture: bool,
    #[serde(default = "default_camera_max_width")]
    pub max_resolution_width: u32,
    #[serde(default = "default_camera_max_height")]
    pub max_resolution_height: u32,
    #[serde(default = "default_camera_max_fps")]
    pub max_fps: u32,
    #[serde(default = "default_true")]
    pub enforce_privacy_shutter_indicator: bool,
    #[serde(default = "default_camera_idle_timeout")]
    pub idle_timeout_secs: u32,
    #[serde(default)]
    pub allowed_groups: Vec<String>,
    #[serde(default)]
    pub allowed_tags: Vec<String>,
    #[serde(default)]
    pub allowed_types: Vec<DeviceType>,
}

impl Default for CameraPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            allow_inbound_stream: true,
            allow_remote_capture: true,
            max_resolution_width: default_camera_max_width(),
            max_resolution_height: default_camera_max_height(),
            max_fps: default_camera_max_fps(),
            enforce_privacy_shutter_indicator: true,
            idle_timeout_secs: default_camera_idle_timeout(),
            allowed_groups: Vec::new(),
            allowed_tags: Vec::new(),
            allowed_types: Vec::new(),
        }
    }
}

impl CameraPolicy {
    pub fn is_peer_eligible(&self, profile: &PeerProfile, as_host: bool) -> bool {
        if !self.enabled {
            return false;
        }
        if as_host && !self.allow_inbound_stream {
            return false;
        }
        if !as_host && !self.allow_remote_capture {
            return false;
        }

        if self.allowed_groups.is_empty()
            && self.allowed_tags.is_empty()
            && self.allowed_types.is_empty()
        {
            return true;
        }

        if let Some(group) = &profile.group {
            if self.allowed_groups.iter().any(|g| g == group) {
                return true;
            }
        }

        if profile.tags.iter().any(|t| self.allowed_tags.contains(t)) {
            return true;
        }

        if self.allowed_types.contains(&profile.device_type) {
            return true;
        }

        false
    }

    pub fn sanitize(&mut self) {
        self.max_resolution_width = self.max_resolution_width.clamp(320, 7680);
        self.max_resolution_height = self.max_resolution_height.clamp(240, 4320);
        self.max_fps = self.max_fps.clamp(1, 60);
        self.idle_timeout_secs = self.idle_timeout_secs.clamp(10, 3600);
        self.allowed_groups.retain(|g| !g.trim().is_empty());
        self.allowed_groups.truncate(32);
        self.allowed_tags.retain(|t| !t.trim().is_empty());
        self.allowed_tags.truncate(32);
        self.allowed_types.truncate(16);
    }
}

pub fn load_camera_policy(dir: &Path) -> CameraPolicy {
    let path = dir.join(CAMERA_POLICY_FILE);
    fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn save_camera_policy(dir: &Path, policy: &CameraPolicy) -> Result<(), NodeStoreError> {
    ensure_dir(dir)?;
    atomic_write(
        &dir.join(CAMERA_POLICY_FILE),
        &serde_json::to_vec_pretty(policy)?,
    )?;
    Ok(())
}

fn default_mic_max_sample_rate() -> u32 {
    48000
}

fn default_mic_idle_timeout() -> u32 {
    300
}

fn default_mic_max_duration() -> u32 {
    3600
}

/// Governance policy for cross-device virtual microphone streaming and voice intercom.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MicrophonePolicy {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub allow_inbound_stream: bool,
    #[serde(default = "default_true")]
    pub allow_remote_capture: bool,
    #[serde(default = "default_true")]
    pub allow_intercom: bool,
    #[serde(default = "default_mic_max_sample_rate")]
    pub max_sample_rate: u32,
    #[serde(default = "default_mic_max_duration")]
    pub max_session_duration_secs: u32,
    #[serde(default = "default_true")]
    pub enforce_privacy_indicator: bool,
    #[serde(default)]
    pub default_muted: bool,
    #[serde(default = "default_mic_idle_timeout")]
    pub idle_timeout_secs: u32,
    #[serde(default)]
    pub allowed_groups: Vec<String>,
    #[serde(default)]
    pub allowed_tags: Vec<String>,
    #[serde(default)]
    pub allowed_types: Vec<DeviceType>,
}

impl Default for MicrophonePolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            allow_inbound_stream: true,
            allow_remote_capture: true,
            allow_intercom: true,
            max_sample_rate: default_mic_max_sample_rate(),
            max_session_duration_secs: default_mic_max_duration(),
            enforce_privacy_indicator: true,
            default_muted: false,
            idle_timeout_secs: default_mic_idle_timeout(),
            allowed_groups: Vec::new(),
            allowed_tags: Vec::new(),
            allowed_types: Vec::new(),
        }
    }
}

impl MicrophonePolicy {
    /// `as_host` selects between inbound consumption (this node plays remote mic feed)
    /// and remote capture (this node's physical microphone is streamed to a peer).
    pub fn is_peer_eligible(&self, profile: &PeerProfile, as_host: bool, intercom: bool) -> bool {
        if !self.enabled {
            return false;
        }
        if as_host && !self.allow_inbound_stream {
            return false;
        }
        if !as_host && !self.allow_remote_capture {
            return false;
        }
        if intercom && !self.allow_intercom {
            return false;
        }

        if self.allowed_groups.is_empty()
            && self.allowed_tags.is_empty()
            && self.allowed_types.is_empty()
        {
            return true;
        }

        if let Some(group) = &profile.group {
            if self.allowed_groups.iter().any(|g| g == group) {
                return true;
            }
        }

        if profile.tags.iter().any(|t| self.allowed_tags.contains(t)) {
            return true;
        }

        if self.allowed_types.contains(&profile.device_type) {
            return true;
        }

        false
    }

    pub fn sanitize(&mut self) {
        self.max_sample_rate = self.max_sample_rate.clamp(8000, 192000);
        self.max_session_duration_secs = self.max_session_duration_secs.clamp(10, 86400);
        self.idle_timeout_secs = self.idle_timeout_secs.clamp(10, 3600);
        self.allowed_groups.retain(|g| !g.trim().is_empty());
        self.allowed_groups.truncate(32);
        self.allowed_tags.retain(|t| !t.trim().is_empty());
        self.allowed_tags.truncate(32);
        self.allowed_types.truncate(16);
    }
}

pub fn load_microphone_policy(dir: &Path) -> MicrophonePolicy {
    let path = dir.join(MICROPHONE_POLICY_FILE);
    fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn save_microphone_policy(dir: &Path, policy: &MicrophonePolicy) -> Result<(), NodeStoreError> {
    ensure_dir(dir)?;
    atomic_write(
        &dir.join(MICROPHONE_POLICY_FILE),
        &serde_json::to_vec_pretty(policy)?,
    )?;
    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum NodeStoreError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("identity file corrupt")]
    Corrupt,
    #[error("credential vault unavailable but identity.ref present")]
    VaultUnavailable,
    #[error("rng failure")]
    Rng,
    #[error(transparent)]
    Core(#[from] tetherly_core::CoreError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use tetherly_core::{TrustStore, TrustedPeer};

    #[test]
    fn identity_roundtrip() {
        let dir = std::env::temp_dir().join(format!("tetherly-id-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let a = load_or_create_identity(&dir).unwrap();
        let b = load_or_create_identity(&dir).unwrap();
        assert_eq!(a.device_id(), b.device_id());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn trust_roundtrip() {
        let dir = std::env::temp_dir().join(format!("tetherly-tr-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        ensure_dir(&dir).unwrap();
        let mut store = MemoryTrustStore::default();
        store
            .put(TrustedPeer {
                device_id: tetherly_core::DeviceId::from_id_pk(&[1u8; 32]),
                id_pk: [1u8; 32],
                n_pk: [2u8; 32],
                alias: "p".into(),
                paired_at: 1,
                revoked: false,
            })
            .unwrap();
        save_trust(&dir, &store).unwrap();
        let loaded = load_trust(&dir).unwrap();
        assert_eq!(loaded.all().len(), 1);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn trust_update_alias_roundtrip() {
        let dir = std::env::temp_dir().join(format!("tetherly-tr-alias-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        ensure_dir(&dir).unwrap();
        let mut store = MemoryTrustStore::default();
        let id = tetherly_core::DeviceId::from_id_pk(&[3u8; 32]);
        store
            .put(TrustedPeer {
                device_id: id.clone(),
                id_pk: [3u8; 32],
                n_pk: [4u8; 32],
                alias: "initial_name".into(),
                paired_at: 1,
                revoked: false,
            })
            .unwrap();
        save_trust(&dir, &store).unwrap();

        assert!(store.update_alias(&id, "My Laptop").unwrap());
        save_trust(&dir, &store).unwrap();

        let loaded = load_trust(&dir).unwrap();
        let peer = loaded.get(&id).expect("peer present");
        assert_eq!(peer.alias, "My Laptop");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn route_remember_and_forget_roundtrip() {
        let dir = std::env::temp_dir().join(format!("tetherly-route-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        ensure_dir(&dir).unwrap();

        remember_route(&dir, "tdev_peer1", "127.0.0.1:45717").unwrap();
        remember_route(&dir, "tdev_peer2", "127.0.0.1:45718").unwrap();
        let routes = load_routes(&dir);
        assert_eq!(routes.routes.len(), 2);

        forget_route(&dir, "tdev_peer1").unwrap();
        let routes2 = load_routes(&dir);
        assert_eq!(routes2.routes.len(), 1);
        assert_eq!(routes2.routes[0].device_id, "tdev_peer2");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn profile_persistence_roundtrip() {
        let dir = std::env::temp_dir().join(format!("tetherly-prof-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        ensure_dir(&dir).unwrap();

        let mut table = ProfileTable::default();
        table.profiles.push(PeerProfile {
            device_id: "tdev_test_1".into(),
            device_type: DeviceType::Server,
            tags: vec!["backup".into(), "nas".into()],
            group: Some("homelab".into()),
            notes: Some("Primary storage node".into()),
            updated_ms: 123456,
        });

        save_profiles(&dir, &table).unwrap();
        let loaded = load_profiles(&dir);
        assert_eq!(loaded.profiles.len(), 1);
        assert_eq!(loaded.profiles[0].device_type, DeviceType::Server);
        assert_eq!(loaded.profiles[0].tags, vec!["backup", "nas"]);
        assert_eq!(loaded.profiles[0].group.as_deref(), Some("homelab"));
        assert_eq!(
            loaded.profiles[0].notes.as_deref(),
            Some("Primary storage node")
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn auto_accept_policy_roundtrip_and_matching() {
        let dir = std::env::temp_dir().join(format!("tetherly-autoacc-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        ensure_dir(&dir).unwrap();

        let policy = AutoAcceptPolicy {
            enabled: true,
            allowed_groups: vec!["homelab".into()],
            allowed_tags: vec!["backup".into()],
            allowed_types: vec![DeviceType::Server],
            max_file_size: Some(10 * 1024 * 1024), // 10 MiB
        };

        save_auto_accept_policy(&dir, &policy).unwrap();
        let loaded = load_auto_accept_policy(&dir);
        assert_eq!(loaded, policy);

        // Test matching:
        let prof_server = PeerProfile {
            device_id: "dev1".into(),
            device_type: DeviceType::Server,
            tags: vec![],
            group: None,
            notes: None,
            updated_ms: 0,
        };
        assert!(loaded.is_eligible(&prof_server, 1024));
        assert!(!loaded.is_eligible(&prof_server, 20 * 1024 * 1024)); // oversized

        let prof_homelab = PeerProfile {
            device_id: "dev2".into(),
            device_type: DeviceType::Laptop,
            tags: vec![],
            group: Some("homelab".into()),
            notes: None,
            updated_ms: 0,
        };
        assert!(loaded.is_eligible(&prof_homelab, 1024));

        let prof_backup_tag = PeerProfile {
            device_id: "dev3".into(),
            device_type: DeviceType::Phone,
            tags: vec!["backup".into()],
            group: None,
            notes: None,
            updated_ms: 0,
        };
        assert!(loaded.is_eligible(&prof_backup_tag, 1024));

        let prof_unmatched = PeerProfile {
            device_id: "dev4".into(),
            device_type: DeviceType::Other,
            tags: vec!["work".into()],
            group: Some("stranger".into()),
            notes: None,
            updated_ms: 0,
        };
        assert!(!loaded.is_eligible(&prof_unmatched, 1024));

        // Disabled policy test
        let disabled_policy = AutoAcceptPolicy {
            enabled: false,
            ..policy
        };
        assert!(!disabled_policy.is_eligible(&prof_server, 1024));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn target_selector_query_parsing() {
        assert_eq!(
            TargetSelector::from_query(None, None, None, None, true).unwrap(),
            TargetSelector::AllOnline
        );
        assert_eq!(
            TargetSelector::from_query(None, Some("lab"), None, None, false).unwrap(),
            TargetSelector::Tag("lab".into())
        );
        assert_eq!(
            TargetSelector::from_query(None, None, Some("ops"), None, false).unwrap(),
            TargetSelector::Group("ops".into())
        );
        assert_eq!(
            TargetSelector::from_query(None, None, None, Some("server"), false).unwrap(),
            TargetSelector::Type(DeviceType::Server)
        );
        assert_eq!(
            TargetSelector::from_query(Some("tdev_p1"), None, None, None, false).unwrap(),
            TargetSelector::Peer("tdev_p1".into())
        );
        assert!(TargetSelector::from_query(None, None, None, None, false).is_err());
    }

    #[test]
    fn clipboard_policy_roundtrip_and_matching() {
        let dir = std::env::temp_dir().join(format!("tetherly-clippol-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        ensure_dir(&dir).unwrap();

        // 1. Default policy check
        let def = load_clipboard_policy(&dir);
        assert_eq!(def.mode, ClipSyncMode::Manual);
        assert!(def.sensitive_guard);
        assert_eq!(def.max_payload_bytes, DEFAULT_MAX_CLIP_AUTO_SYNC_BYTES);
        assert!(def.allowed_groups.is_empty());
        assert!(def.allowed_tags.is_empty());
        assert!(def.allowed_types.is_empty());

        let prof_any = PeerProfile {
            device_id: "dev_any".into(),
            device_type: DeviceType::Desktop,
            tags: vec!["office".into()],
            group: Some("engineering".into()),
            notes: None,
            updated_ms: 0,
        };
        // Empty filters allow all trusted peers
        assert!(def.is_peer_eligible(&prof_any));

        // 2. Custom policy and atomic persistence
        let custom = ClipboardPolicy {
            mode: ClipSyncMode::BidirectionalAuto,
            sensitive_guard: true,
            allowed_groups: vec!["engineering".into()],
            allowed_tags: vec!["secure".into()],
            allowed_types: vec![DeviceType::Laptop, DeviceType::Tablet],
            max_payload_bytes: 32 * 1024,
            auto_clear_timeout_secs: Some(30),
        };

        save_clipboard_policy(&dir, &custom).unwrap();
        let loaded = load_clipboard_policy(&dir);
        assert_eq!(loaded, custom);

        // Group match
        let prof_eng = PeerProfile {
            device_id: "dev_eng".into(),
            device_type: DeviceType::Other,
            tags: vec![],
            group: Some("ENGINEERING".into()), // case-insensitive
            notes: None,
            updated_ms: 0,
        };
        assert!(loaded.is_peer_eligible(&prof_eng));

        // Tag match
        let prof_tag = PeerProfile {
            device_id: "dev_tag".into(),
            device_type: DeviceType::Phone,
            tags: vec!["SECURE".into()], // case-insensitive
            group: None,
            notes: None,
            updated_ms: 0,
        };
        assert!(loaded.is_peer_eligible(&prof_tag));

        // Device type match
        let prof_type = PeerProfile {
            device_id: "dev_type".into(),
            device_type: DeviceType::Tablet,
            tags: vec![],
            group: None,
            notes: None,
            updated_ms: 0,
        };
        assert!(loaded.is_peer_eligible(&prof_type));

        // Unmatched peer
        let prof_unmatched = PeerProfile {
            device_id: "dev_unmatched".into(),
            device_type: DeviceType::Server,
            tags: vec!["cloud".into()],
            group: Some("marketing".into()),
            notes: None,
            updated_ms: 0,
        };
        assert!(!loaded.is_peer_eligible(&prof_unmatched));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn handoff_policy_and_inbox_roundtrip() {
        let dir = std::env::temp_dir().join(format!("tetherly-handoff-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        ensure_dir(&dir).unwrap();

        // 1. Policy test
        let def = load_handoff_policy(&dir);
        assert_eq!(def.action, HandoffAction::PromptOnly);
        assert_eq!(def.max_inbox_entries, DEFAULT_MAX_HANDOFF_INBOX_ENTRIES);

        let custom_policy = HandoffPolicy {
            action: HandoffAction::AutoOpen,
            allowed_groups: vec!["office".into()],
            allowed_tags: vec!["trusted".into()],
            allowed_types: vec![DeviceType::Phone],
            max_inbox_entries: 50,
        };
        save_handoff_policy(&dir, &custom_policy).unwrap();
        let loaded_pol = load_handoff_policy(&dir);
        assert_eq!(loaded_pol, custom_policy);

        let prof_phone = PeerProfile {
            device_id: "phone1".into(),
            device_type: DeviceType::Phone,
            tags: vec![],
            group: None,
            notes: None,
            updated_ms: 0,
        };
        assert!(loaded_pol.is_peer_eligible_for_auto_open(&prof_phone));

        let prof_server = PeerProfile {
            device_id: "srv1".into(),
            device_type: DeviceType::Server,
            tags: vec![],
            group: None,
            notes: None,
            updated_ms: 0,
        };
        assert!(!loaded_pol.is_peer_eligible_for_auto_open(&prof_server));

        // 2. Inbox table test
        let mut inbox = load_handoff_inbox(&dir);
        assert!(inbox.entries.is_empty());

        let e1 = HandoffEntry {
            id: "ho_1".into(),
            sender_id: "peer_a".into(),
            sender_alias: "Laptop".into(),
            url: "https://example.com/a".into(),
            title: "Example A".into(),
            status: HandoffStatus::Pending,
            received_ms: 1000,
            opened_ms: None,
        };
        let e2 = HandoffEntry {
            id: "ho_2".into(),
            sender_id: "peer_b".into(),
            sender_alias: "Phone".into(),
            url: "https://example.com/b".into(),
            title: "Example B".into(),
            status: HandoffStatus::Opened,
            received_ms: 2000,
            opened_ms: Some(2100),
        };

        inbox.add(e1, 10);
        inbox.add(e2, 10);
        assert_eq!(inbox.entries.len(), 2);
        assert_eq!(inbox.entries[0].id, "ho_2"); // latest prepended

        save_handoff_inbox(&dir, &inbox).unwrap();
        let loaded_inbox = load_handoff_inbox(&dir);
        assert_eq!(loaded_inbox.entries.len(), 2);
        assert_eq!(loaded_inbox.entries[0].id, "ho_2");
        assert_eq!(loaded_inbox.entries[1].id, "ho_1");

        // Truncation check
        let mut bounded = HandoffInboxTable::default();
        for i in 0..5 {
            bounded.add(
                HandoffEntry {
                    id: format!("ho_{i}"),
                    sender_id: "p".into(),
                    sender_alias: "P".into(),
                    url: format!("https://example.com/{i}"),
                    title: "".into(),
                    status: HandoffStatus::Pending,
                    received_ms: 0,
                    opened_ms: None,
                },
                3,
            );
        }
        assert_eq!(bounded.entries.len(), 3);
        assert_eq!(bounded.entries[0].id, "ho_4");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn media_policy_roundtrip_and_matching() {
        let dir = std::env::temp_dir().join(format!("tetherly-media-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        ensure_dir(&dir).unwrap();

        // 1. Defaults
        let def = load_media_policy(&dir);
        assert_eq!(def.mode, MediaControlMode::Allow);
        assert_eq!(def.max_volume_limit, 100);
        assert!(def.allow_seek);

        // 2. Custom policy persistence
        let custom = MediaControlPolicy {
            mode: MediaControlMode::Allow,
            allowed_groups: vec!["living-room".into()],
            allowed_tags: vec!["speaker".into()],
            allowed_types: vec![DeviceType::Phone],
            max_volume_limit: 80,
            allow_seek: false,
        };
        save_media_policy(&dir, &custom).unwrap();
        let loaded = load_media_policy(&dir);
        assert_eq!(loaded, custom);

        // 3. Matching
        let prof_ok = PeerProfile {
            device_id: "phone_1".into(),
            device_type: DeviceType::Phone,
            tags: vec![],
            group: None,
            notes: None,
            updated_ms: 0,
        };
        assert!(loaded.is_peer_eligible(&prof_ok));

        let prof_group = PeerProfile {
            device_id: "pc_1".into(),
            device_type: DeviceType::Desktop,
            tags: vec![],
            group: Some("LIVING-ROOM".into()),
            notes: None,
            updated_ms: 0,
        };
        assert!(loaded.is_peer_eligible(&prof_group));

        let prof_tag = PeerProfile {
            device_id: "spk_1".into(),
            device_type: DeviceType::Other,
            tags: vec!["SPEAKER".into()],
            group: None,
            notes: None,
            updated_ms: 0,
        };
        assert!(loaded.is_peer_eligible(&prof_tag));

        let prof_unmatched = PeerProfile {
            device_id: "pc_2".into(),
            device_type: DeviceType::Desktop,
            tags: vec!["gaming".into()],
            group: Some("bedroom".into()),
            notes: None,
            updated_ms: 0,
        };
        assert!(!loaded.is_peer_eligible(&prof_unmatched));

        // 4. Clamping
        assert_eq!(loaded.clamp_volume(95), 80);
        assert_eq!(loaded.clamp_volume(60), 60);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn battery_policy_roundtrip_and_matching() {
        let dir = std::env::temp_dir().join(format!("tetherly-bat-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        ensure_dir(&dir).unwrap();

        // 1. Defaults
        let def = load_battery_policy(&dir);
        assert_eq!(def.low_battery_threshold, 20);
        assert_eq!(def.critical_battery_threshold, 10);
        assert!(def.alert_on_low_battery);
        assert!(!def.auto_throttle_on_battery_saver);

        // 2. Custom policy persistence
        let mut custom = BatteryPolicy {
            low_battery_threshold: 30,
            critical_battery_threshold: 15,
            alert_on_low_battery: true,
            auto_throttle_on_battery_saver: true,
            allowed_groups: vec!["office".into()],
            allowed_tags: vec!["mobile".into()],
            allowed_types: vec![DeviceType::Phone],
        };
        custom.sanitize();
        save_battery_policy(&dir, &custom).unwrap();
        let loaded = load_battery_policy(&dir);
        assert_eq!(loaded, custom);

        // 3. Matching
        let prof_ok = PeerProfile {
            device_id: "phone_1".into(),
            device_type: DeviceType::Phone,
            tags: vec![],
            group: None,
            notes: None,
            updated_ms: 0,
        };
        assert!(loaded.is_peer_eligible(&prof_ok));

        let prof_group = PeerProfile {
            device_id: "pc_1".into(),
            device_type: DeviceType::Desktop,
            tags: vec![],
            group: Some("office".into()),
            notes: None,
            updated_ms: 0,
        };
        assert!(loaded.is_peer_eligible(&prof_group));

        let prof_tag = PeerProfile {
            device_id: "tab_1".into(),
            device_type: DeviceType::Other,
            tags: vec!["mobile".into()],
            group: None,
            notes: None,
            updated_ms: 0,
        };
        assert!(loaded.is_peer_eligible(&prof_tag));

        let prof_unmatched = PeerProfile {
            device_id: "pc_2".into(),
            device_type: DeviceType::Desktop,
            tags: vec!["server".into()],
            group: Some("datacenter".into()),
            notes: None,
            updated_ms: 0,
        };
        assert!(!loaded.is_peer_eligible(&prof_unmatched));

        // 4. Sanitize invalid threshold clamp
        let mut invalid = BatteryPolicy {
            low_battery_threshold: 150,
            critical_battery_threshold: 80,
            ..Default::default()
        };
        invalid.sanitize();
        assert_eq!(invalid.low_battery_threshold, 100);
        assert_eq!(invalid.critical_battery_threshold, 80);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn locator_policy_roundtrip_and_matching() {
        let dir = std::env::temp_dir().join(format!("tetherly-loc-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        ensure_dir(&dir).unwrap();

        // 1. Defaults
        let def = load_locator_policy(&dir);
        assert!(def.enabled);
        assert!(def.allow_override_silent);
        assert_eq!(def.max_duration_secs, 60);

        // 2. Custom policy persistence
        let mut custom = LocatorPolicy {
            enabled: true,
            allow_override_silent: false,
            max_duration_secs: 30,
            allowed_groups: vec!["family".into()],
            allowed_tags: vec!["phone".into()],
            allowed_types: vec![DeviceType::Phone],
        };
        custom.sanitize();
        save_locator_policy(&dir, &custom).unwrap();
        let loaded = load_locator_policy(&dir);
        assert_eq!(loaded, custom);

        // 3. Matching
        let prof_ok = PeerProfile {
            device_id: "phone_1".into(),
            device_type: DeviceType::Phone,
            tags: vec![],
            group: None,
            notes: None,
            updated_ms: 0,
        };
        assert!(loaded.is_peer_eligible(&prof_ok));

        let prof_group = PeerProfile {
            device_id: "pc_1".into(),
            device_type: DeviceType::Desktop,
            tags: vec![],
            group: Some("family".into()),
            notes: None,
            updated_ms: 0,
        };
        assert!(loaded.is_peer_eligible(&prof_group));

        let prof_unmatched = PeerProfile {
            device_id: "pc_2".into(),
            device_type: DeviceType::Desktop,
            tags: vec!["work".into()],
            group: Some("office".into()),
            notes: None,
            updated_ms: 0,
        };
        assert!(!loaded.is_peer_eligible(&prof_unmatched));

        // 4. Disabled policy blocks all
        let disabled = LocatorPolicy {
            enabled: false,
            ..Default::default()
        };
        assert!(!disabled.is_peer_eligible(&prof_ok));

        // 5. Sanitize duration
        let mut invalid = LocatorPolicy {
            max_duration_secs: 120,
            ..Default::default()
        };
        invalid.sanitize();
        assert_eq!(invalid.max_duration_secs, 60);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn local_actions_and_policy_roundtrip_and_matching() {
        let dir = std::env::temp_dir().join(format!("tetherly-act-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        ensure_dir(&dir).unwrap();

        // 1. Defaults
        let def_actions = load_local_actions(&dir);
        assert!(def_actions.is_empty());

        let def_pol = load_action_policy(&dir);
        assert_eq!(def_pol.mode, ActionAccessMode::Allow);
        assert_eq!(def_pol.max_timeout_secs, 10);
        assert!(def_pol.expose_action_list);

        // 2. Add local actions and save
        let mut actions = vec![
            LocalActionDefinition {
                id: "lock_screen".into(),
                name: "Lock Screen".into(),
                description: Some("Locks the desktop session".into()),
                icon: Some("lock".into()),
                command: "loginctl".into(),
                args: vec!["lock-session".into()],
                timeout_secs: 5,
                enabled: true,
            },
            LocalActionDefinition {
                id: "sleep_display".into(),
                name: "Sleep Display".into(),
                description: None,
                icon: None,
                command: "xset".into(),
                args: vec!["dpms".into(), "force".into(), "off".into()],
                timeout_secs: 10,
                enabled: false,
            },
        ];
        for a in &mut actions {
            a.sanitize();
        }
        save_local_actions(&dir, &actions).unwrap();

        let loaded_actions = load_local_actions(&dir);
        assert_eq!(loaded_actions.len(), 2);
        assert_eq!(loaded_actions[0].id, "lock_screen");
        assert_eq!(loaded_actions[1].id, "sleep_display");
        assert_eq!(loaded_actions[0].to_item().name, "Lock Screen");

        // 3. Custom policy persistence
        let mut custom_pol = ActionPolicy {
            mode: ActionAccessMode::PromptOnly,
            max_timeout_secs: 20,
            allowed_groups: vec!["admin".into()],
            allowed_tags: vec!["secure".into()],
            allowed_types: vec![DeviceType::Phone],
            expose_action_list: true,
        };
        custom_pol.sanitize();
        save_action_policy(&dir, &custom_pol).unwrap();

        let loaded_pol = load_action_policy(&dir);
        assert_eq!(loaded_pol, custom_pol);

        // 4. Matching
        let prof_ok = PeerProfile {
            device_id: "phone_1".into(),
            device_type: DeviceType::Phone,
            tags: vec![],
            group: None,
            notes: None,
            updated_ms: 0,
        };
        assert!(loaded_pol.is_peer_eligible(&prof_ok));

        let prof_group = PeerProfile {
            device_id: "pc_1".into(),
            device_type: DeviceType::Desktop,
            tags: vec![],
            group: Some("admin".into()),
            notes: None,
            updated_ms: 0,
        };
        assert!(loaded_pol.is_peer_eligible(&prof_group));

        let prof_unmatched = PeerProfile {
            device_id: "pc_2".into(),
            device_type: DeviceType::Desktop,
            tags: vec!["guest".into()],
            group: Some("visitor".into()),
            notes: None,
            updated_ms: 0,
        };
        assert!(!loaded_pol.is_peer_eligible(&prof_unmatched));

        // 5. Disabled mode blocks all
        let disabled_pol = ActionPolicy {
            mode: ActionAccessMode::Disabled,
            ..Default::default()
        };
        assert!(!disabled_pol.is_peer_eligible(&prof_ok));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn focus_policy_roundtrip_and_quiet_hours_calculation() {
        let dir = std::env::temp_dir().join(format!("tetherly-focus-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        ensure_dir(&dir).unwrap();

        // 1. Defaults
        let def = load_focus_policy(&dir);
        assert!(def.enabled);
        assert_eq!(def.default_mode, FocusMode::Dnd);
        assert!(!def.mirror_inbound_focus);
        assert!(def.allow_emergency_breakthrough);
        assert!(!def.quiet_hours_enabled);
        assert_eq!(def.quiet_hours_start_minute, 1380); // 23:00
        assert_eq!(def.quiet_hours_end_minute, 420); // 07:00
        assert!(def.allowed_groups.is_empty());
        assert!(def.allowed_tags.is_empty());
        assert!(def.allowed_types.is_empty());

        // Quiet hours inactive when quiet_hours_enabled == false
        assert!(!def.is_quiet_hours_active(1380));
        assert!(!def.is_quiet_hours_active(0));

        // 2. Custom policy persistence
        let mut custom = FocusPolicy {
            enabled: true,
            default_mode: FocusMode::Meeting,
            mirror_inbound_focus: true,
            allow_emergency_breakthrough: false,
            quiet_hours_enabled: true,
            quiet_hours_start_minute: 1380, // 23:00
            quiet_hours_end_minute: 420,    // 07:00
            allowed_groups: vec!["exec".into()],
            allowed_tags: vec!["vip".into()],
            allowed_types: vec![DeviceType::Phone],
        };
        custom.sanitize();
        save_focus_policy(&dir, &custom).unwrap();
        let loaded = load_focus_policy(&dir);
        assert_eq!(loaded, custom);

        // 3. Overnight quiet hours (23:00 to 07:00)
        assert!(!loaded.is_quiet_hours_active(1379)); // 22:59
        assert!(loaded.is_quiet_hours_active(1380)); // 23:00
        assert!(loaded.is_quiet_hours_active(1410)); // 23:30
        assert!(loaded.is_quiet_hours_active(0)); // 00:00
        assert!(loaded.is_quiet_hours_active(240)); // 04:00
        assert!(loaded.is_quiet_hours_active(419)); // 06:59
        assert!(!loaded.is_quiet_hours_active(420)); // 07:00
        assert!(!loaded.is_quiet_hours_active(720)); // 12:00

        // 4. Daytime quiet hours (09:00 to 17:00: 540 to 1020)
        let daytime = FocusPolicy {
            quiet_hours_enabled: true,
            quiet_hours_start_minute: 540,
            quiet_hours_end_minute: 1020,
            ..Default::default()
        };
        assert!(!daytime.is_quiet_hours_active(539)); // 08:59
        assert!(daytime.is_quiet_hours_active(540)); // 09:00
        assert!(daytime.is_quiet_hours_active(720)); // 12:00
        assert!(daytime.is_quiet_hours_active(1019)); // 16:59
        assert!(!daytime.is_quiet_hours_active(1020)); // 17:00
        assert!(!daytime.is_quiet_hours_active(0));

        // 5. Peer eligibility matching
        let prof_ok = PeerProfile {
            device_id: "phone_1".into(),
            device_type: DeviceType::Phone,
            tags: vec![],
            group: None,
            notes: None,
            updated_ms: 0,
        };
        assert!(loaded.is_peer_eligible(&prof_ok));

        let prof_group = PeerProfile {
            device_id: "pc_1".into(),
            device_type: DeviceType::Desktop,
            tags: vec![],
            group: Some("exec".into()),
            notes: None,
            updated_ms: 0,
        };
        assert!(loaded.is_peer_eligible(&prof_group));

        let prof_tag = PeerProfile {
            device_id: "tab_1".into(),
            device_type: DeviceType::Other,
            tags: vec!["vip".into()],
            group: None,
            notes: None,
            updated_ms: 0,
        };
        assert!(loaded.is_peer_eligible(&prof_tag));

        let prof_unmatched = PeerProfile {
            device_id: "pc_2".into(),
            device_type: DeviceType::Desktop,
            tags: vec!["guest".into()],
            group: Some("visitor".into()),
            notes: None,
            updated_ms: 0,
        };
        assert!(!loaded.is_peer_eligible(&prof_unmatched));

        // Disabled policy blocks all
        let disabled_focus = FocusPolicy {
            enabled: false,
            ..Default::default()
        };
        assert!(!disabled_focus.is_peer_eligible(&prof_ok));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn topology_layout_and_policy_roundtrip_and_matching() {
        let dir = std::env::temp_dir().join(format!("tetherly-topo-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        ensure_dir(&dir).unwrap();

        // 1. Defaults
        let def_layout = load_topology_layout(&dir);
        assert_eq!(def_layout.display.width, 1920);
        assert_eq!(def_layout.display.height, 1080);
        assert!(def_layout.neighbors.is_empty());

        let def_policy = load_topology_policy(&dir);
        assert!(def_policy.enabled);
        assert!(def_policy.allow_inbound_control);
        assert!(def_policy.auto_link_paired_peers);
        assert!(def_policy.allowed_groups.is_empty());

        // 2. Custom layout persistence
        let mut neighbors = HashMap::new();
        neighbors.insert(
            DisplayEdge::Right,
            EdgeNeighbor {
                peer_id: "peer_right".into(),
                peer_edge: DisplayEdge::Left,
                offset: 50,
            },
        );
        let custom_layout = TopologyLayout {
            display: DisplayRect::new(2560, 1440, 1.25),
            neighbors,
            config: EdgeCrossingConfig {
                resistance_px: 20,
                delay_ms: 150,
                corner_lock_size_px: 40,
                wrap_around: true,
            },
        };
        save_topology_layout(&dir, &custom_layout).unwrap();
        let loaded_layout = load_topology_layout(&dir);
        assert_eq!(loaded_layout, custom_layout);

        // 3. Custom policy persistence
        let custom_policy = TopologyPolicy {
            enabled: true,
            allow_inbound_control: false,
            auto_link_paired_peers: false,
            allowed_groups: vec!["office".into()],
            allowed_tags: vec!["laptop".into()],
            allowed_types: vec![DeviceType::Desktop, DeviceType::Laptop],
        };
        save_topology_policy(&dir, &custom_policy).unwrap();
        let loaded_policy = load_topology_policy(&dir);
        assert_eq!(loaded_policy, custom_policy);

        // 4. Policy matching
        let prof_ok = PeerProfile {
            device_id: "lap_1".into(),
            device_type: DeviceType::Laptop,
            tags: vec![],
            group: None,
            notes: None,
            updated_ms: 0,
        };
        assert!(loaded_policy.is_peer_eligible(&prof_ok));

        let prof_group = PeerProfile {
            device_id: "desk_1".into(),
            device_type: DeviceType::Other,
            tags: vec![],
            group: Some("office".into()),
            notes: None,
            updated_ms: 0,
        };
        assert!(loaded_policy.is_peer_eligible(&prof_group));

        let prof_tag = PeerProfile {
            device_id: "other_1".into(),
            device_type: DeviceType::Other,
            tags: vec!["laptop".into()],
            group: None,
            notes: None,
            updated_ms: 0,
        };
        assert!(loaded_policy.is_peer_eligible(&prof_tag));

        let prof_unmatched = PeerProfile {
            device_id: "phone_1".into(),
            device_type: DeviceType::Phone,
            tags: vec!["mobile".into()],
            group: Some("home".into()),
            notes: None,
            updated_ms: 0,
        };
        assert!(!loaded_policy.is_peer_eligible(&prof_unmatched));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn input_tunnel_policy_roundtrip_and_matching() {
        let dir = std::env::temp_dir().join(format!("tetherly-inp-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        ensure_dir(&dir).unwrap();

        // 1. Defaults
        let def = load_input_policy(&dir);
        assert!(def.enabled);
        assert!(def.allow_inbound_control);
        assert!(def.allow_keyboard);
        assert!(def.allow_mouse);
        assert!(def.hotkey_release_enabled);
        assert_eq!(def.idle_timeout_secs, 300);

        // 2. Custom policy persistence
        let mut custom = InputTunnelPolicy {
            enabled: true,
            allow_inbound_control: true,
            allow_keyboard: false,
            allow_mouse: true,
            hotkey_release_enabled: true,
            idle_timeout_secs: 180,
            allowed_groups: vec!["studio".into()],
            allowed_tags: vec!["peripherals".into()],
            allowed_types: vec![DeviceType::Desktop, DeviceType::Laptop],
        };
        custom.sanitize();
        save_input_policy(&dir, &custom).unwrap();
        let loaded = load_input_policy(&dir);
        assert_eq!(loaded, custom);

        // 3. Matching
        let prof_ok = PeerProfile {
            device_id: "desk_1".into(),
            device_type: DeviceType::Desktop,
            tags: vec![],
            group: None,
            notes: None,
            updated_ms: 0,
        };
        assert!(loaded.is_peer_eligible(&prof_ok));

        let prof_group = PeerProfile {
            device_id: "tab_1".into(),
            device_type: DeviceType::Other,
            tags: vec![],
            group: Some("studio".into()),
            notes: None,
            updated_ms: 0,
        };
        assert!(loaded.is_peer_eligible(&prof_group));

        let prof_tag = PeerProfile {
            device_id: "phone_1".into(),
            device_type: DeviceType::Phone,
            tags: vec!["peripherals".into()],
            group: None,
            notes: None,
            updated_ms: 0,
        };
        assert!(loaded.is_peer_eligible(&prof_tag));

        let prof_unmatched = PeerProfile {
            device_id: "tv_1".into(),
            device_type: DeviceType::Other,
            tags: vec!["entertainment".into()],
            group: Some("living_room".into()),
            notes: None,
            updated_ms: 0,
        };
        assert!(!loaded.is_peer_eligible(&prof_unmatched));

        // 4. Inbound disabled blocks all
        let disabled_inbound = InputTunnelPolicy {
            allow_inbound_control: false,
            ..Default::default()
        };
        assert!(!disabled_inbound.is_peer_eligible(&prof_ok));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn virtual_display_policy_roundtrip_and_matching() {
        let dir = std::env::temp_dir().join(format!("tetherly-vdisp-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        ensure_dir(&dir).unwrap();

        // 1. Defaults
        let def = load_virtual_display_policy(&dir);
        assert!(def.enabled);
        assert!(def.allow_hosting);
        assert!(def.allow_remote_extend);
        assert_eq!(def.max_resolution_width, 3840);
        assert_eq!(def.max_resolution_height, 2160);
        assert_eq!(def.max_fps, 60);
        assert_eq!(def.idle_timeout_secs, 300);

        // 2. Custom policy persistence
        let mut custom = VirtualDisplayPolicy {
            enabled: true,
            allow_hosting: true,
            allow_remote_extend: true,
            max_resolution_width: 2560,
            max_resolution_height: 1440,
            max_fps: 60,
            idle_timeout_secs: 180,
            allowed_groups: vec!["displays".into()],
            allowed_tags: vec!["4k".into()],
            allowed_types: vec![DeviceType::Desktop, DeviceType::Laptop],
        };
        custom.sanitize();
        save_virtual_display_policy(&dir, &custom).unwrap();
        let loaded = load_virtual_display_policy(&dir);
        assert_eq!(loaded, custom);

        // 3. Matching
        let prof_ok = PeerProfile {
            device_id: "tab_1".into(),
            device_type: DeviceType::Desktop,
            tags: vec![],
            group: None,
            notes: None,
            updated_ms: 0,
        };
        assert!(loaded.is_peer_eligible(&prof_ok, true));
        assert!(loaded.is_peer_eligible(&prof_ok, false));

        let prof_group = PeerProfile {
            device_id: "phone_1".into(),
            device_type: DeviceType::Phone,
            tags: vec![],
            group: Some("displays".into()),
            notes: None,
            updated_ms: 0,
        };
        assert!(loaded.is_peer_eligible(&prof_group, true));

        let prof_tag = PeerProfile {
            device_id: "monitor_1".into(),
            device_type: DeviceType::Other,
            tags: vec!["4k".into()],
            group: None,
            notes: None,
            updated_ms: 0,
        };
        assert!(loaded.is_peer_eligible(&prof_tag, true));

        let prof_unmatched = PeerProfile {
            device_id: "iot_1".into(),
            device_type: DeviceType::Other,
            tags: vec!["guest".into()],
            group: Some("visitor".into()),
            notes: None,
            updated_ms: 0,
        };
        assert!(!loaded.is_peer_eligible(&prof_unmatched, true));

        // 4. Hosting / Extend toggles
        let hosting_disabled = VirtualDisplayPolicy {
            allow_hosting: false,
            ..Default::default()
        };
        assert!(!hosting_disabled.is_peer_eligible(&prof_ok, true));
        assert!(hosting_disabled.is_peer_eligible(&prof_ok, false));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn audio_routing_policy_roundtrip_and_matching() {
        let dir = std::env::temp_dir().join(format!("tetherly-audio-pol-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        ensure_dir(&dir).unwrap();

        // 1. Defaults
        let def = load_audio_routing_policy(&dir);
        assert!(def.enabled);
        assert!(def.allow_inbound_sink);
        assert!(def.allow_outbound_routing);
        assert_eq!(def.max_volume_limit, 100);
        assert!(def.allow_spatial_rendering);
        assert_eq!(def.idle_timeout_secs, 300);

        // 2. Custom policy persistence
        let mut custom = AudioRoutingPolicy {
            enabled: true,
            allow_inbound_sink: true,
            allow_outbound_routing: true,
            max_volume_limit: 85,
            allow_spatial_rendering: true,
            idle_timeout_secs: 180,
            allowed_groups: vec!["sound".into()],
            allowed_tags: vec!["surround".into()],
            allowed_types: vec![DeviceType::Tablet, DeviceType::Desktop],
        };
        custom.sanitize();
        save_audio_routing_policy(&dir, &custom).unwrap();
        let loaded = load_audio_routing_policy(&dir);
        assert_eq!(loaded.max_volume_limit, 85);
        assert_eq!(loaded.idle_timeout_secs, 180);

        // 3. Matching
        let prof_ok = PeerProfile {
            device_id: "spk_1".into(),
            device_type: DeviceType::Tablet,
            tags: vec![],
            group: None,
            notes: None,
            updated_ms: 0,
        };
        assert!(loaded.is_peer_eligible(&prof_ok, true));
        assert!(loaded.is_peer_eligible(&prof_ok, false));

        let prof_group = PeerProfile {
            device_id: "box_1".into(),
            device_type: DeviceType::Other,
            tags: vec![],
            group: Some("sound".into()),
            notes: None,
            updated_ms: 0,
        };
        assert!(loaded.is_peer_eligible(&prof_group, true));

        let prof_tag = PeerProfile {
            device_id: "theater_1".into(),
            device_type: DeviceType::Other,
            tags: vec!["surround".into()],
            group: None,
            notes: None,
            updated_ms: 0,
        };
        assert!(loaded.is_peer_eligible(&prof_tag, true));

        let prof_unmatched = PeerProfile {
            device_id: "iot_1".into(),
            device_type: DeviceType::Other,
            tags: vec!["guest".into()],
            group: Some("visitor".into()),
            notes: None,
            updated_ms: 0,
        };
        assert!(!loaded.is_peer_eligible(&prof_unmatched, true));

        // 4. Inbound Sink / Outbound Routing toggles
        let sink_disabled = AudioRoutingPolicy {
            allow_inbound_sink: false,
            ..Default::default()
        };
        assert!(!sink_disabled.is_peer_eligible(&prof_ok, true));
        assert!(sink_disabled.is_peer_eligible(&prof_ok, false));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn camera_policy_roundtrip_and_matching() {
        let dir = std::env::temp_dir().join(format!("tetherly-cam-pol-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        ensure_dir(&dir).unwrap();

        // 1. Defaults
        let def = load_camera_policy(&dir);
        assert!(def.enabled);
        assert!(def.allow_inbound_stream);
        assert!(def.allow_remote_capture);
        assert_eq!(def.max_resolution_width, 3840);
        assert_eq!(def.max_resolution_height, 2160);
        assert_eq!(def.max_fps, 60);
        assert!(def.enforce_privacy_shutter_indicator);
        assert_eq!(def.idle_timeout_secs, 300);

        // 2. Custom policy persistence
        let mut custom = CameraPolicy {
            enabled: true,
            allow_inbound_stream: true,
            allow_remote_capture: true,
            max_resolution_width: 1920,
            max_resolution_height: 1080,
            max_fps: 30,
            enforce_privacy_shutter_indicator: true,
            idle_timeout_secs: 120,
            allowed_groups: vec!["studio".into()],
            allowed_tags: vec!["webcam".into()],
            allowed_types: vec![DeviceType::Phone, DeviceType::Tablet],
        };
        custom.sanitize();
        save_camera_policy(&dir, &custom).unwrap();
        let loaded = load_camera_policy(&dir);
        assert_eq!(loaded.max_resolution_width, 1920);
        assert_eq!(loaded.max_fps, 30);
        assert_eq!(loaded.idle_timeout_secs, 120);

        // 3. Matching
        let prof_ok = PeerProfile {
            device_id: "phone_cam".into(),
            device_type: DeviceType::Phone,
            tags: vec![],
            group: None,
            notes: None,
            updated_ms: 0,
        };
        assert!(loaded.is_peer_eligible(&prof_ok, true));
        assert!(loaded.is_peer_eligible(&prof_ok, false));

        let prof_group = PeerProfile {
            device_id: "cam_grp".into(),
            device_type: DeviceType::Other,
            tags: vec![],
            group: Some("studio".into()),
            notes: None,
            updated_ms: 0,
        };
        assert!(loaded.is_peer_eligible(&prof_group, true));

        let prof_tag = PeerProfile {
            device_id: "cam_tag".into(),
            device_type: DeviceType::Other,
            tags: vec!["webcam".into()],
            group: None,
            notes: None,
            updated_ms: 0,
        };
        assert!(loaded.is_peer_eligible(&prof_tag, true));

        let prof_unmatched = PeerProfile {
            device_id: "iot_unmatched".into(),
            device_type: DeviceType::Other,
            tags: vec!["guest".into()],
            group: Some("visitor".into()),
            notes: None,
            updated_ms: 0,
        };
        assert!(!loaded.is_peer_eligible(&prof_unmatched, true));

        // 4. Inbound Stream / Remote Capture toggles
        let stream_disabled = CameraPolicy {
            allow_inbound_stream: false,
            ..Default::default()
        };
        assert!(!stream_disabled.is_peer_eligible(&prof_ok, true));
        assert!(stream_disabled.is_peer_eligible(&prof_ok, false));

        let capture_disabled = CameraPolicy {
            allow_remote_capture: false,
            ..Default::default()
        };
        assert!(capture_disabled.is_peer_eligible(&prof_ok, true));
        assert!(!capture_disabled.is_peer_eligible(&prof_ok, false));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn microphone_policy_roundtrip_and_matching() {
        let dir = std::env::temp_dir().join(format!("tetherly-mic-pol-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        ensure_dir(&dir).unwrap();

        // 1. Defaults
        let def = load_microphone_policy(&dir);
        assert!(def.enabled);
        assert!(def.allow_inbound_stream);
        assert!(def.allow_remote_capture);
        assert!(def.allow_intercom);
        assert_eq!(def.max_sample_rate, 48000);
        assert!(def.enforce_privacy_indicator);
        assert!(!def.default_muted);
        assert_eq!(def.idle_timeout_secs, 300);

        // 2. Custom policy persistence
        let mut custom = MicrophonePolicy {
            enabled: true,
            allow_inbound_stream: true,
            allow_remote_capture: true,
            allow_intercom: true,
            max_sample_rate: 44100,
            max_session_duration_secs: 1800,
            enforce_privacy_indicator: true,
            default_muted: true,
            idle_timeout_secs: 120,
            allowed_groups: vec!["studio".into()],
            allowed_tags: vec!["intercom".into()],
            allowed_types: vec![DeviceType::Phone],
        };
        custom.sanitize();
        save_microphone_policy(&dir, &custom).unwrap();
        let loaded = load_microphone_policy(&dir);
        assert_eq!(loaded.max_sample_rate, 44100);
        assert_eq!(loaded.max_session_duration_secs, 1800);
        assert!(loaded.default_muted);
        assert_eq!(loaded.idle_timeout_secs, 120);

        // 3. Matching
        let prof_ok = PeerProfile {
            device_id: "phone_mic".into(),
            device_type: DeviceType::Phone,
            tags: vec![],
            group: None,
            notes: None,
            updated_ms: 0,
        };
        assert!(loaded.is_peer_eligible(&prof_ok, true, false));
        assert!(loaded.is_peer_eligible(&prof_ok, false, false));
        assert!(loaded.is_peer_eligible(&prof_ok, true, true));

        let prof_group = PeerProfile {
            device_id: "mic_grp".into(),
            device_type: DeviceType::Other,
            tags: vec![],
            group: Some("studio".into()),
            notes: None,
            updated_ms: 0,
        };
        assert!(loaded.is_peer_eligible(&prof_group, true, false));

        let prof_tag = PeerProfile {
            device_id: "mic_tag".into(),
            device_type: DeviceType::Other,
            tags: vec!["intercom".into()],
            group: None,
            notes: None,
            updated_ms: 0,
        };
        assert!(loaded.is_peer_eligible(&prof_tag, false, true));

        let prof_unmatched = PeerProfile {
            device_id: "iot_unmatched".into(),
            device_type: DeviceType::Other,
            tags: vec!["guest".into()],
            group: Some("visitor".into()),
            notes: None,
            updated_ms: 0,
        };
        assert!(!loaded.is_peer_eligible(&prof_unmatched, true, false));

        // 4. Toggle gating
        let stream_disabled = MicrophonePolicy {
            allow_inbound_stream: false,
            ..Default::default()
        };
        assert!(!stream_disabled.is_peer_eligible(&prof_ok, true, false));
        assert!(stream_disabled.is_peer_eligible(&prof_ok, false, false));

        let capture_disabled = MicrophonePolicy {
            allow_remote_capture: false,
            ..Default::default()
        };
        assert!(capture_disabled.is_peer_eligible(&prof_ok, true, false));
        assert!(!capture_disabled.is_peer_eligible(&prof_ok, false, false));

        let intercom_disabled = MicrophonePolicy {
            allow_intercom: false,
            ..Default::default()
        };
        assert!(intercom_disabled.is_peer_eligible(&prof_ok, true, false));
        assert!(!intercom_disabled.is_peer_eligible(&prof_ok, true, true));

        let disabled_all = MicrophonePolicy {
            enabled: false,
            ..Default::default()
        };
        assert!(!disabled_all.is_peer_eligible(&prof_ok, true, false));

        let _ = fs::remove_dir_all(&dir);
    }
}
