// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Identity storage backed by the OS credential vault, with the on-disk file
//! as a portable fallback.
//!
//! The vault keeps the 64 secret bytes (Ed25519 + X25519 private material) out
//! of a world-readable file and under the OS's per-user protection:
//!
//! - **Windows**: Credential Manager generic credential (`CredWriteW`/…).
//! - **macOS**: Keychain generic password (`security-framework`).
//! - **Linux/BSD**: Secret Service / libsecret (`secret-service`, D-Bus).
//!
//! The data dir keeps only `identity.ref` — a small *public* pointer holding
//! the device id, never key material. When the vault is unavailable (headless
//! Linux with no Secret Service, a locked keyring, a build without the target),
//! or any vault call fails, we fall back to the original `identity.bin` (0600)
//! path and keep using it, so startup never depends on the vault.
//!
//! The vault is a per-user, machine-global store, so it is only used for the
//! **installed** data dir; a temp-dir node (every integration test) keeps its
//! identity in that dir's `identity.bin` and never touches the operator's real
//! vault. `TETHERLY_VAULT=1`/`0` overrides the decision.
//!
//! Secret bytes are never logged. The vault target name and the `.ref` file are
//! public metadata (a fixed service string + the device id), so diagnostics
//! stay meaningful without exposing key material.

use std::path::Path;
use tetherly_crypto::Identity;

use crate::store::NodeStoreError;

/// Public pointer file next to (or instead of) `identity.bin`, holding the
/// device id whose secret lives in the vault.
pub const REF_FILE: &str = "identity.ref";
/// Service prefix for the vault entry: `tetherly.identity:<device_id>`.
pub const CRED_PREFIX: &str = "tetherly.identity";

// One backend per platform. Each module is self-contained (it imports only
// absolute paths) so it can be compile-checked for its own target even from a
// host where the rest of the node crate cannot build (see the CI note in the
// changelog / the `cred_probe` dev check).
#[cfg(windows)]
#[path = "cred/windows.rs"]
mod backend;
#[cfg(target_os = "macos")]
#[path = "cred/macos.rs"]
mod backend;
#[cfg(all(unix, not(target_os = "macos")))]
#[path = "cred/linux.rs"]
mod backend;
#[cfg(not(any(windows, unix)))]
#[path = "cred/other.rs"]
mod backend;

/// Whether the OS vault is usable on this platform/build.
pub fn vault_available() -> bool {
    backend::available()
}

/// Load the identity for `dir`, creating one on first use.
///
/// The vault is authoritative when available: resolve the device id from
/// `identity.ref` (or an existing `identity.bin`, which we then migrate), read
/// the secret from the vault, and keep only the `.ref` pointer on disk. Any
/// failure falls back to — and keeps using — the file-backed identity.
pub fn load_or_create_identity(dir: &Path) -> Result<Identity, NodeStoreError> {
    if vault_enabled_for(dir) {
        // A `.ref` pointer means an identity was already migrated into the
        // vault. If the vault read then fails (locked keyring, denied prompt,
        // D-Bus hiccup, corrupt blob), we must NOT fall through to
        // `load_or_create_identity` — with `identity.bin` already gone that
        // would mint a brand-new identity and overwrite the vault entry,
        // silently rotating the device id and destroying every trust/pairing.
        if let Some(id) = load_from_vault(dir) {
            return Ok(id);
        }
        if dir.join(REF_FILE).exists() {
            return Err(NodeStoreError::VaultUnavailable);
        }
    }
    let id = crate::store::load_or_create_identity(dir)?;
    // Migrate: when a vault became available after a file-backed run, move the
    // secret into the vault and drop the on-disk secret, keeping only the ref.
    // Delete `identity.bin` only once the pointer is durably on disk, so a
    // crashed/failed migration never leaves the secret unreachable.
    if vault_enabled_for(dir)
        && vault_store(&id)
        && write_ref(dir, &id.device_id().to_string()).is_ok()
    {
        let _ = std::fs::remove_file(dir.join(crate::store::IDENTITY_FILE));
    }
    Ok(id)
}

/// Resolve the device id to look up: prefer the public `.ref` pointer, else the
/// device id inside an existing `identity.bin` (first-run migration).
fn known_device_id(dir: &Path) -> Option<String> {
    if let Ok(s) = std::fs::read_to_string(dir.join(REF_FILE)) {
        let s = s.trim().to_string();
        if !s.is_empty() {
            return Some(s);
        }
    }
    let bytes = std::fs::read(dir.join(crate::store::IDENTITY_FILE)).ok()?;
    Identity::from_bytes(&bytes)
        .ok()
        .map(|i| i.device_id().to_string())
}

