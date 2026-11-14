// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Cross-platform system tray icon and background status presence.
//!
//! macOS: NSStatusBar status item with menu and dashboard launcher.
//! Windows: Shell_NotifyIconW notification area icon with popup menu and dashboard launcher.

pub struct TrayHandle {
    shutdown_tx: Option<std::sync::mpsc::Sender<()>>,
}

impl TrayHandle {
    pub fn stop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
}

impl Drop for TrayHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(target_os = "macos")]
mod macos_tray {
    use objc2::rc::{Allocated, Retained};
    use objc2::runtime::{AnyObject, Bool, NSObject, NSObjectProtocol, ProtocolObject};
    use objc2::{define_class, msg_send, sel, AnyThread, ClassType, DefinedClass, MainThreadOnly};
    use objc2_app_kit::{
        NSApplication, NSApplicationActivationPolicy, NSDragOperation, NSDraggingDestination,
        NSDraggingInfo, NSMenu, NSMenuItem, NSPasteboardTypeFileURL, NSStatusBar, NSView,
    };
    use objc2_foundation::{MainThreadMarker, NSArray, NSDictionary, NSRect, NSString, NSURL};

    pub(crate) struct TrayIvars {
        ui_port: u16,
    }

    /// Extract local file paths from a dropping pasteboard.
    ///
    /// Only `file:` URLs are accepted (the dropzone registers exclusively for
    /// `NSPasteboardTypeFileURL`); anything else is ignored, so a hostile or
    /// malformed drag can at worst deliver zero paths.
    fn file_paths_from_drag(sender: &ProtocolObject<dyn NSDraggingInfo>) -> Vec<String> {
        let pb = sender.draggingPasteboard();
        let classes = NSArray::from_slice(&[<NSURL as ClassType>::class()]);
        let options: Option<&NSDictionary<NSString, AnyObject>> = None;
        let urls: Retained<NSArray<AnyObject>> =
            match unsafe { pb.readObjectsForClasses_options(&classes, options) } {
                Some(arr) => arr,
                None => return Vec::new(),
            };
        let mut paths = Vec::new();
        for url in urls.iter() {
            // The dropzone registered for file URLs only; re-check the class
            // anyway so a malformed pasteboard item is skipped, not mis-cast.
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

    /// Fire the drag intent + confirm REST chain on a background thread so the
    /// main thread (AppKit run loop) never blocks on network I/O. The daemon's
    /// confirm handler streams the file and settles MOTW + reveal (ADR-137).
    fn dispatch_tray_drop(ui_port: u16, paths: Vec<String>) {
        std::thread::Builder::new()
            .name("tray-drop".into())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        tracing::warn!(error = %e, "tray drop: no tokio runtime");
                        return;
                    }
                };
                let result = runtime.block_on(async {
                    let intent = serde_json::json!({ "paths": paths });
                    let (code, body) = tetherly_loopback::loopback_post(
                        ui_port,
                        "/api/file/drag/intent",
                        &serde_json::to_vec(&intent).unwrap_or_default(),
                    )
                    .await?;
                    if code != 200 {
                        anyhow::bail!(
                            "drag intent failed ({code}): {}",
                            String::from_utf8_lossy(&body)
                        );
                    }
                    let resp: serde_json::Value = serde_json::from_slice(&body)?;
                    let drag_id = resp["signal"]
                        .get("drag_id")
                        .and_then(|v| v.as_u64())
                        .ok_or_else(|| anyhow::anyhow!("drag intent reply missing drag_id"))?;
                    let confirm = serde_json::json!({ "drag_id": drag_id });
                    // The confirm handler hashes every payload file before it
                    // answers — a 30s budget reports false failures on big
                    // drops while the transfer actually proceeds.
                    const DRAG_CONFIRM_CONNECT: std::time::Duration =
                        std::time::Duration::from_secs(5);
                    const DRAG_CONFIRM_REQUEST: std::time::Duration =
                        std::time::Duration::from_secs(300);
                    let (code, body) = tetherly_loopback::loopback_post_with_deadlines(
                        ui_port,
                        "/api/file/drag/confirm",
                        &serde_json::to_vec(&confirm).unwrap_or_default(),
                        DRAG_CONFIRM_CONNECT,
                        DRAG_CONFIRM_REQUEST,
                    )
                    .await?;
                    if code != 200 {
                        anyhow::bail!(
                            "drag confirm failed ({code}): {}",
                            String::from_utf8_lossy(&body)
                        );
                    }
                    Ok(())
                });
                if let Err(e) = result {
                    tracing::warn!(error = %e, "tray drop delivery failed");
                }
            })
            .expect("spawn tray-drop thread");
    }

    define_class!(
        #[unsafe(super(NSView))]
        // NSDraggingDestination requires main-thread-only objects (AppKit rule).
        #[thread_kind = MainThreadOnly]
        #[name = "TetherlyTrayDropView"]
        #[ivars = TrayIvars]
        pub(crate) struct TrayDropView;

        impl TrayDropView {
            #[unsafe(method_id(initWithFrame:))]
            fn init_with_frame(this: Allocated<Self>, frame: NSRect) -> Retained<Self> {
                let this = this.set_ivars(TrayIvars { ui_port: 0 });
                unsafe { msg_send![super(this), initWithFrame: frame] }
            }
        }

        unsafe impl NSObjectProtocol for TrayDropView {}

        unsafe impl NSDraggingDestination for TrayDropView {
            /// Accept the drag: report Copy so the cursor shows a live dropzone.
            #[unsafe(method(draggingEntered:))]
            fn dragging_entered(
                &self,
                _sender: &ProtocolObject<dyn NSDraggingInfo>,
            ) -> NSDragOperation {
                NSDragOperation::Copy
            }

            /// Keep the copy cursor while hovering over the tray icon.
            #[unsafe(method(draggingUpdated:))]
            fn dragging_updated(
                &self,
                _sender: &ProtocolObject<dyn NSDraggingInfo>,
            ) -> NSDragOperation {
                NSDragOperation::Copy
            }

            /// Commit the drop: extract file URLs and hand them to the daemon
            /// via the loopback drag REST chain (background thread). Returning
            /// `true` ends the drag session cleanly on our side; delivery
            /// progress lands in the outbox / transfer state.
            #[unsafe(method(performDragOperation:))]
            fn perform_drag_operation(&self, sender: &ProtocolObject<dyn NSDraggingInfo>) -> Bool {
                let paths = file_paths_from_drag(sender);
                if paths.is_empty() {
                    tracing::warn!("tray drop contained no readable file URLs");
                    return false.into();
                }
                let port = self.ivars().ui_port;
                dispatch_tray_drop(port, paths);
                true.into()
            }
        }
    );

    define_class!(
        #[unsafe(super(NSObject))]
        #[name = "TetherlyTrayTarget"]
        #[ivars = TrayIvars]
        pub(crate) struct TetherlyTrayTarget;

        impl TetherlyTrayTarget {
            #[unsafe(method(openDashboard:))]
            fn open_dashboard(&self, _sender: &NSMenuItem) {
                let port = self.ivars().ui_port;
                let url = crate::apitoken::dashboard_url(port, tetherly_loopback::api_token().as_deref());
                let _ = std::process::Command::new("open").arg(url).spawn();
            }

            /// ADR-137 §4.3: status-item click toggles the tray popover
            /// (peers + live transfers panel). The sender is the status
            /// item's button — the popover's positioning anchor.
            #[unsafe(method(togglePopover:))]
            fn toggle_popover(&self, sender: &objc2_app_kit::NSStatusBarButton) {
                crate::platform::popover::macos::toggle(self.ivars().ui_port, sender);
            }

            #[unsafe(method(openAccessibility:))]
            fn open_accessibility(&self, _sender: &NSMenuItem) {
                let _ = crate::platform::open_accessibility_settings();
            }

            #[unsafe(method(quitTetherly:))]
            fn quit_tetherly(&self, _sender: &NSMenuItem) {
                std::process::exit(0);
            }
        }

        unsafe impl NSObjectProtocol for TetherlyTrayTarget {}
    );

    /// Build the tray dropzone view covering the full bounds of the status
    /// item's button. The view is the registered dragging destination; the
    /// button stays clickable underneath (the view is non-interfering for
    /// mouse events because a plain NSView passes clicks through its own
    /// hit-testing only when it has no event handlers — the menu still works
    /// since dragging is the only gesture this view intercepts).
    fn attach_drop_view(
        button: &objc2_app_kit::NSStatusBarButton,
        ui_port: u16,
        mtm: MainThreadMarker,
    ) {
        let frame = button.bounds();
        let drop = TrayDropView::alloc(mtm).set_ivars(TrayIvars { ui_port });
        let drop: Retained<TrayDropView> = unsafe { msg_send![super(drop), initWithFrame: frame] };
        let file_type = unsafe { NSPasteboardTypeFileURL };
        drop.registerForDraggedTypes(&NSArray::from_slice(&[file_type]));
        // Auto-resize with the button so the dropzone never shrinks away.
        drop.setAutoresizingMask(
            objc2_app_kit::NSAutoresizingMaskOptions::ViewWidthSizable
                | objc2_app_kit::NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        button.addSubview(&drop);
        // Keep the drop view alive for the process lifetime.
        std::mem::forget(drop);
    }

    pub fn run_macos_tray_on_main_thread(
        ui_port: u16,
        device_alias: String,
        rx: std::sync::mpsc::Receiver<()>,
    ) {
        let Some(mtm) = MainThreadMarker::new() else {
            tracing::warn!("macos_tray: main thread marker unavailable; running headless");
            let _ = rx.recv();
            return;
        };
        let app = NSApplication::sharedApplication(mtm);
        app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);

        let bar = NSStatusBar::systemStatusBar();
        let item = bar.statusItemWithLength(-1.0);
        let target = TetherlyTrayTarget::alloc().set_ivars(TrayIvars { ui_port });
        let target: Retained<TetherlyTrayTarget> = unsafe { msg_send![super(target), init] };
        let target_ref: &AnyObject = target.as_ref();

        if let Some(button) = item.button(mtm) {
            let title = NSString::from_str("⇄ Tetherly");
            button.setTitle(&title);
            // ADR-137 Engine I: the tray icon is a live dropzone — dragging
            // files onto it sends them to the connected peer via the loopback
            // drag REST chain (intent → confirm → stream → MOTW settle).
            attach_drop_view(&button, ui_port, mtm);
        }

        let menu = NSMenu::new(mtm);
        let status_title = NSString::from_str(&format!("🟢 Tetherly: 运行中 ({device_alias})"));
        let status_item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                mtm.alloc(),
                &status_title,
                None,
                &NSString::from_str(""),
            )
        };
        status_item.setEnabled(false);
        menu.addItem(&status_item);

        let sep1 = NSMenuItem::separatorItem(mtm);
        menu.addItem(&sep1);

        let dash_title = NSString::from_str(&format!("🌐 打开管理控制台 (127.0.0.1:{})", ui_port));
        let dash_item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                mtm.alloc(),
                &dash_title,
                Some(sel!(openDashboard:)),
                &NSString::from_str(""),
            )
        };
        unsafe { dash_item.setTarget(Some(target_ref)) };
        dash_item.setEnabled(true);
        menu.addItem(&dash_item);

        // ADR-137 §4.3: opens the popover panel (peers + live transfers)
        // anchored to the status item.
        let panel_title = NSString::from_str("📊 状态面板");
        let panel_item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                mtm.alloc(),
                &panel_title,
                Some(sel!(togglePopover:)),
                &NSString::from_str(""),
            )
        };
        unsafe { panel_item.setTarget(Some(target_ref)) };
        panel_item.setEnabled(true);
        menu.addItem(&panel_item);

        let edge_title = NSString::from_str("🖥️ 漫游拓扑: Deskflow 对齐 (边缘探测)");
        let edge_item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                mtm.alloc(),
                &edge_title,
                None,
                &NSString::from_str(""),
            )
        };
        edge_item.setEnabled(false);
        menu.addItem(&edge_item);

        let keymap_title = NSString::from_str("⌨️ 键位映射: Mac ↔ Windows (Cmd ↔ Ctrl)");
        let keymap_item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                mtm.alloc(),
                &keymap_title,
                None,
                &NSString::from_str(""),
            )
        };
        keymap_item.setEnabled(false);
        menu.addItem(&keymap_item);

        let trusted = crate::platform::is_accessibility_trusted();
        let ax_title = if trusted {
            NSString::from_str("🔒 辅助功能权限: 已授权")
        } else {
            NSString::from_str("⚠️ 辅助功能权限: 未授权 (点击授权)")
        };
        let ax_item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                mtm.alloc(),
                &ax_title,
                if trusted {
                    None
                } else {
                    Some(sel!(openAccessibility:))
                },
                &NSString::from_str(""),
            )
        };
        if !trusted {
            unsafe { ax_item.setTarget(Some(target_ref)) };
            ax_item.setEnabled(true);
        } else {
            ax_item.setEnabled(false);
        }
        menu.addItem(&ax_item);

        let sep2 = NSMenuItem::separatorItem(mtm);
        menu.addItem(&sep2);

        let quit_title = NSString::from_str("❌ 退出 Tetherly");
        let quit_item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                mtm.alloc(),
                &quit_title,
                Some(sel!(quitTetherly:)),
                &NSString::from_str(""),
            )
        };
        unsafe { quit_item.setTarget(Some(target_ref)) };
        quit_item.setEnabled(true);
        menu.addItem(&quit_item);

        item.setMenu(Some(&menu));

        std::thread::spawn(move || {
            let _ = rx.recv();
            std::process::exit(0);
        });

        app.run();
    }
}

