//! Windows tray panel (§226): the left-click popup that hosts the SPA
//! `#tray` route in WebView2 — the Windows twin of the macOS `NSPanel` in
//! `tray.rs`. Everything here runs on the tray thread (STA + message pump):
//! every COM call, every WebView2 completion/event callback, and the panel
//! state, which is a thread-local that never crosses threads.
//!
//! Two `windows` majors meet in this file. The window plumbing stays on
//! `windows-sys` 0.59 like the rest of the tray; `webview2-com` 0.39 is
//! pinned to `windows` 0.62 (`windows062`), so only the WebView2 handshake
//! names that copy. They touch at exactly one point: the raw HWND pointer.
//!
//! Failure policy: WebView2 is best-effort. If the runtime is missing or
//! refuses this process (SYSTEM sandbox, broken install), the panel marks
//! itself unavailable and the tray's left click falls back to the
//! browser/Tauri dashboard — the tray never dead-ends.

use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use webview2_com::Microsoft::Web::WebView2::Win32::{
    CreateCoreWebView2EnvironmentWithOptions, ICoreWebView2, ICoreWebView2Controller,
    ICoreWebView2Controller2, ICoreWebView2Environment, ICoreWebView2EnvironmentOptions,
    ICoreWebView2WebMessageReceivedEventArgs, COREWEBVIEW2_COLOR,
    COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC,
};
use webview2_com::{
    take_pwstr, AddScriptToExecuteOnDocumentCreatedCompletedHandler,
    CreateCoreWebView2ControllerCompletedHandler, CreateCoreWebView2EnvironmentCompletedHandler,
    WebMessageReceivedEventHandler,
};
use windows062::core::{Interface, PCWSTR, PWSTR};
use windows062::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, S_OK, WPARAM};
use windows_sys::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
};
use windows_sys::Win32::Graphics::Gdi::{
    CreateSolidBrush, GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Shell::{Shell_NotifyIconGetRect, NOTIFYICONIDENTIFIER};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetClientRect, GetCursorPos, IsWindowVisible,
    RegisterClassExW, SetForegroundWindow, SetWindowPos, ShowWindow, CS_HREDRAW, CS_VREDRAW,
    HWND_TOPMOST, SWP_SHOWWINDOW, SW_HIDE, WA_INACTIVE, WM_ACTIVATE, WM_CLOSE, WM_DESTROY,
    WM_DPICHANGED, WM_SIZE, WNDCLASSEXW, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

use super::windows_tray::{
    brand_icon, now_ms, to_wide, LAST_HIDE_MS, PANEL_H, PANEL_MAX_H, PANEL_MIN_H, PANEL_W,
    RETOGGLE_GUARD_MS, TRAY_ICON_ID,
};

/// Gap between the taskbar edge and the panel, in logical pixels.
const PANEL_GAP: f64 = 8.0;
/// Panel window class; registered once per process.
const CLASS_NAME: &str = "TetherlyTrayPanel";
/// Window backdrop behind the (93%-opaque) page surface — matches the SPA's
/// dark `--bg` so the 7% bleed reads as depth, not as a white halo.
const BACKDROP_BGR: u32 = 0x001C_1714;

/// Installed on every document before the SPA runs: `traybridge.ts` probes
/// `globalThis.tetherlyTray` and posts its JSON actions through it. Kept as
/// a tiny shim so the page never has to know about `chrome.webview`
/// (which the Tauri shell also exposes, for its own IPC).
const BRIDGE_SHIM_JS: &str = "(() => {\
  const wv = globalThis.chrome && globalThis.chrome.webview;\
  if (!wv) return;\
  globalThis.tetherlyTray = { postMessage: (m) => wv.postMessage(typeof m === 'string' ? m : JSON.stringify(m)) };\
})();";

struct Panel {
    hwnd: HWND,
    tray_hwnd: HWND,
    controller: Option<ICoreWebView2Controller>,
    /// Held so the event registrations outlive the handshake.
    #[allow(dead_code)]
    web: Option<ICoreWebView2>,
    /// Logical (96-dpi) height; the page's `resize` action updates it.
    height: f64,
}

thread_local! {
    static PANEL: RefCell<Option<Panel>> = const { RefCell::new(None) };
}

/// Set once WebView2 refuses; the tray then routes left clicks to the
/// dashboard for the rest of the process lifetime.
static UNAVAILABLE: AtomicBool = AtomicBool::new(false);

pub(super) fn available() -> bool {
    !UNAVAILABLE.load(Ordering::Relaxed)
}

pub(super) fn is_visible() -> bool {
    PANEL.with(|slot| {
        slot.borrow()
            .as_ref()
            .map(|p| unsafe { IsWindowVisible(p.hwnd) } != 0)
            .unwrap_or(false)
    })
}

/// Left-click entry: open the panel, or close it if it is up. Returns
/// `false` when the panel is unavailable so the caller can fall back.
/// A click on the tray icon while the panel is open first deactivates the
/// panel (hiding it) and then delivers the click; the guard window makes
/// that read as "close" instead of "close+reopen" (same as macOS).
pub(super) fn toggle(tray_hwnd: HWND, ui_port: u16) -> bool {
    if !available() {
        return false;
    }
    if is_visible() {
        hide();
        return true;
    }
    if now_ms().saturating_sub(LAST_HIDE_MS.load(Ordering::Relaxed)) < RETOGGLE_GUARD_MS {
        return true;
    }
    show(tray_hwnd, ui_port);
    available()
}

pub(super) fn hide() {
    let target = PANEL.with(|slot| {
        slot.borrow()
            .as_ref()
            .map(|p| (p.hwnd as usize, p.controller.clone()))
    });
    let Some((hwnd, controller)) = target else {
        return;
    };
    let hwnd = hwnd as HWND;
    if unsafe { IsWindowVisible(hwnd) } == 0 {
        return;
    }
    unsafe {
        ShowWindow(hwnd, SW_HIDE);
    }
    LAST_HIDE_MS.store(now_ms(), Ordering::Relaxed);
    if let Some(c) = controller {
        // Parked (not destroyed): the next open is instant and keeps the
        // page's live SSE state.
        let _ = unsafe { c.SetIsVisible(false) };
    }
}

fn show(tray_hwnd: HWND, ui_port: u16) {
    let Some(hwnd) = ensure_window(tray_hwnd, ui_port) else {
        return;
    };
    let (controller, height) = PANEL.with(|slot| {
        slot.borrow()
            .as_ref()
            .map(|p| (p.controller.clone(), p.height))
            .unwrap_or((None, PANEL_H))
    });
    place(hwnd, tray_hwnd, height);
    unsafe {
        SetForegroundWindow(hwnd);
    }
    if let Some(c) = controller {
        unsafe {
            let _ = c.SetIsVisible(true);
            let _ = c.MoveFocus(COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC);
        }
    }
    // Controller still in flight: `wire_controller` shows the view when it
    // lands, provided the window is still up.
}

/// Create the popup window and kick off the WebView2 handshake on first
/// use. Returns `None` (and flags the panel unavailable) when either step
/// refuses synchronously.
fn ensure_window(tray_hwnd: HWND, ui_port: u16) -> Option<HWND> {
    if let Some(hwnd) = PANEL.with(|slot| slot.borrow().as_ref().map(|p| p.hwnd as usize)) {
        return Some(hwnd as HWND);
    }
    // STA is already up for the drop target; a second init on the same
    // thread returns S_FALSE, which is still success.
    if unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_err() {
        fail("CoInitializeEx refused on the tray thread");
        return None;
    }
    let class_name = to_wide(CLASS_NAME);
    let icon = brand_icon();
    let wndclass = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(panel_wnd_proc),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: std::ptr::null_mut(),
        hIcon: icon,
        hCursor: std::ptr::null_mut(),
        hbrBackground: unsafe { CreateSolidBrush(BACKDROP_BGR) },
        lpszMenuName: std::ptr::null(),
        lpszClassName: class_name.as_ptr(),
        hIconSm: icon,
    };
    // Re-registration after a torn-down panel fails harmlessly (class kept).
    unsafe {
        RegisterClassExW(&wndclass);
    }
    let title = to_wide("Tetherly");
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
            class_name.as_ptr(),
            title.as_ptr(),
            WS_POPUP,
            0,
            0,
            PANEL_W as i32,
            PANEL_H as i32,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null(),
        )
    };
    if hwnd.is_null() {
        fail("CreateWindowExW refused for the tray panel");
        return None;
    }
    // Windows 11 rounds the popup like a flyout; older builds ignore it.
    let round: u32 = DWMWCP_ROUND as u32;
    unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE as u32,
            (&round as *const u32).cast(),
            std::mem::size_of::<u32>() as u32,
        );
    }
    PANEL.with(|slot| {
        *slot.borrow_mut() = Some(Panel {
            hwnd,
            tray_hwnd,
            controller: None,
            web: None,
            height: PANEL_H,
        });
    });
    if let Err(e) = create_webview(hwnd, ui_port) {
        fail(&format!("CreateCoreWebView2EnvironmentWithOptions: {e}"));
        return None;
    }
    Some(hwnd)
}

