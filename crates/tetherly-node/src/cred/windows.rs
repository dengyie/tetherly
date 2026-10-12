// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Windows Credential Manager backend for `cred`.
//!
//! Stores the 64 secret bytes as a **generic credential**
//! (`CredWriteW`/`CredReadW`/`CredDeleteW`, `CRED_TYPE_GENERIC`,
//! `CRED_PERSIST_LOCAL_MACHINE`) under the target `tetherly.identity:<id>`,
//! protected by the per-user Credential Manager.
//!
//! Self-contained: imports only absolute paths so this file compiles for the
//! Windows target on its own (see the module docs in the parent `cred`).

use std::ffi::c_void;
use windows_sys::Win32::Foundation::{GetLastError, ERROR_NOT_FOUND};
use windows_sys::Win32::Security::Credentials::{
    CredDeleteW, CredFree, CredReadW, CredWriteW, CREDENTIALW, CRED_PERSIST_LOCAL_MACHINE,
    CRED_TYPE_GENERIC,
};

pub fn available() -> bool {
    true
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Write `blob` as a generic credential. Returns false on any Win32 error.
pub fn write(target: &str, blob: &[u8]) -> bool {
    let mut target_w = wide(target);
    let mut user_w = wide("tetherly");
    // The blob must outlive the call; the API rejects empty blobs.
    let mut blob_copy = blob.to_vec();
    let cred = CREDENTIALW {
        Flags: 0,
        Type: CRED_TYPE_GENERIC,
        TargetName: target_w.as_mut_ptr(),
        Comment: std::ptr::null_mut(),
        LastWritten: windows_sys::Win32::Foundation::FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        },
        CredentialBlobSize: blob_copy.len() as u32,
        CredentialBlob: blob_copy.as_mut_ptr(),
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        AttributeCount: 0,
        Attributes: std::ptr::null_mut(),
        TargetAlias: std::ptr::null_mut(),
        UserName: user_w.as_mut_ptr(),
    };
    // SAFETY: `cred` and both wide buffers are valid for the call; the API
    // copies the blob into the vault before returning.
    unsafe { CredWriteW(&cred, 0) != 0 }
}

/// Read the blob for `target`. `None` if not found or on error.
pub fn read(target: &str) -> Option<Vec<u8>> {
    let target_w = wide(target);
    let mut ptr: *mut CREDENTIALW = std::ptr::null_mut();
    // SAFETY: `target_w` is a valid NUL-terminated wide string; on success the
    // API allocates `*ptr`, which we copy and release with CredFree.
    let ok = unsafe { CredReadW(target_w.as_ptr(), CRED_TYPE_GENERIC, 0, &mut ptr) };
    if ok == 0 || ptr.is_null() {
        return None;
    }
    // SAFETY: `ptr` is non-null and owned by the credential API until freed.
    unsafe {
        let c = &*ptr;
        let out = if c.CredentialBlob.is_null() || c.CredentialBlobSize == 0 {
            None
        } else {
            Some(
                std::slice::from_raw_parts(c.CredentialBlob, c.CredentialBlobSize as usize)
                    .to_vec(),
            )
        };
        CredFree(ptr as *const c_void);
        out
    }
}

/// Delete `target` if present. True when the entry no longer exists.
pub fn delete(target: &str) -> bool {
    let target_w = wide(target);
    // SAFETY: valid NUL-terminated wide string; a missing entry returns
    // ERROR_NOT_FOUND, which we treat as success.
    let ok = unsafe { CredDeleteW(target_w.as_ptr(), CRED_TYPE_GENERIC, 0) };
    if ok != 0 {
        return true;
    }
    // SAFETY: plain accessor.
    unsafe { GetLastError() == ERROR_NOT_FOUND }
}
