//! v0.2.89 shared shelf ("共享架"): files dropped on the Transfers page are
//! registered by path (metadata only) and announced to trusted peers; a peer
//! pulls one with [`ShelfMsg::Fetch`] and the owner pushes it through the
//! ordinary file-offer path, after re-checking it did not change.

use super::*;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tetherly_core::shelf::{ShelfEntry, ShelfMsg, ShelfRefusal, SHELF_MAX_ENTRIES};
use tetherly_core::DeviceId;
use tracing::{info, warn};

const SHELF_FILE: &str = "shelf.json";

/// One locally registered file. The path never leaves this machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct LocalShelfItem {
    pub(crate) id: String,
    pub(crate) path: PathBuf,
    pub(crate) name: String,
    pub(crate) size: u64,
    pub(crate) mtime_ms: u64,
    pub(crate) added_ms: u64,
}

#[derive(Debug, Default)]
pub(crate) struct ShelfState {
    file: Option<PathBuf>,
    local: Vec<LocalShelfItem>,
    /// device id string → (peer, announced entries).
    remote: HashMap<String, (DeviceId, Vec<ShelfEntry>)>,
}

fn file_stamp(path: &Path) -> std::io::Result<(u64, u64)> {
    let meta = std::fs::metadata(path)?;
    if !meta.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "不是普通文件（暂不支持文件夹）",
        ));
    }
    let mtime_ms = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    Ok((meta.len(), mtime_ms))
}

/// Locations a shelf entry may never point into. The daemon reads shelf
/// files under its own OS identity (LocalSystem on Windows) while the shelf
/// is open to the Write-tier client token, so the daemon's own state and
/// the OS-protected trees must not become a file-read proxy.
fn denied_roots(data_dir: Option<&Path>) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = data_dir.into_iter().map(Path::to_path_buf).collect();
    #[cfg(windows)]
    {
        for var in [
            "SystemRoot",
            "ProgramData",
            "ProgramFiles",
            "ProgramFiles(x86)",
        ] {
            if let Some(v) = std::env::var_os(var) {
                roots.push(PathBuf::from(v));
            }
        }
    }
    #[cfg(not(windows))]
    {
        for r in [
            "/etc",
            "/private/etc",
            "/var/root",
            "/System",
            "/Library/Keychains",
        ] {
            roots.push(PathBuf::from(r));
        }
    }
    roots
        .into_iter()
        .map(|r| std::fs::canonicalize(&r).unwrap_or(r))
        .collect()
}

fn path_denied(path: &Path, data_dir: Option<&Path>) -> bool {
    let canon = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let lower = canon.to_string_lossy().to_ascii_lowercase();
    if lower.contains("system volume information") || lower.contains("$recycle.bin") {
        return true;
    }
    denied_roots(data_dir).iter().any(|root| {
        if cfg!(windows) {
            let r = root.to_string_lossy().to_ascii_lowercase();
            lower.starts_with(&r)
        } else {
            canon.starts_with(root)
        }
    })
}

impl LocalShelfItem {
    /// Whether the file is still exactly what was registered.
    pub(crate) fn available(&self) -> bool {
        matches!(file_stamp(&self.path), Ok((size, mtime)) if size == self.size && mtime == self.mtime_ms)
    }

    fn entry(&self) -> ShelfEntry {
        ShelfEntry {
            id: self.id.clone(),
            name: self.name.clone(),
            size: self.size,
            mtime_ms: self.mtime_ms,
            available: self.available(),
        }
    }
}

fn new_id() -> String {
    let mut b = [0u8; 8];
    let _ = getrandom::getrandom(&mut b);
    b.iter().map(|x| format!("{x:02x}")).collect()
}

impl ShelfState {
    pub(crate) fn load(data_dir: &Path) -> Self {
        let file = data_dir.join(SHELF_FILE);
        let local = std::fs::read(&file)
            .ok()
            .and_then(|b| serde_json::from_slice::<Vec<LocalShelfItem>>(&b).ok())
            .unwrap_or_default();
        Self {
            file: Some(file),
            local,
            remote: HashMap::new(),
        }
    }

    fn save(&self) {
        let Some(file) = &self.file else { return };
        match serde_json::to_vec_pretty(&self.local) {
            Ok(bytes) => {
                if let Err(e) = crate::store::atomic_write(file, &bytes) {
                    warn!(error = %e, "shelf save failed");
                }
            }
            Err(e) => warn!(error = %e, "shelf encode failed"),
        }
    }