/// Where WebView2 keeps its profile. The daemon may run as SYSTEM (session
/// 1, via `TetherlySystemLauncher`), whose `%LOCALAPPDATA%` is the system
/// profile — writable by SYSTEM, which is all the runtime needs.
fn user_data_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let dir = base.join("tetherly").join("webview2");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn create_webview(hwnd: HWND, ui_port: u16) -> windows062::core::Result<()> {
    let udf = to_wide(&user_data_dir().to_string_lossy());
    let hwnd_bits = hwnd as usize;
    let env_handler = CreateCoreWebView2EnvironmentCompletedHandler::create(Box::new(
        move |res: windows062::core::Result<()>, env: Option<ICoreWebView2Environment>| {
            let env = match (res, env) {
                (Ok(()), Some(env)) => env,
                (res, _) => {
                    fail(&format!("WebView2 environment: {res:?}"));
                    return Ok(());
                }
            };
            let ctrl_handler = CreateCoreWebView2ControllerCompletedHandler::create(Box::new(
                move |res: windows062::core::Result<()>, ctrl: Option<ICoreWebView2Controller>| {
                    match (res, ctrl) {
                        (Ok(()), Some(ctrl)) => {
                            if let Err(e) = wire_controller(hwnd_bits as HWND, ctrl, ui_port) {
                                fail(&format!("WebView2 wiring: {e}"));
                            }
                        }
                        (res, _) => fail(&format!("WebView2 controller: {res:?}")),
                    }
                    Ok(())
                },
            ));
            let parent = windows062::Win32::Foundation::HWND(hwnd_bits as *mut core::ffi::c_void);
            if let Err(e) = unsafe { env.CreateCoreWebView2Controller(parent, &ctrl_handler) } {
                fail(&format!("CreateCoreWebView2Controller: {e}"));
            }
            Ok(())
        },
    ));
    unsafe {
        CreateCoreWebView2EnvironmentWithOptions(
            PCWSTR::null(),
            PCWSTR(udf.as_ptr()),
            None::<&ICoreWebView2EnvironmentOptions>,
            &env_handler,
        )
    }
}