#[cfg(target_os = "macos")]
pub use macos_tray::run_macos_tray_on_main_thread;

#[cfg(target_os = "macos")]
pub fn spawn_tray(_ui_port: u16, _device_alias: String) -> Option<TrayHandle> {
    None
}

#[cfg(windows)]
mod windows_tray {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use std::sync::atomic::{AtomicU16, Ordering};
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
    use windows_sys::Win32::UI::Shell::{
        Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow,
        DispatchMessageW, GetCursorPos, GetMessageW, LoadIconW, PostQuitMessage, RegisterClassExW,
        SetForegroundWindow, TrackPopupMenuEx, TranslateMessage, HICON, HMENU, IDI_APPLICATION,
        MF_GRAYED, MF_SEPARATOR, MF_STRING, MSG, TPM_RIGHTBUTTON, WM_APP, WM_CLOSE, WM_COMMAND,
        WM_DESTROY, WM_LBUTTONDBLCLK, WM_LBUTTONUP, WM_RBUTTONUP, WNDCLASSEXW,
    };

    const WM_TRAYICON: u32 = WM_APP + 42;
    const IDM_STATUS: usize = 1001;
    const IDM_PANEL: usize = 1005;
    const IDM_DASHBOARD: usize = 1002;
    const IDM_ROAMING: usize = 1003;
    const IDM_EXIT: usize = 1004;

