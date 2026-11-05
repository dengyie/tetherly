// SPDX-License-Identifier: Apache-2.0 OR MIT
//! OS clipboard, conservative insert, and allowlist-driven app open.
//! insert() and open() only run on a user click.

use std::sync::{Arc, Mutex};
use tetherly_core::{
    insertion_plan, Clip, CoreError, InsertionPlan, Insertor, NotificationPayload,
    NotificationUrgency, Notifier, Opener,
};

pub mod cursor_shield;
pub mod tray;

pub struct SystemClip;

impl Clip for SystemClip {
    fn set_text(&self, text: &str) -> Result<(), CoreError> {
        let mut cb = arboard::Clipboard::new().map_err(clipboard_err)?;
        cb.set_text(text.to_string()).map_err(clipboard_err)
    }

    fn get_text(&self) -> Result<Option<String>, CoreError> {
        let mut cb = arboard::Clipboard::new().map_err(clipboard_err)?;
        match cb.get_text() {
            Ok(t) => Ok(Some(t)),
            Err(arboard::Error::ContentNotAvailable) => Ok(None),
            Err(e) => Err(clipboard_err(e)),
        }
    }
}

/// Map an OS clipboard failure to a labeled error: on hosts with no display
/// server (headless Linux) this is the expected arboard/X11 outcome, and the
/// label must not masquerade as a serialization problem.
fn clipboard_err(e: arboard::Error) -> CoreError {
    CoreError::Clipboard(e.to_string())
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

/// Launch a local-scheme url through the OS handler. No shell is involved: the
/// url is passed as a single argv element, and callers only ever pass a url
/// that came from the validated allowlist.
pub struct SystemOpener;

impl Opener for SystemOpener {
    fn open(&self, url: &str) -> Result<(), CoreError> {
        let mut cmd = if cfg!(target_os = "windows") {
            let mut c = std::process::Command::new("rundll32.exe");
            c.arg("url.dll,FileProtocolHandler");
            c
        } else if cfg!(target_os = "macos") {
            std::process::Command::new("open")
        } else {
            std::process::Command::new("xdg-open")
        };
        cmd.arg(url)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        cmd.spawn()
            .map(|_| ())
            .map_err(|e| CoreError::OpenRefused(e.to_string()))
    }
}

/// Records opened urls instead of launching anything.
#[derive(Clone, Default)]
pub struct MemoryOpener {
    urls: Arc<Mutex<Vec<String>>>,
}

impl MemoryOpener {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn opened(&self) -> Vec<String> {
        self.urls.lock().expect("opener").clone()
    }
}

impl Opener for MemoryOpener {
    fn open(&self, url: &str) -> Result<(), CoreError> {
        self.urls.lock().expect("opener").push(url.to_string());
        Ok(())
    }
}

/// Dispatches native OS notification banners (macOS Notification Center /
/// Windows balloon & toast / Linux notify-send).
pub struct SystemNotifier;

impl Notifier for SystemNotifier {
    fn notify(&self, payload: &NotificationPayload) -> Result<(), CoreError> {
        let title = payload.title.replace('"', "\\\"").replace('\n', " ");
        let body = payload.body.replace('"', "\\\"").replace('\n', " ");

        let mut cmd = if cfg!(target_os = "macos") {
            let script = if payload.sound {
                format!(
                    "display notification \"{}\" with title \"{}\" sound name \"default\"",
                    body, title
                )
            } else {
                format!("display notification \"{}\" with title \"{}\"", body, title)
            };
            let mut c = std::process::Command::new("osascript");
            c.arg("-e").arg(script);
            c
        } else if cfg!(target_os = "windows") {
            let ps_script = format!(
                "[reflection.assembly]::loadwithpartialname('System.Windows.Forms') | Out-Null; \
                 $notify = new-object system.windows.forms.notifyicon; \
                 $notify.icon = [system.drawing.systemicons]::information; \
                 $notify.visible = $true; \
                 $notify.showballoontip(5000, '{}', '{}', [system.windows.forms.tooltipicon]::Info)",
                title.replace('\'', "''"),
                body.replace('\'', "''")
            );
            let mut c = std::process::Command::new("powershell.exe");
            c.arg("-NoProfile")
                .arg("-NonInteractive")
                .arg("-Command")
                .arg(ps_script);
            c
        } else {
            let mut c = std::process::Command::new("notify-send");
            if let Some(app) = &payload.app_name {
                c.arg("-a").arg(app);
            }
            let urgency_str = match payload.urgency {
                NotificationUrgency::Low => "low",
                NotificationUrgency::Normal => "normal",
                NotificationUrgency::Critical => "critical",
            };
            c.arg("-u").arg(urgency_str);
            c.arg(&title).arg(&body);
            c
        };

        cmd.stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());

        match cmd.spawn() {
            Ok(_) => Ok(()),
            Err(e) => {
                tracing::warn!(err = %e, "system notifier spawn failed");
                Err(CoreError::Json(format!("system notify failed: {e}")))
            }
        }
    }
}

/// Dispatches a non-intrusive system toast notification asynchronously without blocking caller.
pub fn show_system_toast(title: &str, body: &str) {
    let mut payload = NotificationPayload::new(title, body);
    payload.app_name = Some("Tetherly".into());
    payload.sound = false;
    std::thread::spawn(move || {
        let _ = SystemNotifier.notify(&payload);
    });
}

/// In-memory notification sink for headless CI, unit tests and assertions.
#[derive(Clone, Default)]
pub struct MemoryNotifier {
    records: Arc<Mutex<Vec<NotificationPayload>>>,
}

impl MemoryNotifier {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn notifications(&self) -> Vec<NotificationPayload> {
        self.records.lock().expect("notifier").clone()
    }

    pub fn records(&self) -> Vec<NotificationPayload> {
        self.records.lock().expect("notifier").clone()
    }

    pub fn count(&self) -> usize {
        self.records.lock().expect("notifier").len()
    }

    pub fn latest(&self) -> Option<NotificationPayload> {
        self.records.lock().expect("notifier").last().cloned()
    }

    pub fn clear(&self) {
        self.records.lock().expect("notifier").clear();
    }
}

impl Notifier for MemoryNotifier {
    fn notify(&self, payload: &NotificationPayload) -> Result<(), CoreError> {
        self.records.lock().expect("notifier").push(payload.clone());
        Ok(())
    }

    fn show(&self, title: &str, body_len: usize, score: i32) -> Result<(), CoreError> {
        let payload =
            NotificationPayload::new(title, format!("<len:{body_len}>")).with_sound(score > 0);
        self.records.lock().expect("notifier").push(payload);
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

// ---------------------------------------------------------------------------
// SystemInputSink — real key/mouse injection (controlled side of 45719)
// ---------------------------------------------------------------------------

/// Convert `TIN1` screen units (0..=1000, tenths of a percent) to the absolute
/// coordinates `SendInput` expects (0..=65535 across the primary monitor).
#[cfg(windows)]
fn units_to_abs(v: i32) -> i32 {
    v.clamp(0, tetherly_core::SCREEN_UNITS) * 65_535 / tetherly_core::SCREEN_UNITS
}

/// Real injection for the controlled side. `move_abs` receives TIN1 screen
/// units and maps them onto this machine's primary monitor; `key` codes are
/// Win32 virtual-key codes (SSOT §9.5). `mods` is informational: modifier
/// keys arrive as their own key events.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemInputSink;

#[cfg(windows)]
impl tetherly_core::InputSink for SystemInputSink {
    fn move_abs(&mut self, x: i32, y: i32) {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
            MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_MOVE,
        };
        use windows_sys::Win32::UI::WindowsAndMessaging::SetCursorPos;
        let (w, h) = screen_size();
        // Round to nearest: must mirror the controller engine's rounded
        // norm/denorm, else each round trip truncates in one direction and
        // repeated crossings walk the cursor (production bug).
        let px = (x.clamp(0, tetherly_core::SCREEN_UNITS) as i64 * (w.saturating_sub(1) as i64)
            + tetherly_core::SCREEN_UNITS as i64 / 2)
            / (tetherly_core::SCREEN_UNITS as i64);
        let py = (y.clamp(0, tetherly_core::SCREEN_UNITS) as i64 * (h.saturating_sub(1) as i64)
            + tetherly_core::SCREEN_UNITS as i64 / 2)
            / (tetherly_core::SCREEN_UNITS as i64);
        let (px, py) = (px as i32, py as i32);
        unsafe {
            let _ = SetCursorPos(px, py);
        }
        inject_mouse(
            MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE,
            units_to_abs(x),
            units_to_abs(y),
            0,
        );
    }

    fn button(&mut self, mask: u8, down: bool) {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
        let flags = match mask {
            m if m & tetherly_core::BTN_LEFT != 0 => {
                if down {
                    MOUSEEVENTF_LEFTDOWN
                } else {
                    MOUSEEVENTF_LEFTUP
                }
            }
            m if m & tetherly_core::BTN_RIGHT != 0 => {
                if down {
                    MOUSEEVENTF_RIGHTDOWN
                } else {
                    MOUSEEVENTF_RIGHTUP
                }
            }
            m if m & tetherly_core::BTN_MIDDLE != 0 => {
                if down {
                    MOUSEEVENTF_MIDDLEDOWN
                } else {
                    MOUSEEVENTF_MIDDLEUP
                }
            }
            _ => return,
        };
        inject_mouse(flags, 0, 0, 0);
    }

    fn wheel(&mut self, dx: i16, dy: i16) {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
            MOUSEEVENTF_HWHEEL, MOUSEEVENTF_WHEEL,
        };
        if dy != 0 {
            let delta = (dy as i32) * 120;
            inject_mouse(MOUSEEVENTF_WHEEL, 0, 0, delta as u32);
        }
        if dx != 0 {
            let delta = (dx as i32) * 120;
            inject_mouse(MOUSEEVENTF_HWHEEL, 0, 0, delta as u32);
        }
    }

