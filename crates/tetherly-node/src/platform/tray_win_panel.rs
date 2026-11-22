//! Windows tray panel (§226, re-hosted in §227): the left-click popup that
//! hosts the SPA `#tray` route in WebView2 — the Windows twin of the macOS
//! `NSPanel` in `tray.rs`.
//!
//! §227 — why a child process. The production daemon runs as LocalSystem on
//! the interactive desktop (`TetherlySystemLauncher`, session 1), and
//! WebView2 refuses to live there: `CreateCoreWebView2Controller` fails with
//! `0x8000FFFF` (E_UNEXPECTED, "灾难性故障") under SYSTEM (WebView2Feedback
//! #1907 / #4731). So the daemon never hosts the panel itself. The tray
//! thread spawns `tetherly.exe --data-dir … tray-panel …` as the console
//! user (`platform::spawn_as_console_user`, or a plain `Command` when the
//! daemon already is a user process) and that process owns the window, the
//! STA pump and the WebView2 controller. The API token travels in
//! `TETHERLY_API_TOKEN` (the CLI already honours it), never on the command
//! line. One process per open: the child exits when the panel hides, so
//! "panel is up" ≡ "child is alive", and the daemon's toggle logic needs no
//! cross-process window state.
//!
//! Layout: [`launcher`] is the daemon side (toggle / hide / process
//! bookkeeping); everything from [`run_tray_panel_process`] down runs in the
//! child, on its main thread (STA + message pump — every COM call, every
//! WebView2 callback, and the thread-local panel state).
//!
//! Two `windows` majors meet in the child. The window plumbing stays on
//! `windows-sys` 0.59 like the rest of the tray; `webview2-com` 0.39 is
//! pinned to `windows` 0.62 (`windows062`), so only the WebView2 handshake
//! names that copy. They touch at exactly one point: the raw HWND pointer.
//!
//! Failure policy: WebView2 stays best-effort. If the runtime is missing or
//! the handshake fails, the child logs once, opens the browser/Tauri
//! dashboard in the user's context instead, and exits 2 — the tray never
//! dead-ends.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;

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
use windows_sys::Win32::UI::HiDpi::{
    GetDpiForWindow, SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, VK_LBUTTON, VK_MBUTTON, VK_RBUTTON,
};
use windows_sys::Win32::UI::Shell::{Shell_NotifyIconGetRect, NOTIFYICONIDENTIFIER};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect, GetCursorPos,
    GetForegroundWindow, GetMessageW, GetWindowRect, IsWindowVisible, KillTimer, PostQuitMessage,
    RegisterClassExW, SetForegroundWindow, SetTimer, SetWindowPos, ShowWindow, TranslateMessage,
    CS_HREDRAW, CS_VREDRAW, HWND_TOPMOST, MSG, SWP_SHOWWINDOW, SW_HIDE, WA_INACTIVE, WM_ACTIVATE,
    WM_CLOSE, WM_DESTROY, WM_DPICHANGED, WM_SIZE, WM_TIMER, WNDCLASSEXW, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_POPUP,
};

use super::windows_tray::{to_wide, PANEL_H, PANEL_MAX_H, PANEL_MIN_H, PANEL_W, TRAY_ICON_ID};

/// Gap between the taskbar edge and the panel, in logical pixels.
const PANEL_GAP: f64 = 8.0;
/// Panel window class; the daemon finds the child's window by it.
const CLASS_NAME: &str = "TetherlyTrayPanel";
/// Window backdrop behind the (93%-opaque) page surface — matches the SPA's
/// dark `--bg` so the 7% bleed reads as depth, not as a white halo.
const BACKDROP_BGR: u32 = 0x001C_1714;
/// Child exit codes: 0 closed normally, 2 WebView2 unavailable (dashboard
/// opened instead).
const EXIT_OK: i32 = 0;
const EXIT_WEBVIEW_UNAVAILABLE: i32 = 2;
/// The controller must land within this long or the child gives up.
const WATCHDOG_TIMER_ID: usize = 0xC0FF_EE10;
const WATCHDOG_MS: u32 = 10_000;
/// Click-away backstop for the case where the panel never got foreground
/// rights (then `WM_ACTIVATE` never fires).
const CLICKAWAY_TIMER_ID: usize = 0xC0FF_EE11;
const CLICKAWAY_POLL_MS: u32 = 150;