    fn to_wide(s: &str) -> Vec<u16> {
        OsStr::new(s).encode_wide().chain(Some(0)).collect()
    }

    static GLOBAL_UI_PORT: AtomicU16 = AtomicU16::new(45716);

    unsafe extern "system" fn tray_wnd_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match msg {
            WM_TRAYICON => {
                let event = lparam as u32;
                if event == WM_LBUTTONUP {
                    // ADR-137 §4.3: single left-click toggles the status
                    // popover (peers + live transfers panel).
                    popover::toggle_popover();
                } else if event == WM_LBUTTONDBLCLK {
                    let port = GLOBAL_UI_PORT.load(Ordering::Relaxed);
                    let url = crate::apitoken::dashboard_url(
                        port,
                        tetherly_loopback::api_token().as_deref(),
                    );
                    let _ = std::process::Command::new("cmd")
                        .args(["/C", "start", &url])
                        .spawn();
                } else if event == WM_RBUTTONUP {
                    let mut pt = POINT { x: 0, y: 0 };
                    GetCursorPos(&mut pt);
                    SetForegroundWindow(hwnd);

                    let port = GLOBAL_UI_PORT.load(Ordering::Relaxed);
                    let menu: HMENU = CreatePopupMenu();
                    let status_str = to_wide("🟢 Tetherly: 运行中");
                    let panel_str = to_wide("📊 状态面板");
                    let dash_str = to_wide(&format!("🌐 打开管理控制台 (127.0.0.1:{port})"));
                    let roam_str = to_wide("🖥️ Deskflow 漫游: 活跃");
                    let exit_str = to_wide("❌ 退出 Tetherly");

                    AppendMenuW(menu, MF_STRING | MF_GRAYED, IDM_STATUS, status_str.as_ptr());
                    AppendMenuW(menu, MF_STRING, IDM_PANEL, panel_str.as_ptr());
                    AppendMenuW(menu, MF_STRING, IDM_DASHBOARD, dash_str.as_ptr());
                    AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
                    AppendMenuW(menu, MF_STRING | MF_GRAYED, IDM_ROAMING, roam_str.as_ptr());
                    AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
                    AppendMenuW(menu, MF_STRING, IDM_EXIT, exit_str.as_ptr());

                    TrackPopupMenuEx(menu, TPM_RIGHTBUTTON, pt.x, pt.y, hwnd, std::ptr::null());
                    DestroyMenu(menu);
                }
                0
            }
            WM_COMMAND => {
                let id = wparam & 0xFFFF;
                if id == IDM_PANEL {
                    popover::toggle_popover();
                } else if id == IDM_DASHBOARD {
                    let port = GLOBAL_UI_PORT.load(Ordering::Relaxed);
                    let url = crate::apitoken::dashboard_url(
                        port,
                        tetherly_loopback::api_token().as_deref(),
                    );
                    let _ = std::process::Command::new("cmd")
                        .args(["/C", "start", &url])
                        .spawn();
                } else if id == IDM_EXIT {
                    std::process::exit(0);
                }
                0
            }
            WM_CLOSE => {
                DestroyWindow(hwnd);
                0
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                0
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }

    /// ADR-137 §4.3 status popover: a borderless topmost popup window that
    /// renders peer status + live transfers, fed exclusively by the loopback
    /// REST plane. Owned by the tray message pump thread; a WM_TIMER refresh
    /// kicks a background fetch so the STA pump never blocks on I/O.
    mod popover {
        use std::ffi::OsStr;
        use std::os::windows::ffi::OsStrExt;
        use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
        use std::sync::Mutex;
        use windows_sys::Win32::Foundation::{
            COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
        };
        use windows_sys::Win32::Graphics::Gdi::{
            BeginPaint, CreateSolidBrush, DeleteObject, DrawTextW, EndPaint, FillRect,
            InvalidateRect, SelectObject, SetBkMode, SetTextColor, DEFAULT_GUI_FONT, DT_LEFT,
            DT_NOPREFIX, DT_WORDBREAK, HBRUSH, PAINTSTRUCT, TRANSPARENT,
        };
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, GetCursorPos, KillTimer, RegisterClassExW,
            SetForegroundWindow, SetTimer, ShowWindow, SystemParametersInfoW, SPI_GETWORKAREA,
            SW_HIDE, SW_SHOWNOACTIVATE, WM_KILLFOCUS, WM_PAINT, WM_TIMER, WNDCLASSEXW,
            WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
        };

        const POPOVER_CLASS: &str = "TetherlyPopoverClass";
        const TIMER_ID: usize = 0xC0FF_EE01;
        const REFRESH_MS: u32 = 1000;

        // Panel geometry (ADR-137 §4.3, mirrors the macOS popover).
        const PANEL_W: i32 = 340;
        const PANEL_H: i32 = 260;

        // ADR-137 palette: dark panel, light text, green/red accents via
        // emoji glyphs in the text itself.
        const BG_COLOR: COLORREF = 0x00201F1E; // BGR dark
        const FG_COLOR: COLORREF = 0x00EDEBE9; // BGR light

        /// Rendered rows shared paint ← fetch. A plain Mutex of a small
        /// String is fine: the paint path holds it for microseconds.
        static ROWS: Mutex<String> = Mutex::new(String::new());
        static POPUP_HWND: AtomicIsize = AtomicIsize::new(0);
        static VISIBLE: AtomicBool = AtomicBool::new(false);
        /// At most one fetch in flight: the 1s WM_TIMER must not pile up
        /// threads/runtimes when the daemon answers slower than the tick, and
        /// a late old fetch must not overwrite fresher rows.
        static FETCH_IN_FLIGHT: AtomicBool = AtomicBool::new(false);

        fn to_wide(s: &str) -> Vec<u16> {
            OsStr::new(s).encode_wide().chain(Some(0)).collect()
        }

        /// Position under the tray cursor, inside the monitor work area
        /// (the tray can sit on any edge — clamp into the work area).
        fn place_near_tray() -> (i32, i32) {
            let mut pt = POINT { x: 0, y: 0 };
            unsafe { GetCursorPos(&mut pt) };
            let mut wa = RECT {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            };
            let ok = unsafe {
                SystemParametersInfoW(
                    SPI_GETWORKAREA,
                    0,
                    &mut wa as *mut RECT as *mut core::ffi::c_void,
                    0,
                )
            };
            if ok == 0 {
                wa = RECT {
                    left: 0,
                    top: 0,
                    right: PANEL_W * 4,
                    bottom: PANEL_H * 4,
                };
            }
            // Prefer below the cursor; flip up when the tray is at the bottom.
            let mut x = pt.x - PANEL_W / 2;
            let mut y = pt.y + 8;
            if y + PANEL_H > wa.bottom {
                y = pt.y - PANEL_H - 8;
            }
            x = x.clamp(wa.left, (wa.right - PANEL_W).max(wa.left));
            y = y.clamp(wa.top, (wa.bottom - PANEL_H).max(wa.top));
            (x, y)
        }

        unsafe extern "system" fn popover_wnd_proc(
            hwnd: HWND,
            msg: u32,
            wparam: WPARAM,
            lparam: LPARAM,
        ) -> LRESULT {
            match msg {
                WM_PAINT => unsafe {
                    let mut ps = std::mem::zeroed::<PAINTSTRUCT>();
                    let hdc = BeginPaint(hwnd, &mut ps);
                    if hdc.is_null() {
                        return 0;
                    }
                    let brush: HBRUSH = CreateSolidBrush(BG_COLOR);
                    FillRect(hdc, &ps.rcPaint, brush);
                    DeleteObject(brush as *mut _);
                    SetBkMode(hdc, TRANSPARENT as i32);
                    SetTextColor(hdc, FG_COLOR);
                    // Use the default GUI font for a native look.
                    let font = default_gui_font();
                    let old = SelectObject(hdc, font);
                    let rows = ROWS.lock().unwrap().clone();
                    let text = to_wide(&rows);
                    let mut rc = ps.rcPaint;
                    rc.left += 12;
                    rc.top += 10;
                    rc.right -= 12;
                    rc.bottom -= 10;
                    DrawTextW(
                        hdc,
                        text.as_ptr(),
                        (text.len().saturating_sub(1)) as i32,
                        &mut rc,
                        DT_LEFT | DT_NOPREFIX | DT_WORDBREAK,
                    );
                    SelectObject(hdc, old);
                    EndPaint(hwnd, &ps);
                    0
                },
                WM_TIMER if wparam as u32 as usize == TIMER_ID => {
                    // Kick a background fetch; the pump thread must not block
                    // on the loopback round-trips.
                    spawn_fetch();
                    0
                }
                WM_KILLFOCUS => {
                    // Clicked outside → hide, matching Transient popover
                    // behavior on macOS.
                    hide();
                    0
                }
                _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
            }
        }

        unsafe fn default_gui_font() -> windows_sys::Win32::Graphics::Gdi::HGDIOBJ {
            windows_sys::Win32::Graphics::Gdi::GetStockObject(DEFAULT_GUI_FONT)
        }

        fn spawn_fetch() {
            // One fetch at a time; the timer simply skips this tick when the
            // previous round-trip is still draining.
            if FETCH_IN_FLIGHT
                .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                .is_err()
            {
                return;
            }
            let port = super::GLOBAL_UI_PORT.load(Ordering::Relaxed);
            let spawned = std::thread::Builder::new()
                .name("tray-popover-fetch".into())
                .spawn(move || {
                    let text = (|| {
                        let runtime = tokio::runtime::Builder::new_current_thread()
                            .enable_all()
                            .build()
                            .ok()?;
                        Some(runtime.block_on(async { render_rows(port).await }))
                    })();
                    if let Some(text) = text {
                        if let Ok(mut rows) = ROWS.lock() {
                            *rows = text;
                        }
                        let hwnd = POPUP_HWND.load(Ordering::Relaxed);
                        if hwnd != 0 && VISIBLE.load(Ordering::Relaxed) {
                            unsafe {
                                InvalidateRect(hwnd as HWND, std::ptr::null(), 0);
                            }
                        }
                    }
                    FETCH_IN_FLIGHT.store(false, Ordering::SeqCst);
                });
            if spawned.is_err() {
                // No worker will ever run — release the gate or fetches stop
                // forever.
                FETCH_IN_FLIGHT.store(false, Ordering::SeqCst);
            }
        }

        /// Same rows as the macOS popover (ADR-137 §4.3): peer status line +
        /// live transfer list with progress bars.
        async fn render_rows(port: u16) -> String {
            let mut out = String::new();

            match fetch_json(port, "/api/peers").await {
                Some(v) => match v.get("live").and_then(|l| l.as_array()) {
                    Some(arr) if !arr.is_empty() => {
                        out.push_str("Peers:\n");
                        for p in arr {
                            let name = p["alias"]
                                .as_str()
                                .or_else(|| p["name"].as_str())
                                .unwrap_or("?");
                            let platform = p["platform"].as_str().unwrap_or("");
                            out.push_str(&format!("  🟢 {name} ({platform})\n"));
                        }
                    }
                    _ => out.push_str("未连接对端 — 拖文件到托盘图标可直接投递\n"),
                },
                None => out.push_str("⚠️ 无法连接本地守护进程\n"),
            }

            out.push('\n');

            match fetch_json(port, "/api/file/transfers").await {
                Some(v) => {
                    let items = v.get("transfers").and_then(|t| t.as_array());
                    let empty = items.map(|a| a.is_empty()).unwrap_or(true);
                    if empty {
                        out.push_str("传输: 空 — 拖文件到托盘 ⇄\n");
                    } else if let Some(arr) = items {
                        out.push_str("传输:\n");
                        for t in arr {
                            let name = t["filename"].as_str().unwrap_or("file");
                            let total = t["total"].as_u64().unwrap_or(0);
                            let done = t["transferred"].as_u64().unwrap_or(0);
                            let speed = t["speed_bps"].as_u64().unwrap_or(0);
                            let phase = t["phase"].as_str().unwrap_or("");
                            match phase {
                                "Completed" => {
                                    out.push_str(&format!("  ✅ {name} ({})\n", fmt_bytes(total)))
                                }
                                "Failed" => {
                                    let e = t["error"].as_str().unwrap_or("failed");
                                    out.push_str(&format!("  ❌ {name} — {e}\n"));
                                }
                                _ => {
                                    let frac = if total > 0 {
                                        done as f64 / total as f64
                                    } else {
                                        0.0
                                    };
                                    let pct = (frac * 100.0) as u64;
                                    let arrow = if phase == "Verifying" { "🔍" } else { "⇄" };
                                    out.push_str(&format!(
                                        "  {arrow} {name}\n    {} {}% ({}/{}, {})\n",
                                        progress_bar(frac),
                                        pct,
                                        fmt_bytes(done),
                                        fmt_bytes(total),
                                        fmt_bps(speed)
                                    ));
                                }
                            }
                        }
                    }
                }
                None => out.push_str("传输: 无法读取状态\n"),
            }
            out
        }

        async fn fetch_json(port: u16, path: &str) -> Option<serde_json::Value> {
            let (code, body) = tetherly_loopback::loopback_get(port, path).await.ok()?;
            if code != 200 {
                return None;
            }
            serde_json::from_slice(&body).ok()
        }

        fn fmt_bytes(n: u64) -> String {
            const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
            let mut v = n as f64;
            let mut u = 0;
            while v >= 1024.0 && u < UNITS.len() - 1 {
                v /= 1024.0;
                u += 1;
            }
            if u == 0 {
                format!("{n} B")
            } else {
                format!("{v:.1} {}", UNITS[u])
            }
        }

        fn fmt_bps(bps: u64) -> String {
            if bps == 0 {
                "—".to_string()
            } else {
                format!("{}/s", fmt_bytes(bps))
            }
        }

        fn progress_bar(done: f64) -> String {
            const WIDTH: usize = 10;
            let filled = ((done * WIDTH as f64).round() as usize).clamp(0, WIDTH);
            format!("{}{}", "█".repeat(filled), "░".repeat(WIDTH - filled))
        }

        fn hide() {
            VISIBLE.store(false, Ordering::Relaxed);
            let hwnd = POPUP_HWND.load(Ordering::Relaxed);
            if hwnd != 0 {
                unsafe {
                    KillTimer(hwnd as HWND, TIMER_ID);
                    ShowWindow(hwnd as HWND, SW_HIDE);
                }
            }
        }

        /// Toggle the popover window near the tray icon. Called on the tray
        /// pump thread (WM_TRAYICON left-click / IDM_PANEL), which owns the
        /// message loop — safe for window + timer creation here.
        pub(super) fn toggle_popover() {
            let hwnd = POPUP_HWND.load(Ordering::Relaxed);
            if hwnd != 0 && VISIBLE.load(Ordering::Relaxed) {
                hide();
                return;
            }
            unsafe {
                let hwnd = ensure_window(hwnd);
                POPUP_HWND.store(hwnd as isize, Ordering::Relaxed);
                let (x, y) = place_near_tray();
                let _ = set_window_pos_top(hwnd, x, y);
                SetForegroundWindow(hwnd);
                ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                VISIBLE.store(true, Ordering::Relaxed);
                SetTimer(hwnd, TIMER_ID, REFRESH_MS, None);
                // First paint immediately.
                spawn_fetch();
            }
        }

        unsafe fn ensure_window(existing: isize) -> HWND {
            if existing != 0 {
                return existing as HWND;
            }
            let class_name = to_wide(POPOVER_CLASS);
            let wndclass = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                style: 0,
                lpfnWndProc: Some(popover_wnd_proc),
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: std::ptr::null_mut(),
                hIcon: std::ptr::null_mut(),
                hCursor: std::ptr::null_mut(),
                hbrBackground: std::ptr::null_mut(),
                lpszMenuName: std::ptr::null(),
                lpszClassName: class_name.as_ptr(),
                hIconSm: std::ptr::null_mut(),
            };
            RegisterClassExW(&wndclass);
            CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
                class_name.as_ptr(),
                to_wide("Tetherly Status").as_ptr(),
                WS_POPUP,
                0,
                0,
                PANEL_W,
                PANEL_H,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            )
        }

        /// `SetWindowPos` wrapper (HWND_TOPMOST, no-activate, show).
        unsafe fn set_window_pos_top(hwnd: HWND, x: i32, y: i32) -> bool {
            let hwnd_insert_after = -1isize as HWND; // HWND_TOPMOST
            let swp_noactivate: u32 = 0x0010;
            let swp_showwindow: u32 = 0x0040;
            windows_sys::Win32::UI::WindowsAndMessaging::SetWindowPos(
                hwnd,
                hwnd_insert_after,
                x,
                y,
                PANEL_W,
                PANEL_H,
                swp_noactivate | swp_showwindow,
            ) != 0
        }
    }

    pub fn run_windows_tray(ui_port: u16, rx: std::sync::mpsc::Receiver<()>) {
        unsafe {
            GLOBAL_UI_PORT.store(ui_port, Ordering::Relaxed);

            let class_name = to_wide("TetherlyTrayClass");
            let icon: HICON = LoadIconW(std::ptr::null_mut(), IDI_APPLICATION);

            let wndclass = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                style: 0,
                lpfnWndProc: Some(tray_wnd_proc),
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: std::ptr::null_mut(),
                hIcon: icon,
                hCursor: std::ptr::null_mut(),
                hbrBackground: std::ptr::null_mut(),
                lpszMenuName: std::ptr::null(),
                lpszClassName: class_name.as_ptr(),
                hIconSm: icon,
            };

            RegisterClassExW(&wndclass);

            let hwnd = CreateWindowExW(
                0,
                class_name.as_ptr(),
                class_name.as_ptr(),
                0,
                0,
                0,
                0,
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            );

            if hwnd.is_null() {
                return;
            }

            // ADR-137 Engine I: the tray window is a live dropzone — dropped
            // files stream to the connected peer through the loopback drag
            // REST chain. Best-effort: a failed registration leaves a working
            // (non-droppable) tray.
            drop_target::register_tray_drop_target(hwnd, ui_port);

            let mut nid = std::mem::zeroed::<NOTIFYICONDATAW>();
            nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
            nid.hWnd = hwnd;
            nid.uID = 100;
            nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
            nid.uCallbackMessage = WM_TRAYICON;
            nid.hIcon = icon;

            let tip = to_wide("Tetherly (Running) - Deskflow Roaming");
            let len = tip.len().min(nid.szTip.len() - 1);
            nid.szTip[..len].copy_from_slice(&tip[..len]);

            Shell_NotifyIconW(NIM_ADD, &nid);

            let hwnd_send = hwnd as usize;
            std::thread::spawn(move || {
                let _ = rx.recv();
                let hwnd = hwnd_send as HWND;
                windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(hwnd, WM_CLOSE, 0, 0);
            });

            let mut msg = std::mem::zeroed::<MSG>();
            while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }

            Shell_NotifyIconW(NIM_DELETE, &nid);
        }
    }

    /// OLE drop-target wiring for the tray window, kept in its own module so
    /// the `windows` crate types never mix with the `windows-sys` types the
    /// tray message pump above uses (the two crates' HWND/handle newtypes are
    /// distinct).
    mod drop_target {
        use windows::core::implement;
        use windows::Win32::Foundation::POINTL;
        use windows::Win32::System::Com::{
            CoInitializeEx, CoUninitialize, IDataObject, COINIT_APARTMENTTHREADED, FORMATETC,
        };
        use windows::Win32::System::Ole::{
            IDropTarget, IDropTarget_Impl, RegisterDragDrop, ReleaseStgMedium, RevokeDragDrop,
            DROPEFFECT,
        };
        use windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS;
        use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};

        // CF_HDROP: the standard "list of files" drag/clipboard format.
        const CF_HDROP: u16 = 15;
        const DROPEFFECT_COPY: DROPEFFECT = DROPEFFECT(1);

        /// Extract file paths from an OLE data object carrying a `CF_HDROP`.
        ///
        /// Bound-checked: `DragQueryFileW` with `None` as the buffer returns the
        /// item count and per-item length, so a malformed payload cannot
        /// overflow. `ReleaseStgMedium` frees the transferred HGLOBAL.
        fn paths_from_data_object(data: &IDataObject) -> Vec<String> {
            // DVASPECT_CONTENT = 1, TYMED_HGLOBAL = 1 (raw u32 fields in 0.59).
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
            // u32::MAX asks for the item count; a null buffer asks for a name length.
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

        /// Fire the drag intent + confirm REST chain on a background thread so
        /// the STA message loop never blocks on network I/O. The daemon's
        /// confirm handler streams the file and settles MOTW + reveal
        /// (ADR-137); mirrors the macOS tray drop path.
        fn dispatch_tray_drop(ui_port: u16, paths: Vec<String>) {
            std::thread::Builder::new()
                .name("tray-drop".into())
                .spawn(move || {
                    let runtime = match tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                    {
                        Ok(rt) => rt,
                        Err(e) => {
                            tracing::warn!(error = %e, "tray drop: no tokio runtime");
                            return;
                        }
                    };
                    let result = runtime.block_on(async {
                        let intent = serde_json::json!({ "paths": paths });
                        let (code, body) = tetherly_loopback::loopback_post(
                            ui_port,
                            "/api/file/drag/intent",
                            &serde_json::to_vec(&intent).unwrap_or_default(),
                        )
                        .await?;
                        if code != 200 {
                            anyhow::bail!(
                                "drag intent failed ({code}): {}",
                                String::from_utf8_lossy(&body)
                            );
                        }
                        let resp: serde_json::Value = serde_json::from_slice(&body)?;
                        let drag_id = resp["signal"]
                            .get("drag_id")
                            .and_then(|v| v.as_u64())
                            .ok_or_else(|| anyhow::anyhow!("drag intent reply missing drag_id"))?;
                        let confirm = serde_json::json!({ "drag_id": drag_id });
                        // Wide budget: confirm hashes the payload before it
                        // answers (see the macOS drop path above).
                        const DRAG_CONFIRM_CONNECT: std::time::Duration =
                            std::time::Duration::from_secs(5);
                        const DRAG_CONFIRM_REQUEST: std::time::Duration =
                            std::time::Duration::from_secs(300);
                        let (code, body) = tetherly_loopback::loopback_post_with_deadlines(
                            ui_port,
                            "/api/file/drag/confirm",
                            &serde_json::to_vec(&confirm).unwrap_or_default(),
                            DRAG_CONFIRM_CONNECT,
                            DRAG_CONFIRM_REQUEST,
                        )
                        .await?;
                        if code != 200 {
                            anyhow::bail!(
                                "drag confirm failed ({code}): {}",
                                String::from_utf8_lossy(&body)
                            );
                        }
                        Ok(())
                    });
                    if let Err(e) = result {
                        tracing::warn!(error = %e, "tray drop delivery failed");
                    }
                })
                .expect("spawn tray-drop thread");
        }

        /// OLE drop target for the tray window: accepts dropped files and
        /// forwards them to the local daemon over the loopback drag REST chain.
        #[implement(IDropTarget)]
        struct TrayDropTarget {
            ui_port: u16,
        }

        impl IDropTarget_Impl for TrayDropTarget_Impl {
            fn DragEnter(
                &self,
                _pdataobj: windows::core::Ref<'_, IDataObject>,
                _grfkeystate: MODIFIERKEYS_FLAGS,
                _pt: &POINTL,
                pdweffect: *mut DROPEFFECT,
            ) -> windows::core::Result<()> {
                unsafe {
                    *pdweffect = DROPEFFECT_COPY;
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
                    *pdweffect = DROPEFFECT_COPY;
                }
                Ok(())
            }

            fn DragLeave(&self) -> windows::core::Result<()> {
                Ok(())
            }

            fn Drop(
                &self,
                pdataobj: windows::core::Ref<'_, IDataObject>,
                _grfkeystate: MODIFIERKEYS_FLAGS,
                _pt: &POINTL,
                pdweffect: *mut DROPEFFECT,
            ) -> windows::core::Result<()> {
                unsafe {
                    *pdweffect = DROPEFFECT_COPY;
                }
                let Ok(data) = pdataobj.ok() else {
                    return Ok(());
                };
                let paths = paths_from_data_object(data);
                if paths.is_empty() {
                    tracing::warn!("tray drop contained no readable file paths");
                    return Ok(());
                }
                dispatch_tray_drop(self.ui_port, paths);
                Ok(())
            }
        }

        /// Make the tray window a live dropzone: init COM as STA (the tray
        /// thread owns the message pump), register the drop target, and hand
        /// back a revoker for teardown.
        pub(super) fn register_tray_drop_target(
            hwnd: windows_sys::Win32::Foundation::HWND,
            ui_port: u16,
        ) -> Option<impl FnOnce()> {
            // STA is mandatory for OLE drag-and-drop; the tray thread runs a
            // message pump for its whole lifetime, satisfying the STA
            // requirements.
            if unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_err() {
                return None;
            }
            // windows-sys HWND is a bare pointer; the windows crate wraps it.
            let hwnd_win = windows::Win32::Foundation::HWND(hwnd);
            let target: IDropTarget = TrayDropTarget { ui_port }.into();
            unsafe {
                RegisterDragDrop(hwnd_win, &target).ok()?;
            }
            Some(move || unsafe {
                let _ = RevokeDragDrop(hwnd_win);
                CoUninitialize();
            })
        }
    }
}

#[cfg(windows)]
pub fn spawn_tray(ui_port: u16, _device_alias: String) -> Option<TrayHandle> {
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    std::thread::Builder::new()
        .name("tetherly-tray".into())
        .spawn(move || {
            windows_tray::run_windows_tray(ui_port, rx);
        })
        .ok()?;

    Some(TrayHandle {
        shutdown_tx: Some(tx),
    })
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn spawn_tray(_ui_port: u16, _device_alias: String) -> Option<TrayHandle> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tray_spawn_and_stop() {
        let handle = spawn_tray(45716, "TEST-PEER".into());
        #[cfg(windows)]
        {
            assert!(handle.is_some());
            if let Some(mut h) = handle {
                h.stop();
            }
        }
        #[cfg(not(windows))]
        {
            assert!(handle.is_none());
        }
    }
}