    /// Register `path`; re-adding the same path refreshes its stamp.
    pub(crate) fn add(&mut self, path: &Path, now: u64) -> Result<LocalShelfItem, String> {
        if !path.is_absolute() {
            return Err(format!("{} 不是绝对路径", path.display()));
        }
        let data_dir = self.file.as_deref().and_then(Path::parent);
        if path_denied(path, data_dir) {
            return Err(format!("{} 位于受保护目录，不能共享", path.display()));
        }
        let (size, mtime_ms) = file_stamp(path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::PermissionDenied && cfg!(target_os = "macos") {
                format!(
                    "无法读取 {}：需要在「系统设置 → 隐私与安全性 → 完全磁盘访问权限」中授权 Tetherly",
                    path.display()
                )
            } else {
                format!("无法读取 {}：{e}", path.display())
            }
        })?;
        if size > tetherly_core::FILE_MAX {
            return Err(format!("{} 超过单文件上限", path.display()));
        }
        let raw = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("file.bin");
        let name = tetherly_core::sanitize_file_name(raw).map_err(|e| e.to_string())?;
        self.local.retain(|i| i.path != path);
        if self.local.len() >= SHELF_MAX_ENTRIES {
            return Err(format!("共享架最多 {SHELF_MAX_ENTRIES} 个文件"));
        }
        let item = LocalShelfItem {
            id: new_id(),
            path: path.to_path_buf(),
            name,
            size,
            mtime_ms,
            added_ms: now,
        };
        // Newest first: a truncated catalog keeps the latest additions.
        self.local.insert(0, item.clone());
        self.save();
        Ok(item)
    }

    pub(crate) fn remove(&mut self, id: &str) -> bool {
        let before = self.local.len();
        self.local.retain(|i| i.id != id);
        let removed = self.local.len() != before;
        if removed {
            self.save();
        }
        removed
    }

    pub(crate) fn local_item(&self, id: &str) -> Option<LocalShelfItem> {
        self.local.iter().find(|i| i.id == id).cloned()
    }

    pub(crate) fn local_items(&self) -> Vec<LocalShelfItem> {
        self.local.clone()
    }

    pub(crate) fn set_remote(&mut self, peer: &DeviceId, entries: Vec<ShelfEntry>) {
        if entries.is_empty() {
            self.remote.remove(peer.as_str());
        } else {
            self.remote
                .insert(peer.as_str().to_string(), (peer.clone(), entries));
        }
    }

    pub(crate) fn drop_remote(&mut self, peer: &DeviceId) -> bool {
        self.remote.remove(peer.as_str()).is_some()
    }

    pub(crate) fn remote_entry(&self, peer: &DeviceId, id: &str) -> Option<ShelfEntry> {
        self.remote
            .get(peer.as_str())
            .and_then(|(_, es)| es.iter().find(|e| e.id == id).cloned())
    }
}

fn api_item(e: &ShelfEntry) -> tetherly_api::ShelfItem {
    tetherly_api::ShelfItem {
        id: e.id.clone(),
        name: e.name.clone(),
        size: e.size,
        mtime_ms: e.mtime_ms,
        available: e.available,
    }
}

impl Node {
    fn shelf_catalog(&self) -> Vec<ShelfEntry> {
        let items = self.shared.shelf.lock().expect("shelf").local_items();
        items.iter().map(LocalShelfItem::entry).collect()
    }

    /// `GET /api/v1/shelf`.
    pub fn shelf_view(&self) -> tetherly_api::Shelf {
        let mine = self.shelf_catalog().iter().map(api_item).collect();
        let remote: Vec<(DeviceId, Vec<ShelfEntry>)> = self
            .shared
            .shelf
            .lock()
            .expect("shelf")
            .remote
            .values()
            .cloned()
            .collect();
        let mut peers: Vec<tetherly_api::ShelfPeer> = remote
            .into_iter()
            .filter(|(peer, _)| self.is_trusted(peer))
            .map(|(peer, entries)| tetherly_api::ShelfPeer {
                device_id: peer.as_str().to_string(),
                name: self.drag_peer_label_pub(&peer),
                items: entries.iter().map(api_item).collect(),
            })
            .collect();
        peers.sort_by(|a, b| a.name.cmp(&b.name));
        tetherly_api::Shelf { mine, peers }
    }

