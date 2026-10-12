// SPDX-License-Identifier: Apache-2.0 OR MIT
//! App-private persistence for the Android client: `identity.bin` (64 raw
//! secret bytes) and `trusted.json` in the same `TrustFile` JSON shape the
//! desktop node writes, so a phone-paired trust file is interchangeable.

use std::fs;
use std::io::Write;
use std::path::Path;

use tetherly_core::{MemoryTrustStore, TrustFile};
use tetherly_crypto::Identity;

pub const IDENTITY_FILE: &str = "identity.bin";
pub const TRUST_FILE: &str = "trusted.json";

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("corrupt identity")]
    Corrupt,
    #[error("rng unavailable")]
    Rng,
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

fn ensure_dir(dir: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dir)?;
    restrict(dir, 0o700);
    Ok(())
}

#[cfg(unix)]
fn restrict(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(meta) = fs::metadata(path) {
        let mut p = meta.permissions();
        p.set_mode(mode);
        let _ = fs::set_permissions(path, p);
    }
}

#[cfg(not(unix))]
fn restrict(_path: &Path, _mode: u32) {}

/// Temp file + rename, then narrow permissions. Never logs secret bytes.
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
    restrict(path, 0o600);
    Ok(())
}

pub fn load_or_create_identity(dir: &Path) -> Result<Identity, StoreError> {
    ensure_dir(dir)?;
    let path = dir.join(IDENTITY_FILE);
    if path.exists() {
        let bytes = fs::read(&path)?;
        return Identity::from_bytes(&bytes).map_err(|_| StoreError::Corrupt);
    }
    let id = Identity::generate().map_err(|_| StoreError::Rng)?;
    atomic_write(&path, &id.to_bytes())?;
    Ok(id)
}

/// Missing or unreadable trust is an empty store, not an error: the phone then
/// re-pairs, which is the honest behaviour for a first run or a wiped file.
pub fn load_trust(dir: &Path) -> MemoryTrustStore {
    let mut store = MemoryTrustStore::default();
    if let Ok(bytes) = fs::read(dir.join(TRUST_FILE)) {
        if let Ok(file) = serde_json::from_slice::<TrustFile>(&bytes) {
            if let Ok(peers) = file.into_peers() {
                store.load(peers);
            }
        }
    }
    store
}

pub fn save_trust(dir: &Path, store: &MemoryTrustStore) -> Result<(), StoreError> {
    let file = TrustFile::from_peers(&store.all());
    let bytes = serde_json::to_vec_pretty(&file)?;
    atomic_write(&dir.join(TRUST_FILE), &bytes)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tetherly_core::TrustStore;

    #[test]
    fn identity_roundtrip_and_trust_file() {
        let dir =
            std::env::temp_dir().join(format!("tetherly-android-store-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let a = load_or_create_identity(&dir).unwrap();
        let b = load_or_create_identity(&dir).unwrap();
        assert_eq!(a.device_id(), b.device_id(), "identity persists");

        let mut store = MemoryTrustStore::default();
        store
            .put(tetherly_core::TrustedPeer {
                device_id: a.device_id().clone(),
                id_pk: *a.id_pk(),
                n_pk: *a.n_pk(),
                alias: "phone".into(),
                paired_at: 7,
                revoked: false,
            })
            .unwrap();
        save_trust(&dir, &store).unwrap();
        let back = load_trust(&dir);
        assert_eq!(back.all().len(), 1);
        assert_eq!(back.all()[0].n_pk, *a.n_pk());
        let _ = fs::remove_dir_all(&dir);
    }
}