/// Read the identity from the vault for this dir. `None` when there is no
/// pointer/no usable entry (which selects the file path).
fn load_from_vault(dir: &Path) -> Option<Identity> {
    let device_id = known_device_id(dir)?;
    let bytes = backend::read(&target_for(&device_id))?;
    match Identity::from_bytes(&bytes) {
        Ok(id) => {
            tracing::info!(device_id = %id.device_id(), "loaded identity from credential vault");
            Some(id)
        }
        Err(_) => None,
    }
}

/// Whether `dir` should use the vault. The vault is a per-user, machine-global
/// store, so it is only appropriate for the **installed** data dir; a
/// temp-dir node (every integration test) keeps its identity in that dir's
/// `identity.bin` and never touches the operator's real vault. An explicit
/// `TETHERLY_VAULT=1`/`0` overrides the decision (used by tests).
fn vault_enabled_for(dir: &Path) -> bool {
    if !vault_available() {
        return false;
    }
    match std::env::var("TETHERLY_VAULT").ok().as_deref() {
        Some("1") => return true,
        Some("0") => return false,
        _ => {}
    }
    dir == crate::store::default_data_dir()
}

/// Persist an identity's secrets to the vault. Returns false when unsupported
/// or on any vault error; callers keep the file fallback in that case.
pub fn vault_store(identity: &Identity) -> bool {
    if !vault_available() {
        return false;
    }
    let target = target_for(&identity.device_id().to_string());
    if backend::write(&target, &identity.to_bytes()) {
        tracing::info!(device_id = %identity.device_id(), "identity stored in credential vault");
        true
    } else {
        tracing::warn!(device_id = %identity.device_id(), "credential vault write failed; file fallback in use");
        false
    }
}

/// Delete the vault entry for `device_id`, if any. Best-effort; returns whether
/// it is now absent (true when deleted or when the backend is unavailable).
pub fn vault_delete(device_id: &str) -> bool {
    if !vault_available() {
        return true;
    }
    backend::delete(&target_for(device_id))
}

fn target_for(device_id: &str) -> String {
    format!("{CRED_PREFIX}:{device_id}")
}

/// Write the public device-id pointer (atomic, 0600). The device id is not
/// secret, but we keep it user-only for hygiene.
fn write_ref(dir: &Path, device_id: &str) -> std::io::Result<()> {
    crate::store::atomic_write(&dir.join(REF_FILE), device_id.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_fallback_roundtrips_on_every_platform() {
        // On a platform without a usable vault this is the only path; where a
        // vault exists it is the fallback. Either way it must be stable.
        if vault_available() {
            return; // covered by the vault test(s)
        }
        let dir = std::env::temp_dir().join(format!("tetherly-cred-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let a = load_or_create_identity(&dir).unwrap();
        let b = load_or_create_identity(&dir).unwrap();
        assert_eq!(a.device_id(), b.device_id());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn installed_data_dir_opts_into_the_vault() {
        // Guards the hygiene rule: only the installed data dir uses the vault,
        // so integration tests never write to the operator's real vault.
        if !vault_available() {
            return;
        }
        assert!(vault_enabled_for(&crate::store::default_data_dir()));
        assert!(!vault_enabled_for(&std::env::temp_dir().join("tetherly-x")));
    }

    /// Vault round-trip, gated to `TETHERLY_VAULT_TEST=1` because it writes a
    /// real entry into the OS vault. Documented in the changelog so it can be
    /// run on a real macOS/Linux machine (`TETHERLY_VAULT_TEST=1 cargo test`).
    #[test]
    fn vault_roundtrips_via_the_ref_pointer() {
        if !vault_available() || std::env::var("TETHERLY_VAULT_TEST").ok().as_deref() != Some("1") {
            return;
        }
        let dir = std::env::temp_dir().join(format!("tetherly-vault-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let id = Identity::generate().unwrap();
        assert!(vault_store(&id), "vault write should succeed");
        write_ref(&dir, &id.device_id().to_string()).unwrap();

        // The secret is in the vault; only the public pointer is on disk.
        let loaded = load_from_vault(&dir).expect("vault entry resolves via identity.ref");
        assert_eq!(loaded.device_id(), id.device_id());
        assert!(
            !dir.join(crate::store::IDENTITY_FILE).exists(),
            "identity.bin must not hold the secret when the vault is in use"
        );

        assert!(vault_delete(&id.device_id().to_string()));
        assert!(
            load_from_vault(&dir).is_none(),
            "deleted entry is not found"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