    /// `POST /api/v1/shelf`.
    pub async fn shelf_add(&self, paths: Vec<String>) -> tetherly_api::ShelfAddReport {
        let now = self.now_ms();
        let node = self.clone_handle();
        let (added, errors) = tokio::task::spawn_blocking(move || {
            let mut added = Vec::new();
            let mut errors = Vec::new();
            let mut st = node.shared.shelf.lock().expect("shelf");
            for p in paths {
                match st.add(Path::new(&p), now) {
                    Ok(item) => added.push(api_item(&item.entry())),
                    Err(e) => errors.push(e),
                }
            }
            (added, errors)
        })
        .await
        .unwrap_or_else(|e| (Vec::new(), vec![e.to_string()]));
        if !added.is_empty() {
            self.shelf_changed(None);
            self.broadcast_shelf().await;
        }
        tetherly_api::ShelfAddReport { added, errors }
    }

    /// `DELETE /api/v1/shelf/{id}`.
    pub async fn shelf_remove(&self, id: &str) -> bool {
        let removed = self.shared.shelf.lock().expect("shelf").remove(id);
        if removed {
            self.shelf_changed(None);
            self.broadcast_shelf().await;
        }
        removed
    }

    /// `POST /api/v1/shelf/{owner}/{id}/fetch`: ask the owner for one file.
    pub async fn shelf_fetch(&self, owner: &DeviceId, id: &str) -> Result<(), CoreError> {
        if !self.is_trusted(owner) {
            return Err(CoreError::PeerBlocked(owner.as_str().to_string()));
        }
        let entry = self
            .shared
            .shelf
            .lock()
            .expect("shelf")
            .remote_entry(owner, id)
            .ok_or(CoreError::UnknownTransfer)?;
        self.expect_offer(owner, &entry.name, entry.size);
        let frame = ShelfMsg::Fetch { id: id.to_string() }.to_frame()?;
        self.send_to(owner, frame).await
    }

    fn shelf_changed(&self, peer: Option<&DeviceId>) {
        self.emit(UiEvent::ShelfChanged {
            device_id: peer.map(|p| p.as_str().to_string()),
        });
    }

    /// Catalog frame for a freshly attached session (None when untrusted).
    pub(crate) fn shelf_catalog_frame(&self, peer: &DeviceId) -> Option<tetherly_core::InnerFrame> {
        if !self.is_trusted(peer) {
            return None;
        }
        (ShelfMsg::Catalog {
            entries: self.shelf_catalog(),
        })
        .to_frame()
        .ok()
    }

    /// Announce the local catalog to one trusted peer.
    pub(crate) async fn send_shelf_catalog(&self, peer: &DeviceId) {
        if !self.is_trusted(peer) {
            return;
        }
        let entries = self.shelf_catalog();
        let frame = match (ShelfMsg::Catalog { entries }).to_frame() {
            Ok(f) => f,
            Err(e) => {
                warn!(error = %e, "shelf catalog encode failed");
                return;
            }
        };
        if let Err(e) = self.send_to(peer, frame).await {
            tracing::debug!(peer = %peer, error = %e, "shelf catalog send failed");
        }
    }

    /// Announce the local catalog to every connected trusted peer.
    pub(crate) async fn broadcast_shelf(&self) {
        let peers: Vec<DeviceId> = self
            .live_peers()
            .into_iter()
            .map(|p| p.device_id)
            .filter(|p| self.is_trusted(p))
            .collect();
        for p in peers {
            self.send_shelf_catalog(&p).await;
        }
    }

    /// A peer went away: its catalog is no longer reachable.
    pub(crate) fn shelf_peer_down(&self, peer: &DeviceId) {
        if self.shared.shelf.lock().expect("shelf").drop_remote(peer) {
            self.shelf_changed(Some(peer));
        }
    }

    /// Inbound [`ShelfMsg`] dispatch.
    pub(crate) async fn receive_shelf(&self, peer: &DeviceId, msg: ShelfMsg) {
        match msg {
            ShelfMsg::Catalog { entries } => {
                if !self.is_trusted(peer) {
                    return;
                }
                self.shared
                    .shelf
                    .lock()
                    .expect("shelf")
                    .set_remote(peer, entries);
                self.shelf_changed(Some(peer));
            }
            ShelfMsg::Fetch { id } => {
                if let Err(reason) = self.serve_shelf_fetch(peer, &id).await {
                    warn!(peer = %peer, id = %id, ?reason, "shelf fetch refused");
                    if let Ok(f) = (ShelfMsg::FetchRefused { id, reason }).to_frame() {
                        let _ = self.send_to(peer, f).await;
                    }
                }
            }
            ShelfMsg::FetchRefused { id, reason } => {
                warn!(peer = %peer, id = %id, ?reason, "peer refused shelf fetch");
                let why = match reason {
                    ShelfRefusal::NotFound => "文件已从对方共享架移除",
                    ShelfRefusal::Changed => "对方的文件已修改或删除",
                    ShelfRefusal::Untrusted => "对方未信任本机",
                    ShelfRefusal::Unreadable => "对方无法读取该文件",
                };
                self.emit(UiEvent::Status {
                    message: format!("共享架下载失败：{why}"),
                });
                // Its availability probably changed: ask for a fresh view.
                self.shelf_changed(Some(peer));
            }
        }
    }