/// Installed on every document before the SPA runs: `traybridge.ts` probes
/// `globalThis.tetherlyTray` and posts its JSON actions through it. Kept as
/// a tiny shim so the page never has to know about `chrome.webview`
/// (which the Tauri shell also exposes, for its own IPC).
const BRIDGE_SHIM_JS: &str = "(() => {\
  const wv = globalThis.chrome && globalThis.chrome.webview;\
  if (!wv) return;\
  globalThis.tetherlyTray = { postMessage: (m) => wv.postMessage(typeof m === 'string' ? m : JSON.stringify(m)) };\
})();";

// ---------------------------------------------------------------------------
// Daemon side
// ---------------------------------------------------------------------------

/// Daemon-side process bookkeeping for the panel child. Everything here runs
/// on the tray thread (`tray_wnd_proc`), so the child handle is a
/// thread-local; only the data dir — set once from `spawn_tray` — crosses
/// threads.
pub(super) mod launcher {
    use std::cell::RefCell;
    use std::path::PathBuf;
    use std::sync::Mutex;

    use windows_sys::Win32::Foundation::{CloseHandle, FILETIME, HANDLE, WAIT_TIMEOUT};
    use windows_sys::Win32::System::SystemInformation::GetSystemTimeAsFileTime;
    use windows_sys::Win32::System::Threading::{
        GetProcessTimes, TerminateProcess, WaitForSingleObject, CREATE_NO_WINDOW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        AllowSetForegroundWindow, FindWindowW, PostMessageW, WM_CLOSE,
    };

    use super::super::windows_tray::{to_wide, RETOGGLE_GUARD_MS};
    use super::{icon_anchor, CLASS_NAME};

    struct Child {
        process: HANDLE,
        pid: u32,
    }

    impl Drop for Child {
        fn drop(&mut self) {
            // SAFETY: the handle came from CreateProcess*/`Child::into_raw_handle`
            // and is owned by this struct alone.
            unsafe {
                CloseHandle(self.process);
            }
        }
    }

    thread_local! {
        static CHILD: RefCell<Option<Child>> = const { RefCell::new(None) };
    }

    /// `--data-dir` for the child, so it logs next to the daemon and finds
    /// the token file if the env var is ever missing.
    static DATA_DIR: Mutex<Option<PathBuf>> = Mutex::new(None);

    pub(in super::super) fn set_data_dir(dir: PathBuf) {
        *DATA_DIR.lock().unwrap_or_else(|e| e.into_inner()) = Some(dir);
    }

    /// Is a panel child alive? (It exits whenever the panel hides.)
    pub(in super::super) fn is_running() -> bool {
        CHILD.with(|slot| {
            slot.borrow()
                .as_ref()
                .map(|c| unsafe { WaitForSingleObject(c.process, 0) } == WAIT_TIMEOUT)
                .unwrap_or(false)
        })
    }

    /// Did the last child exit within `guard_ms`? A tray click while the
    /// panel is open deactivates the panel (child hides and exits) *before*
    /// the click reaches the daemon; inside this window the click means
    /// "close", not "close and reopen" — same guard as macOS.
    fn exited_within(guard_ms: u64) -> bool {
        CHILD
            .with(|slot| {
                let slot = slot.borrow();
                let c = slot.as_ref()?;
                let mut times: [FILETIME; 4] = unsafe { std::mem::zeroed() };
                let [ct, et, kt, ut] = &mut times;
                if unsafe { GetProcessTimes(c.process, ct, et, kt, ut) } == 0 {
                    return None;
                }
                let ticks =
                    |f: &FILETIME| ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64;
                let exit = ticks(et);
                if exit == 0 {
                    return None; // still running
                }
                let mut now: FILETIME = unsafe { std::mem::zeroed() };
                unsafe { GetSystemTimeAsFileTime(&mut now) };
                // FILETIME ticks are 100 ns.
                Some(ticks(&now).saturating_sub(exit) < guard_ms * 10_000)
            })
            .unwrap_or(false)
    }

    /// Left-click entry: open the panel, or close it if it is up. Returns
    /// `false` when no child could be started so the caller can fall back
    /// to the dashboard.
    pub(in super::super) fn toggle(
        tray_hwnd: windows_sys::Win32::Foundation::HWND,
        ui_port: u16,
    ) -> bool {
        if is_running() {
            hide();
            return true;
        }
        if exited_within(RETOGGLE_GUARD_MS) {
            return true;
        }
        spawn(tray_hwnd, ui_port)
    }