    fn key(&mut self, code: u16, down: bool, mods: u8) {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
            MapVirtualKeyW, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT,
            KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP,
        };
        let _ = mods; // modifiers travel as their own key events
        let mut flags = if down { 0 } else { KEYEVENTF_KEYUP };
        match code {
            0x21..=0x28 // PageUp, PageDown, End, Home, Left, Up, Right, Down
            | 0x2D | 0x2E // Insert, Delete
            | 0x5B | 0x5C // LWIN, RWIN
            | 0x5D // VK_APPS (Context Menu)
            | 0xA3 | 0xA5 // RCONTROL, RMENU
            | 0x6F // VK_DIVIDE
            | 0x90 // VK_NUMLOCK
            | 0xA6..=0xB7 => { // Browser keys, Media keys, Launch keys
                flags |= KEYEVENTF_EXTENDEDKEY;
            }
            _ => {}
        }
        let scan = unsafe { MapVirtualKeyW(code as u32, 0) } as u16;
        let input = KEYBDINPUT {
            wVk: code,
            wScan: scan,
            dwFlags: flags,
            time: 0,
            dwExtraInfo: 0,
        };
        unsafe {
            SendInput(
                1,
                [INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: INPUT_0 { ki: input },
                }]
                .as_mut_ptr(),
                std::mem::size_of::<INPUT>() as i32,
            );
        }
    }
}

#[cfg(target_os = "macos")]
const TETHERLY_INJECTED_MAGIC: i64 = 0x5445_5448_4552_4C59; // "TETHERLY" in ASCII

#[cfg(target_os = "macos")]
impl tetherly_core::InputSink for SystemInputSink {
    fn move_abs(&mut self, x: i32, y: i32) {
        use objc2_core_foundation::CGPoint;
        use objc2_core_graphics::{
            CGEvent, CGEventField, CGEventTapLocation, CGEventType, CGMouseButton,
        };

        let (w, h) = screen_size();
        // Round to nearest: must mirror the controller engine's rounded
        // norm/denorm, else each round trip truncates in one direction and
        // repeated crossings walk the cursor (production bug).
        let px = (x.clamp(0, tetherly_core::SCREEN_UNITS) as f64 * (w.saturating_sub(1) as f64)
            / (tetherly_core::SCREEN_UNITS as f64))
            .round();
        let py = (y.clamp(0, tetherly_core::SCREEN_UNITS) as f64 * (h.saturating_sub(1) as f64)
            / (tetherly_core::SCREEN_UNITS as f64))
            .round();
        let pt = CGPoint { x: px, y: py };

        if let Some(event) =
            CGEvent::new_mouse_event(None, CGEventType::MouseMoved, pt, CGMouseButton::Left)
        {
            CGEvent::set_integer_value_field(
                Some(&event),
                CGEventField::EventSourceUserData,
                TETHERLY_INJECTED_MAGIC,
            );
            CGEvent::post(CGEventTapLocation::HIDEventTap, Some(&event));
        }
    }

    fn button(&mut self, mask: u8, down: bool) {
        use objc2_core_graphics::{
            CGEvent, CGEventField, CGEventTapLocation, CGEventType, CGMouseButton,
        };

        let (ev_type, btn) = match mask {
            m if m & tetherly_core::BTN_LEFT != 0 => (
                if down {
                    CGEventType::LeftMouseDown
                } else {
                    CGEventType::LeftMouseUp
                },
                CGMouseButton::Left,
            ),
            m if m & tetherly_core::BTN_RIGHT != 0 => (
                if down {
                    CGEventType::RightMouseDown
                } else {
                    CGEventType::RightMouseUp
                },
                CGMouseButton::Right,
            ),
            m if m & tetherly_core::BTN_MIDDLE != 0 => (
                if down {
                    CGEventType::OtherMouseDown
                } else {
                    CGEventType::OtherMouseUp
                },
                CGMouseButton::Center,
            ),
            _ => return,
        };

        let (cx, cy) = cursor_pos();
        let pt = objc2_core_foundation::CGPoint {
            x: cx as objc2_core_foundation::CGFloat,
            y: cy as objc2_core_foundation::CGFloat,
        };
        if let Some(event) = CGEvent::new_mouse_event(None, ev_type, pt, btn) {
            CGEvent::set_integer_value_field(
                Some(&event),
                CGEventField::EventSourceUserData,
                TETHERLY_INJECTED_MAGIC,
            );
            CGEvent::post(CGEventTapLocation::HIDEventTap, Some(&event));
        }
    }

    fn wheel(&mut self, dx: i16, dy: i16) {
        use objc2_core_graphics::{CGEvent, CGEventField, CGEventTapLocation, CGScrollEventUnit};

        let wheel_count = if dx != 0 { 2 } else { 1 };
        if let Some(event) = CGEvent::new_scroll_wheel_event2(
            None,
            CGScrollEventUnit::Line,
            wheel_count,
            dy as i32,
            dx as i32,
            0,
        ) {
            CGEvent::set_integer_value_field(
                Some(&event),
                CGEventField::EventSourceUserData,
                TETHERLY_INJECTED_MAGIC,
            );
            CGEvent::post(CGEventTapLocation::HIDEventTap, Some(&event));
        }
    }

