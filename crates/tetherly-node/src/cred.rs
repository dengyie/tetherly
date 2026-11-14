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

/// Run a vault call on a plain thread. The Linux Secret Service client (zbus)
/// drives its blocking API with its own runtime and panics if invoked from
/// within one — the node's `#[tokio::main]` binary and every `#[tokio::test]`
/// are exactly that. A panicking backend degrades to `fallback` instead of
/// taking the process down, matching the file-fallback contract.
fn off_runtime<T: Send>(fallback: T, f: impl FnOnce() -> T + Send) -> T {
    std::thread::scope(|scope| scope.spawn(f).join().unwrap_or(fallback))
}

/// Whether the OS vault is usable on this platform/build.
pub fn vault_available() -> bool {
    off_runtime(false, backend::available)
}

/// Load the identity for `dir`, creating one on first use.
///
/// v0.2.20 dual-store semantics: `identity.bin` and the OS vault are BOTH
/// authoritative and must always resolve to the same identity. Whichever
/// store is readable in the current process context serves the load; a
/// missing side is repaired from the other one (idempotent). This decouples
/// the device identity from the process privilege level — the production
/// Windows daemon runs both as the interactive user (vault-readable) and as
/// LocalSystem (vault invisible by design, file-readable), and both chains
/// must load the same identity or every trust/pairing would silently rotate
/// on a service restart.
///
/// Never delete `identity.bin` on successful vault migration: the two stores
/// have no cross-store atomicity, and a later vault entry loss (machine
/// credential cleanup, user profile reset) with the file gone left
/// `identity.ref` pointing at an unreadable vault — the daemon then refused
/// to start (`VaultUnavailable`, production 2026-11-12) with the pairing
/// keys gone. A restored file side makes every such state recoverable.
pub fn load_or_create_identity(dir: &Path) -> Result<Identity, NodeStoreError> {
    let id = if vault_enabled_for(dir) {
        load_dual_store(dir)?
    } else {
        crate::store::load_or_create_identity(dir)?
    };
    repair_other_store(dir, &id);
    Ok(id)
}

/// Dual-store load for vault-enabled contexts.
///
/// Precedence: vault (no key bytes on disk) → file → mint. A `.ref` pointer
/// without a usable vault entry is still recoverable while `identity.bin`
/// exists — the `VaultUnavailable` refusal below only triggers when BOTH
/// stores are unreadable; minting then would be exactly the identity-rotation
/// bug the guard exists to prevent, so failing loudly stays correct (the
/// operator can delete `identity.ref` deliberately to accept a fresh
/// identity).
fn load_dual_store(dir: &Path) -> Result<Identity, NodeStoreError> {
    if let Some(id) = load_from_vault(dir) {
        return Ok(id);
    }
    if dir.join(REF_FILE).exists() && !dir.join(crate::store::IDENTITY_FILE).exists() {
        return Err(NodeStoreError::VaultUnavailable);
    }
    crate::store::load_or_create_identity(dir)
}

