// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Cross-screen file-drag probe (v0.2.89).
//!
//! When the cursor leaves a screen with LMB held, the node asks "is an OS
//! file drag in flight, and which files does it carry?":
//!
//! - macOS: the system drag pasteboard (`NSPasteboardNameDrag`). Its
//!   `changeCount` bumps on every drag start, so a count that moved since the
//!   button went down proves *this* gesture is a drag (stale contents from an
//!   older drag never qualify).
//! - Windows: only an OLE drop target can see a foreign drag's data. The
//!   daemon (LocalSystem, session 0 desktop) spawns `tetherly drag-probe` as
//!   the console user; it parks a 1-alpha topmost window under the cursor,
//!   nudges the cursor so the source's `DoDragDrop` loop hit-tests it, reads
//!   `CF_HDROP` in `DragEnter` (refusing the drop), and posts the paths back
//!   to `POST /api/v1/drag/probe`.
//!
//! INVARIANTS:
//! - The probe never accepts a drop (`DROPEFFECT_NONE`); the caller cancels
//!   the OS drag with Esc afterwards, so nothing lands locally.
//! - Only absolute local paths leave this module.

/// Drag pasteboard change counter; `None` where no such pasteboard exists.
#[cfg(target_os = "macos")]
pub fn drag_pasteboard_change_count() -> Option<i64> {
    use objc2_app_kit::{NSPasteboard, NSPasteboardNameDrag};
    let pb = NSPasteboard::pasteboardWithName(unsafe { NSPasteboardNameDrag });
    Some(pb.changeCount() as i64)
}

#[cfg(not(target_os = "macos"))]
pub fn drag_pasteboard_change_count() -> Option<i64> {
    None
}

/// File paths currently on the drag pasteboard (macOS); empty elsewhere.
#[cfg(target_os = "macos")]
pub fn drag_pasteboard_files() -> Vec<String> {
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2::ClassType;
    use objc2_app_kit::{NSPasteboard, NSPasteboardNameDrag};
    use objc2_foundation::{NSArray, NSDictionary, NSString, NSURL};
    let pb = NSPasteboard::pasteboardWithName(unsafe { NSPasteboardNameDrag });
    let classes = NSArray::from_slice(&[<NSURL as ClassType>::class()]);
    let options: Option<&NSDictionary<NSString, AnyObject>> = None;
    let urls: Retained<NSArray<AnyObject>> =
        match unsafe { pb.readObjectsForClasses_options(&classes, options) } {
            Some(arr) => arr,
            None => return Vec::new(),
        };
    let mut paths = Vec::new();
    for url in urls.iter() {
        let Ok(url) = url.downcast::<NSURL>() else {
            continue;
        };
        if url.isFileURL() {
            if let Some(path) = url.path() {
                paths.push(path.to_string());
            }
        }
    }
    paths
}

/// Type identifiers on the drag pasteboard (macOS), for probe diagnostics
/// when a drag carried no file URLs.
#[cfg(target_os = "macos")]
pub fn drag_pasteboard_types() -> Vec<String> {
    use objc2_app_kit::{NSPasteboard, NSPasteboardNameDrag};
    let pb = NSPasteboard::pasteboardWithName(unsafe { NSPasteboardNameDrag });
    pb.types()
        .map(|arr| arr.iter().take(16).map(|t| t.to_string()).collect())
        .unwrap_or_default()
}

#[cfg(not(target_os = "macos"))]
pub fn drag_pasteboard_files() -> Vec<String> {
    Vec::new()
}

/// Launch the Windows probe child for `probe_id`; it reports through the
/// loopback API. Errors are human-readable spawn failures.
#[cfg(windows)]
pub fn spawn_drag_probe(ui_port: u16, probe_id: u64) -> Result<(), String> {
    use windows_sys::Win32::Foundation::CloseHandle;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
    let exe_str = exe.to_string_lossy().into_owned();
    let token = tetherly_loopback::api_token_for(ui_port);
    let args = [
        "drag-probe".to_string(),
        "--ui-port".into(),
        ui_port.to_string(),
        "--probe-id".into(),
        probe_id.to_string(),
    ];
    if crate::cred::process_runs_as_local_system() {
        let line = std::iter::once(exe_str.clone())
            .chain(args.iter().cloned())
            .map(|a| quote_arg(&a))
            .collect::<Vec<_>>()
            .join(" ");
        let env: Vec<(&str, &str)> = token
            .iter()
            .map(|t| ("TETHERLY_API_TOKEN", t.as_str()))
            .collect();
        let child = crate::platform::spawn_as_console_user(&exe_str, &line, &env)
            .map_err(|e| format!("spawn as console user: {e}"))?;
        unsafe {
            CloseHandle(child.thread);
            CloseHandle(child.process);
        }
        Ok(())
    } else {
        use std::os::windows::process::CommandExt;
        let mut cmd = std::process::Command::new(&exe);
        cmd.args(args).creation_flags(CREATE_NO_WINDOW);
        if let Some(t) = &token {
            cmd.env("TETHERLY_API_TOKEN", t);
        }
        cmd.spawn().map(|_| ()).map_err(|e| format!("spawn: {e}"))
    }
}