    fn key(&mut self, code: u16, down: bool, _mods: u8) {
        use objc2_core_graphics::{CGEvent, CGEventField, CGEventTapLocation};

        if let Some(cg_code) = win_vk_to_mac_keycode(code) {
            if let Some(event) = CGEvent::new_keyboard_event(None, cg_code, down) {
                CGEvent::set_integer_value_field(
                    Some(&event),
                    CGEventField::EventSourceUserData,
                    TETHERLY_INJECTED_MAGIC,
                );
                CGEvent::post(CGEventTapLocation::HIDEventTap, Some(&event));
            }
        }
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
impl tetherly_core::InputSink for SystemInputSink {
    fn move_abs(&mut self, _x: i32, _y: i32) {}
    fn button(&mut self, _mask: u8, _down: bool) {}
    fn wheel(&mut self, _dx: i16, _dy: i16) {}
    fn key(&mut self, _code: u16, _down: bool, _mods: u8) {}
}

#[cfg(target_os = "macos")]
#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGWarpMouseCursorPosition(new_cursor_position: objc2_core_foundation::CGPoint) -> i32;
    fn CGAssociateMouseAndMouseCursorPosition(connected: bool) -> i32;
    fn CGSetLocalEventsSuppressionInterval(seconds: f64) -> i32;
    fn CGDisplayHideCursor(display: u32) -> i32;
    fn CGDisplayShowCursor(display: u32) -> i32;
    fn CGGetOnlineDisplayList(
        max_displays: u32,
        online_displays: *mut u32,
        display_count: *mut u32,
    ) -> i32;
    fn CGCursorIsVisible() -> libc::boolean_t;
    fn _CGSDefaultConnection() -> i32;
    fn CGSSetConnectionProperty(
        cid: i32,
        target_cid: i32,
        key: *const std::ffi::c_void,
        val: *const std::ffi::c_void,
    ) -> i32;
}

#[cfg(target_os = "macos")]
static CURSOR_HIDDEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

// --- Programmatic warp echo suppression (Deskflow OSXScreen warp handling) ---
//
// A programmatic cursor warp (park / rescue / seat return) can surface as a
// synthetic MouseMoved event in the capture hook, carrying the full warp
// displacement as its delta. Feeding that echo into the roaming engine
// displaces the remote cursor away from the proportional edge-entry point
// (towards wherever the warp parked it) and races with real user input as
// occasional drift. Each warp registers its target here; the hook drops the
// first event that lands on it before the engine ever sees it.
// (Hook platforms only: Linux has no capture hooks and never warps.)
#[cfg(any(windows, target_os = "macos"))]
static PENDING_WARP_X: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
#[cfg(any(windows, target_os = "macos"))]
static PENDING_WARP_Y: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
#[cfg(any(windows, target_os = "macos"))]
static PENDING_WARP_VALID: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

#[cfg(any(windows, target_os = "macos"))]
fn mark_pending_warp(x: i32, y: i32) {
    use std::sync::atomic::Ordering;
    PENDING_WARP_X.store(x, Ordering::SeqCst);
    PENDING_WARP_Y.store(y, Ordering::SeqCst);
    PENDING_WARP_VALID.store(true, Ordering::SeqCst);
}

/// Returns true when the given hook position is the echo of the pending
/// programmatic warp (and consumes the pending marker).
#[cfg(any(windows, target_os = "macos"))]
fn take_warp_echo(x: i32, y: i32) -> bool {
    use std::sync::atomic::Ordering;
    if !PENDING_WARP_VALID.load(Ordering::SeqCst) {
        return false;
    }
    let wx = PENDING_WARP_X.load(Ordering::SeqCst);
    let wy = PENDING_WARP_Y.load(Ordering::SeqCst);
    if (x - wx).abs() <= 2 && (y - wy).abs() <= 2 {
        PENDING_WARP_VALID.store(false, Ordering::SeqCst);
        return true;
    }
    false
}

#[cfg(target_os = "macos")]
pub fn warp_mouse_position(x: i32, y: i32) {
    let point = objc2_core_foundation::CGPoint {
        x: x as f64,
        y: y as f64,
    };
    mark_pending_warp(x, y);
    unsafe {
        let _ = CGWarpMouseCursorPosition(point);
    }
    // Deskflow PR #10231 alignment:
    // A warp sends no event, so WindowServer and the Dock/Menu Bar keep active hover tracking
    // unless a mouse move event is seen leaving the hot-zone.
    // We post a synthetic MouseMoved event to SessionEventTap (below HIDEventTap so our own
    // tap ignores it) to prompt WindowServer to dismiss hover magnification and release cursor ownership.
    if let Some(event) = objc2_core_graphics::CGEvent::new_mouse_event(
        None,
        objc2_core_graphics::CGEventType::MouseMoved,
        point,
        objc2_core_graphics::CGMouseButton::Left,
    ) {
        objc2_core_graphics::CGEvent::post(
            objc2_core_graphics::CGEventTapLocation::SessionEventTap,
            Some(&event),
        );
    }
}

#[cfg(target_os = "macos")]
pub fn set_mouse_cursor_associated(associated: bool) {
    unsafe {
        // Deskflow OSXScreen::leave & OSXScreen::enter alignment:
        // Set "SetsCursorInBackground" on the CGS connection so WindowServer honors cursor operations
        // even when Tetherly is a background tray / accessory process without window focus.
        if let Ok(key_cstr) = std::ffi::CString::new("SetsCursorInBackground") {
            let key = CFStringCreateWithCString(
                std::ptr::null(),
                key_cstr.as_ptr(),
                0, // kCFStringEncodingMacRoman
            );
            let cid = _CGSDefaultConnection();
            let _ = CGSSetConnectionProperty(cid, cid, key, kCFBooleanTrue);
            if !key.is_null() {
                CFRelease(key);
            }
        }

        let mut displays = [0u32; 16];
        let mut count = 0u32;
        let _ = CGGetOnlineDisplayList(16, displays.as_mut_ptr(), &mut count);
        let count = if count == 0 {
            displays[0] = objc2_core_graphics::CGMainDisplayID();
            1
        } else {
            count as usize
        };

        if !associated {
            // Deskflow OSXScreen::leave & PR-10231:
            // 1. Immediately switch to blank transparent cursor so cursor is invisible prior to warp
            cursor_shield::set_blank_cursor_now();

            // 2. Deploy full-screen borderless transparent shield windows at NSScreenSaverWindowLevel (level 1000)
            //    covering all active displays with ignoresMouseEvents: NO. This completely blocks any
            //    underlying application or system element from receiving hover and overriding cursor shape.
            cursor_shield::show_cursor_shields();

            // 3. Hide hardware cursor across all active displays & Cocoa cursor stack
            if !CURSOR_HIDDEN.swap(true, std::sync::atomic::Ordering::SeqCst) {
                for &disp in &displays[..count] {
                    let _ = CGDisplayHideCursor(disp);
                }
                cursor_shield::hide_appkit_cursor();
            }

            // 4. Set suppression interval and disassociate mouse hardware from screen coordinates
            let _ = CGSetLocalEventsSuppressionInterval(0.0001);
            let _ = CGAssociateMouseAndMouseCursorPosition(false);

            // 5. Deskflow rehide verification: If WindowServer still reports the cursor is visible,
            //    cycle show/hide once to reset WindowServer's internal hide counter.
            if CGCursorIsVisible() != 0 {
                for &disp in &displays[..count] {
                    let _ = CGDisplayShowCursor(disp);
                    let _ = CGDisplayHideCursor(disp);
                }
            }
        } else {
            // Deskflow OSXScreen::enter:
            // 1. Remove all shield windows immediately, restoring native app hover and input
            cursor_shield::hide_cursor_shields();

            // 2. Show hardware cursor across all active displays & unhide AppKit cursor
            if CURSOR_HIDDEN.swap(false, std::sync::atomic::Ordering::SeqCst) {
                for &disp in &displays[..count] {
                    let _ = CGDisplayShowCursor(disp);
                }
                cursor_shield::unhide_appkit_cursor();
            }

            // 3. Re-associate mouse and reset suppression
            let _ = CGAssociateMouseAndMouseCursorPosition(true);
            let _ = CGSetLocalEventsSuppressionInterval(0.0);
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub fn set_mouse_cursor_associated(_associated: bool) {}

#[cfg(windows)]
pub fn warp_mouse_position(x: i32, y: i32) {
    use windows_sys::Win32::UI::WindowsAndMessaging::SetCursorPos;
    mark_pending_warp(x, y);
    unsafe {
        let _ = SetCursorPos(x, y);
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn warp_mouse_position(_x: i32, _y: i32) {}

#[cfg(windows)]
fn inject_mouse(flags: u32, dx: i32, dy: i32, mouse_data: u32) {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_MOUSE, MOUSEINPUT,
    };
    let input = MOUSEINPUT {
        dx,
        dy,
        mouseData: mouse_data,
        dwFlags: flags,
        time: 0,
        dwExtraInfo: 0,
    };
    unsafe {
        let sent = SendInput(
            1,
            [INPUT {
                r#type: INPUT_MOUSE,
                Anonymous: INPUT_0 { mi: input },
            }]
            .as_mut_ptr(),
            std::mem::size_of::<INPUT>() as i32,
        );
        if sent == 0 {
            let err = windows_sys::Win32::Foundation::GetLastError();
            tracing::warn!(flags, err, "SendInput failed to inject mouse event");
        }
    }
}

// ---------------------------------------------------------------------------
// SystemInputCapture — low-level hooks on the controller side
// ---------------------------------------------------------------------------

/// One raw local input observation from the hook thread. Mouse coordinates
/// are physical pixels; keys are Win32 virtual-key codes.
///
/// Defined on every platform: the Linux `start_input_hooks` stub keeps the
/// same signature as the real Windows/macOS hooks, so callers and the lib
/// re-export compile unchanged on targets without hook support.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawInput {
    Move { x: i32, y: i32, dx: i32, dy: i32 },
    Button { mask: u8, down: bool },
    Wheel { dx: i16, dy: i16 },
    Key { code: u16, down: bool },
    EmergencyEscape,
}

/// Bidirectional macOS CGKeyCode ↔ Win32 Virtual-Key mapping table.
pub const MAC_TO_WIN_KEYS: &[(u16, u16)] = &[
    (0x00, 0x41), // A
    (0x01, 0x53), // S
    (0x02, 0x44), // D
    (0x03, 0x46), // F
    (0x04, 0x48), // H
    (0x05, 0x47), // G
    (0x06, 0x5A), // Z
    (0x07, 0x58), // X
    (0x08, 0x43), // C
    (0x09, 0x56), // V
    (0x0A, 0xDC), // Section / ISO key (§ / ±) -> VK_OEM_5
    (0x0B, 0x42), // B
    (0x0C, 0x51), // Q
    (0x0D, 0x57), // W
    (0x0E, 0x45), // E
    (0x0F, 0x52), // R
    (0x10, 0x59), // Y
    (0x11, 0x54), // T
    (0x12, 0x31), // 1
    (0x13, 0x32), // 2
    (0x14, 0x33), // 3
    (0x15, 0x34), // 4
    (0x16, 0x36), // 6
    (0x17, 0x35), // 5
    (0x18, 0xBB), // = (VK_OEM_PLUS)
    (0x19, 0x39), // 9
    (0x1A, 0x37), // 7
    (0x1B, 0xBD), // - (VK_OEM_MINUS)
    (0x1C, 0x38), // 8
    (0x1D, 0x30), // 0
    (0x1E, 0xDD), // ] (VK_OEM_6)
    (0x1F, 0x4F), // O
    (0x20, 0x55), // U
    (0x21, 0xDB), // [ (VK_OEM_4)
    (0x22, 0x49), // I
    (0x23, 0x50), // P
    (0x24, 0x0D), // Return / Enter
    (0x25, 0x4C), // L
    (0x26, 0x4A), // J
    (0x27, 0xDE), // ' (VK_OEM_7)
    (0x28, 0x4B), // K
    (0x29, 0xBA), // ; (VK_OEM_1)
    (0x2A, 0xDC), // \ (VK_OEM_5)
    (0x2B, 0xBC), // , (VK_OEM_COMMA)
    (0x2C, 0xBF), // / (VK_OEM_2)
    (0x2D, 0x4E), // N
    (0x2E, 0x4D), // M
    (0x2F, 0xBE), // . (VK_OEM_PERIOD)
    (0x30, 0x09), // Tab
    (0x31, 0x20), // Space
    (0x32, 0xC0), // ` (VK_OEM_3)
    (0x33, 0x08), // Backspace (Delete)
    (0x35, 0x1B), // Escape
    (0x36, 0x5C), // Right Command -> VK_RWIN
    (0x37, 0x5B), // Left Command -> VK_LWIN
    (0x38, 0xA0), // Left Shift -> VK_LSHIFT
    (0x39, 0x14), // Caps Lock -> VK_CAPITAL
    (0x3A, 0xA4), // Left Option -> VK_LMENU
    (0x3B, 0xA2), // Left Control -> VK_LCONTROL
    (0x3C, 0xA1), // Right Shift -> VK_RSHIFT
    (0x3D, 0xA5), // Right Option -> VK_RMENU
    (0x3E, 0xA3), // Right Control -> VK_RCONTROL
    (0x40, 0x80), // F17 -> VK_F17
    (0x41, 0x6E), // Keypad Decimal
    (0x43, 0x6A), // Keypad Multiply
    (0x45, 0x6B), // Keypad Add
    (0x47, 0x0C), // Keypad Clear
    (0x4B, 0x6F), // Keypad Divide
    (0x4C, 0x0D), // Keypad Enter
    (0x4E, 0x6D), // Keypad Subtract
    (0x4F, 0x81), // F18 -> VK_F18
    (0x50, 0x82), // F19 -> VK_F19
    (0x51, 0xBB), // Keypad Equals -> VK_OEM_PLUS
    (0x52, 0x60), // Keypad 0
    (0x53, 0x61), // Keypad 1
    (0x54, 0x62), // Keypad 2
    (0x55, 0x63), // Keypad 3
    (0x56, 0x64), // Keypad 4
    (0x57, 0x65), // Keypad 5
    (0x58, 0x66), // Keypad 6
    (0x59, 0x67), // Keypad 7
    (0x5A, 0x83), // F20 -> VK_F20
    (0x5B, 0x68), // Keypad 8
    (0x5C, 0x69), // Keypad 9
    (0x60, 0x74), // F5
    (0x61, 0x75), // F6
    (0x62, 0x76), // F7
    (0x63, 0x72), // F3
    (0x64, 0x77), // F8
    (0x65, 0x78), // F9
    (0x67, 0x7A), // F11
    (0x69, 0x7C), // F13
    (0x6A, 0x7F), // F16 -> VK_F16
    (0x6B, 0x7D), // F14
    (0x6D, 0x79), // F10
    (0x6F, 0x7B), // F12
    (0x71, 0x7E), // F15
    (0x72, 0x2F), // Help
    (0x73, 0x24), // Home
    (0x74, 0x21), // PageUp
    (0x75, 0x2E), // Forward Delete
    (0x76, 0x73), // F4
    (0x77, 0x23), // End
    (0x78, 0x71), // F2
    (0x79, 0x22), // PageDown
    (0x7A, 0x70), // F1
    (0x7B, 0x25), // Left Arrow
    (0x7C, 0x27), // Right Arrow
    (0x7D, 0x28), // Down Arrow
    (0x7E, 0x26), // Up Arrow
];

pub fn mac_keycode_to_win_vk(mac_code: u16) -> Option<u16> {
    for &(mac, vk) in MAC_TO_WIN_KEYS {
        if mac == mac_code {
            return Some(vk);
        }
    }
    None
}

pub fn win_vk_to_mac_keycode(vk_code: u16) -> Option<u16> {
    match vk_code {
        0x10 => return Some(0x38), // VK_SHIFT -> Left Shift
        0x11 => return Some(0x3B), // VK_CONTROL -> Left Control
        0x12 => return Some(0x3A), // VK_MENU (Alt) -> Left Option
        _ => {}
    }
    for &(mac, vk) in MAC_TO_WIN_KEYS {
        if vk == vk_code {
            return Some(mac);
        }
    }
    None
}

/// Current cursor position in physical pixels. Verification helper for the
/// injection chain (the probe asserts the OS cursor actually moved).
#[cfg(windows)]
pub fn cursor_pos() -> (i32, i32) {
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;
    let mut pt = POINT { x: 0, y: 0 };
    unsafe { GetCursorPos(&mut pt) };
    (pt.x, pt.y)
}

#[cfg(target_os = "macos")]
pub fn cursor_pos() -> (i32, i32) {
    if let Some(ev) = objc2_core_graphics::CGEvent::new(None) {
        let pt = objc2_core_graphics::CGEvent::location(Some(&ev));
        (pt.x as i32, pt.y as i32)
    } else {
        (0, 0)
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn cursor_pos() -> (i32, i32) {
    (0, 0)
}

/// Primary-monitor size in pixels, for the controller's edge state machine.
#[cfg(windows)]
pub fn screen_size() -> (i32, i32) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};
    unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) }
}

#[cfg(target_os = "macos")]
pub fn screen_size() -> (i32, i32) {
    let main_id = objc2_core_graphics::CGMainDisplayID();
    let bounds = objc2_core_graphics::CGDisplayBounds(main_id);
    let w = bounds.size.width as i32;
    let h = bounds.size.height as i32;
    (w.max(1), h.max(1))
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn screen_size() -> (i32, i32) {
    (1920, 1080)
}

/// Arm/disarm the keyboard hook on the capture thread. The keyboard hook
/// records real keystrokes, so it is only armed while the seat is on the
/// remote machine; mouse capture stays on for edge-crossing detection and
/// reads cursor position only.
#[cfg(windows)]
#[derive(Debug, Clone, Copy)]
pub struct HookControl {
    thread_id: u32,
}

#[cfg(windows)]
impl HookControl {
    pub fn is_armed(&self) -> bool {
        true
    }

    pub fn set_keyboard(&self, on: bool) -> Result<(), CoreError> {
        use windows_sys::Win32::UI::WindowsAndMessaging::PostThreadMessageW;
        const WM_HOOK_KB_ON: u32 = 0x8000 + 1; // WM_APP + 1
        const WM_HOOK_KB_OFF: u32 = 0x8000 + 2; // WM_APP + 2
        unsafe {
            if PostThreadMessageW(
                self.thread_id,
                if on { WM_HOOK_KB_ON } else { WM_HOOK_KB_OFF },
                0,
                0,
            ) == 0
            {
                return Err(CoreError::Json("PostThreadMessageW failed".into()));
            }
        }
        Ok(())
    }
}

#[cfg(target_os = "macos")]
#[derive(Debug, Clone)]
pub struct HookControl {
    keyboard_armed: std::sync::Arc<std::sync::atomic::AtomicBool>,
    tap_port: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    pressed_modifiers: std::sync::Arc<std::sync::Mutex<std::collections::HashSet<u16>>>,
    tx: std::sync::mpsc::Sender<RawInput>,
}

#[cfg(target_os = "macos")]
impl HookControl {
    pub fn is_armed(&self) -> bool {
        self.keyboard_armed
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    pub fn set_keyboard(&self, on: bool) -> Result<(), CoreError> {
        let prev = self
            .keyboard_armed
            .swap(on, std::sync::atomic::Ordering::SeqCst);
        if on {
            // Proactively ensure the event tap is enabled in WindowServer whenever arming
            let port_addr = self.tap_port.load(std::sync::atomic::Ordering::SeqCst);
            if port_addr != 0 {
                let port = unsafe { &*(port_addr as *const objc2_core_foundation::CFMachPort) };
                objc2_core_graphics::CGEvent::tap_enable(port, true);
            }
        } else if let Ok(mut pressed) = self.pressed_modifiers.lock() {
            // Seat returning to Local: flush key-up events for any held modifiers to remote
            // before clearing, preventing stuck modifiers (Shift, Ctrl, Alt, Command) on target.
            for mac_key in pressed.drain() {
                if let Some(vk) = mac_keycode_to_win_vk(mac_key) {
                    let _ = self.tx.send(RawInput::Key {
                        code: vk,
                        down: false,
                    });
                }
            }
        }
        if prev != on {
            // Deskflow OSXScreen::leave / OSXScreen::enter alignment:
            // When leaving local screen (on = true), disassociate hardware mouse cursor
            // so raw deltas stream smoothly to target without macOS screen border friction.
            // When returning home (on = false), re-associate hardware mouse cursor.
            set_mouse_cursor_associated(!on);
        }
        Ok(())
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
#[derive(Debug, Clone, Copy)]
pub struct HookControl;

#[cfg(not(any(windows, target_os = "macos")))]
impl HookControl {
    pub fn is_armed(&self) -> bool {
        false
    }

    pub fn set_keyboard(&self, _on: bool) -> Result<(), CoreError> {
        Ok(())
    }
}

/// Install the low-level mouse hook (always) on a dedicated pumped thread and
/// stream raw events into `tx`. Injected events (this node's own SendInput on
/// a single-machine setup) are filtered out, so a machine that is both
/// controller and controlled cannot feed itself back. The keyboard hook is
/// armed later through the returned control handle, once the seat moves
/// remote.
#[cfg(windows)]
pub fn start_input_hooks(tx: std::sync::mpsc::Sender<RawInput>) -> Result<HookControl, CoreError> {
    use windows_sys::Win32::Foundation::{LPARAM, LRESULT, POINT, WPARAM};
    use windows_sys::Win32::System::Threading::GetCurrentThreadId;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, GetMessageW, SetWindowsHookExW, UnhookWindowsHookEx, HHOOK, HOOKPROC,
        KBDLLHOOKSTRUCT, LLKHF_INJECTED, LLMHF_INJECTED, MSG, MSLLHOOKSTRUCT, WH_KEYBOARD_LL,
        WH_MOUSE_LL, WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN,
        WM_MBUTTONUP, WM_MOUSEHWHEEL, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_RBUTTONDOWN, WM_RBUTTONUP,
        WM_SYSKEYDOWN, WM_SYSKEYUP,
    };

    const WM_HOOK_KB_ON: u32 = 0x8000 + 1; // WM_APP + 1
    const WM_HOOK_KB_OFF: u32 = 0x8000 + 2; // WM_APP + 2

    static SENDER: std::sync::OnceLock<std::sync::mpsc::Sender<RawInput>> =
        std::sync::OnceLock::new();
    SENDER
        .set(tx)
        .map_err(|_| CoreError::Json("input hooks already started".into()))?;

    unsafe fn push(ev: RawInput) {
        if let Some(tx) = SENDER.get() {
            // Best effort under a stalled consumer: the system unhooks a slow
            // low-level hook anyway, so never block the callback.
            let _ = tx.send(ev);
        }
    }

    unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code >= 0 {
            let mi = &*(lparam as *const MSLLHOOKSTRUCT);
            // Never re-capture this node's own injected input.
            if mi.flags & LLMHF_INJECTED == 0 {
                let ev = wparam as u32;
                match ev {
                    WM_MOUSEMOVE => {
                        // Drop the echo of a programmatic SetCursorPos warp
                        // before it can reach the roaming engine.
                        if take_warp_echo(mi.pt.x, mi.pt.y) {
                            return CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam);
                        }
                        push(RawInput::Move {
                            x: mi.pt.x,
                            y: mi.pt.y,
                            dx: 0,
                            dy: 0,
                        })
                    }
                    WM_LBUTTONDOWN => push(RawInput::Button {
                        mask: tetherly_core::BTN_LEFT,
                        down: true,
                    }),
                    WM_LBUTTONUP => push(RawInput::Button {
                        mask: tetherly_core::BTN_LEFT,
                        down: false,
                    }),
                    WM_RBUTTONDOWN => push(RawInput::Button {
                        mask: tetherly_core::BTN_RIGHT,
                        down: true,
                    }),
                    WM_RBUTTONUP => push(RawInput::Button {
                        mask: tetherly_core::BTN_RIGHT,
                        down: false,
                    }),
                    WM_MBUTTONDOWN => push(RawInput::Button {
                        mask: tetherly_core::BTN_MIDDLE,
                        down: true,
                    }),
                    WM_MBUTTONUP => push(RawInput::Button {
                        mask: tetherly_core::BTN_MIDDLE,
                        down: false,
                    }),
                    WM_MOUSEWHEEL => push(RawInput::Wheel {
                        dx: 0,
                        dy: (mi.mouseData >> 16) as i16,
                    }),
                    WM_MOUSEHWHEEL => push(RawInput::Wheel {
                        dx: (mi.mouseData >> 16) as i16,
                        dy: 0,
                    }),
                    _ => {}
                }
            }
        }
        CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam)
    }

    unsafe extern "system" fn keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code >= 0 {
            let ki = &*(lparam as *const KBDLLHOOKSTRUCT);
            if ki.flags & LLKHF_INJECTED == 0 {
                let ev = wparam as u32;
                if matches!(ev, WM_KEYDOWN | WM_SYSKEYDOWN) {
                    push(RawInput::Key {
                        code: ki.vkCode as u16,
                        down: true,
                    });
                } else if matches!(ev, WM_KEYUP | WM_SYSKEYUP) {
                    push(RawInput::Key {
                        code: ki.vkCode as u16,
                        down: false,
                    });
                }
            }
        }
        CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam)
    }

    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<u32, String>>();
    std::thread::Builder::new()
        .name("tetherly-input-hooks".into())
        .spawn(move || unsafe {
            let mouse_proc_fn: HOOKPROC = Some(mouse_proc);
            let keyboard_proc_fn: HOOKPROC = Some(keyboard_proc);
            let mouse = SetWindowsHookExW(WH_MOUSE_LL, mouse_proc_fn, std::ptr::null_mut(), 0);
            let mut keyboard: HHOOK = std::ptr::null_mut();
            if mouse.is_null() {
                let _ = ready_tx.send(Err("SetWindowsHookExW(mouse) failed".into()));
                return;
            }
            let _ = ready_tx.send(Ok(GetCurrentThreadId()));

            let mut msg = MSG {
                hwnd: std::ptr::null_mut(),
                message: 0,
                wParam: 0,
                lParam: 0,
                time: 0,
                pt: POINT { x: 0, y: 0 },
            };
            loop {
                // 0 = WM_QUIT, -1 = error: both end the pump.
                if GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) <= 0 {
                    break;
                }
                match msg.message {
                    WM_HOOK_KB_ON => {
                        if keyboard.is_null() {
                            keyboard = SetWindowsHookExW(
                                WH_KEYBOARD_LL,
                                keyboard_proc_fn,
                                std::ptr::null_mut(),
                                0,
                            );
                        }
                    }
                    WM_HOOK_KB_OFF if !keyboard.is_null() => {
                        UnhookWindowsHookEx(keyboard);
                        keyboard = std::ptr::null_mut();
                    }
                    _ => {}
                }
            }
            if !keyboard.is_null() {
                UnhookWindowsHookEx(keyboard);
            }
            UnhookWindowsHookEx(mouse);
        })
        .map_err(|e| CoreError::Json(format!("spawn input hooks: {e}")))?;

    let thread_id = ready_rx
        .recv()
        .map_err(|e| CoreError::Json(format!("input hook thread: {e}")))?
        .map_err(CoreError::Json)?;
    Ok(HookControl { thread_id })
}