/// Idempotently bring the non-authoritative store in line with the loaded
/// identity. Best effort — a repair failure never fails startup; the store
/// that served the load still serves the next run.
fn repair_other_store(dir: &Path, id: &Identity) {
    let device_id = id.device_id().to_string();

    if !dir.join(REF_FILE).exists() && write_ref(dir, &device_id).is_ok() {
        tracing::info!(device_id = %device_id, "repaired missing identity.ref pointer");
    }

    if !dir.join(crate::store::IDENTITY_FILE).exists() {
        // Only the vault side holds the secret here. Writing the file back
        // (0600 / user-only ACL) makes a LocalSystem process — which cannot
        // read this user's vault — able to start the daemon without
        // rotating the identity.
        if crate::store::atomic_write(&dir.join(crate::store::IDENTITY_FILE), &id.to_bytes())
            .is_ok()
        {
            tracing::info!(
                device_id = %device_id,
                "repaired missing identity.bin from credential vault (dual-store)"
            );
        } else {
            tracing::warn!(device_id = %device_id, "identity.bin repair write failed");
        }
    }

    // Vault side (only where the vault is enabled for this dir): ensure an
    // entry exists for THIS identity. A stale entry under a different device
    // id is left alone — the loaded identity is authoritative for repair.
    if vault_enabled_for(dir) {
        let stale = match backend::read(&target_for(&device_id)) {
            Some(bytes) => bytes != id.to_bytes(),
            None => true,
        };
        if stale && !vault_store(id) {
            tracing::warn!(
                device_id = %device_id,
                "vault entry missing/stale and vault write failed; file side remains authoritative"
            );
        } else if stale {
            tracing::info!(device_id = %device_id, "repaired missing/stale vault entry");
        }
    }
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
    let bytes = off_runtime(None, || backend::read(&target_for(&device_id)))?;
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
/// `TETHERLY_VAULT=1`/`0` overrides the decision (used by tests). The cheap
/// env/dir checks run first so a non-installed dir never probes D-Bus.
///
/// v0.2.20: a process running as LocalSystem never opts into the vault.
/// Windows Credential Manager entries are per-logon-session/user — the
/// interactive user's entries are invisible to the SYSTEM session by
/// design (production: `CredReadW` fails with ERROR_NO_SUCH_LOGON_SESSION
/// 1312 from a network-style logon, and the SYSTEM session simply has no
/// entry). Opting in there would either fail the startup guard against a
/// healthy vault or mint a second identity. The file side (identity.bin,
/// ACL-granted to SYSTEM by `store::grant_identity_file_system_read`)
/// serves the SYSTEM chain instead.
fn vault_enabled_for(_dir: &Path) -> bool {
    match std::env::var("TETHERLY_VAULT").ok().as_deref() {
        Some("1") => {}
        Some("0") => return false,
        _ => {
            #[cfg(windows)]
            if process_runs_as_local_system() {
                return false;
            }
            // macOS Keychain generic passwords trigger modal authorization prompts
            // whenever the binary is updated/recompiled or ad-hoc signed.
            // Default to file storage (identity.bin with 0600 permissions) on macOS
            // to eliminate login keychain password prompts, unless explicitly requested via TETHERLY_VAULT=1.
            #[cfg(target_os = "macos")]
            return false;

            #[cfg(not(any(target_os = "macos", windows)))]
            if _dir != crate::store::default_data_dir() {
                return false;
            }
        }
    }
    vault_available()
}

/// True when this process's primary token is LocalSystem (`S-1-5-18`).
///
/// Windows only. Best effort: any token API failure returns false, which
/// just means the vault path is attempted as before (and its read failure
/// falls back to the file side).
#[cfg(windows)]
pub(crate) fn process_runs_as_local_system() -> bool {
    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::Security::{GetSidSubAuthority, GetSidSubAuthorityCount, TokenUser};
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    const LOCAL_SYSTEM_RID: u32 = 18;
    const SID_AUTHORITY_NT: [u8; 6] = [0, 0, 0, 0, 0, 5];
    const TOKEN_QUERY: u32 = 0x0008;

    let mut token: HANDLE = std::ptr::null_mut();
    // SAFETY: GetCurrentProcess is a pseudo-handle factory; the token handle
    // is closed below before every return.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return false;
    }
    let result = 'ok: {
        // TokenUser needs a buffer; query the size first, then fetch. The
        // SID of the user rides in the returned TOKEN_USER buffer.
        let mut len: u32 = 0;
        // SAFETY: len is an out-param; the null buffer path is documented.
        unsafe {
            let _ = windows_sys::Win32::Security::GetTokenInformation(
                token,
                TokenUser,
                std::ptr::null_mut(),
                0,
                &mut len,
            );
        }
        if len == 0 {
            break 'ok false;
        }
        let mut buf = vec![0u8; len as usize];
        // SAFETY: buf is allocated to the size the previous call reported.
        let ok = unsafe {
            windows_sys::Win32::Security::GetTokenInformation(
                token,
                TokenUser,
                buf.as_mut_ptr() as *mut _,
                len,
                &mut len,
            )
        };
        if ok == 0 {
            break 'ok false;
        }
        // SAFETY: the SID pointer is inside the freshly filled buffer; PSID
        // is an opaque void pointer that points at a `SID` record.
        unsafe {
            let user = &*(buf.as_ptr() as *const windows_sys::Win32::Security::TOKEN_USER);
            let sid = user.User.Sid as *const windows_sys::Win32::Security::SID;
            if sid.is_null() {
                break 'ok false;
            }
            if (*sid).Revision != 1 {
                break 'ok false;
            }
            if (*sid).IdentifierAuthority.Value != SID_AUTHORITY_NT {
                break 'ok false;
            }
            let count = *GetSidSubAuthorityCount(sid as *mut _) as usize;
            if count != 1 {
                break 'ok false;
            }
            *GetSidSubAuthority(sid as *mut _, 0) == LOCAL_SYSTEM_RID
        }
    };
    // SAFETY: token is a valid handle from OpenProcessToken above.
    unsafe {
        let _ = windows_sys::Win32::Foundation::CloseHandle(token);
    }
    result
}

