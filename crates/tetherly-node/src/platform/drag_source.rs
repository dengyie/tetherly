// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Native receive-side file drag (v0.3.0, Windows).
//!
//! A file drag carried in from the controller continues as a real OLE drag
//! on this screen. The daemon (LocalSystem, session 0) spawns
//! `tetherly drag-source` as the console user. The helper:
//!
//! 1. polls `POST /api/v1/drag/native/poll` until the payload paths are known
//!    (the bytes may still be in flight);
//! 2. parks a 1-alpha window under the cursor and presses the left button on
//!    it, so the injected press reaches no application;
//! 3. runs `DoDragDrop` with a `CF_HDROP` data object (copy only). The
//!    controller's forwarded button-up ends the drag; when the transfer has
//!    not finished yet the drop is held until the daemon reports `ready`;
//! 4. reports the outcome to `POST /api/v1/drag/native/result`. The daemon
//!    keeps a dropped copy and delivers everything else to the inbox.
//!
//! INVARIANTS:
//! - The data object offers only `DROPEFFECT_COPY` of daemon-published paths.
//! - A drop is only released once every file is complete on disk.
//! - The helper always reports, so the daemon never waits on a dead child.

#[cfg(windows)]
pub fn spawn_drag_source(ui_port: u16, drag_id: u64) -> Result<(), String> {
    crate::platform::drag_probe::spawn_helper(
        ui_port,
        &[
            "drag-source".to_string(),
            "--ui-port".into(),
            ui_port.to_string(),
            "--drag-id".into(),
            drag_id.to_string(),
        ],
    )
}

#[cfg(not(windows))]
pub fn spawn_drag_source(_ui_port: u16, _drag_id: u64) -> Result<(), String> {
    Err("native drag source is Windows-only".into())
}

/// `CF_HDROP` payload: a `DROPFILES` header (pFiles, pt, fNC, fWide) then
/// the wide paths, each NUL-terminated, plus a final NUL.
#[cfg_attr(not(any(windows, test)), allow(dead_code))]
pub(crate) fn hdrop_bytes(paths: &[String]) -> Vec<u8> {
    const HEADER: u32 = 20;
    let mut out = Vec::with_capacity(HEADER as usize + paths.len() * 64);
    out.extend_from_slice(&HEADER.to_le_bytes()); // pFiles
    out.extend_from_slice(&0i32.to_le_bytes()); // pt.x
    out.extend_from_slice(&0i32.to_le_bytes()); // pt.y
    out.extend_from_slice(&0u32.to_le_bytes()); // fNC
    out.extend_from_slice(&1u32.to_le_bytes()); // fWide
    for p in paths {
        for u in p.encode_utf16().chain(std::iter::once(0)) {
            out.extend_from_slice(&u.to_le_bytes());
        }
    }
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

/// Last status the poller saw.
#[derive(Debug, Clone, Default)]
#[cfg_attr(not(windows), allow(dead_code))]
struct Status {
    state: String,
    released: bool,
    paths: Vec<String>,
}

#[cfg(windows)]
pub fn run_drag_source_process(ui_port: u16, drag_id: u64) -> i32 {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    let status = Arc::new(Mutex::new(Status::default()));
    let stop = Arc::new(AtomicBool::new(false));
    let poller = {
        let status = status.clone();
        let stop = stop.clone();
        std::thread::spawn(move || {
            let Ok(rt) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            else {
                return;
            };
            let body =
                serde_json::to_vec(&serde_json::json!({ "drag_id": drag_id })).unwrap_or_default();
            let mut failures = 0u32;
            while !stop.load(Ordering::Relaxed) {
                let res = rt.block_on(tetherly_loopback::loopback_post(
                    ui_port,
                    "/api/v1/drag/native/poll",
                    &body,
                ));
                let parsed = res.ok().and_then(|(code, bytes)| {
                    (code < 300)
                        .then(|| {
                            serde_json::from_slice::<tetherly_api::NativeDragStatus>(&bytes).ok()
                        })
                        .flatten()
                });
                match parsed {
                    Some(s) => {
                        failures = 0;
                        let mut st = status.lock().expect("status");
                        st.state = s.state;
                        st.released = s.released;
                        st.paths = s.paths;
                    }
                    None => {
                        failures += 1;
                        if failures >= 25 {
                            // The daemon is gone; nothing can complete.
                            status.lock().expect("status").state = "failed".into();
                        }
                    }
                }
                std::thread::sleep(Duration::from_millis(120));
            }
        })
    };

    // Wait for the names (the bytes may still be in flight).
    let start = Instant::now();
    let early: Option<&'static str> = loop {
        let st = status.lock().expect("status").clone();
        match st.state.as_str() {
            "cancel" => break Some("cancelled"),
            "failed" => break Some("failed"),
            _ => {}
        }
        if !st.paths.is_empty() {
            break None;
        }
        if st.released {
            break Some("released");
        }
        if start.elapsed() > Duration::from_secs(30) {
            break Some("failed");
        }
        std::thread::sleep(Duration::from_millis(40));
    };
    let outcome = match early {
        Some(o) => o,
        None => {
            let st = status.lock().expect("status").clone();
            if st.released && st.state != "ready" {
                // Released before the drag could start: nothing to hold.
                "released"
            } else {
                win::run_drag(st.paths, status.clone())
            }
        }
    };
    stop.store(true, Ordering::Relaxed);
    let _ = poller.join();
    tracing::info!(drag_id, outcome, "drag source finished");

    let body = serde_json::json!({ "drag_id": drag_id, "outcome": outcome });
    let Ok(rt) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        return 1;
    };
    match rt.block_on(tetherly_loopback::loopback_post(
        ui_port,
        "/api/v1/drag/native/result",
        &serde_json::to_vec(&body).unwrap_or_default(),
    )) {
        Ok((code, _)) if code < 300 => 0,
        Ok((code, _)) => {
            tracing::warn!(code, "drag source result refused");
            1
        }
        Err(e) => {
            tracing::warn!(error = %e, "drag source result failed");
            1
        }
    }
}