#[cfg(target_os = "macos")]
#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> bool;
    fn AXIsProcessTrustedWithOptions(options: *const std::ffi::c_void) -> bool;
    fn CFStringCreateWithCString(
        alloc: *const std::ffi::c_void,
        c_str: *const std::ffi::c_char,
        encoding: u32,
    ) -> *const std::ffi::c_void;
    fn CFDictionaryCreate(
        allocator: *const std::ffi::c_void,
        keys: *const *const std::ffi::c_void,
        values: *const *const std::ffi::c_void,
        num_values: isize,
        key_callbacks: *const std::ffi::c_void,
        value_callbacks: *const std::ffi::c_void,
    ) -> *const std::ffi::c_void;
    fn CFRelease(cf: *const std::ffi::c_void);
    static kCFBooleanTrue: *const std::ffi::c_void;
    static kCFTypeDictionaryKeyCallBacks: std::ffi::c_void;
    static kCFTypeDictionaryValueCallBacks: std::ffi::c_void;
}

#[cfg(target_os = "macos")]
pub fn is_accessibility_trusted() -> bool {
    // Pure, silent query: never prompts or triggers system modal permission popups.
    unsafe { AXIsProcessTrusted() }
}

#[cfg(not(target_os = "macos"))]
pub fn is_accessibility_trusted() -> bool {
    true
}

