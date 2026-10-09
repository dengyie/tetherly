// SPDX-License-Identifier: Apache-2.0 OR MIT
//! OS clipboard and conservative insert. insert() only runs on a user click.

use std::sync::{Arc, Mutex};
use tetherly_core::{insertion_plan, Clip, CoreError, InsertionPlan, Insertor};

pub struct SystemClip;

impl Clip for SystemClip {
    fn set_text(&self, text: &str) -> Result<(), CoreError> {
        let mut cb = arboard::Clipboard::new().map_err(|e| CoreError::Json(e.to_string()))?;
        cb.set_text(text.to_string())
            .map_err(|e| CoreError::Json(e.to_string()))
    }

    fn get_text(&self) -> Result<Option<String>, CoreError> {
        let mut cb = arboard::Clipboard::new().map_err(|e| CoreError::Json(e.to_string()))?;
        match cb.get_text() {
            Ok(t) => Ok(Some(t)),
            Err(arboard::Error::ContentNotAvailable) => Ok(None),
            Err(e) => Err(CoreError::Json(e.to_string())),
        }
    }
}

/// In-memory clipboard for tests and headless CI.
#[derive(Clone, Default)]
pub struct MemoryClip {
    inner: Arc<Mutex<Option<String>>>,
}

impl MemoryClip {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Clip for MemoryClip {
    fn set_text(&self, text: &str) -> Result<(), CoreError> {
        *self.inner.lock().expect("clip") = Some(text.to_string());
        Ok(())
    }

    fn get_text(&self) -> Result<Option<String>, CoreError> {
        Ok(self.inner.lock().expect("clip").clone())
    }
}

pub struct MemoryInsertor {
    pub current: Arc<Mutex<String>>,
    pub read_only: bool,
    pub last: Arc<Mutex<Option<InsertionPlan>>>,
}

impl MemoryInsertor {
    pub fn new(current: impl Into<String>, read_only: bool) -> Self {
        Self {
            current: Arc::new(Mutex::new(current.into())),
            read_only,
            last: Arc::new(Mutex::new(None)),
        }
    }
}

impl Insertor for MemoryInsertor {
    fn insert(&self, value: &str) -> Result<(), CoreError> {
        let current = self.current.lock().expect("ins").clone();
        let plan = insertion_plan(Some(current.as_str()), Some(value), self.read_only);
        *self.last.lock().expect("ins") = Some(plan.clone());
        if !plan.should_write() {
            return Err(CoreError::InsertRefused(plan.reason));
        }
        if let Some(v) = plan.value {
            *self.current.lock().expect("ins") = v;
        }
        Ok(())
    }
}

/// Probe a field then apply `insertion::plan`. Never blind-types into the
/// foreground without a current-value probe.
pub fn insert_with_probe(
    current: Option<&str>,
    code: &str,
    read_only: bool,
    write: impl FnOnce(&str) -> Result<(), CoreError>,
) -> Result<InsertionPlan, CoreError> {
    let plan = insertion_plan(current, Some(code), read_only);
    if !plan.should_write() {
        return Err(CoreError::InsertRefused(plan.reason.clone()));
    }
    let typed = plan.typed.clone().unwrap_or_default();
    write(&typed)?;
    Ok(plan)
}

#[cfg(windows)]
pub fn insert_foreground(code: &str) -> Result<InsertionPlan, CoreError> {
    windows_insert(code)
}

#[cfg(not(windows))]
pub fn insert_foreground(code: &str) -> Result<InsertionPlan, CoreError> {
    let _ = code;
    Err(CoreError::InsertRefused(
        "foreground insert is Windows-only in Phase 1; mac next iteration".into(),
    ))
}

#[cfg(windows)]
fn windows_insert(code: &str) -> Result<InsertionPlan, CoreError> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        FindWindowExA, GetForegroundWindow, GetWindowLongA, IsWindow, SendMessageA, GWL_STYLE,
        WM_SETTEXT,
    };

    const ES_READONLY: i32 = 0x0800;
    const WM_GETTEXTLENGTH: u32 = 0x000E;
    const WM_GETTEXT: u32 = 0x000D;
    const EDIT_CLASS: [u8; 5] = *b"Edit\0";

    unsafe {
        let fg = GetForegroundWindow();
        if fg.is_null() || IsWindow(fg) == 0 {
            return Err(CoreError::InsertRefused("no foreground window".into()));
        }
        let mut hwnd = FindWindowExA(
            fg,
            std::ptr::null_mut(),
            EDIT_CLASS.as_ptr(),
            std::ptr::null(),
        );
        if hwnd.is_null() {
            hwnd = fg;
        }
        let style = GetWindowLongA(hwnd, GWL_STYLE);
        let read_only = (style & ES_READONLY) != 0;
        let len = SendMessageA(hwnd, WM_GETTEXTLENGTH, 0, 0);
        let current = if len > 0 {
            let mut buf = vec![0u8; len as usize + 2];
            let n = SendMessageA(
                hwnd,
                WM_GETTEXT,
                buf.len() as usize,
                buf.as_mut_ptr() as isize,
            );
            if n > 0 {
                String::from_utf8_lossy(&buf[..n as usize]).into_owned()
            } else {
                String::new()
            }
        } else {
            String::new()
        };
        insert_with_probe(Some(current.as_str()), code, read_only, |typed| {
            let full = insertion_plan(Some(current.as_str()), Some(code), false)
                .value
                .unwrap_or_else(|| typed.to_string());
            let mut cstr = full.into_bytes();
            cstr.push(0);
            SendMessageA(hwnd, WM_SETTEXT, 0, cstr.as_ptr() as isize);
            Ok(())
        })
    }
}