#[cfg(not(windows))]
pub fn spawn_drag_probe(_ui_port: u16, _probe_id: u64) -> Result<(), String> {
    Err("drag probe child is Windows-only".into())
}

/// Windows CRT argument quoting (inverse of the child's argv parsing).
#[cfg(any(windows, test))]
fn quote_arg(a: &str) -> String {
    if !a.is_empty() && !a.chars().any(|c| matches!(c, ' ' | '\t' | '"')) {
        return a.to_string();
    }
    let mut out = String::from("\"");
    let mut backslashes = 0usize;
    for c in a.chars() {
        if c == '\\' {
            backslashes += 1;
            continue;
        }
        if c == '"' {
            out.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
        } else {
            out.extend(std::iter::repeat_n('\\', backslashes));
        }
        out.push(c);
        backslashes = 0;
    }
    out.extend(std::iter::repeat_n('\\', backslashes * 2));
    out.push('"');
    out
}

/// Body of `tetherly drag-probe`: find the in-flight drag's files and post
/// them to the daemon. Always reports (an empty list means "no file drag"),
/// so the daemon's waiter never has to time out on a healthy child.
#[cfg(windows)]
pub fn run_drag_probe_process(ui_port: u16, probe_id: u64) -> i32 {
    let paths = win::probe_paths(std::time::Duration::from_millis(700));
    tracing::info!(probe_id, files = paths.len(), "drag probe finished");
    let body = serde_json::json!({ "probe_id": probe_id, "paths": paths });
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            tracing::warn!(error = %e, "drag probe: no tokio runtime");
            return 1;
        }
    };
    let res = runtime.block_on(tetherly_loopback::loopback_post(
        ui_port,
        "/api/v1/drag/probe",
        &serde_json::to_vec(&body).unwrap_or_default(),
    ));
    match res {
        Ok((code, _)) if code < 300 => 0,
        Ok((code, _)) => {
            tracing::warn!(code, "drag probe report refused");
            1
        }
        Err(e) => {
            tracing::warn!(error = %e, "drag probe report failed");
            1
        }
    }
}