pub fn open_accessibility_settings() -> Result<(), CoreError> {
    #[cfg(target_os = "macos")]
    {
        unsafe {
            if let Ok(prompt_key_str) = std::ffi::CString::new("AXTrustedCheckOptionPrompt") {
                let prompt_key = CFStringCreateWithCString(
                    std::ptr::null(),
                    prompt_key_str.as_ptr(),
                    0x0800_0100, // kCFStringEncodingUTF8
                );
                if !prompt_key.is_null() {
                    let keys = [prompt_key];
                    let values = [kCFBooleanTrue];
                    let dict = CFDictionaryCreate(
                        std::ptr::null(),
                        keys.as_ptr(),
                        values.as_ptr(),
                        1,
                        &kCFTypeDictionaryKeyCallBacks as *const _ as *const _,
                        &kCFTypeDictionaryValueCallBacks as *const _ as *const _,
                    );
                    if !dict.is_null() {
                        let _ = AXIsProcessTrustedWithOptions(dict);
                        CFRelease(dict);
                    }
                    CFRelease(prompt_key);
                }
            }
        }
        std::process::Command::new("open")
            .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
            .spawn()
            .map_err(|e| CoreError::Json(format!("open settings: {e}")))?;
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        // Be honest instead of pretending success: Windows/Linux have no
        // macOS-style Accessibility prompt to open. The UI uses `ok` to
        // tell the user whether anything actually happened.
        Err(CoreError::Json(
            "accessibility settings are only applicable on macOS".to_string(),
        ))
    }
}

#[cfg(target_os = "macos")]
struct MacTapContext {
    tx: std::sync::mpsc::Sender<RawInput>,
    keyboard_armed: std::sync::Arc<std::sync::atomic::AtomicBool>,
    pressed_modifiers: std::sync::Arc<std::sync::Mutex<std::collections::HashSet<u16>>>,
    tap_port: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    _retained_port: std::sync::Mutex<
        Option<objc2_core_foundation::CFRetained<objc2_core_foundation::CFMachPort>>,
    >,
}

