// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Identity and trust persistence. Temp file + rename. Unix 0600; Windows
//! lives under %LOCALAPPDATA% (user ACL). Never logs secret bytes.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use tetherly_core::{MemoryTrustStore, TrustFile};
use tetherly_crypto::Identity;
use tracing::info;

pub const IDENTITY_FILE: &str = "identity.bin";
pub const TRUST_FILE: &str = "trusted.json";
pub const ROUTES_FILE: &str = "routes.json";

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
    let tmp = path.with_extension("tmp");
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

pub fn remember_route(dir: &Path, device_id: &str, addr: &str) -> Result<(), NodeStoreError> {
    let mut table = load_routes(dir);
    if let Some(r) = table.routes.iter_mut().find(|r| r.device_id == device_id) {
        r.addr = addr.to_string();
    } else {
        table.routes.push(Route {
            device_id: device_id.to_string(),
            addr: addr.to_string(),
        });
    }
    save_routes(dir, &table)
}

#[derive(Debug, thiserror::Error)]
pub enum NodeStoreError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("identity file corrupt")]
    Corrupt,
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
}