/// Controller landed: trim the chrome, install the bridge shim, hook
/// messages, size the view and load the SPA tray route.
fn wire_controller(
    hwnd: HWND,
    ctrl: ICoreWebView2Controller,
    ui_port: u16,
) -> windows062::core::Result<()> {
    let web = unsafe { ctrl.CoreWebView2() }?;
    if let Ok(c2) = ctrl.cast::<ICoreWebView2Controller2>() {
        // Transparent default: the class backdrop shows through the page's
        // 7% translucency, and there is no white flash before first paint.
        let _ = unsafe {
            c2.SetDefaultBackgroundColor(COREWEBVIEW2_COLOR {
                A: 0,
                R: 0,
                G: 0,
                B: 0,
            })
        };
    }
    if let Ok(settings) = unsafe { web.Settings() } {
        unsafe {
            let _ = settings.SetAreDefaultContextMenusEnabled(false);
            let _ = settings.SetAreDevToolsEnabled(false);
            let _ = settings.SetIsZoomControlEnabled(false);
            let _ = settings.SetIsStatusBarEnabled(false);
        }
    }
    let shim = to_wide(BRIDGE_SHIM_JS);
    unsafe {
        web.AddScriptToExecuteOnDocumentCreated(
            PCWSTR(shim.as_ptr()),
            &AddScriptToExecuteOnDocumentCreatedCompletedHandler::create(Box::new(
                |_: windows062::core::Result<()>, _: String| Ok(()),
            )),
        )
    }?;
    let on_message = WebMessageReceivedEventHandler::create(Box::new(
        move |_web: Option<ICoreWebView2>,
              args: Option<ICoreWebView2WebMessageReceivedEventArgs>| {
            if let Some(args) = args {
                let mut raw = PWSTR::null();
                if unsafe { args.TryGetWebMessageAsString(&mut raw) }.is_ok() && !raw.is_null() {
                    handle_panel_action(ui_port, &take_pwstr(raw));
                }
            }
            Ok(())
        },
    ));
    let mut token = 0i64;
    unsafe { web.add_WebMessageReceived(&on_message, &mut token) }?;
    unsafe { ctrl.SetBounds(client_rect(hwnd)) }?;
    let url = to_wide(&panel_url(ui_port));
    unsafe { web.Navigate(PCWSTR(url.as_ptr())) }?;

    PANEL.with(|slot| {
        if let Some(p) = slot.borrow_mut().as_mut() {
            p.controller = Some(ctrl.clone());
            p.web = Some(web);
        }
    });
    // The click that started the handshake already showed the window; if
    // it is still up, light the view now.
    let visible = unsafe { IsWindowVisible(hwnd) } != 0;
    unsafe {
        let _ = ctrl.SetIsVisible(visible);
        if visible {
            let _ = ctrl.MoveFocus(COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC);
        }
    }
    tracing::info!(ui_port, "tray panel: WebView2 ready");
    Ok(())
}