#[cfg(target_os = "macos")]
pub fn start_input_hooks(tx: std::sync::mpsc::Sender<RawInput>) -> Result<HookControl, CoreError> {
    use objc2_core_foundation::{kCFRunLoopCommonModes, CFMachPort, CFRunLoop};
    use objc2_core_graphics::{
        CGEvent, CGEventField, CGEventFlags, CGEventMask, CGEventTapLocation, CGEventTapOptions,
        CGEventTapPlacement, CGEventTapProxy, CGEventType,
    };
    use std::sync::atomic::Ordering;

    if !is_accessibility_trusted() {
        return Err(CoreError::Json(
            "macOS Accessibility permission is required for global input capture (System Settings > Privacy & Security > Accessibility)".into(),
        ));
    }

    let keyboard_armed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let tap_port = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let pressed_modifiers =
        std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashSet::new()));

    let control = HookControl {
        keyboard_armed: std::sync::Arc::clone(&keyboard_armed),
        tap_port: std::sync::Arc::clone(&tap_port),
        pressed_modifiers: std::sync::Arc::clone(&pressed_modifiers),
        tx: tx.clone(),
    };
    let keyboard_armed_thread = std::sync::Arc::clone(&keyboard_armed);
    let tap_port_thread = std::sync::Arc::clone(&tap_port);
    let pressed_modifiers_thread = std::sync::Arc::clone(&pressed_modifiers);

    unsafe extern "C-unwind" fn tap_callback(
        _proxy: CGEventTapProxy,
        event_type: CGEventType,
        event: std::ptr::NonNull<CGEvent>,
        user_info: *mut std::ffi::c_void,
    ) -> *mut CGEvent {
        let ctx = &*(user_info as *const MacTapContext);
        let cg_event = event.as_ref();

        if event_type == CGEventType::TapDisabledByTimeout
            || event_type == CGEventType::TapDisabledByUserInput
        {
            let port_addr = ctx.tap_port.load(Ordering::SeqCst);
            if port_addr != 0 {
                let port = unsafe { &*(port_addr as *const CFMachPort) };
                CGEvent::tap_enable(port, true);
            }
            return event.as_ptr();
        }

        let user_data =
            CGEvent::integer_value_field(Some(cg_event), CGEventField::EventSourceUserData);
        if user_data == TETHERLY_INJECTED_MAGIC {
            return event.as_ptr();
        }

        match event_type {
            CGEventType::MouseMoved
            | CGEventType::LeftMouseDragged
            | CGEventType::RightMouseDragged
            | CGEventType::OtherMouseDragged => {
                let loc = CGEvent::location(Some(cg_event));
                let dx =
                    CGEvent::integer_value_field(Some(cg_event), CGEventField::MouseEventDeltaX)
                        as i32;
                let dy =
                    CGEvent::integer_value_field(Some(cg_event), CGEventField::MouseEventDeltaY)
                        as i32;
                if take_warp_echo(loc.x as i32, loc.y as i32) {
                    // Echo of our own programmatic warp: swallow it without
                    // feeding the roaming engine (the delta is the warp
                    // displacement, not user motion).
                    let armed = ctx.keyboard_armed.load(Ordering::SeqCst);
                    return if armed {
                        std::ptr::null_mut()
                    } else {
                        event.as_ptr()
                    };
                }
                let _ = ctx.tx.send(RawInput::Move {
                    x: loc.x as i32,
                    y: loc.y as i32,
                    dx,
                    dy,
                });
                let armed = ctx.keyboard_armed.load(Ordering::SeqCst);
                if armed {
                    std::ptr::null_mut()
                } else {
                    event.as_ptr()
                }
            }
            CGEventType::LeftMouseDown => {
                let armed = ctx.keyboard_armed.load(Ordering::SeqCst);
                if armed {
                    let _ = ctx.tx.send(RawInput::Button {
                        mask: tetherly_core::BTN_LEFT,
                        down: true,
                    });
                    std::ptr::null_mut()
                } else {
                    event.as_ptr()
                }
            }
            CGEventType::LeftMouseUp => {
                let armed = ctx.keyboard_armed.load(Ordering::SeqCst);
                if armed {
                    let _ = ctx.tx.send(RawInput::Button {
                        mask: tetherly_core::BTN_LEFT,
                        down: false,
                    });
                    std::ptr::null_mut()
                } else {
                    event.as_ptr()
                }
            }
            CGEventType::RightMouseDown => {
                let armed = ctx.keyboard_armed.load(Ordering::SeqCst);
                if armed {
                    let _ = ctx.tx.send(RawInput::Button {
                        mask: tetherly_core::BTN_RIGHT,
                        down: true,
                    });
                    std::ptr::null_mut()
                } else {
                    event.as_ptr()
                }
            }
            CGEventType::RightMouseUp => {
                let armed = ctx.keyboard_armed.load(Ordering::SeqCst);
                if armed {
                    let _ = ctx.tx.send(RawInput::Button {
                        mask: tetherly_core::BTN_RIGHT,
                        down: false,
                    });
                    std::ptr::null_mut()
                } else {
                    event.as_ptr()
                }
            }
            CGEventType::OtherMouseDown => {
                let armed = ctx.keyboard_armed.load(Ordering::SeqCst);
                if armed {
                    let _ = ctx.tx.send(RawInput::Button {
                        mask: tetherly_core::BTN_MIDDLE,
                        down: true,
                    });
                    std::ptr::null_mut()
                } else {
                    event.as_ptr()
                }
            }
            CGEventType::OtherMouseUp => {
                let armed = ctx.keyboard_armed.load(Ordering::SeqCst);
                if armed {
                    let _ = ctx.tx.send(RawInput::Button {
                        mask: tetherly_core::BTN_MIDDLE,
                        down: false,
                    });
                    std::ptr::null_mut()
                } else {
                    event.as_ptr()
                }
            }
            CGEventType::ScrollWheel => {
                let armed = ctx.keyboard_armed.load(Ordering::SeqCst);
                if armed {
                    let mut dy = CGEvent::integer_value_field(
                        Some(cg_event),
                        CGEventField::ScrollWheelEventDeltaAxis1,
                    ) as i16;
                    let mut dx = CGEvent::integer_value_field(
                        Some(cg_event),
                        CGEventField::ScrollWheelEventDeltaAxis2,
                    ) as i16;
                    if dy == 0 && dx == 0 {
                        let pty = CGEvent::integer_value_field(
                            Some(cg_event),
                            CGEventField::ScrollWheelEventPointDeltaAxis1,
                        ) as i16;
                        let ptx = CGEvent::integer_value_field(
                            Some(cg_event),
                            CGEventField::ScrollWheelEventPointDeltaAxis2,
                        ) as i16;
                        if pty != 0 || ptx != 0 {
                            dy = pty.signum();
                            dx = ptx.signum();
                        }
                    }
                    if dy != 0 || dx != 0 {
                        let _ = ctx.tx.send(RawInput::Wheel { dx, dy });
                    }
                    std::ptr::null_mut()
                } else {
                    event.as_ptr()
                }
            }
            CGEventType::KeyDown => {
                let armed = ctx.keyboard_armed.load(Ordering::SeqCst);
                let keycode = CGEvent::integer_value_field(
                    Some(cg_event),
                    CGEventField::KeyboardEventKeycode,
                ) as u16;

                // Emergency disarm valve:
                // 1) Escape keycode on macOS is 0x35. If triple-tapped within 400ms per interval,
                //    immediately force-disarm the event tap and return control locally.
                //    (Vim double-tap Esc will NOT falsely trigger disarm).
                // 2) Any Ctrl+Alt+Esc or Ctrl+Cmd+Esc combo triggers immediate disarm.
                //    (Single-modifier Ctrl+Esc / Alt+Esc are preserved for Windows Start Menu / Task Switch).
                if keycode == 0x35 {
                    static LAST_ESC_MS: std::sync::atomic::AtomicU64 =
                        std::sync::atomic::AtomicU64::new(0);
                    static ESC_TAP_COUNT: std::sync::atomic::AtomicU32 =
                        std::sync::atomic::AtomicU32::new(0);
                    let now_ms = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_millis() as u64)
                        .unwrap_or(0);
                    let prev_esc = LAST_ESC_MS.swap(now_ms, Ordering::SeqCst);
                    let flags = CGEvent::flags(Some(cg_event));
                    let has_ctrl = flags.contains(CGEventFlags::MaskControl);
                    let has_alt = flags.contains(CGEventFlags::MaskAlternate);
                    let has_cmd = flags.contains(CGEventFlags::MaskCommand);

                    let is_combo = (has_ctrl && has_alt) || (has_ctrl && has_cmd);
                    let tap_interval = now_ms.saturating_sub(prev_esc);
                    let is_triple_tap = if tap_interval < 400 {
                        ESC_TAP_COUNT.fetch_add(1, Ordering::SeqCst) + 1 >= 3
                    } else {
                        ESC_TAP_COUNT.store(1, Ordering::SeqCst);
                        false
                    };

                    if armed && (is_combo || is_triple_tap) {
                        ESC_TAP_COUNT.store(0, Ordering::SeqCst);
                        ctx.keyboard_armed.store(false, Ordering::SeqCst);
                        set_mouse_cursor_associated(true);
                        let _ = ctx.tx.send(RawInput::EmergencyEscape);
                        return event.as_ptr();
                    }
                }

                if armed {
                    if let Some(vk) = mac_keycode_to_win_vk(keycode) {
                        let _ = ctx.tx.send(RawInput::Key {
                            code: vk,
                            down: true,
                        });
                    }
                    std::ptr::null_mut()
                } else {
                    event.as_ptr()
                }
            }
            CGEventType::KeyUp => {
                let armed = ctx.keyboard_armed.load(Ordering::SeqCst);
                if armed {
                    let keycode = CGEvent::integer_value_field(
                        Some(cg_event),
                        CGEventField::KeyboardEventKeycode,
                    ) as u16;
                    if let Some(vk) = mac_keycode_to_win_vk(keycode) {
                        let _ = ctx.tx.send(RawInput::Key {
                            code: vk,
                            down: false,
                        });
                    }
                    std::ptr::null_mut()
                } else {
                    event.as_ptr()
                }
            }
            CGEventType::FlagsChanged => {
                let armed = ctx.keyboard_armed.load(Ordering::SeqCst);
                let keycode = CGEvent::integer_value_field(
                    Some(cg_event),
                    CGEventField::KeyboardEventKeycode,
                ) as u16;

                if keycode == 0x39 {
                    // Caps Lock on macOS only generates FlagsChanged toggle state transitions without
                    // individual hardware KeyUp events. On Windows, Caps Lock requires a full press
                    // cycle (KeyDown + KeyUp) to toggle the VK_CAPITAL latch state cleanly without
                    // leaving VK_CAPITAL permanently held down.
                    if armed {
                        let _ = ctx.tx.send(RawInput::Key {
                            code: 0x14, // VK_CAPITAL
                            down: true,
                        });
                        let _ = ctx.tx.send(RawInput::Key {
                            code: 0x14, // VK_CAPITAL
                            down: false,
                        });
                        return std::ptr::null_mut();
                    } else {
                        return event.as_ptr();
                    }
                }

                let flags = CGEvent::flags(Some(cg_event));

                let mask = match keycode {
                    0x37 | 0x36 => CGEventFlags::MaskCommand,
                    0x38 | 0x3C => CGEventFlags::MaskShift,
                    0x3A | 0x3D => CGEventFlags::MaskAlternate,
                    0x3B | 0x3E => CGEventFlags::MaskControl,
                    0x3F => CGEventFlags::MaskSecondaryFn,
                    _ => CGEventFlags::empty(),
                };

                if mask.is_empty() {
                    return event.as_ptr();
                }

                let is_down = if let Ok(mut pressed) = ctx.pressed_modifiers.lock() {
                    if flags.contains(mask) && !mask.is_empty() {
                        if pressed.contains(&keycode) {
                            pressed.remove(&keycode);
                            false
                        } else {
                            pressed.insert(keycode);
                            true
                        }
                    } else {
                        pressed.remove(&keycode);
                        false
                    }
                } else {
                    flags.contains(mask)
                };

                if armed {
                    if let Some(vk) = mac_keycode_to_win_vk(keycode) {
                        let _ = ctx.tx.send(RawInput::Key {
                            code: vk,
                            down: is_down,
                        });
                    }
                    std::ptr::null_mut()
                } else {
                    event.as_ptr()
                }
            }
            _ => {
                const NX_SYSDEFINED: u32 = 14;
                if event_type.0 == NX_SYSDEFINED {
                    let armed = ctx.keyboard_armed.load(Ordering::SeqCst);
                    if armed {
                        let mut swallowed = false;
                        objc2::rc::autoreleasepool(|_| {
                            if let Some(ns_event) =
                                objc2_app_kit::NSEvent::eventWithCGEvent(cg_event)
                            {
                                if ns_event.subtype().0 == 8 {
                                    swallowed = true;
                                    let data1 = ns_event.data1() as i64;
                                    let nx_key_id = ((data1 & 0xFFFF_0000) >> 16) as u32;
                                    let is_down = (data1 & 0x100) == 0;
                                    let win_vk = match nx_key_id {
                                        0 => Some(0xAF),       // NX_KEYTYPE_SOUND_UP -> VK_VOLUME_UP
                                        1 => Some(0xAE), // NX_KEYTYPE_SOUND_DOWN -> VK_VOLUME_DOWN
                                        7 => Some(0xAD), // NX_KEYTYPE_MUTE -> VK_VOLUME_MUTE
                                        16 => Some(0xB3), // NX_KEYTYPE_PLAY -> VK_MEDIA_PLAY_PAUSE
                                        17 | 19 => Some(0xB0), // NX_KEYTYPE_NEXT / FAST -> VK_MEDIA_NEXT_TRACK
                                        18 | 20 => Some(0xB1), // NX_KEYTYPE_PREVIOUS / REWIND -> VK_MEDIA_PREV_TRACK
                                        5 => Some(0x2F),       // NX_KEYTYPE_HELP -> VK_HELP
                                        13 => Some(0x5B),      // NX_KEYTYPE_LAUNCH_PANEL -> VK_LWIN
                                        _ => None,
                                    };
                                    if let Some(vk) = win_vk {
                                        if matches!(vk, 0xAD..=0xB3) {
                                            // Media pulse keys (Volume, Mute, Play/Pause, Next/Prev):
                                            // Send a full down+up pulse for each tap or repeat event,
                                            // preventing stuck media keys and enabling smooth volume repeat.
                                            if is_down {
                                                let _ = ctx.tx.send(RawInput::Key {
                                                    code: vk,
                                                    down: true,
                                                });
                                                let _ = ctx.tx.send(RawInput::Key {
                                                    code: vk,
                                                    down: false,
                                                });
                                            }
                                        } else {
                                            let _ = ctx.tx.send(RawInput::Key {
                                                code: vk,
                                                down: is_down,
                                            });
                                        }
                                    }
                                }
                            }
                        });
                        if swallowed {
                            return std::ptr::null_mut();
                        }
                    }
                }
                event.as_ptr()
            }
        }
    }

    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(), String>>();

    std::thread::Builder::new()
        .name("tetherly-input-tap".into())
        .spawn(move || {
            let ctx = Box::new(MacTapContext {
                tx,
                keyboard_armed: keyboard_armed_thread,
                pressed_modifiers: pressed_modifiers_thread,
                tap_port: tap_port_thread,
                _retained_port: std::sync::Mutex::new(None),
            });

            let mask: CGEventMask = (1u64 << CGEventType::MouseMoved.0)
                | (1u64 << CGEventType::LeftMouseDown.0)
                | (1u64 << CGEventType::LeftMouseUp.0)
                | (1u64 << CGEventType::LeftMouseDragged.0)
                | (1u64 << CGEventType::RightMouseDown.0)
                | (1u64 << CGEventType::RightMouseUp.0)
                | (1u64 << CGEventType::RightMouseDragged.0)
                | (1u64 << CGEventType::OtherMouseDown.0)
                | (1u64 << CGEventType::OtherMouseUp.0)
                | (1u64 << CGEventType::OtherMouseDragged.0)
                | (1u64 << CGEventType::ScrollWheel.0)
                | (1u64 << CGEventType::KeyDown.0)
                | (1u64 << CGEventType::KeyUp.0)
                | (1u64 << CGEventType::FlagsChanged.0)
                | (1u64 << 14) // NX_SYSDEFINED = 14 (media keys, volume up/down/mute, etc.)
                // kCGEventTapDisabledByTimeout (0xFFFFFFFE) and
                // kCGEventTapDisabledByUserInput (0xFFFFFFFF) land on mask
                // bits 62/63 on 64-bit (CGEventMaskBit wraps the shift).
                // Without them the tap never receives the disabled
                // notification and the self-healing re-enable in
                // `tap_callback` stays unreachable.
                | (1u64 << 62)
                | (1u64 << 63);

            let user_info_ptr = &*ctx as *const MacTapContext as *mut std::ffi::c_void;

            let tap = unsafe {
                CGEvent::tap_create(
                    CGEventTapLocation::HIDEventTap,
                    CGEventTapPlacement::HeadInsertEventTap,
                    CGEventTapOptions::Default,
                    mask,
                    Some(tap_callback),
                    user_info_ptr,
                )
            };

            let port = match tap {
                Some(p) => p,
                None => {
                    let _ = ready_tx.send(Err(
                        "CGEventTapCreate returned null (Accessibility permission required)".into(),
                    ));
                    return;
                }
            };

            let rl_source = match CFMachPort::new_run_loop_source(None, Some(&port), 0) {
                Some(s) => s,
                None => {
                    let _ = ready_tx.send(Err("CFMachPortCreateRunLoopSource failed".into()));
                    return;
                }
            };

            if let Some(rl) = CFRunLoop::current() {
                rl.add_source(Some(&rl_source), unsafe { kCFRunLoopCommonModes });
                CGEvent::tap_enable(&port, true);
                ctx.tap_port
                    .store(&*port as *const CFMachPort as usize, Ordering::SeqCst);
                if let Ok(mut guard) = ctx._retained_port.lock() {
                    *guard = Some(port);
                }
                let _ = ready_tx.send(Ok(()));
                CFRunLoop::run();
            } else {
                let _ = ready_tx.send(Err("CFRunLoopGetCurrent returned null".into()));
            }
        })
        .map_err(|e| CoreError::Json(format!("spawn input tap thread: {e}")))?;

    ready_rx
        .recv()
        .map_err(|e| CoreError::Json(format!("input hook ready channel: {e}")))?
        .map_err(CoreError::Json)?;

    Ok(control)
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn start_input_hooks(_tx: std::sync::mpsc::Sender<RawInput>) -> Result<HookControl, CoreError> {
    Err(CoreError::Json(
        "input hooks are currently supported on Windows and macOS".into(),
    ))
}