    /// Ask a live child to close (it exits on `WM_CLOSE`); a child that has
    /// not even created its window yet is terminated instead.
    pub(in super::super) fn hide() {
        if !is_running() {
            return;
        }
        let class = to_wide(CLASS_NAME);
        let hwnd = unsafe { FindWindowW(class.as_ptr(), std::ptr::null()) };
        if !hwnd.is_null() {
            unsafe {
                PostMessageW(hwnd, WM_CLOSE, 0, 0);
            }
            return;
        }
        CHILD.with(|slot| {
            if let Some(c) = slot.borrow().as_ref() {
                unsafe {
                    TerminateProcess(c.process, 1);
                }
            }
        });
    }

    /// Windows CRT argument quoting (the inverse of what `std::env::args`
    /// parses in the child).
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

    fn spawn(tray_hwnd: windows_sys::Win32::Foundation::HWND, ui_port: u16) -> bool {
        // Drop the previous (exited) child's handle before replacing it.
        CHILD.with(|slot| slot.borrow_mut().take());

        let exe = match std::env::current_exe() {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(error = %e, "tray panel: current_exe unavailable");
                return false;
            }
        };
        let exe_str = exe.to_string_lossy().into_owned();
        let data_dir = DATA_DIR.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let anchor = icon_anchor(tray_hwnd, None);
        let token = tetherly_loopback::api_token_for(ui_port);

        let mut args: Vec<String> = Vec::new();
        if let Some(d) = &data_dir {
            args.push("--data-dir".into());
            args.push(d.to_string_lossy().into_owned());
        }
        args.extend([
            "tray-panel".to_string(),
            "--ui-port".into(),
            ui_port.to_string(),
            "--anchor-hwnd".into(),
            (tray_hwnd as usize).to_string(),
            "--anchor-x".into(),
            anchor.x.to_string(),
            "--anchor-y".into(),
            anchor.y.to_string(),
        ]);