/// The SPA tray route with the per-port API token in the fragment
/// (`#tray&token=…`; App.svelte strips the token and keeps `#tray`).
fn panel_url(ui_port: u16) -> String {
    match tetherly_loopback::api_token_for(ui_port) {
        Some(token) => format!("http://127.0.0.1:{ui_port}/#tray&token={token}"),
        None => format!("http://127.0.0.1:{ui_port}/#tray"),
    }
}

/// Dispatch one SPA tray action — the same vocabulary as the macOS panel.
fn handle_panel_action(ui_port: u16, raw: &str) {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(raw) else {
        tracing::debug!(raw, "tray panel: non-JSON bridge message ignored");
        return;
    };
    match value["action"].as_str().unwrap_or_default() {
        "open-main" => {
            hide();
            crate::apitoken::open_client_or_dashboard(
                ui_port,
                tetherly_loopback::api_token_for(ui_port).as_deref(),
            );
        }
        "close" => hide(),
        "quit" => std::process::exit(0),
        "resize" => {
            if let Some(h) = value["height"].as_f64() {
                resize_panel(h.clamp(PANEL_MIN_H, PANEL_MAX_H));
            }
        }
        "error" => {
            tracing::warn!(message = %value["message"], "tray panel reported an error");
        }
        other => tracing::debug!(action = other, "tray panel action"),
    }
}

/// Fit the panel to the page's content height. The anchor logic keeps the
/// taskbar-side edge fixed, so a bottom taskbar sees the panel grow upward.
fn resize_panel(height: f64) {
    let target = PANEL.with(|slot| {
        let mut slot = slot.borrow_mut();
        let p = slot.as_mut()?;
        if (p.height - height).abs() < 0.5 {
            return None;
        }
        p.height = height;
        (unsafe { IsWindowVisible(p.hwnd) } != 0).then_some((p.hwnd as usize, p.tray_hwnd as usize))
    });
    if let Some((hwnd, tray_hwnd)) = target {
        place(hwnd as HWND, tray_hwnd as HWND, height);
    }
}

fn client_rect(hwnd: HWND) -> windows062::Win32::Foundation::RECT {
    let mut rc: RECT = unsafe { std::mem::zeroed() };
    unsafe {
        GetClientRect(hwnd, &mut rc);
    }
    windows062::Win32::Foundation::RECT {
        left: 0,
        top: 0,
        right: rc.right - rc.left,
        bottom: rc.bottom - rc.top,
    }
}

fn fit_webview(hwnd: HWND) {
    let controller = PANEL.with(|slot| slot.borrow().as_ref().and_then(|p| p.controller.clone()));
    if let Some(c) = controller {
        let _ = unsafe { c.SetBounds(client_rect(hwnd)) };
    }
}