#[cfg(test)]
mod input_tests {
    use super::*;

    #[test]
    fn mac_win_keycodes_bidirectional() {
        // Test essential keys
        assert_eq!(mac_keycode_to_win_vk(0x00), Some(0x41)); // 'A'
        assert_eq!(win_vk_to_mac_keycode(0x41), Some(0x00));

        assert_eq!(mac_keycode_to_win_vk(0x31), Some(0x20)); // Space
        assert_eq!(win_vk_to_mac_keycode(0x20), Some(0x31));

        assert_eq!(mac_keycode_to_win_vk(0x24), Some(0x0D)); // Enter
        assert_eq!(win_vk_to_mac_keycode(0x0D), Some(0x24));

        assert_eq!(mac_keycode_to_win_vk(0x37), Some(0x5B)); // Cmd -> VK_LWIN
        assert_eq!(win_vk_to_mac_keycode(0x5B), Some(0x37));

        assert_eq!(mac_keycode_to_win_vk(0x3B), Some(0xA2)); // Ctrl -> VK_LCONTROL
        assert_eq!(win_vk_to_mac_keycode(0xA2), Some(0x3B));

        // Generic VK modifiers map to left Mac modifiers
        assert_eq!(win_vk_to_mac_keycode(0x10), Some(0x38)); // VK_SHIFT -> Left Shift
        assert_eq!(win_vk_to_mac_keycode(0x11), Some(0x3B)); // VK_CONTROL -> Left Control
        assert_eq!(win_vk_to_mac_keycode(0x12), Some(0x3A)); // VK_MENU -> Left Option
    }

    #[cfg(windows)]
    #[test]
    fn units_map_to_full_absolute_range() {
        use super::units_to_abs;
        assert_eq!(units_to_abs(0), 0);
        assert_eq!(units_to_abs(1000), 65_535);
        assert_eq!(units_to_abs(500), 32_767);
        assert_eq!(units_to_abs(-5), 0);
        assert_eq!(units_to_abs(2000), 65_535);
        let mut prev = -1;
        for u in 0..=1000 {
            let abs = units_to_abs(u);
            assert!(abs >= prev);
            prev = abs;
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn mac_screen_size_uses_logical_display_bounds() {
        let (w, h) = screen_size();
        assert!(w > 0, "screen width must be positive");
        assert!(h > 0, "screen height must be positive");
        // On macOS Retina displays, logical bounds are scaled points (e.g. 1512x982 or 1728x1117),
        // whereas physical backing buffer pixels would be 2x or 3x (3024x1964 or 3456x2234).
        assert!(
            w <= 4096,
            "logical width in points should be reasonable, got {w}"
        );
    }

    #[test]
    fn modifier_tracking_dual_keys_state_machine() {
        use std::collections::HashSet;

        fn step(pressed: &mut HashSet<u16>, keycode: u16, mask_active: bool) -> bool {
            if mask_active {
                if pressed.contains(&keycode) {
                    pressed.remove(&keycode);
                    false
                } else {
                    pressed.insert(keycode);
                    true
                }
            } else {
                pressed.remove(&keycode);
                false
            }
        }

        let mut pressed = HashSet::new();

        // 1. Press Left Command (0x37) -> down
        assert!(step(&mut pressed, 0x37, true));
        assert!(pressed.contains(&0x37));

        // 2. Press Right Command (0x36) while Left is still down -> down
        assert!(step(&mut pressed, 0x36, true));
        assert!(pressed.contains(&0x36));

        // 3. Release Left Command (0x37) while Right Command keeps mask active -> up (down: false)
        assert!(!step(&mut pressed, 0x37, true));
        // Critical fix verification: Left Command must be removed from pressed!
        assert!(!pressed.contains(&0x37));
        assert!(pressed.contains(&0x36));

        // 4. Press Left Command (0x37) AGAIN while Right is still held -> down
        assert!(step(&mut pressed, 0x37, true));
        assert!(pressed.contains(&0x37));

        // 5. Release Right Command (0x36) -> up
        assert!(!step(&mut pressed, 0x36, true));
        assert!(!pressed.contains(&0x36));

        // 6. Release Left Command (0x37) with mask now cleared -> up
        assert!(!step(&mut pressed, 0x37, false));
        assert!(!pressed.contains(&0x37));
        assert!(pressed.is_empty());
    }
}

// ---------------------------------------------------------------------------
// System & Process Metrics Telemetry Probing (SysProbe)
// ---------------------------------------------------------------------------

/// Point-in-time process resource consumption sample.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ProcessSample {
    pub rss_bytes: u64,
    pub virt_bytes: u64,
    pub thread_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub open_fds: Option<u32>,
    pub cpu_time_ms: u64,
}

/// Point-in-time host hardware resource sample.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HostSample {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_memory_bytes: Option<u64>,
}

