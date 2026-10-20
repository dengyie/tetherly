// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Identity and trust persistence. Temp file + rename. Unix 0600; Windows
//! lives under %LOCALAPPDATA% (user ACL). Never logs secret bytes.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use tetherly_core::{ClipSyncMode, MemoryTrustStore, TrustFile};
use tetherly_crypto::Identity;
use tracing::info;

pub const IDENTITY_FILE: &str = "identity.bin";
pub const TRUST_FILE: &str = "trusted.json";
pub const ROUTES_FILE: &str = "routes.json";
pub const BLOCKED_FILE: &str = "blocked.json";
pub const PROFILES_FILE: &str = "profiles.json";

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

pub const CLIPBOARD_POLICY_FILE: &str = "clipboard_policy.json";
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
}