        let child = if crate::cred::process_runs_as_local_system() {
            // Production: SYSTEM daemon → user-token child on winsta0\default.
            let line = std::iter::once(exe_str.as_str())
                .chain(args.iter().map(String::as_str))
                .map(quote_arg)
                .collect::<Vec<_>>()
                .join(" ");
            let env: Vec<(&str, &str)> = token
                .iter()
                .map(|t| ("TETHERLY_API_TOKEN", t.as_str()))
                .collect();
            match crate::platform::spawn_as_console_user(&exe_str, &line, &env) {
                Ok(c) => {
                    unsafe {
                        CloseHandle(c.thread);
                    }
                    Child {
                        process: c.process,
                        pid: c.pid,
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, "tray panel: spawn as console user refused");
                    return false;
                }
            }
        } else {
            use std::os::windows::io::IntoRawHandle;
            use std::os::windows::process::CommandExt;
            let mut cmd = std::process::Command::new(&exe);
            cmd.args(&args).creation_flags(CREATE_NO_WINDOW);
            if let Some(t) = &token {
                cmd.env("TETHERLY_API_TOKEN", t);
            }
            match cmd.spawn() {
                Ok(c) => {
                    let pid = c.id();
                    Child {
                        process: c.into_raw_handle() as HANDLE,
                        pid,
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, "tray panel: spawn refused");
                    return false;
                }
            }
        };
        // The tray click gave this process foreground rights; hand them to
        // the child so its `SetForegroundWindow` sticks (click-away then
        // arrives as WM_ACTIVATE instead of relying on the poll backstop).
        unsafe {
            AllowSetForegroundWindow(child.pid);
        }
        tracing::info!(
            pid = child.pid,
            ui_port,
            as_system = crate::cred::process_runs_as_local_system(),
            "tray panel: child spawned"
        );
        CHILD.with(|slot| *slot.borrow_mut() = Some(child));
        true
    }

    #[cfg(test)]
    mod tests {
        use super::quote_arg;

        #[test]
        fn quote_arg_follows_crt_rules() {
            assert_eq!(quote_arg("plain"), "plain");
            assert_eq!(
                quote_arg(r"C:\Users\x\AppData\Local\tetherly"),
                r"C:\Users\x\AppData\Local\tetherly"
            );
            assert_eq!(quote_arg(r"C:\Program Files\t"), r#""C:\Program Files\t""#);
            assert_eq!(quote_arg(r"C:\dir with space\"), r#""C:\dir with space\\""#);
            assert_eq!(quote_arg(r#"say "hi""#), r#""say \"hi\"""#);
            assert_eq!(quote_arg(""), r#""""#);
        }
    }
}

// ---------------------------------------------------------------------------
// Child side
// ---------------------------------------------------------------------------

struct Panel {
    hwnd: HWND,
    /// The daemon's tray window: identifies the notification icon for
    /// `Shell_NotifyIconGetRect`.
    tray_hwnd: HWND,
    /// Where the daemon saw the icon when it spawned us (fallback anchor).
    anchor_hint: POINT,
    ui_port: u16,
    controller: Option<ICoreWebView2Controller>,
    /// Held so the event registrations outlive the handshake.
    #[allow(dead_code)]
    web: Option<ICoreWebView2>,
    /// Logical (96-dpi) height; the page's `resize` action updates it.
    height: f64,
    /// Set once `GetForegroundWindow` reported us; afterwards losing it
    /// means click-away even if `WM_ACTIVATE` was swallowed.
    had_foreground: bool,
}

thread_local! {
    static PANEL: RefCell<Option<Panel>> = const { RefCell::new(None) };
    static EXIT_CODE: Cell<i32> = const { Cell::new(EXIT_OK) };
}

/// Child entry (`tetherly tray-panel`): show the panel anchored at the
/// daemon's tray icon and pump messages until it hides. Returns the process
/// exit code. Runs on the calling thread, which becomes the STA.
pub fn run_tray_panel_process(
    ui_port: u16,
    anchor_hwnd: usize,
    anchor_x: i32,
    anchor_y: i32,
) -> i32 {
    unsafe {
        // The panel sizes itself from `GetDpiForWindow`, which only reports
        // the real monitor DPI on a per-monitor-aware thread; the daemon exe
        // carries no DPI manifest.
        SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    if unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_err() {
        fail(ui_port, "CoInitializeEx refused");
        return EXIT_WEBVIEW_UNAVAILABLE;
    }
    let tray_hwnd = anchor_hwnd as HWND;
    let anchor_hint = POINT {
        x: anchor_x,
        y: anchor_y,
    };
    let Some(hwnd) = create_window(tray_hwnd, anchor_hint, ui_port) else {
        return EXIT_WEBVIEW_UNAVAILABLE;
    };
    if let Err(e) = create_webview(hwnd, ui_port) {
        fail(
            ui_port,
            &format!("CreateCoreWebView2EnvironmentWithOptions: {e}"),
        );
        return EXIT_WEBVIEW_UNAVAILABLE;
    }
    place(hwnd, PANEL_H);
    unsafe {
        SetForegroundWindow(hwnd);
        SetTimer(hwnd, WATCHDOG_TIMER_ID, WATCHDOG_MS, None);
        SetTimer(hwnd, CLICKAWAY_TIMER_ID, CLICKAWAY_POLL_MS, None);
    }
    tracing::info!(ui_port, pid = std::process::id(), "tray panel: child up");

    let mut msg: MSG = unsafe { std::mem::zeroed() };
    while unsafe { GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) } > 0 {
        unsafe {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    EXIT_CODE.with(|c| c.get())
}

fn create_window(tray_hwnd: HWND, anchor_hint: POINT, ui_port: u16) -> Option<HWND> {
    let class_name = to_wide(CLASS_NAME);
    let wndclass = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(panel_wnd_proc),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: std::ptr::null_mut(),
        hIcon: std::ptr::null_mut(),
        hCursor: std::ptr::null_mut(),
        hbrBackground: unsafe { CreateSolidBrush(BACKDROP_BGR) },
        lpszMenuName: std::ptr::null(),
        lpszClassName: class_name.as_ptr(),
        hIconSm: std::ptr::null_mut(),
    };
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
        fail(ui_port, "CreateWindowExW refused for the tray panel");
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
            anchor_hint,
            ui_port,
            controller: None,
            web: None,
            height: PANEL_H,
            had_foreground: false,
        });
    });
    Some(hwnd)
}

/// Where WebView2 keeps its profile: the *user's* `%LOCALAPPDATA%` now that
/// the child runs with the console user's token.
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
                    fail(ui_port, &format!("WebView2 environment: {res:?}"));
                    return Ok(());
                }
            };
            let ctrl_handler = CreateCoreWebView2ControllerCompletedHandler::create(Box::new(
                move |res: windows062::core::Result<()>, ctrl: Option<ICoreWebView2Controller>| {
                    match (res, ctrl) {
                        (Ok(()), Some(ctrl)) => {
                            if let Err(e) = wire_controller(hwnd_bits as HWND, ctrl, ui_port) {
                                fail(ui_port, &format!("WebView2 wiring: {e}"));
                            }
                        }
                        (res, _) => fail(ui_port, &format!("WebView2 controller: {res:?}")),
                    }
                    Ok(())
                },
            ));
            let parent = windows062::Win32::Foundation::HWND(hwnd_bits as *mut core::ffi::c_void);
            if let Err(e) = unsafe { env.CreateCoreWebView2Controller(parent, &ctrl_handler) } {
                fail(ui_port, &format!("CreateCoreWebView2Controller: {e}"));
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
    unsafe {
        KillTimer(hwnd, WATCHDOG_TIMER_ID);
    }
    let visible = unsafe { IsWindowVisible(hwnd) } != 0;
    unsafe {
        let _ = ctrl.SetIsVisible(visible);
        if visible {
            let _ = ctrl.MoveFocus(COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC);
        }
    }
    tracing::info!(
        ui_port,
        pid = std::process::id(),
        "tray panel: WebView2 ready"
    );
    Ok(())
}