/// Abstract system and process resource telemetry probe.
pub trait SysProbe: Send + Sync {
    /// Sample resource consumption of the specified process.
    fn sample_process(&self, pid: u32) -> Result<ProcessSample, CoreError>;

    /// Sample host hardware statistics (such as total physical RAM).
    fn sample_host(&self) -> Result<HostSample, CoreError>;
}

/// In-memory mock system telemetry probe for deterministic testing and CI.
#[derive(Clone, Default)]
pub struct MemorySysProbe {
    process_sample: Arc<Mutex<ProcessSample>>,
    host_sample: Arc<Mutex<HostSample>>,
}

impl MemorySysProbe {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_samples(process: ProcessSample, host: HostSample) -> Self {
        Self {
            process_sample: Arc::new(Mutex::new(process)),
            host_sample: Arc::new(Mutex::new(host)),
        }
    }

    pub fn set_process(&self, process: ProcessSample) {
        *self.process_sample.lock().expect("sys probe") = process;
    }

    pub fn set_host(&self, host: HostSample) {
        *self.host_sample.lock().expect("sys probe") = host;
    }
}

impl SysProbe for MemorySysProbe {
    fn sample_process(&self, _pid: u32) -> Result<ProcessSample, CoreError> {
        Ok(self.process_sample.lock().expect("sys probe").clone())
    }

    fn sample_host(&self) -> Result<HostSample, CoreError> {
        Ok(self.host_sample.lock().expect("sys probe").clone())
    }
}

/// Live platform telemetry probe reading OS task statistics and kernel counters.
#[derive(Clone, Copy, Default)]
pub struct PlatformSysProbe;

impl SysProbe for PlatformSysProbe {
    fn sample_process(&self, _pid: u32) -> Result<ProcessSample, CoreError> {
        let mut sample = ProcessSample::default();

        #[cfg(unix)]
        {
            unsafe {
                let mut rusage = std::mem::zeroed::<libc::rusage>();
                if libc::getrusage(libc::RUSAGE_SELF, &mut rusage) == 0 {
                    let u_ms = (rusage.ru_utime.tv_sec as u64)
                        .saturating_mul(1000)
                        .saturating_add((rusage.ru_utime.tv_usec as u64) / 1000);
                    let s_ms = (rusage.ru_stime.tv_sec as u64)
                        .saturating_mul(1000)
                        .saturating_add((rusage.ru_stime.tv_usec as u64) / 1000);
                    sample.cpu_time_ms = u_ms.saturating_add(s_ms);

                    #[cfg(target_os = "macos")]
                    {
                        sample.rss_bytes = rusage.ru_maxrss as u64;
                    }
                    #[cfg(not(target_os = "macos"))]
                    {
                        sample.rss_bytes = (rusage.ru_maxrss as u64).saturating_mul(1024);
                    }
                }
            }

            #[cfg(target_os = "macos")]
            #[allow(deprecated)]
            unsafe {
                let mut info = std::mem::zeroed::<libc::mach_task_basic_info>();
                let mut count = (std::mem::size_of::<libc::mach_task_basic_info>()
                    / std::mem::size_of::<libc::natural_t>())
                    as libc::mach_msg_type_number_t;
                if libc::task_info(
                    libc::mach_task_self(),
                    libc::MACH_TASK_BASIC_INFO,
                    &mut info as *mut _ as *mut libc::integer_t,
                    &mut count,
                ) == libc::KERN_SUCCESS
                {
                    sample.rss_bytes = info.resident_size;
                    sample.virt_bytes = info.virtual_size;
                }

                let mut thread_list: libc::thread_act_array_t = std::ptr::null_mut();
                let mut thread_count_mach: libc::mach_msg_type_number_t = 0;
                if libc::task_threads(
                    libc::mach_task_self(),
                    &mut thread_list,
                    &mut thread_count_mach,
                ) == libc::KERN_SUCCESS
                {
                    sample.thread_count = thread_count_mach as u32;
                    libc::vm_deallocate(
                        libc::mach_task_self(),
                        thread_list as libc::vm_address_t,
                        (thread_count_mach as usize * std::mem::size_of::<libc::thread_act_t>())
                            as libc::vm_size_t,
                    );
                }
            }

            #[cfg(all(unix, not(target_os = "macos")))]
            {
                if let Ok(content) = std::fs::read_to_string("/proc/self/statm") {
                    let mut parts = content.split_whitespace();
                    if let (Some(v_pages), Some(r_pages)) = (parts.next(), parts.next()) {
                        let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
                        let page_size = if page_size > 0 {
                            page_size as u64
                        } else {
                            4096
                        };
                        if let Ok(v) = v_pages.parse::<u64>() {
                            sample.virt_bytes = v.saturating_mul(page_size);
                        }
                        if let Ok(r) = r_pages.parse::<u64>() {
                            sample.rss_bytes = r.saturating_mul(page_size);
                        }
                    }
                }
                if let Ok(content) = std::fs::read_to_string("/proc/self/status") {
                    for line in content.lines() {
                        if let Some(rest) = line.strip_prefix("Threads:") {
                            if let Ok(tc) = rest.trim().parse::<u32>() {
                                sample.thread_count = tc;
                            }
                        }
                    }
                }
            }

            sample.open_fds = std::fs::read_dir("/dev/fd")
                .or_else(|_| std::fs::read_dir("/proc/self/fd"))
                .ok()
                .map(|entries| entries.filter_map(Result::ok).count() as u32);
        }

        #[cfg(windows)]
        {
            use windows_sys::Win32::Foundation::{CloseHandle, FILETIME, INVALID_HANDLE_VALUE};
            use windows_sys::Win32::System::Diagnostics::ToolHelp::{
                CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD,
                THREADENTRY32,
            };
            use windows_sys::Win32::System::ProcessStatus::{
                GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
            };
            use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};

            // SAFETY: every out-pointer below is an initialized stack local of
            // the matching Win32 type; handles are the current-process pseudo
            // handle (never closed) or a snapshot handle we close ourselves.
            unsafe {
                let mut counters = std::mem::zeroed::<PROCESS_MEMORY_COUNTERS>();
                if GetProcessMemoryInfo(
                    GetCurrentProcess(),
                    &mut counters,
                    std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
                ) != 0
                {
                    sample.rss_bytes = counters.WorkingSetSize as u64;
                }

                let (mut creation, mut exit, mut kernel, mut user) = (
                    std::mem::zeroed::<FILETIME>(),
                    std::mem::zeroed::<FILETIME>(),
                    std::mem::zeroed::<FILETIME>(),
                    std::mem::zeroed::<FILETIME>(),
                );
                if GetProcessTimes(
                    GetCurrentProcess(),
                    &mut creation,
                    &mut exit,
                    &mut kernel,
                    &mut user,
                ) != 0
                {
                    let ticks =
                        |t: FILETIME| (t.dwLowDateTime as u64) | ((t.dwHighDateTime as u64) << 32);
                    // FILETIME units are 100-ns ticks.
                    sample.cpu_time_ms = ticks(kernel).saturating_add(ticks(user)) / 10_000;
                }

                let snap = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
                if snap != INVALID_HANDLE_VALUE {
                    let pid = std::process::id();
                    let mut entry = THREADENTRY32 {
                        dwSize: std::mem::size_of::<THREADENTRY32>() as u32,
                        ..std::mem::zeroed()
                    };
                    let mut threads = 0u32;
                    if Thread32First(snap, &mut entry) != 0 {
                        loop {
                            if entry.th32OwnerProcessID == pid {
                                threads = threads.saturating_add(1);
                            }
                            if Thread32Next(snap, &mut entry) == 0 {
                                break;
                            }
                        }
                    }
                    if threads > 0 {
                        sample.thread_count = threads;
                    }
                    CloseHandle(snap);
                }
            }
        }

        if sample.thread_count == 0 {
            sample.thread_count = 1;
        }

        Ok(sample)
    }

    fn sample_host(&self) -> Result<HostSample, CoreError> {
        #[cfg(unix)]
        let total_memory_bytes = unsafe {
            let pages = libc::sysconf(libc::_SC_PHYS_PAGES);
            let page_size = libc::sysconf(libc::_SC_PAGESIZE);
            (pages > 0 && page_size > 0).then(|| (pages as u64).saturating_mul(page_size as u64))
        };
        #[cfg(windows)]
        let total_memory_bytes = {
            use windows_sys::Win32::System::SystemInformation::{
                GlobalMemoryStatusEx, MEMORYSTATUSEX,
            };
            // SAFETY: `zeroed` builds a valid all-zero MEMORYSTATUSEX and the
            // out-pointer we hand over is correctly sized (dwLength preset).
            unsafe {
                let mut status = MEMORYSTATUSEX {
                    dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
                    ..std::mem::zeroed()
                };
                (GlobalMemoryStatusEx(&mut status) != 0).then_some(status.ullTotalPhys)
            }
        };
        #[cfg(not(any(unix, windows)))]
        let total_memory_bytes: Option<u64> = None;

        Ok(HostSample { total_memory_bytes })
    }
}