#[cfg(windows)]
mod win {
    use std::cell::RefCell;
    use windows::core::implement;
    use windows::Win32::Foundation::POINTL;
    use windows::Win32::System::Com::{IDataObject, FORMATETC};
    use windows::Win32::System::Ole::{
        IDropTarget, IDropTarget_Impl, OleInitialize, OleUninitialize, RegisterDragDrop,
        ReleaseStgMedium, RevokeDragDrop, DROPEFFECT,
    };
    use windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS;
    use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_MOVE, MOUSEINPUT,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetCursorPos,
        MsgWaitForMultipleObjects, PeekMessageW, RegisterClassW, SetLayeredWindowAttributes,
        ShowWindow, TranslateMessage, LWA_ALPHA, MSG, PM_REMOVE, QS_ALLINPUT, SW_SHOWNOACTIVATE,
        WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
    };

    const CF_HDROP: u16 = 15;
    const DROPEFFECT_NONE: DROPEFFECT = DROPEFFECT(0);
    const PROBE_SIZE: i32 = 48;

    thread_local! {
        static FOUND: RefCell<Option<Vec<String>>> = const { RefCell::new(None) };
    }

    fn paths_from_data_object(data: &IDataObject) -> Vec<String> {
        let fmt = FORMATETC {
            cfFormat: CF_HDROP,
            ptd: std::ptr::null_mut(),
            dwAspect: 1,
            lindex: -1,
            tymed: 1,
        };
        let medium = match unsafe { data.GetData(&fmt) } {
            Ok(m) => m,
            Err(_) => return Vec::new(),
        };
        let hdrop = HDROP(unsafe { medium.u.hGlobal }.0);
        let count = unsafe { DragQueryFileW(hdrop, u32::MAX, None) };
        let mut paths = Vec::with_capacity(count.min(1024) as usize);
        for i in 0..count {
            let len = unsafe { DragQueryFileW(hdrop, i, None) } as usize;
            if len == 0 {
                continue;
            }
            let mut buf = vec![0u16; len + 1];
            let n = unsafe { DragQueryFileW(hdrop, i, Some(&mut buf)) };
            buf.truncate(n as usize);
            paths.push(String::from_utf16_lossy(&buf));
        }
        unsafe {
            ReleaseStgMedium(&medium as *const _ as *mut _);
        }
        paths
    }

    #[implement(IDropTarget)]
    struct ProbeTarget;

    impl IDropTarget_Impl for ProbeTarget_Impl {
        fn DragEnter(
            &self,
            pdataobj: windows::core::Ref<'_, IDataObject>,
            _grfkeystate: MODIFIERKEYS_FLAGS,
            _pt: &POINTL,
            pdweffect: *mut DROPEFFECT,
        ) -> windows::core::Result<()> {
            unsafe {
                *pdweffect = DROPEFFECT_NONE;
            }
            if let Ok(data) = pdataobj.ok() {
                let paths = paths_from_data_object(data);
                FOUND.with(|f| *f.borrow_mut() = Some(paths));
            }
            Ok(())
        }

        fn DragOver(
            &self,
            _grfkeystate: MODIFIERKEYS_FLAGS,
            _pt: &POINTL,
            pdweffect: *mut DROPEFFECT,
        ) -> windows::core::Result<()> {
            unsafe {
                *pdweffect = DROPEFFECT_NONE;
            }
            Ok(())
        }

        fn DragLeave(&self) -> windows::core::Result<()> {
            Ok(())
        }

        fn Drop(
            &self,
            _pdataobj: windows::core::Ref<'_, IDataObject>,
            _grfkeystate: MODIFIERKEYS_FLAGS,
            _pt: &POINTL,
            pdweffect: *mut DROPEFFECT,
        ) -> windows::core::Result<()> {
            unsafe {
                *pdweffect = DROPEFFECT_NONE;
            }
            Ok(())
        }
    }

    unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
        DefWindowProcW(hwnd, msg, w, l)
    }

    fn nudge(dx: i32) {
        let input = INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dx,
                    dy: 0,
                    mouseData: 0,
                    dwFlags: MOUSEEVENTF_MOVE,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        };
        unsafe {
            SendInput(1, &input, std::mem::size_of::<INPUT>() as i32);
        }
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// Run one probe: window under the cursor, nudges, bounded pump.
    pub(super) fn probe_paths(budget: std::time::Duration) -> Vec<String> {
        if unsafe { OleInitialize(None) }.is_err() {
            tracing::warn!("drag probe: OleInitialize refused");
            return Vec::new();
        }
        let paths = probe_inner(budget);
        unsafe { OleUninitialize() };
        paths
    }

    fn probe_inner(budget: std::time::Duration) -> Vec<String> {
        let class = wide("TetherlyDragProbe");
        let title = wide("");
        let wc = WNDCLASSW {
            style: 0,
            lpfnWndProc: Some(wnd_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: std::ptr::null_mut(),
            hIcon: std::ptr::null_mut(),
            hCursor: std::ptr::null_mut(),
            hbrBackground: std::ptr::null_mut(),
            lpszMenuName: std::ptr::null(),
            lpszClassName: class.as_ptr(),
        };
        unsafe { RegisterClassW(&wc) };
        let mut pt = POINT { x: 0, y: 0 };
        unsafe { GetCursorPos(&mut pt) };
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                class.as_ptr(),
                title.as_ptr(),
                WS_POPUP,
                pt.x - PROBE_SIZE / 2,
                pt.y - PROBE_SIZE / 2,
                PROBE_SIZE,
                PROBE_SIZE,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            )
        };
        if hwnd.is_null() {
            tracing::warn!("drag probe: CreateWindowExW failed");
            return Vec::new();
        }
        unsafe {
            // Alpha 1: invisible to the eye but still hit-tested by OLE.
            SetLayeredWindowAttributes(hwnd, 0, 1, LWA_ALPHA);
            ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        }
        let target: IDropTarget = ProbeTarget.into();
        let hwnd_win = windows::Win32::Foundation::HWND(hwnd);
        if unsafe { RegisterDragDrop(hwnd_win, &target) }.is_err() {
            tracing::warn!("drag probe: RegisterDragDrop refused");
            unsafe { DestroyWindow(hwnd) };
            return Vec::new();
        }
        let start = std::time::Instant::now();
        let mut nudges = 0u32;
        let mut result = None;
        while start.elapsed() < budget {
            // The source's DoDragDrop loop re-hit-tests on mouse input; a
            // ±1 px injected wiggle makes it find the probe window.
            let due = start.elapsed().as_millis() as u32 / 120;
            if nudges <= due && nudges < 6 {
                nudge(if nudges % 2 == 0 { 1 } else { -1 });
                nudges += 1;
            }
            let mut msg: MSG = unsafe { std::mem::zeroed() };
            while unsafe { PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) } != 0 {
                unsafe {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
            if let Some(found) = FOUND.with(|f| f.borrow_mut().take()) {
                result = Some(found);
                break;
            }
            unsafe { MsgWaitForMultipleObjects(0, std::ptr::null(), 0, 20, QS_ALLINPUT) };
        }
        if nudges % 2 == 1 {
            nudge(-1);
        }
        unsafe {
            let _ = RevokeDragDrop(hwnd_win);
            DestroyWindow(hwnd);
        }
        result.unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::quote_arg;

    #[test]
    fn quote_arg_wraps_spaces_and_escapes_quotes() {
        assert_eq!(quote_arg("drag-probe"), "drag-probe");
        assert_eq!(
            quote_arg(r"C:\Program Files\x.exe"),
            r#""C:\Program Files\x.exe""#
        );
        assert_eq!(quote_arg(r#"a"b"#), r#""a\"b""#);
    }
}