/// The SPA tray route with the per-port API token in the fragment
/// (`#tray&token=…`; App.svelte strips the token and keeps `#tray`). In the
/// child the token comes from `TETHERLY_API_TOKEN` via `set_api_token`.
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
        "quit" => {
            // The daemon is another process now: ask it to exit over its own
            // loopback API (Admin tier, token in the header), then go.
            request_daemon_shutdown(ui_port);
            hide();
        }
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

/// Blocking `POST /api/v1/system/shutdown` — the panel is on the STA thread
/// with no tokio runtime, so a bare `TcpStream` with short timeouts does it.
fn request_daemon_shutdown(ui_port: u16) {
    use std::io::{Read, Write};
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], ui_port));
    let timeout = std::time::Duration::from_secs(2);
    let mut stream = match std::net::TcpStream::connect_timeout(&addr, timeout) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = %e, ui_port, "tray panel: shutdown request could not connect");
            return;
        }
    };
    let _ = stream.set_read_timeout(Some(timeout));
    let _ = stream.set_write_timeout(Some(timeout));
    let auth = tetherly_loopback::api_token_for(ui_port)
        .map(|t| format!("X-Tetherly-Token: {t}\r\n"))
        .unwrap_or_default();
    let head = format!(
        "POST /api/v1/system/shutdown HTTP/1.1\r\nHost: 127.0.0.1\r\n{auth}Content-Type: application/json\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    );
    if let Err(e) = stream.write_all(head.as_bytes()) {
        tracing::warn!(error = %e, "tray panel: shutdown request write failed");
        return;
    }
    let mut buf = [0u8; 64];
    let status = match stream.read(&mut buf) {
        Ok(n) => String::from_utf8_lossy(&buf[..n])
            .lines()
            .next()
            .unwrap_or_default()
            .to_string(),
        Err(e) => format!("read failed: {e}"),
    };
    tracing::info!(ui_port, status, "tray panel: asked the daemon to shut down");
}

