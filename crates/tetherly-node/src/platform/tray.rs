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
    use objc2::rc::Retained;
    use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol};
    use objc2::{define_class, msg_send, sel, AnyThread, DefinedClass};
    use objc2_app_kit::{
        NSApplication, NSApplicationActivationPolicy, NSMenu, NSMenuItem, NSStatusBar,
    };
    use objc2_foundation::{MainThreadMarker, NSString};

    pub(crate) struct TrayIvars {
        ui_port: u16,
    }

    define_class!(
        #[unsafe(super(NSObject))]
        #[name = "TetherlyTrayTarget"]
        #[ivars = TrayIvars]
        pub(crate) struct TetherlyTrayTarget;

        impl TetherlyTrayTarget {
            #[unsafe(method(openDashboard:))]
            fn open_dashboard(&self, _sender: &NSMenuItem) {
                let port = self.ivars().ui_port;
                let url = format!("http://127.0.0.1:{port}");
                let _ = std::process::Command::new("open").arg(url).spawn();
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
        if let Some(button) = item.button(mtm) {
            let title = NSString::from_str("⇄ Tetherly");
            button.setTitle(&title);
        }

        let target = TetherlyTrayTarget::alloc().set_ivars(TrayIvars { ui_port });
        let target: Retained<TetherlyTrayTarget> = unsafe { msg_send![super(target), init] };
        let target_ref: &AnyObject = target.as_ref();

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
                if event == WM_LBUTTONUP || event == WM_LBUTTONDBLCLK {
                    let port = GLOBAL_UI_PORT.load(Ordering::Relaxed);
                    let url = format!("http://127.0.0.1:{port}");
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
                    let dash_str = to_wide(&format!("🌐 打开管理控制台 (127.0.0.1:{port})"));
                    let roam_str = to_wide("🖥️ Deskflow 漫游: 活跃");
                    let exit_str = to_wide("❌ 退出 Tetherly");

                    AppendMenuW(menu, MF_STRING | MF_GRAYED, IDM_STATUS, status_str.as_ptr());
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
                if id == IDM_DASHBOARD {
                    let port = GLOBAL_UI_PORT.load(Ordering::Relaxed);
                    let url = format!("http://127.0.0.1:{port}");
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