    async fn serve_shelf_fetch(&self, peer: &DeviceId, id: &str) -> Result<(), ShelfRefusal> {
        if !self.is_trusted(peer) {
            return Err(ShelfRefusal::Untrusted);
        }
        let item = self
            .shared
            .shelf
            .lock()
            .expect("shelf")
            .local_item(id)
            .ok_or(ShelfRefusal::NotFound)?;
        if !item.available() {
            // Tell everyone the entry went stale.
            self.shelf_changed(None);
            self.broadcast_shelf().await;
            return Err(ShelfRefusal::Changed);
        }
        let staging = self.send_staging_dir();
        let now = self.now_ms();
        let src = item.path.clone();
        let (staged, name) = tokio::task::spawn_blocking(move || {
            super::drag_handoff::stage_user_file(&staging, &src, now)
        })
        .await
        .map_err(|_| ShelfRefusal::Unreadable)?
        .map_err(|e| {
            warn!(error = %e, "shelf stage failed");
            ShelfRefusal::Unreadable
        })?;
        info!(peer = %peer, file = %name, "shelf: serving fetch");
        let node = self.clone_handle();
        let peer = peer.clone();
        // The transfer runs for as long as the file takes; do not hold the
        // session dispatch loop.
        tokio::spawn(async move {
            // Only the offer is sent here; the bytes are read when the peer
            // accepts. The staged copy is aged out by the stage sweep.
            if let Err(e) = node.offer_and_send_file(&peer, &staged, Some(&name)).await {
                warn!(peer = %peer, error = %e, "shelf send failed");
            }
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("tetherly-shelf-{tag}-{}", new_id()));
        std::fs::create_dir_all(d.join("data")).unwrap();
        d
    }

    #[test]
    fn shelf_registers_persists_and_marks_changed_files_unavailable() {
        let dir = tmp("persist");
        let f = dir.join("files").join("a.txt");
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(&f, b"one").unwrap();
        let mut st = ShelfState::load(&dir.join("data"));
        let item = st.add(&f, 10).unwrap();
        assert_eq!(item.name, "a.txt");
        assert_eq!(item.size, 3);
        assert!(item.available());

        let reloaded = ShelfState::load(&dir.join("data"));
        assert_eq!(reloaded.local_items(), vec![item.clone()]);

        std::fs::write(&f, b"changed!").unwrap();
        assert!(!reloaded.local_item(&item.id).unwrap().available());
        std::fs::remove_file(&f).unwrap();
        assert!(!reloaded.local_item(&item.id).unwrap().entry().available);

        assert!(st.add(Path::new("relative.txt"), 0).is_err());
        std::fs::create_dir_all(dir.join("data")).unwrap();
        let secret = dir.join("data").join("api.token");
        std::fs::write(&secret, b"t").unwrap();
        assert!(
            st.add(&secret, 0).is_err(),
            "the daemon's own data dir is never shareable"
        );
        assert!(
            st.add(&dir.join("files"), 0).is_err(),
            "directories are refused"
        );
        assert!(st.remove(&item.id));
        assert!(ShelfState::load(&dir.join("data")).local_items().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn re_adding_a_path_replaces_its_entry() {
        let dir = tmp("readd");
        let f = dir.join("files").join("b.bin");
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(&f, b"x").unwrap();
        let mut st = ShelfState::load(&dir.join("data"));
        st.add(&f, 1).unwrap();
        std::fs::write(&f, b"xy").unwrap();
        let second = st.add(&f, 2).unwrap();
        assert_eq!(st.local_items().len(), 1);
        assert_eq!(st.local_items()[0].size, 2);
        assert!(second.available());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn catalog_from_untrusted_peer_is_ignored() {
        let (node, _dir) =
            crate::runtime::clipboard_policy_tests::temp_test_node("shelf-untrusted");
        let peer = DeviceId::from_id_pk(&[9u8; 32]);
        node.receive_shelf(
            &peer,
            ShelfMsg::Catalog {
                entries: vec![ShelfEntry {
                    id: "1".into(),
                    name: "x".into(),
                    size: 1,
                    mtime_ms: 0,
                    available: true,
                }],
            },
        )
        .await;
        assert!(node.shelf_view().peers.is_empty());
        assert!(node.shelf_fetch(&peer, "1").await.is_err());
    }
}