/// Persist an identity's secrets to the vault. Returns false when unsupported
/// or on any vault error; callers keep the file fallback in that case.
pub fn vault_store(identity: &Identity) -> bool {
    if !vault_available() {
        return false;
    }
    let target = target_for(&identity.device_id().to_string());
    let blob = identity.to_bytes();
    if off_runtime(false, || backend::write(&target, &blob)) {
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
    off_runtime(true, || backend::delete(&target_for(device_id)))
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
        // Vault is enabled for the installed dir only, not arbitrary temp dirs,
        // so integration tests never write to the operator's real vault.
        if !vault_available() {
            return;
        }
        #[cfg(target_os = "macos")]
        {
            assert!(!vault_enabled_for(&crate::store::default_data_dir()));
        }
        #[cfg(not(target_os = "macos"))]
        {
            assert!(vault_enabled_for(&crate::store::default_data_dir()));
        }
        // Windows opt-in is privilege-context-based since v0.2.20 (LocalSystem
        // never opts in; the interactive user always does) — `vault_enabled_for`
        // does NOT consult the dir on Windows, so a temp dir is exactly as
        // vault-eligible as the default data dir. The old dir-based opt-out
        // assertion (Phase 1-5 era) was unsatisfiable there: it contradicted
        // the default-dir assertion above whenever the vault was healthy, and
        // failed every Windows CI run that reached it (production 2026-11-12,
        // PR #23; main's runs never hit it because phase3 aborted earlier).
        // Linux keeps the real dir gate (non-default dirs never opt in).
        #[cfg(windows)]
        assert_eq!(
            vault_enabled_for(&std::env::temp_dir().join("tetherly-x")),
            vault_enabled_for(&crate::store::default_data_dir()),
            "windows vault opt-in is privilege-context-based, not dir-based"
        );
        #[cfg(not(any(target_os = "macos", windows)))]
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

        // The vault resolves via the public pointer alone.
        let loaded = load_from_vault(&dir).expect("vault entry resolves via identity.ref");
        assert_eq!(loaded.device_id(), id.device_id());

        assert!(vault_delete(&id.device_id().to_string()));
        assert!(
            load_from_vault(&dir).is_none(),
            "deleted entry is not found"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Production regression (2026-11-12, v0.2.20): a `identity.ref` pointer
    /// whose vault entry is gone must NOT refuse to start while
    /// `identity.bin` still exists — the file side recovers the secret. The
    /// old code returned `VaultUnavailable` whenever the ref was present and
    /// the vault read failed, which is only correct when the file side is
    /// ALSO gone.
    #[test]
    fn ref_without_vault_entry_recovers_from_identity_bin() {
        if vault_available() {
            // Simulate the unreadable-vault state with a device id that has
            // no entry; on vaultless platforms this test exercises the same
            // code path unconditionally.
        }
        let dir = std::env::temp_dir().join(format!("tetherly-refrec-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let id = Identity::generate().unwrap();
        // File side exists; pointer exists; vault entry does NOT.
        crate::store::atomic_write(&dir.join(crate::store::IDENTITY_FILE), &id.to_bytes()).unwrap();
        write_ref(&dir, &id.device_id().to_string()).unwrap();

        // The dual-store load must serve the file side and repair the vault.
        let loaded = load_or_create_identity(&dir).expect("file side recovers the identity");
        assert_eq!(loaded.device_id(), id.device_id());
        assert!(
            dir.join(crate::store::IDENTITY_FILE).exists(),
            "dual-store keeps identity.bin on disk"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The dual-store invariant: identity.bin and the vault (where enabled)
    /// must resolve to the same identity on every load, and a missing side is
    /// repaired idempotently. Gated to `TETHERLY_VAULT_TEST=1` on vaulted
    /// platforms because it writes a real vault entry.
    #[test]
    fn dual_store_repairs_missing_sides_idempotently() {
        if vault_available() && std::env::var("TETHERLY_VAULT_TEST").ok().as_deref() != Some("1") {
            return;
        }
        let dir = std::env::temp_dir().join(format!("tetherly-dual-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // First load creates the identity; with the vault enabled it also
        // lands in the vault. Either way the file side must exist after the
        // load (the old "delete bin on migration" behavior is gone).
        let a = load_or_create_identity(&dir).unwrap();
        assert!(
            dir.join(crate::store::IDENTITY_FILE).exists(),
            "identity.bin is never deleted by the dual-store repair"
        );

        // Delete the file: the next load must recover from the vault (or, on
        // vaultless platforms, mint — but vaultless platforms skip here).
        if vault_available() {
            std::fs::remove_file(dir.join(crate::store::IDENTITY_FILE)).unwrap();
            let b = load_or_create_identity(&dir).unwrap();
            assert_eq!(a.device_id(), b.device_id(), "vault restores the file side");
            assert!(dir.join(crate::store::IDENTITY_FILE).exists());

            // Delete the vault entry: the file side restores the vault.
            assert!(vault_delete(&a.device_id().to_string()));
            let c = load_or_create_identity(&dir).unwrap();
            assert_eq!(a.device_id(), c.device_id(), "file restores the vault side");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The vault-disabled (file-only) path — SYSTEM context, macOS, tests —
    /// must also keep the pointer coherent with the file identity.
    #[test]
    fn file_only_path_keeps_ref_pointer_coherent() {
        if vault_available() {
            return; // the dual-store test above covers the vaulted path
        }
        let dir = std::env::temp_dir().join(format!("tetherly-fonly-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let a = load_or_create_identity(&dir).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join(REF_FILE)).unwrap().trim(),
            a.device_id().to_string(),
            "ref pointer tracks the file identity"
        );
        // Deleting the pointer is repaired on the next load.
        std::fs::remove_file(dir.join(REF_FILE)).unwrap();
        let b = load_or_create_identity(&dir).unwrap();
        assert_eq!(a.device_id(), b.device_id());
        assert!(dir.join(REF_FILE).exists(), "pointer is repaired");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