/// Size the panel for the window's DPI and anchor it next to the tray icon
/// on the taskbar's free side, clamped to the monitor work area.
fn place(hwnd: HWND, tray_hwnd: HWND, height: f64) {
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    let scale = if dpi == 0 { 1.0 } else { dpi as f64 / 96.0 };
    let w = (PANEL_W * scale).round() as i32;
    let h = (height * scale).round() as i32;
    let gap = (PANEL_GAP * scale).round() as i32;
    let anchor = icon_anchor(tray_hwnd);
    let (x, y) = anchored_origin(anchor, w, h, gap);
    unsafe {
        SetWindowPos(hwnd, HWND_TOPMOST, x, y, w, h, SWP_SHOWWINDOW);
    }
}

/// Centre of the notification-area icon, or the cursor when the shell
/// cannot tell (icon folded into the overflow chevron).
fn icon_anchor(tray_hwnd: HWND) -> POINT {
    let mut id: NOTIFYICONIDENTIFIER = unsafe { std::mem::zeroed() };
    id.cbSize = std::mem::size_of::<NOTIFYICONIDENTIFIER>() as u32;
    id.hWnd = tray_hwnd;
    id.uID = TRAY_ICON_ID;
    let mut rc: RECT = unsafe { std::mem::zeroed() };
    if unsafe { Shell_NotifyIconGetRect(&id, &mut rc) } == S_OK && rc.right > rc.left {
        return POINT {
            x: (rc.left + rc.right) / 2,
            y: (rc.top + rc.bottom) / 2,
        };
    }
    let mut pt = POINT { x: 0, y: 0 };
    unsafe {
        GetCursorPos(&mut pt);
    }
    pt
}

/// Top-left of a `w`×`h` panel anchored at `anchor`, on the side of the
/// taskbar that has room (the taskbar is wherever the work area is
/// shorter than the monitor), kept inside the work area.
fn anchored_origin(anchor: POINT, w: i32, h: i32, gap: i32) -> (i32, i32) {
    let mut mi: MONITORINFO = unsafe { std::mem::zeroed() };
    mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
    let mon = unsafe { MonitorFromPoint(anchor, MONITOR_DEFAULTTONEAREST) };
    if mon.is_null() || unsafe { GetMonitorInfoW(mon, &mut mi) } == 0 {
        return ((anchor.x - w / 2).max(0), (anchor.y - h - gap).max(0));
    }
    let wa = mi.rcWork;
    let full = mi.rcMonitor;
    let clamp = |v: i32, lo: i32, hi: i32| v.max(lo).min(hi.max(lo));
    let x_centred = clamp(anchor.x - w / 2, wa.left + gap, wa.right - w - gap);
    let y_centred = clamp(anchor.y - h / 2, wa.top + gap, wa.bottom - h - gap);
    if wa.bottom < full.bottom {
        (x_centred, wa.bottom - h - gap)
    } else if wa.top > full.top {
        (x_centred, wa.top + gap)
    } else if wa.right < full.right {
        (wa.right - w - gap, y_centred)
    } else if wa.left > full.left {
        (wa.left + gap, y_centred)
    } else {
        (x_centred, wa.bottom - h - gap)
    }
}

/// Give up on the panel for this process: log once, hide whatever is up,
/// and route future left clicks to the dashboard.
fn fail(reason: &str) {
    if !UNAVAILABLE.swap(true, Ordering::Relaxed) {
        tracing::warn!(
            reason,
            "tray panel: WebView2 unavailable; left click opens the dashboard"
        );
    }
    let hwnd = PANEL.with(|slot| slot.borrow_mut().take().map(|p| p.hwnd as usize));
    if let Some(hwnd) = hwnd {
        unsafe {
            DestroyWindow(hwnd as HWND);
        }
    }
}

unsafe extern "system" fn panel_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        // Click-away: the panel loses activation to whatever was clicked.
        WM_ACTIVATE => {
            if (wparam & 0xFFFF) as u32 == WA_INACTIVE {
                hide();
            }
            0
        }
        WM_SIZE => {
            fit_webview(hwnd);
            0
        }
        WM_DPICHANGED => {
            let target = PANEL.with(|slot| {
                slot.borrow()
                    .as_ref()
                    .map(|p| (p.tray_hwnd as usize, p.height))
            });
            if let Some((tray_hwnd, height)) = target {
                if IsWindowVisible(hwnd) != 0 {
                    place(hwnd, tray_hwnd as HWND, height);
                }
            }
            0
        }
        WM_CLOSE => {
            hide();
            0
        }
        WM_DESTROY => {
            PANEL.with(|slot| {
                let _ = slot.borrow_mut().take();
            });
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