#[cfg(windows)]
mod win {
    use super::Status;
    use std::cell::Cell;
    use std::sync::{Arc, Mutex};
    use windows::core::{implement, HRESULT};
    use windows::Win32::Foundation::{
        BOOL, DRAGDROP_S_CANCEL, DRAGDROP_S_DROP, DRAGDROP_S_USEDEFAULTCURSORS, DV_E_FORMATETC,
        E_NOTIMPL, HGLOBAL, OLE_E_ADVISENOTSUPPORTED, S_OK,
    };
    use windows::Win32::System::Com::{
        IAdviseSink, IDataObject, IDataObject_Impl, IEnumFORMATETC, IEnumSTATDATA, FORMATETC,
        STGMEDIUM, STGMEDIUM_0,
    };
    use windows::Win32::System::Ole::{
        DoDragDrop, IDropSource, IDropSource_Impl, OleInitialize, OleUninitialize, DROPEFFECT,
    };
    use windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS;
    use windows::Win32::UI::Shell::SHCreateStdEnumFmtEtc;
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::System::Memory::{
        GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE,
    };
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, GetKeyState, GetKeyboardState, SendInput, SetKeyboardState, INPUT,
        INPUT_0, INPUT_MOUSE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEINPUT, VK_LBUTTON,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetSystemMetrics,
        KillTimer, MsgWaitForMultipleObjects, PeekMessageW, RegisterClassW,
        SetLayeredWindowAttributes, SetTimer, ShowWindow, TranslateMessage, LWA_ALPHA,
        MA_NOACTIVATE, MSG, PM_REMOVE, QS_ALLINPUT, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN,
        SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, SW_HIDE, SW_SHOWNOACTIVATE, WM_LBUTTONDOWN,
        WM_MOUSEACTIVATE, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
        WS_EX_TOPMOST, WS_POPUP,
    };

    const CF_HDROP: u16 = 15;
    const TYMED_HGLOBAL: u32 = 1;
    const DATADIR_GET: u32 = 1;
    const MK_LBUTTON: u32 = 0x0001;
    const DROPEFFECT_NONE: DROPEFFECT = DROPEFFECT(0);
    const DROPEFFECT_COPY: DROPEFFECT = DROPEFFECT(1);
    /// Keeps `QueryContinueDrag` ticking while the drop waits with no input.
    const TICK_MS: u32 = 50;

    thread_local! {
        static PRESSED: Cell<bool> = const { Cell::new(false) };
    }

    fn hdrop_format() -> FORMATETC {
        FORMATETC {
            cfFormat: CF_HDROP,
            ptd: std::ptr::null_mut(),
            dwAspect: 1,
            lindex: -1,
            tymed: TYMED_HGLOBAL,
        }
    }

    fn is_hdrop(fmt: *const FORMATETC) -> bool {
        if fmt.is_null() {
            return false;
        }
        let f = unsafe { &*fmt };
        f.cfFormat == CF_HDROP && f.tymed & TYMED_HGLOBAL != 0
    }

    #[implement(IDataObject)]
    struct FileData {
        hdrop: Vec<u8>,
    }

    impl IDataObject_Impl for FileData_Impl {
        fn GetData(&self, fmt: *const FORMATETC) -> windows::core::Result<STGMEDIUM> {
            if !is_hdrop(fmt) {
                return Err(DV_E_FORMATETC.into());
            }
            let h = unsafe { GlobalAlloc(GMEM_MOVEABLE, self.hdrop.len()) };
            if h.is_null() {
                return Err(E_NOTIMPL.into());
            }
            unsafe {
                let p = GlobalLock(h) as *mut u8;
                if p.is_null() {
                    return Err(E_NOTIMPL.into());
                }
                std::ptr::copy_nonoverlapping(self.hdrop.as_ptr(), p, self.hdrop.len());
                GlobalUnlock(h);
            }
            Ok(STGMEDIUM {
                tymed: TYMED_HGLOBAL,
                u: STGMEDIUM_0 {
                    hGlobal: HGLOBAL(h),
                },
                pUnkForRelease: std::mem::ManuallyDrop::new(None),
            })
        }

        fn GetDataHere(
            &self,
            _fmt: *const FORMATETC,
            _medium: *mut STGMEDIUM,
        ) -> windows::core::Result<()> {
            Err(E_NOTIMPL.into())
        }

        fn QueryGetData(&self, fmt: *const FORMATETC) -> HRESULT {
            if is_hdrop(fmt) {
                S_OK
            } else {
                DV_E_FORMATETC
            }
        }

        fn GetCanonicalFormatEtc(&self, _in: *const FORMATETC, out: *mut FORMATETC) -> HRESULT {
            if !out.is_null() {
                unsafe { (*out).ptd = std::ptr::null_mut() };
            }
            E_NOTIMPL
        }

        fn SetData(
            &self,
            _fmt: *const FORMATETC,
            _medium: *const STGMEDIUM,
            _release: BOOL,
        ) -> windows::core::Result<()> {
            Err(E_NOTIMPL.into())
        }

        fn EnumFormatEtc(&self, direction: u32) -> windows::core::Result<IEnumFORMATETC> {
            if direction != DATADIR_GET {
                return Err(E_NOTIMPL.into());
            }
            unsafe { SHCreateStdEnumFmtEtc(&[hdrop_format()]) }
        }

        fn DAdvise(
            &self,
            _fmt: *const FORMATETC,
            _advf: u32,
            _sink: windows::core::Ref<'_, IAdviseSink>,
        ) -> windows::core::Result<u32> {
            Err(OLE_E_ADVISENOTSUPPORTED.into())
        }

        fn DUnadvise(&self, _conn: u32) -> windows::core::Result<()> {
            Err(OLE_E_ADVISENOTSUPPORTED.into())
        }

        fn EnumDAdvise(&self) -> windows::core::Result<IEnumSTATDATA> {
            Err(OLE_E_ADVISENOTSUPPORTED.into())
        }
    }

    #[implement(IDropSource)]
    struct Source {
        status: Arc<Mutex<Status>>,
        /// Why the drag was cancelled, when it was.
        reason: Arc<Mutex<Option<&'static str>>>,
    }

    impl IDropSource_Impl for Source_Impl {
        fn QueryContinueDrag(&self, esc: BOOL, keys: MODIFIERKEYS_FLAGS) -> HRESULT {
            if esc.as_bool() {
                *self.reason.lock().expect("reason") = Some("esc");
                return DRAGDROP_S_CANCEL;
            }
            let (state, daemon_released) = {
                let st = self.status.lock().expect("status");
                (st.state.clone(), st.released)
            };
            match state.as_str() {
                "cancel" => {
                    *self.reason.lock().expect("reason") = Some("cancelled");
                    return DRAGDROP_S_CANCEL;
                }
                "failed" => {
                    *self.reason.lock().expect("reason") = Some("failed");
                    return DRAGDROP_S_CANCEL;
                }
                _ => {}
            }
            let released = keys.0 & MK_LBUTTON == 0 || daemon_released;
            if released && state == "ready" {
                DRAGDROP_S_DROP
            } else {
                // Held, or released while the bytes are still arriving: the
                // drop waits (user decision: wait on release).
                S_OK
            }
        }

        fn GiveFeedback(&self, _effect: DROPEFFECT) -> HRESULT {
            DRAGDROP_S_USEDEFAULTCURSORS
        }
    }

    unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
        match msg {
            WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT,
            WM_LBUTTONDOWN => {
                PRESSED.with(|p| p.set(true));
                0
            }
            _ => DefWindowProcW(hwnd, msg, w, l),
        }
    }

    fn mouse(flags: u32) {
        let input = INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dx: 0,
                    dy: 0,
                    mouseData: 0,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        };
        unsafe {
            SendInput(1, &input, std::mem::size_of::<INPUT>() as i32);
        }
    }

    fn lmb_down() -> bool {
        (unsafe { GetAsyncKeyState(VK_LBUTTON as i32) } as u16 & 0x8000) != 0
    }

    /// This thread's synchronous view of the left button: what
    /// `DoDragDrop` checks on entry (no button down there → `E_FAIL`).
    fn thread_lmb_down() -> bool {
        (unsafe { GetKeyState(VK_LBUTTON as i32) } as u16 & 0x8000) != 0
    }

    /// Mark the left button down in this thread's key state when the press
    /// was delivered elsewhere (already held, or the injected press raced a
    /// window change): `DoDragDrop` then starts, and the release still ends
    /// it through `QueryContinueDrag`'s daemon-reported `released`.
    fn force_thread_lmb_down() -> bool {
        let mut keys = [0u8; 256];
        unsafe {
            if GetKeyboardState(keys.as_mut_ptr()) == 0 {
                return false;
            }
            keys[VK_LBUTTON as usize] |= 0x80;
            SetKeyboardState(keys.as_ptr()) != 0
        }
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn pump_until_pressed(budget: std::time::Duration) {
        let start = std::time::Instant::now();
        while start.elapsed() < budget && !PRESSED.with(|p| p.get()) {
            let mut msg: MSG = unsafe { std::mem::zeroed() };
            while unsafe { PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) } != 0 {
                unsafe {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
            unsafe { MsgWaitForMultipleObjects(0, std::ptr::null(), 0, 10, QS_ALLINPUT) };
        }
    }

    /// Run the drag; returns the outcome token for the daemon.
    pub(super) fn run_drag(paths: Vec<String>, status: Arc<Mutex<Status>>) -> &'static str {
        if unsafe { OleInitialize(None) }.is_err() {
            tracing::warn!("drag source: OleInitialize refused");
            return "failed";
        }
        let outcome = run_inner(paths, status);
        unsafe { OleUninitialize() };
        outcome
    }

    fn run_inner(paths: Vec<String>, status: Arc<Mutex<Status>>) -> &'static str {
        let class = wide("TetherlyDragSource");
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
        // Cover the whole virtual screen: the cursor keeps moving while the
        // controller drags, so a window placed at "the cursor" is already
        // elsewhere when the press is injected (v0.3.0: the press hit some
        // other window and `DoDragDrop` failed with E_FAIL).
        let (vx, vy, vw, vh) = unsafe {
            (
                GetSystemMetrics(SM_XVIRTUALSCREEN),
                GetSystemMetrics(SM_YVIRTUALSCREEN),
                GetSystemMetrics(SM_CXVIRTUALSCREEN),
                GetSystemMetrics(SM_CYVIRTUALSCREEN),
            )
        };
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                class.as_ptr(),
                title.as_ptr(),
                WS_POPUP,
                vx,
                vy,
                vw.max(1),
                vh.max(1),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            )
        };
        if hwnd.is_null() {
            tracing::warn!("drag source: CreateWindowExW failed");
            return "failed";
        }
        unsafe {
            // Alpha 1: invisible, but still hit-tested, so the injected
            // press lands here and nowhere else.
            SetLayeredWindowAttributes(hwnd, 0, 1, LWA_ALPHA);
            ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        }
        // Let the window reach the hit-test tree before the press.
        pump_until_pressed(std::time::Duration::from_millis(30));
        let injected = !lmb_down();
        if injected {
            mouse(MOUSEEVENTF_LEFTDOWN);
            pump_until_pressed(std::time::Duration::from_millis(300));
        }
        let pressed = PRESSED.with(|p| p.get());
        let forced = !thread_lmb_down() && force_thread_lmb_down();
        tracing::info!(
            injected,
            pressed,
            forced,
            global_down = lmb_down(),
            thread_down = thread_lmb_down(),
            "drag source: press state before DoDragDrop"
        );
        unsafe {
            ShowWindow(hwnd, SW_HIDE);
            SetTimer(hwnd, 1, TICK_MS, None);
        }
        let reason = Arc::new(Mutex::new(None));
        let data: IDataObject = FileData {
            hdrop: super::hdrop_bytes(&paths),
        }
        .into();
        let source: IDropSource = Source {
            status: status.clone(),
            reason: reason.clone(),
        }
        .into();
        let mut effect = DROPEFFECT_NONE;
        let hr = unsafe { DoDragDrop(&data, &source, DROPEFFECT_COPY, &mut effect) };
        let outcome = if hr == DRAGDROP_S_DROP && effect != DROPEFFECT_NONE {
            "dropped"
        } else if hr == DRAGDROP_S_DROP || hr == DRAGDROP_S_CANCEL {
            reason.lock().expect("reason").unwrap_or("missed")
        } else {
            tracing::warn!(hr = hr.0, "drag source: DoDragDrop failed");
            "failed"
        };
        // The press this helper injected must not outlive the drag when the
        // controller already released (its button-up may have raced ahead).
        let released = status.lock().expect("status").released;
        if injected && lmb_down() && (released || outcome != "esc") {
            mouse(MOUSEEVENTF_LEFTUP);
        }
        unsafe {
            KillTimer(hwnd, 1);
            DestroyWindow(hwnd);
        }
        outcome
    }
}

#[cfg(test)]
mod tests {
    use super::hdrop_bytes;

    #[test]
    fn hdrop_payload_is_wide_and_double_nul_terminated() {
        let b = hdrop_bytes(&[r"C:\a.txt".into(), "b".into()]);
        assert_eq!(&b[0..4], &20u32.to_le_bytes());
        assert_eq!(&b[16..20], &1u32.to_le_bytes(), "fWide");
        let wide: Vec<u16> = b[20..]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        let expect: Vec<u16> = "C:\\a.txt\0b\0\0".encode_utf16().collect();
        assert_eq!(wide, expect);
    }
}