/// Hide ≡ exit: release the controller, tear the window down; `WM_DESTROY`
/// posts the quit that ends the pump.
fn hide() {
    let taken = PANEL.with(|slot| slot.borrow_mut().take());
    let Some(p) = taken else {
        return;
    };
    unsafe {
        ShowWindow(p.hwnd, SW_HIDE);
    }
    if let Some(c) = &p.controller {
        let _ = unsafe { c.Close() };
    }
    unsafe {
        DestroyWindow(p.hwnd);
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
        (unsafe { IsWindowVisible(p.hwnd) } != 0).then_some(p.hwnd as usize)
    });
    if let Some(hwnd) = target {
        place(hwnd as HWND, height);
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
fn place(hwnd: HWND, height: f64) {
    let (tray_hwnd, hint) = PANEL.with(|slot| {
        slot.borrow()
            .as_ref()
            .map(|p| (p.tray_hwnd as usize, p.anchor_hint))
            .unwrap_or((0, POINT { x: 0, y: 0 }))
    });
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    let scale = if dpi == 0 { 1.0 } else { dpi as f64 / 96.0 };
    let w = (PANEL_W * scale).round() as i32;
    let h = (height * scale).round() as i32;
    let gap = (PANEL_GAP * scale).round() as i32;
    let anchor = icon_anchor(tray_hwnd as HWND, Some(hint));
    let (x, y) = anchored_origin(anchor, w, h, gap);
    unsafe {
        SetWindowPos(hwnd, HWND_TOPMOST, x, y, w, h, SWP_SHOWWINDOW);
    }
}

/// Centre of the notification-area icon owned by `tray_hwnd`; when the shell
/// cannot tell (icon folded into the overflow chevron), the `fallback` point
/// the daemon passed, else the cursor.
pub(super) fn icon_anchor(tray_hwnd: HWND, fallback: Option<POINT>) -> POINT {
    if !tray_hwnd.is_null() {
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
    }
    if let Some(p) = fallback.filter(|p| p.x != 0 || p.y != 0) {
        return p;
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

/// Give up on WebView2 in this process: log, open the dashboard in the
/// user's context instead, tear down and exit 2.
fn fail(ui_port: u16, reason: &str) {
    tracing::warn!(
        reason,
        ui_port,
        "tray panel: WebView2 unavailable; opening the dashboard instead"
    );
    EXIT_CODE.with(|c| c.set(EXIT_WEBVIEW_UNAVAILABLE));
    crate::apitoken::open_client_or_dashboard(
        ui_port,
        tetherly_loopback::api_token_for(ui_port).as_deref(),
    );
    let had_window = PANEL.with(|slot| slot.borrow().is_some());
    if had_window {
        hide();
    } else {
        unsafe {
            PostQuitMessage(EXIT_WEBVIEW_UNAVAILABLE);
        }
    }
}

/// Click-away backstop (`CLICKAWAY_TIMER_ID`): `WM_ACTIVATE` only reports
/// deactivation if we were ever active, which needs foreground rights the
/// daemon may not have been able to pass on. Once we have seen ourselves in
/// the foreground, losing it means click-away; until then, any mouse button
/// pressed outside the panel does.
fn poll_click_away(hwnd: HWND) {
    let fg = unsafe { GetForegroundWindow() };
    if fg == hwnd {
        PANEL.with(|slot| {
            if let Some(p) = slot.borrow_mut().as_mut() {
                p.had_foreground = true;
            }
        });
        return;
    }
    let had = PANEL.with(|slot| {
        slot.borrow()
            .as_ref()
            .map(|p| p.had_foreground)
            .unwrap_or(false)
    });
    if had {
        hide();
        return;
    }
    let pressed = [VK_LBUTTON, VK_RBUTTON, VK_MBUTTON]
        .iter()
        .any(|vk| (unsafe { GetAsyncKeyState(*vk as i32) } as u16) & 0x8000 != 0);
    if !pressed {
        return;
    }
    let mut pt = POINT { x: 0, y: 0 };
    let mut rc: RECT = unsafe { std::mem::zeroed() };
    unsafe {
        GetCursorPos(&mut pt);
        GetWindowRect(hwnd, &mut rc);
    }
    let inside = pt.x >= rc.left && pt.x < rc.right && pt.y >= rc.top && pt.y < rc.bottom;
    if !inside {
        hide();
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
            let height = PANEL.with(|slot| slot.borrow().as_ref().map(|p| p.height));
            if let Some(height) = height {
                if IsWindowVisible(hwnd) != 0 {
                    place(hwnd, height);
                }
            }
            0
        }
        WM_TIMER if wparam == WATCHDOG_TIMER_ID => {
            KillTimer(hwnd, WATCHDOG_TIMER_ID);
            let pending = PANEL.with(|slot| {
                slot.borrow()
                    .as_ref()
                    .is_some_and(|p| p.controller.is_none())
            });
            if pending {
                let port =
                    PANEL.with(|slot| slot.borrow().as_ref().map(|p| p.ui_port).unwrap_or(0));
                fail(port, "WebView2 controller did not arrive in time");
            }
            0
        }
        WM_TIMER if wparam == CLICKAWAY_TIMER_ID => {
            poll_click_away(hwnd);
            0
        }
        WM_CLOSE => {
            hide();
            0
        }
        WM_DESTROY => {
            KillTimer(hwnd, WATCHDOG_TIMER_ID);
            KillTimer(hwnd, CLICKAWAY_TIMER_ID);
            PostQuitMessage(EXIT_CODE.with(|c| c.get()));
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
