// SPDX-License-Identifier: Apache-2.0 OR MIT
//! OS clipboard, conservative insert, and allowlist-driven app open.
//! insert() and open() only run on a user click.

use std::sync::{Arc, Mutex};
use tetherly_core::{
    insertion_plan, Clip, CoreError, InsertionPlan, Insertor, NotificationPayload,
    NotificationUrgency, Notifier, Opener,
};

pub mod cursor_shield;
pub mod popover;
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

        // v0.2.31: a move while an injected button is held is a DRAG to
        // AppKit — `MouseMoved` with a button down never selects text or
        // drags a window.
        let held = INJECTED_BUTTONS_DOWN.load(std::sync::atomic::Ordering::SeqCst);
        let (ev_type, btn) = match injected_move_kind(held) {
            InjectedMoveKind::Moved => (CGEventType::MouseMoved, CGMouseButton::Left),
            InjectedMoveKind::LeftDragged => (CGEventType::LeftMouseDragged, CGMouseButton::Left),
            InjectedMoveKind::RightDragged => {
                (CGEventType::RightMouseDragged, CGMouseButton::Right)
            }
            InjectedMoveKind::OtherDragged => {
                (CGEventType::OtherMouseDragged, CGMouseButton::Center)
            }
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
        let held = INJECTED_BUTTONS_DOWN.load(std::sync::atomic::Ordering::SeqCst);
        INJECTED_BUTTONS_DOWN.store(
            next_injected_buttons(held, mask, down),
            std::sync::atomic::Ordering::SeqCst,
        );

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
            // Modifier combos need the CGEventFlags: a synthetic Cmd keydown
            // followed by a synthetic grave tap is two unrelated keys to
            // macOS unless the tap carries kCGEventFlagMaskCommand (v0.2.27
            // — before this, every forwarded combo to a mac target, gestures
            // and plain typed shortcuts alike, arrived flag-less and macOS
            // saw none of them). Single transition rule: compute (flags to
            // post, tracker state to store after the post) in one place.
            let (post_flags, new_state) = next_injected_flags(
                INJECTED_MODIFIER_FLAGS.load(std::sync::atomic::Ordering::SeqCst),
                code,
                down,
            );
            if let Some(event) = CGEvent::new_keyboard_event(None, cg_code, down) {
                CGEvent::set_integer_value_field(
                    Some(&event),
                    CGEventField::EventSourceUserData,
                    TETHERLY_INJECTED_MAGIC,
                );
                CGEvent::set_flags(Some(&event), post_flags);
                CGEvent::post(CGEventTapLocation::HIDEventTap, Some(&event));
            }
            INJECTED_MODIFIER_FLAGS.store(new_state, std::sync::atomic::Ordering::SeqCst);
        }
    }
}

/// Mouse buttons currently held by the INJECTED pointer stream on this mac
/// target (`BTN_*` mask). A `MouseMoved` posted while a button is down is
/// not a drag to AppKit: selections, window drags and drag-and-drop need
/// `LeftMouseDragged` / `RightMouseDragged` / `OtherMouseDragged`, so
/// `move_abs` picks the event type from this state (v0.2.31). Process
/// global for the same reason as `INJECTED_MODIFIER_FLAGS`.
#[cfg(target_os = "macos")]
static INJECTED_BUTTONS_DOWN: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

/// Which pointer-motion event an injected move must be, given the injected
/// buttons currently held. Precedence Left > Right > Middle mirrors
/// `SystemInputSink::button`'s mask decoding.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InjectedMoveKind {
    Moved,
    LeftDragged,
    RightDragged,
    OtherDragged,
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn injected_move_kind(buttons_down: u8) -> InjectedMoveKind {
    if buttons_down & tetherly_core::BTN_LEFT != 0 {
        InjectedMoveKind::LeftDragged
    } else if buttons_down & tetherly_core::BTN_RIGHT != 0 {
        InjectedMoveKind::RightDragged
    } else if buttons_down & tetherly_core::BTN_MIDDLE != 0 {
        InjectedMoveKind::OtherDragged
    } else {
        InjectedMoveKind::Moved
    }
}

/// Next value of the injected-button tracker after a button event.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn next_injected_buttons(state: u8, mask: u8, down: bool) -> u8 {
    if down {
        state | mask
    } else {
        state & !mask
    }
}

/// Modifier state of the INJECTED keyboard stream on this mac target:
/// which Win32 modifier VKs are currently held by remote input. Process
/// global by nature — the real modifier state the system reacts to is
/// process-global too, and `SystemInputSink` is a stateless unit struct
/// shared by every node in the process.
#[cfg(target_os = "macos")]
static INJECTED_MODIFIER_FLAGS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The keyboard-modifier `CGEventFlag` a Win32 VK carries, if any.
#[cfg(target_os = "macos")]
fn modifier_event_flag(code: u16) -> Option<objc2_core_graphics::CGEventFlags> {
    use objc2_core_graphics::CGEventFlags;
    match code {
        0x10 | 0xA0 | 0xA1 => Some(CGEventFlags::MaskShift),
        0x11 | 0xA2 | 0xA3 => Some(CGEventFlags::MaskControl),
        0x12 | 0xA4 | 0xA5 => Some(CGEventFlags::MaskAlternate),
        0x5B | 0x5C => Some(CGEventFlags::MaskCommand),
        _ => None,
    }
}

/// One transition of the injected-modifier tracker. Returns the flags the
/// event must carry and the tracker state to store AFTER the event is
/// posted, in that order:
/// - modifier down: the mask joins the state before posting (the keydown
///   itself must already carry it);
/// - modifier up: the keyup still carries the mask (hardware behavior) and
///   the state drops it afterwards;
/// - any other key: post with the currently held masks, state unchanged.
#[cfg(target_os = "macos")]
fn next_injected_flags(
    state: u64,
    code: u16,
    down: bool,
) -> (objc2_core_graphics::CGEventFlags, u64) {
    use objc2_core_graphics::CGEventFlags;
    let current = CGEventFlags::from_bits_truncate(state);
    match modifier_event_flag(code) {
        Some(f) if down => {
            let flags = current | f;
            (flags, flags.bits())
        }
        Some(f) => (current | f, state & !f.bits()),
        None => (current, state),
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
// Root-fix E3: on macOS one warp produces TWO hook-visible events — the real
// CGWarp jump and the synthetic MouseMoved posted to SessionEventTap to
// dismiss hover tracking. A single-shot marker consumed the first echo and
// let the second through, where it re-entered the engine at the park
// position and re-placed the remote cursor (production: cursor snapped to
// screen center right after a correct proportional entry). Count the echoes.
#[cfg(target_os = "macos")]
static PENDING_WARP_ECHOES: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
// Root-fix F: events after a warp whose deltas would span the teleport get
// their deltas zeroed for this many events (see the macOS tap below).
#[cfg(target_os = "macos")]
static WARP_SETTLE_EVENTS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
// Time bound for the warp marker (both platforms). If the post-warp events
// never surface (tap disabled by timeout, events lost, SetCursorPos failed,
// or the echo consumed by another window's LL hook), the marker would
// otherwise stay armed forever and swallow a future REAL event that happens
// to land on the warp target — with per-batch roaming recenters, the target
// is screen center, where the roaming hand is most of the time. The marker
// expires after this window; a warp that delivered no echo within it is
// indistinguishable from a warp that delivered it.
#[cfg(any(windows, target_os = "macos"))]
const WARP_MARKER_MAX_AGE_MS: u64 = 250;
#[cfg(any(windows, target_os = "macos"))]
static WARP_MARKED_AT_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

#[cfg(any(windows, target_os = "macos"))]
fn mark_pending_warp(x: i32, y: i32) {
    use std::sync::atomic::Ordering;
    PENDING_WARP_X.store(x, Ordering::SeqCst);
    PENDING_WARP_Y.store(y, Ordering::SeqCst);
    PENDING_WARP_VALID.store(true, Ordering::SeqCst);
    WARP_MARKED_AT_MS.store(now_mono_ms(), Ordering::SeqCst);
    #[cfg(target_os = "macos")]
    {
        PENDING_WARP_ECHOES.store(2, Ordering::SeqCst);
        // Zero the deltas of the first few post-warp events: macOS computes
        // them across the teleport, so they carry the park displacement.
        WARP_SETTLE_EVENTS.store(4, Ordering::SeqCst);
    }
}

#[cfg(any(windows, target_os = "macos"))]
fn now_mono_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// True while the warp marker (echo + settle counters) is still fresh. A
/// marker older than [`WARP_MARKER_MAX_AGE_MS`] is dropped on read: its
/// events never surfaced (lost or tap disabled), so letting the counters hang
/// would zero the deltas of the user's next real motions.
#[cfg(any(windows, target_os = "macos"))]
fn warp_marker_fresh() -> bool {
    use std::sync::atomic::Ordering;
    let marked = WARP_MARKED_AT_MS.load(Ordering::SeqCst);
    if marked == 0 {
        return false;
    }
    now_mono_ms().saturating_sub(marked) <= WARP_MARKER_MAX_AGE_MS
}

/// Returns true when the given hook position is the echo of the pending
/// programmatic warp (and consumes one pending echo slot).
#[cfg(any(windows, target_os = "macos"))]
fn take_warp_echo(x: i32, y: i32) -> bool {
    use std::sync::atomic::Ordering;
    if !PENDING_WARP_VALID.load(Ordering::SeqCst) {
        return false;
    }
    // Expired marker: its events never surfaced (tap disabled, events lost,
    // SetCursorPos failed, or the echo eaten by another LL hook). Drop the
    // whole marker instead of letting a stale echo slot eat a future REAL
    // event that happens to land on the warp target.
    if !warp_marker_fresh() {
        PENDING_WARP_VALID.store(false, Ordering::SeqCst);
        #[cfg(target_os = "macos")]
        {
            PENDING_WARP_ECHOES.store(0, Ordering::SeqCst);
            WARP_SETTLE_EVENTS.store(0, Ordering::SeqCst);
        }
        return false;
    }
    let wx = PENDING_WARP_X.load(Ordering::SeqCst);
    let wy = PENDING_WARP_Y.load(Ordering::SeqCst);
    if (x - wx).abs() <= 2 && (y - wy).abs() <= 2 {
        #[cfg(target_os = "macos")]
        {
            let left = PENDING_WARP_ECHOES.fetch_sub(1, Ordering::SeqCst);
            if left <= 1 {
                PENDING_WARP_VALID.store(false, Ordering::SeqCst);
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            PENDING_WARP_VALID.store(false, Ordering::SeqCst);
        }
        return true;
    }
    false
}

// Delta baseline of the Windows mouse hook (LL hooks deliver cursor
// position only — see `mouse_proc`). Thread-local: LL hooks deliver every
// event on one dedicated thread.
//
// Compiled outside Windows only under `cfg(test)` on macOS, so the
// warp-echo/baseline coupling stays unit-tested where development happens;
// production macOS never touches this (its tap consumes CGEvent deltas,
// not positions).
#[cfg(any(windows, all(target_os = "macos", test)))]
mod hook_move_state {
    thread_local! {
        static LAST_MOVE_POS: std::cell::RefCell<Option<(i32, i32)>> =
            const { std::cell::RefCell::new(None) };
    }

    /// One WM_MOUSEMOVE through the warp-echo gate. Returns true when the
    /// event is the echo of a pending programmatic warp and was swallowed;
    /// the delta baseline is ANCHORED at the echo position (the warp target)
    /// with it — Deskflow PRE_WARP `saveMousePosition` alignment. The next
    /// real motion is then measured from wherever the cursor actually is, so
    /// per-batch roaming recenters never lose motion; the pre-warp in-flight
    /// hazard of the anchored baseline is covered by `hook_delta_is_bogus`.
    pub(super) fn take_warp_echo_reset_baseline(x: i32, y: i32) -> bool {
        if super::take_warp_echo(x, y) {
            LAST_MOVE_POS.with(|p| *p.borrow_mut() = Some((x, y)));
            true
        } else {
            false
        }
    }

    /// Derive the (dx, dy) of a move to (x, y) from the previous hook
    /// position and advance the baseline to it.
    pub(super) fn advance(x: i32, y: i32) -> (i32, i32) {
        LAST_MOVE_POS.with(|p| {
            let mut p = p.borrow_mut();
            let (dx, dy) = match *p {
                Some((lx, ly)) => (x - lx, y - ly),
                None => (0, 0),
            };
            *p = Some((x, y));
            (dx, dy)
        })
    }
}

/// Test-only serialization of the process-global warp marker state. The
/// warp-echo unit tests touch `PENDING_WARP_*` statics that production
/// serializes on the single hook thread; under parallel `cargo test` two
/// such tests can interleave mark/take pairs and steal each other's echo
/// slots (observed once as a flaky `edge_park_recenter` failure). Holding
/// this guard for the duration of each test restores the production
/// single-thread ordering. Compiled only where its callers exist (the
/// macOS-test warp replays); on other targets the caller-less helper would
/// be dead code under `-D warnings`.
#[cfg(all(target_os = "macos", test))]
fn warp_state_test_gate() -> std::sync::MutexGuard<'static, ()> {
    static GATE: std::sync::Mutex<()> = std::sync::Mutex::new(());
    GATE.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
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

/// Deskflow OSXScreen::leave & OSXScreen::enter alignment: set
/// "SetsCursorInBackground" on the CGS connection so WindowServer honors
/// cursor operations even when Tetherly is a background accessory process.
#[cfg(target_os = "macos")]
fn enable_background_cursor_ops() {
    unsafe {
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
    }
}

#[cfg(target_os = "macos")]
pub fn set_mouse_cursor_associated(associated: bool) {
    // An idle hide must not stack under the seat-capture hide counter.
    reveal_idle_hidden_cursor();
    enable_background_cursor_ops();
    unsafe {
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
            // v0.2.37: undo the shield's blank NSCursor as well.
            cursor_shield::set_arrow_cursor_now();

            // 3. Re-associate mouse and reset suppression
            let _ = CGAssociateMouseAndMouseCursorPosition(true);
            let _ = CGSetLocalEventsSuppressionInterval(0.0);
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub fn set_mouse_cursor_associated(_associated: bool) {}

/// v0.2.34: set while the cursor is hidden by `hide_cursor_until_local_motion`
/// (Target idle after the controller left); the tap reveals it on the next
/// physical mouse event. Separate from `CURSOR_HIDDEN` (seat capture) so the
/// two CGDisplayHideCursor counters stay balanced.
#[cfg(target_os = "macos")]
static IDLE_HIDDEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[cfg(target_os = "macos")]
fn online_displays() -> Vec<u32> {
    let mut displays = [0u32; 16];
    let mut count = 0u32;
    unsafe {
        let _ = CGGetOnlineDisplayList(16, displays.as_mut_ptr(), &mut count);
    }
    if count == 0 {
        vec![objc2_core_graphics::CGMainDisplayID()]
    } else {
        displays[..count as usize].to_vec()
    }
}

/// Hide the macOS cursor until the next physical mouse event (revealed by
/// the event tap) or the next controller Enter. No-op while the seat
/// capture already holds it hidden.
#[cfg(target_os = "macos")]
pub fn hide_cursor_until_local_motion() {
    use std::sync::atomic::Ordering;
    if CURSOR_HIDDEN.load(Ordering::SeqCst) || IDLE_HIDDEN.swap(true, Ordering::SeqCst) {
        return;
    }
    enable_background_cursor_ops();
    for disp in online_displays() {
        unsafe {
            let _ = CGDisplayHideCursor(disp);
        }
    }
}

/// Undo `hide_cursor_until_local_motion`; idempotent.
#[cfg(target_os = "macos")]
pub fn reveal_idle_hidden_cursor() {
    use std::sync::atomic::Ordering;
    if !IDLE_HIDDEN.swap(false, Ordering::SeqCst) {
        return;
    }
    for disp in online_displays() {
        unsafe {
            let _ = CGDisplayShowCursor(disp);
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub fn hide_cursor_until_local_motion() {}

#[cfg(not(target_os = "macos"))]
pub fn reveal_idle_hidden_cursor() {}

/// v0.2.37: force the system arrow image (macOS: undo a blank NSCursor left
/// by a cursor shield). No-op elsewhere.
pub fn restore_arrow_cursor() {
    cursor_shield::set_arrow_cursor_now();
}

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

/// Per-batch roaming recenter target (Deskflow MSWindowsComputer
/// alignment): while roaming the Remote seat on the Windows hook-diff
/// pipeline, the runtime pulls the physical cursor back to screen center
/// after every consumed motion batch (Deskflow: `onMouseMove` warps to
/// `m_xCenter`/`m_yCenter` after every secondary motion). The delta chain
/// — anchored at the warp target by the echo gate (`hook_move_state`) —
/// then always measures genuine hand motion from center and can never
/// saturate at a screen boundary: the Windows hand and the Mac cursor own
/// independent positions, exactly as in Deskflow.
///
/// Pure geometry; callers own the seat check and the actual warp.
pub(crate) fn roaming_recenter_target(width: i32, height: i32) -> (i32, i32) {
    ((width.max(1)) / 2, (height.max(1)) / 2)
}

/// v0.2.32: Windows controller cursor hiding (Deskflow
/// `MSWindowsScreen::showCursor(false)` idiom). While this node controls a
/// remote seat the physical cursor is frozen at the recenter point, so
/// without this the hand "roams" the Mac while a Windows arrow sits parked
/// at screen center (production symptom 1, v0.2.31 3-minute test). A
/// process-wide `ShowCursor(FALSE)` only affects windows of the calling
/// thread, so Deskflow replaces every *system* cursor with a blank one
/// via `SetSystemCursor(CopyIcon(blank), OCR_*)` and restores the user's
/// scheme with `SystemParametersInfo(SPI_SETCURSORS)`.
///
/// The id table and the blank planes are plain data kept cross-platform
/// so the unit test pins them on every host; the Win32 calls live in the
/// `#[cfg(windows)]` block below.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) mod system_cursor {
    /// Every `OCR_*` id `SetSystemCursor` accepts (WinUser.h). Order is
    /// irrelevant; the test asserts uniqueness and coverage.
    pub const SYSTEM_CURSOR_IDS: [u32; 14] = [
        32512, // OCR_NORMAL
        32513, // OCR_IBEAM
        32514, // OCR_WAIT
        32515, // OCR_CROSS
        32516, // OCR_UP
        32642, // OCR_SIZENWSE
        32643, // OCR_SIZENESW
        32644, // OCR_SIZEWE
        32645, // OCR_SIZENS
        32646, // OCR_SIZEALL
        32648, // OCR_NO
        32649, // OCR_HAND
        32650, // OCR_APPSTARTING
        32651, // OCR_HELP
    ];

    /// Blank cursor edge in pixels (monochrome `CreateCursor`).
    pub const BLANK_CURSOR_SIZE: i32 = 32;

    /// Monochrome cursor planes for a fully transparent cursor: AND mask
    /// all ones (keep the screen), XOR plane all zeros (draw nothing).
    pub fn blank_cursor_planes() -> (Vec<u8>, Vec<u8>) {
        let bytes = (BLANK_CURSOR_SIZE * BLANK_CURSOR_SIZE / 8) as usize;
        (vec![0xFF; bytes], vec![0x00; bytes])
    }

    #[cfg(windows)]
    static HIDDEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

    /// Replace every system cursor with a blank one. Idempotent: a second
    /// call while hidden is a no-op so a re-arm never leaks cursor handles.
    /// Partial failures are logged and the remaining ids still processed —
    /// a half-hidden scheme is strictly better than a parked arrow.
    #[cfg(windows)]
    pub fn hide() {
        use std::sync::atomic::Ordering;
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            CopyIcon, CreateCursor, DestroyCursor, SetSystemCursor,
        };
        if HIDDEN.swap(true, Ordering::SeqCst) {
            return;
        }
        let (and_plane, xor_plane) = blank_cursor_planes();
        // SAFETY: both planes are exactly (32*32/8) bytes as CreateCursor
        // requires for a 32x32 monochrome cursor; a null HINSTANCE is
        // permitted for cursors created from raw planes.
        let blank = unsafe {
            CreateCursor(
                std::ptr::null_mut(),
                0,
                0,
                BLANK_CURSOR_SIZE,
                BLANK_CURSOR_SIZE,
                and_plane.as_ptr().cast(),
                xor_plane.as_ptr().cast(),
            )
        };
        if blank.is_null() {
            HIDDEN.store(false, Ordering::SeqCst);
            tracing::warn!("system_cursor: CreateCursor failed; Windows cursor stays visible");
            return;
        }
        let mut replaced = 0usize;
        for id in SYSTEM_CURSOR_IDS {
            // SetSystemCursor destroys the handle it is given, so every id
            // needs its own copy of the blank cursor.
            // SAFETY: `blank` is a valid cursor handle owned by this fn.
            let copy = unsafe { CopyIcon(blank) };
            if copy.is_null() {
                tracing::warn!(id, "system_cursor: CopyIcon failed");
                continue;
            }
            // SAFETY: `copy` is a valid cursor handle; ownership transfers
            // to the system on success (and is destroyed on failure too).
            if unsafe { SetSystemCursor(copy, id) } == 0 {
                tracing::warn!(id, "system_cursor: SetSystemCursor failed");
            } else {
                replaced += 1;
            }
        }
        // SAFETY: `blank` was never handed to the system; we still own it.
        unsafe { DestroyCursor(blank) };
        tracing::info!(
            replaced,
            total = SYSTEM_CURSOR_IDS.len(),
            "system_cursor: hidden"
        );
        eprintln!(
            "[cursor] hidden: replaced {replaced}/{} system cursors",
            SYSTEM_CURSOR_IDS.len()
        );
    }

    /// Reload the user's cursor scheme from the registry (undoes `hide`).
    /// `force` restores even when this process never hid the cursor —
    /// used once at hook start so a previous crashed daemon stint can
    /// never leave the machine cursorless. Note: from a SYSTEM-session
    /// daemon the "user scheme" is SYSTEM's, i.e. the Windows default.
    #[cfg(windows)]
    pub fn restore(force: bool) {
        use std::sync::atomic::Ordering;
        use windows_sys::Win32::UI::WindowsAndMessaging::{SystemParametersInfoW, SPI_SETCURSORS};
        if !HIDDEN.swap(false, Ordering::SeqCst) && !force {
            return;
        }
        // SAFETY: SPI_SETCURSORS takes no parameter block; null + 0 is the
        // documented call shape.
        let ok = unsafe { SystemParametersInfoW(SPI_SETCURSORS, 0, std::ptr::null_mut(), 0) };
        if ok == 0 {
            tracing::warn!("system_cursor: SystemParametersInfoW(SPI_SETCURSORS) failed");
        } else {
            tracing::info!(force, "system_cursor: restored");
            eprintln!("[cursor] restored (force={force})");
        }
    }
}

/// Deskflow bogus-motion filter (MSWindowsComputer::onMouseMove
/// `bogusZoneSize`): on the Windows hook the delta baseline is anchored at
/// screen center by the roaming recenter, so a delta spanning roughly the
/// center-to-edge distance cannot be a hand motion — it is the leaked
/// displacement of a cursor warp racing the hook FIFO (an in-flight
/// pre-warp event consumed after the echo anchored the baseline at
/// center). Deskflow drops motion beyond the center-to-edge distance
/// minus a 10px tolerance.
///
/// v0.2.26: applied on the macOS tap as well. The macOS tap reads raw
/// CGEvent deltas, but a WARP's echo event carries exactly such a
/// displacement as its raw delta; the settle-counter (Root-fix F) only
/// covers the first few post-warp events inside its 250ms marker window,
/// and an echo that surfaces after that fed the roaming engine a "fast
/// push" that instantly re-crossed the shared edge (production:
/// ghost-controller bounce loop, rescue docked inside the redundancy
/// band, Windows target's physical mouse swallowed every ~13s).
///
/// Returns true when the delta must be dropped (zeroed) as bogus.
#[cfg(any(windows, target_os = "macos"))]
pub(crate) fn hook_delta_is_bogus(dx: i32, dy: i32, width: i32, height: i32) -> bool {
    const BOGUS_ZONE_PX: i32 = 10;
    let (w, h) = (width.max(1), height.max(1));
    let (cx, cy) = (w / 2, h / 2);
    let bogus_x = if dx > 0 {
        dx + BOGUS_ZONE_PX > w - cx
    } else {
        -dx + BOGUS_ZONE_PX > cx
    };
    let bogus_y = if dy > 0 {
        dy + BOGUS_ZONE_PX > h - cy
    } else {
        -dy + BOGUS_ZONE_PX > cy
    };
    bogus_x || bogus_y
}

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
    Move {
        x: i32,
        y: i32,
        dx: i32,
        dy: i32,
    },
    Button {
        mask: u8,
        down: bool,
    },
    Wheel {
        dx: i16,
        dy: i16,
    },
    Key {
        code: u16,
        down: bool,
    },
    /// Four-finger swipe on the LOCAL trackpad, observed while this node is
    /// controlling a remote desktop.
    ///
    /// Horizontal: forwarded as the remote's app switch (Alt+Tab on a
    /// Windows target). `direction`: -1 = fingers moved left (next window),
    /// 1 = fingers moved right (previous window) — the same
    /// content-follows-fingers convention macOS Spaces uses.
    ///
    /// Vertical: forwarded as the remote's Task View toggle
    /// (`direction: 0` — direction-agnostic; Win+Tab on a Windows target,
    /// swipe again to dismiss, mirroring Mission Control's toggle).
    GestureSwipe {
        direction: i8,
    },
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

/// What the HID tap's keyboard branches must do with a physical key event
/// while capture is ARMED. Pure decision — the tap closure only executes it
/// — so the matrix (mapped/unmapped × KeyDown/KeyUp/FlagsChanged) is unit
/// testable without CGEvent infrastructure.
///
/// Swallowing an unmapped key (Fn/globe 0x3F etc.) strands its press state
/// locally: the observed production failure was a stuck `MaskSecondaryFn`
/// latch in CGEventSource after a roam (the tap is a `Default`-option tap,
/// so a swallowed FlagsChanged never reaches the OS). Unmapped keys are
/// local-only by definition — pass them through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArmedKeyDecision {
    /// Forward to the peer as `RawInput::Key { vk, down }` and swallow locally.
    Forward { vk: u16 },
    /// No cross-end semantics: let the OS see the event untouched.
    PassThrough,
}

pub fn armed_key_decision(mac_code: u16) -> ArmedKeyDecision {
    match mac_keycode_to_win_vk(mac_code) {
        Some(vk) => ArmedKeyDecision::Forward { vk },
        None => ArmedKeyDecision::PassThrough,
    }
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
// Deskflow kHOOK_RELAY_EVENTS: true while this node controls a remote
// seat — the mouse hook swallows physical motion/button events (local
// cursor frozen, local UI blind to the hand) and only relays deltas.
// Toggled from the runtime via WM_HOOK_KB_ON/OFF on the hook pump
// thread; read lock-free in the LL callback and by `HookControl`.
static MOUSE_RELAY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[cfg(windows)]
impl HookControl {
    pub fn is_armed(&self) -> bool {
        MOUSE_RELAY.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// v0.2.33: hide the system cursors ahead of the center park warp so
    /// the arrow never paints at screen center (production 13:05:10.998
    /// warp vs 13:05:11.004 hidden — a visible flash on every entry). The
    /// later `set_keyboard(true)` hide is then an idempotent no-op.
    pub fn hide_cursor(&self) {
        system_cursor::hide();
    }

    pub fn set_keyboard(&self, on: bool) -> Result<(), CoreError> {
        use windows_sys::Win32::UI::WindowsAndMessaging::PostThreadMessageW;
        const WM_HOOK_KB_ON: u32 = 0x8000 + 1; // WM_APP + 1
        const WM_HOOK_KB_OFF: u32 = 0x8000 + 2; // WM_APP + 2
                                                // v0.2.32: the seat leaving this machine hides the parked Windows
                                                // cursor; the seat returning restores the scheme (mirror of the
                                                // macOS `set_mouse_cursor_associated(!on)` in the other impl).
                                                // Done here on the runtime thread, not on the hook pump: the
                                                // SetSystemCursor loop is ~14 USER calls and must never sit in
                                                // the LL hook callback's latency budget.
        if on {
            system_cursor::hide();
        } else {
            system_cursor::restore(false);
        }
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

    /// macOS hides through `set_keyboard` (dissociate + CGDisplayHideCursor
    /// path); the pre-warp hide step is Windows-only.
    pub fn hide_cursor(&self) {}

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

    pub fn hide_cursor(&self) {}

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
        CallNextHookEx, GetCursorPos, GetMessageW, SetWindowsHookExW, UnhookWindowsHookEx, HHOOK,
        HOOKPROC, KBDLLHOOKSTRUCT, LLKHF_INJECTED, LLMHF_INJECTED, MSG, MSLLHOOKSTRUCT,
        WH_KEYBOARD_LL, WH_MOUSE_LL, WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN, WM_LBUTTONUP,
        WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEHWHEEL, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_RBUTTONDOWN,
        WM_RBUTTONUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
    };

    const WM_HOOK_KB_ON: u32 = 0x8000 + 1; // WM_APP + 1
    const WM_HOOK_KB_OFF: u32 = 0x8000 + 2; // WM_APP + 2

    // v0.2.32: a daemon that died mid-roam left the system cursors blank;
    // a fresh hook start is the one moment we know no seat is remote, so
    // always reload the scheme here.
    system_cursor::restore(true);

    // Previous physical mouse position + warp gate live in hook_move_state
    // (module level) so the warp-echo/baseline coupling is unit-testable;
    // LL hooks deliver every event on one thread, so thread-local state
    // there is safe.
    static SENDER: std::sync::OnceLock<std::sync::mpsc::Sender<RawInput>> =
        std::sync::OnceLock::new();
    SENDER
        .set(tx)
        .map_err(|_| CoreError::Json("input hooks already started".into()))?;

    // Hook-side screen geometry for the bogus-motion filter
    // (`hook_delta_is_bogus`), read once at hook install. A display
    // change mid-session re-resizes the engine elsewhere; a stale filter
    // bound only loosens/tightens by the size delta and stays correct
    // within the ±10px bogus zone against plausible hand motions.
    static SCREEN_W: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
    static SCREEN_H: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
    use std::sync::atomic::Ordering;
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};
    SCREEN_W.store(unsafe { GetSystemMetrics(SM_CXSCREEN) }, Ordering::Relaxed);
    SCREEN_H.store(unsafe { GetSystemMetrics(SM_CYSCREEN) }, Ordering::Relaxed);

    // WM_MOUSEMOVE plus the button/wheel events the roaming engine relays:
    // everything the physical mouse generates that must not reach local
    // apps while controlling.
    fn is_mouse_motion_event(ev: u32) -> bool {
        matches!(
            ev,
            WM_MOUSEMOVE
                | WM_LBUTTONDOWN
                | WM_LBUTTONUP
                | WM_RBUTTONDOWN
                | WM_RBUTTONUP
                | WM_MBUTTONDOWN
                | WM_MBUTTONUP
                | WM_MOUSEWHEEL
                | WM_MOUSEHWHEEL
        )
    }

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
                // Deskflow kHOOK_RELAY_EVENTS alignment: while this node
                // controls a remote seat, the physical mouse belongs to the
                // REMOTE machine — the local UI must never see physical
                // motion (local cursor frozen at the roaming center; the
                // v0.2.9 visible center jitter and the "controlling two
                // devices at once" symptom both came from letting physical
                // moves through). Local seat (relay off) passes everything
                // through untouched.
                //
                // Feed-then-swallow (macOS tap twin: tap_callback sends the
                // RawInput BEFORE the armed-swallow check). Relay is armed
                // exactly while this node is the CONTROLLER — its physical
                // deltas are the remote seat's input stream, so the events
                // must reach the roaming engine even though the local UI is
                // blocked. Swallowing without feeding starves the engine:
                // the takeover flips the seat but not a single delta streams
                // and the remote cursor never moves (production 2026-11-13:
                // Windows→Mac mouse dead — Win input_tx_frames=6 over 26min
                // vs Mac tx=20822; every controller stint ended before any
                // delta reached the peer).
                //
                // Delta source while frozen: the hook blocks the motion, so
                // `pt` is the system's proposal = frozen_position +
                // hardware_delta for EVERY event — differencing consecutive
                // proposals is flat. The frozen actual cursor position is
                // the stable reference: delta = pt - GetCursorPos() (the
                // Barrier center-warp trick, without the warp — the cursor
                // never moves). A proposal equal to the frozen position is
                // a clamped tail or an echo, not motion. Buttons/wheel
                // carry no position state and forward unchanged.
                if MOUSE_RELAY.load(Ordering::SeqCst) && is_mouse_motion_event(ev) {
                    if ev == WM_MOUSEMOVE {
                        let mut cur = POINT { x: 0, y: 0 };
                        GetCursorPos(&mut cur);
                        let dx = mi.pt.x - cur.x;
                        let dy = mi.pt.y - cur.y;
                        // v0.2.32: the relay diff is anchored at the recenter
                        // point like the non-relay path, so a proposal that
                        // spans the center-to-edge distance is a leaked warp
                        // echo, not the hand (Deskflow bogusZoneSize). The
                        // non-relay branch already filtered it; the relay
                        // branch fed it to the engine as a violent flick.
                        let bogus = hook_delta_is_bogus(
                            dx,
                            dy,
                            SCREEN_W.load(Ordering::Relaxed),
                            SCREEN_H.load(Ordering::Relaxed),
                        );
                        if (dx != 0 || dy != 0) && !bogus {
                            push(RawInput::Move {
                                x: mi.pt.x,
                                y: mi.pt.y,
                                dx,
                                dy,
                            });
                        }
                    } else {
                        match ev {
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
                    return 1;
                }
                match ev {
                    WM_MOUSEMOVE => {
                        // Drop the echo of a programmatic SetCursorPos warp
                        // before it can reach the roaming engine; forgetting
                        // the delta baseline with it makes the first real
                        // event settle with dx=dy=0 instead of reporting the
                        // warp displacement (Windows twin of macOS Root-fix
                        // F).
                        if hook_move_state::take_warp_echo_reset_baseline(mi.pt.x, mi.pt.y) {
                            return CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam);
                        }
                        // MSLLHOOKSTRUCT carries no deltas (macOS CGEvent
                        // does via MouseEventDeltaX/Y). Derive them from the
                        // previous hook position: LL hooks observe every
                        // physical move, so the consecutive-position
                        // difference IS the real displacement. The roaming
                        // engine's remote-roam arithmetic and the reverse
                        // takeover's direction discipline (`Bottom => dy > 0`)
                        // both consume these — hard zeros here silently dead
                        // the Windows→Mac takeover (production regression).
                        let (mut dx, mut dy) = hook_move_state::advance(mi.pt.x, mi.pt.y);
                        // Deskflow bogus-motion filter: a delta spanning the
                        // center-to-edge distance is the leaked displacement
                        // of a recenter/park warp racing this hook's FIFO
                        // (in-flight pre-warp event consumed after the echo
                        // anchored the baseline at the warp target) — drop
                        // the motion, keep the position chain.
                        if hook_delta_is_bogus(
                            dx,
                            dy,
                            SCREEN_W.load(Ordering::Relaxed),
                            SCREEN_H.load(Ordering::Relaxed),
                        ) {
                            dx = 0;
                            dy = 0;
                        }
                        push(RawInput::Move {
                            x: mi.pt.x,
                            y: mi.pt.y,
                            dx,
                            dy,
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
                        // Remote seat: arm the keyboard hook AND switch the
                        // mouse hook into relay mode (swallow physical mouse
                        // events; Deskflow kHOOK_RELAY_EVENTS).
                        MOUSE_RELAY.store(true, Ordering::SeqCst);
                        if keyboard.is_null() {
                            keyboard = SetWindowsHookExW(
                                WH_KEYBOARD_LL,
                                keyboard_proc_fn,
                                std::ptr::null_mut(),
                                0,
                            );
                        }
                    }
                    WM_HOOK_KB_OFF => {
                        // Local seat: pass everything through again.
                        MOUSE_RELAY.store(false, Ordering::SeqCst);
                        if !keyboard.is_null() {
                            UnhookWindowsHookEx(keyboard);
                            keyboard = std::ptr::null_mut();
                        }
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

// Win32 priority-class values (winbase.h), spelled out so the pure ordering
// helper below compiles and is tested on every platform.
pub const WIN_IDLE_PRIORITY_CLASS: u32 = 0x0000_0040;
pub const WIN_BELOW_NORMAL_PRIORITY_CLASS: u32 = 0x0000_4000;
pub const WIN_NORMAL_PRIORITY_CLASS: u32 = 0x0000_0020;
pub const WIN_ABOVE_NORMAL_PRIORITY_CLASS: u32 = 0x0000_8000;
pub const WIN_HIGH_PRIORITY_CLASS: u32 = 0x0000_0080;
pub const WIN_REALTIME_PRIORITY_CLASS: u32 = 0x0000_0100;

/// Resident working-set floor (hard minimum) for the daemon: keeps the
/// rarely-run takeover/crossing path from being trimmed out between stints.
pub const DAEMON_MIN_WORKING_SET: usize = 64 * 1024 * 1024;
/// Upper bound passed alongside the floor; soft (MAX_DISABLE), so it never
/// caps legitimate growth (screen capture buffers etc.).
pub const DAEMON_MAX_WORKING_SET: usize = 512 * 1024 * 1024;
// The floor must cover the cold takeover path (a trimmed 6.5 MB working set
// is what stalled) without hoarding host memory.
const _: () = assert!(
    DAEMON_MIN_WORKING_SET >= 32 * 1024 * 1024
        && DAEMON_MIN_WORKING_SET < DAEMON_MAX_WORKING_SET
        && DAEMON_MAX_WORKING_SET <= 1024 * 1024 * 1024
);

/// Whether a process running at `class` must be lifted to AboveNormal.
/// Never demotes High/Realtime; unknown values are treated as needing it.
pub fn daemon_priority_needs_raise(class: u32) -> bool {
    !matches!(
        class,
        WIN_ABOVE_NORMAL_PRIORITY_CLASS | WIN_HIGH_PRIORITY_CLASS | WIN_REALTIME_PRIORITY_CLASS
    )
}

/// What [`raise_daemon_responsiveness`] managed to apply.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DaemonResponsiveness {
    /// Priority class was below AboveNormal before the call.
    pub priority_was_low: bool,
    /// Priority class is AboveNormal or higher after the call.
    pub priority_ok: bool,
    /// EcoQoS execution-speed throttling opted out.
    pub throttling_disabled: bool,
    /// Hard working-set floor installed.
    pub working_set_pinned: bool,
}

/// Production 2026-11-13: the Windows daemon inherited BelowNormal from the
/// scheduled-task launcher (Task Scheduler default priority 7) and shared it
/// with a 35-thread CPU miner, while the idle trimmer cut its working set to
/// 6.5 MB. The first edge push after a quiet spell then paged the cold
/// takeover path back in under contention — 2.9 s from the fast-push
/// decision to the takeover request; every later entry was instant. An
/// input-sharing daemon is latency-critical, so on start it lifts itself to
/// AboveNormal, opts out of EcoQoS throttling and pins a resident floor.
/// Each step is best-effort and independent; the report says what held.
#[cfg(windows)]
pub fn raise_daemon_responsiveness() -> DaemonResponsiveness {
    use windows_sys::Win32::System::Memory::{
        SetProcessWorkingSetSizeEx, QUOTA_LIMITS_HARDWS_MAX_DISABLE, QUOTA_LIMITS_HARDWS_MIN_ENABLE,
    };
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, GetPriorityClass, ProcessPowerThrottling, SetPriorityClass,
        SetProcessInformation, ABOVE_NORMAL_PRIORITY_CLASS,
        PROCESS_POWER_THROTTLING_CURRENT_VERSION, PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
        PROCESS_POWER_THROTTLING_STATE,
    };
    let mut report = DaemonResponsiveness::default();
    unsafe {
        let process = GetCurrentProcess();
        let before = GetPriorityClass(process);
        report.priority_was_low = daemon_priority_needs_raise(before);
        if report.priority_was_low {
            SetPriorityClass(process, ABOVE_NORMAL_PRIORITY_CLASS);
        }
        report.priority_ok = !daemon_priority_needs_raise(GetPriorityClass(process));

        // ControlMask selects the policy, StateMask=0 turns it OFF.
        let state = PROCESS_POWER_THROTTLING_STATE {
            Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
            ControlMask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
            StateMask: 0,
        };
        report.throttling_disabled = SetProcessInformation(
            process,
            ProcessPowerThrottling,
            &state as *const PROCESS_POWER_THROTTLING_STATE as *const std::ffi::c_void,
            std::mem::size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
        ) != 0;

        report.working_set_pinned = SetProcessWorkingSetSizeEx(
            process,
            DAEMON_MIN_WORKING_SET,
            DAEMON_MAX_WORKING_SET,
            QUOTA_LIMITS_HARDWS_MIN_ENABLE | QUOTA_LIMITS_HARDWS_MAX_DISABLE,
        ) != 0;
    }
    report
}

/// No CPU-starvation analogue off Windows (the Mac daemon runs at normal
/// QoS in the user session); honest no-op.
#[cfg(not(windows))]
pub fn raise_daemon_responsiveness() -> DaemonResponsiveness {
    DaemonResponsiveness::default()
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
    /// Logical screen size, snapshotted at tap install for the
    /// `hook_delta_is_bogus` filter (v0.2.26): a display change only
    /// loosens/tightens the bogus zone by the size delta, same trade the
    /// Windows hook makes with its `SCREEN_W`/`SCREEN_H` snapshot.
    screen_w: i32,
    screen_h: i32,
    _retained_port: std::sync::Mutex<
        Option<objc2_core_foundation::CFRetained<objc2_core_foundation::CFMachPort>>,
    >,
}

/// Private WindowServer/Dock gesture-routing fields, reverse-engineered by
/// the Instant Space Switcher / Mac Mouse Fix / dockswipe projects (the
/// layout is undocumented but stable across recent macOS releases).
#[cfg(target_os = "macos")]
mod dock_swipe {
    use objc2_core_foundation::CFMachPort;
    use objc2_core_graphics::{CGEvent, CGEventField};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    pub const K_CGS_EVENT_GESTURE: u32 = 29; // companion marker event
    pub const K_CGS_EVENT_DOCK_CONTROL: u32 = 30; // the actual dock-swipe event
    pub const K_MOTION_HORIZONTAL: i64 = 1;
    pub const K_MOTION_VERTICAL: i64 = 2;
    pub const PHASE_BEGAN: i64 = 1;
    pub const PHASE_CHANGED: i64 = 2;
    pub const PHASE_ENDED: i64 = 4;
    pub const PHASE_CANCELLED: i64 = 8;

    // CGEventField numbers (private).
    const F_SWIPE_MOTION: u32 = 123; // 1 horizontal, 2 vertical, 3 pinch
    const F_SWIPE_PROGRESS: u32 = 124; // cumulative distance, signed
    const F_GESTURE_PHASE: u32 = 132; // began/changed/ended/cancelled bits

    /// Minimum |progress| for a gesture to count as a deliberate swipe.
    /// Progress is a cumulative, screen-normalized distance; a full swipe
    /// reaches ~1.0. Calibrated against the per-event dumps if needed.
    pub const SWIPE_MIN_PROGRESS: f64 = 0.10;

    pub struct DockSwipeState {
        pub in_gesture: bool,
        pub max_abs_progress: f64,
        pub events: u64,
        pub dumped: u32,
        pub last_fire_ms: u128,
    }

    pub struct DockSwipeContext {
        pub armed: Arc<AtomicBool>,
        pub tx: std::sync::mpsc::Sender<crate::platform::RawInput>,
        pub state: Mutex<DockSwipeState>,
        pub tap_port: AtomicUsize,
    }

    /// macOS 26 reports horizontal swipe progress with the opposite sign of
    /// earlier releases (iss: ISS_SWIPE_DIRECTION_REVERSED).
    fn os_major() -> u32 {
        static MAJOR: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
        *MAJOR.get_or_init(|| {
            // /System/Library/CoreServices/SystemVersion.plist carries
            // ProductVersion (e.g. "26.1") — dependency-free runtime read.
            let plist = std::fs::read_to_string("/System/Library/CoreServices/SystemVersion.plist")
                .unwrap_or_default();
            let mut major = 15u32;
            if let Some(pos) = plist.find("<key>ProductVersion</key>") {
                if let Some(rest) = plist.get(pos..) {
                    if let Some(start) = rest.find("<string>").map(|p| p + 8) {
                        if let Some(end) = rest[start..].find("</string>") {
                            let v = &rest[start..start + end];
                            if let Some(m) = v.split('.').next() {
                                if let Ok(n) = m.parse() {
                                    major = n;
                                }
                            }
                        }
                    }
                }
            }
            major
        })
    }

    pub unsafe extern "C-unwind" fn dock_swipe_callback(
        _proxy: objc2_core_graphics::CGEventTapProxy,
        event_type: objc2_core_graphics::CGEventType,
        event: core::ptr::NonNull<CGEvent>,
        user_info: *mut std::ffi::c_void,
    ) -> *mut CGEvent {
        let ctx = &*(user_info as *const DockSwipeContext);
        let cg_event = event.as_ref();

        // Tap-disabled self-heal (same as the main input tap).
        if event_type.0 == 0xFFFF_FFFE || event_type.0 == 0xFFFF_FFFF {
            let port = ctx.tap_port.load(Ordering::SeqCst);
            if port != 0 {
                let p = unsafe { &*(port as *const CFMachPort) };
                CGEvent::tap_enable(p, true);
            }
            return event.as_ptr();
        }

        // The type-29 marker accompanies every dock-swipe step; consume it
        // while armed so the system sees no partial gesture stream.
        if event_type.0 == K_CGS_EVENT_GESTURE {
            return if ctx.armed.load(Ordering::SeqCst) {
                std::ptr::null_mut()
            } else {
                event.as_ptr()
            };
        }
        if event_type.0 != K_CGS_EVENT_DOCK_CONTROL {
            return event.as_ptr();
        }

        // Horizontal dock swipes (Spaces switching) forward as the remote
        // app switch; vertical (Mission Control / App Exposé) forward as the
        // remote Task View toggle. Pinch passes through untouched.
        let motion = CGEvent::integer_value_field(Some(cg_event), CGEventField(F_SWIPE_MOTION));
        if motion != K_MOTION_HORIZONTAL && motion != K_MOTION_VERTICAL {
            return event.as_ptr();
        }

        let phase = CGEvent::integer_value_field(Some(cg_event), CGEventField(F_GESTURE_PHASE));
        let progress = CGEvent::double_value_field(Some(cg_event), CGEventField(F_SWIPE_PROGRESS));
        let armed = ctx.armed.load(Ordering::SeqCst);

        let mut state = match ctx.state.lock() {
            Ok(s) => s,
            Err(_) => return event.as_ptr(),
        };
        if state.dumped < 12 {
            state.dumped += 1;
            tracing::info!(
                phase,
                progress = format!("{progress:.4}"),
                motion,
                armed,
                "dock-swipe event"
            );
        }
        // Field 132 is a BIT MASK (began=1, changed=2, ended=4, cancelled=8)
        // — combined values (e.g. 6 = changed|ended on the terminal event)
        // are legal. Test bits; exact equality misses real gestures.
        let began = phase & PHASE_BEGAN != 0;
        let changed = phase & PHASE_CHANGED != 0;
        let ended = phase & PHASE_ENDED != 0;
        let cancelled = phase & PHASE_CANCELLED != 0;
        if began {
            state.in_gesture = true;
            state.events = 1;
            state.max_abs_progress = progress.abs();
        } else if changed || (state.in_gesture && !ended && !cancelled) {
            if !state.in_gesture {
                state.in_gesture = true;
                state.events = 0;
                state.max_abs_progress = 0.0;
            }
            state.events += 1;
            if progress.abs() > state.max_abs_progress {
                state.max_abs_progress = progress.abs();
            }
        }
        if ended || cancelled {
            state.in_gesture = false;
            if progress.abs() > state.max_abs_progress {
                state.max_abs_progress = progress.abs();
            }
            // Finalize: fire the remote gesture when the swipe was deliberate.
            if armed
                && ended
                && !cancelled
                && state.events >= 2
                && state.max_abs_progress >= SWIPE_MIN_PROGRESS
            {
                // Vertical = Mission Control toggle. Direction-agnostic: the
                // remote side gets Win+Tab, which is itself a toggle (swipe
                // again to dismiss) — and the macOS>=26 progress-sign flip
                // is unobservable in that semantics.
                if motion == K_MOTION_VERTICAL {
                    let now_ms = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_millis())
                        .unwrap_or(0);
                    if now_ms.saturating_sub(state.last_fire_ms) >= 400 {
                        state.last_fire_ms = now_ms;
                        tracing::info!(
                            progress = format!("{progress:.4}"),
                            "dock swipe (vertical) suppressed locally; forwarding remote task view toggle"
                        );
                        let _ = ctx
                            .tx
                            .send(crate::platform::RawInput::GestureSwipe { direction: 0 });
                    }
                } else {
                    let mut direction: i8 = if progress < 0.0 { -1 } else { 1 };
                    if os_major() >= 26 {
                        direction = -direction;
                    }
                    let now_ms = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_millis())
                        .unwrap_or(0);
                    if now_ms.saturating_sub(state.last_fire_ms) >= 400 {
                        state.last_fire_ms = now_ms;
                        tracing::info!(
                            progress = format!("{progress:.4}"),
                            direction,
                            "dock swipe suppressed locally; forwarding remote app switch"
                        );
                        let _ = ctx
                            .tx
                            .send(crate::platform::RawInput::GestureSwipe { direction });
                    }
                }
            }
            *state = DockSwipeState {
                in_gesture: false,
                max_abs_progress: 0.0,
                events: 0,
                dumped: state.dumped,
                last_fire_ms: state.last_fire_ms,
            };
        }

        if armed {
            // Consumed: the Dock never sees the swipe, so macOS does not
            // switch the local Space. The app switch happens on the remote.
            std::ptr::null_mut()
        } else {
            event.as_ptr()
        }
    }
}

/// Install the session-level dock-swipe event tap. Runs on its own thread
/// with its own CFRunLoop (same pattern as the HID input tap).
#[cfg(target_os = "macos")]
pub fn start_dock_swipe_tap(
    armed: std::sync::Arc<std::sync::atomic::AtomicBool>,
    tx: std::sync::mpsc::Sender<RawInput>,
) -> Result<(), CoreError> {
    use objc2_core_foundation::{kCFRunLoopCommonModes, CFMachPort, CFRunLoop};
    use objc2_core_graphics::{
        CGEvent, CGEventMask, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};

    if !is_accessibility_trusted() {
        return Err(CoreError::Json(
            "macOS Accessibility permission is required for gesture capture".into(),
        ));
    }

    let mask: CGEventMask = (1u64 << dock_swipe::K_CGS_EVENT_GESTURE)
        | (1u64 << dock_swipe::K_CGS_EVENT_DOCK_CONTROL)
        | (1u64 << 62)
        | (1u64 << 63);

    let (ready_tx, ready_rx) = std::sync::mpsc::channel();

    let builder = std::thread::Builder::new().name("dock-swipe-tap".into());
    builder
        .spawn(move || unsafe {
            let ctx = Box::new(dock_swipe::DockSwipeContext {
                armed,
                tx,
                state: Mutex::new(dock_swipe::DockSwipeState {
                    in_gesture: false,
                    max_abs_progress: 0.0,
                    events: 0,
                    dumped: 0,
                    last_fire_ms: 0,
                }),
                tap_port: AtomicUsize::new(0),
            });
            let user_info_ptr =
                &*ctx as *const dock_swipe::DockSwipeContext as *mut std::ffi::c_void;
            // The context must outlive this function; the runloop below never
            // returns, so leaking is unnecessary — but the callback holds the
            // raw pointer, so keep the Box alive by forgetting it. The tap is
            // torn down only at process exit.
            std::mem::forget(ctx);

            let tap = CGEvent::tap_create(
                CGEventTapLocation::SessionEventTap,
                CGEventTapPlacement::HeadInsertEventTap,
                CGEventTapOptions::Default,
                mask,
                Some(dock_swipe::dock_swipe_callback),
                user_info_ptr,
            );
            let port = match tap {
                Some(p) => p,
                None => {
                    let _ = ready_tx.send(Err(
                        "CGEventTapCreate(session) returned null (Accessibility required)".into(),
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
                rl.add_source(Some(&rl_source), kCFRunLoopCommonModes);
                CGEvent::tap_enable(&port, true);
                // The context was leaked (still valid): update its tap port
                // through the same pointer the callback uses.
                let ctx_ref = &*(user_info_ptr as *const dock_swipe::DockSwipeContext);
                ctx_ref
                    .tap_port
                    .store(&*port as *const CFMachPort as usize, Ordering::SeqCst);
                let _ = ready_tx.send(Ok(()));
                CFRunLoop::run();
            } else {
                let _ = ready_tx.send(Err("CFRunLoopGetCurrent returned null".into()));
            }
        })
        .map_err(|e| CoreError::Json(format!("spawn dock-swipe tap thread: {e}")))?;

    ready_rx
        .recv()
        .map_err(|e| CoreError::Json(format!("dock-swipe tap ready channel: {e}")))?
        .map_err(CoreError::Json)?;
    tracing::info!(
        "dock-swipe tap armed (four-finger swipe → remote app switch while controlling)"
    );
    Ok(())
}

#[cfg(target_os = "macos")]
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

    // Dock-swipe tap (session level): intercepts the private type-30
    // kIOHIDEventTypeDockSwipe stream BEFORE the Dock sees it. While this
    // node controls a remote desktop the horizontal swipe is suppressed
    // locally (no macOS Spaces switch) and forwarded as the remote app
    // switch; vertical gestures and everything else pass through.
    {
        let armed = std::sync::Arc::clone(&keyboard_armed);
        let tx = tx.clone();
        std::thread::Builder::new()
            .name("dock-swipe-tap".into())
            .spawn(move || {
                if let Err(e) = start_dock_swipe_tap(armed, tx) {
                    tracing::warn!("dock-swipe tap unavailable: {e}");
                }
            })
            .ok();
    }

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

        // v0.2.34: any physical pointer event reveals the idle hide applied
        // when the controller left this screen.
        if matches!(
            event_type,
            CGEventType::MouseMoved
                | CGEventType::LeftMouseDragged
                | CGEventType::RightMouseDragged
                | CGEventType::OtherMouseDragged
                | CGEventType::LeftMouseDown
                | CGEventType::RightMouseDown
                | CGEventType::OtherMouseDown
                | CGEventType::ScrollWheel
        ) {
            reveal_idle_hidden_cursor();
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
                // Root-fix F: the deltas macOS computes for the FIRST events
                // after a warp span the teleport (they include the park
                // displacement), so applying them drives the remote cursor
                // to the park position's map (production: entry placement
                // snapped to screen center). Zero the deltas for the first
                // few post-warp events; their positions are near the park
                // point and the deltas carry no real hand motion. The counter
                // is time-bounded: if its events never surfaced within
                // WARP_MARKER_MAX_AGE_MS it is dropped rather than eating
                // later real motion.
                let settle = WARP_SETTLE_EVENTS.load(Ordering::SeqCst);
                let (mut dx, mut dy) = if settle > 0 && warp_marker_fresh() {
                    WARP_SETTLE_EVENTS.store(settle - 1, Ordering::SeqCst);
                    (0, 0)
                } else {
                    if settle > 0 {
                        WARP_SETTLE_EVENTS.store(0, Ordering::SeqCst);
                    }
                    (dx, dy)
                };
                // v0.2.26: Deskflow bogus-motion filter, macOS twin of the
                // Windows hook's `hook_delta_is_bogus`. The settle counter
                // above only covers the FIRST events after a warp while its
                // 250ms marker is fresh; an echo event that surfaces later
                // (or a second echo macOS coalesces past the gate) still
                // carries the whole warp displacement — hundreds of px. Fed
                // to the roaming engine it is a "fast push" that instantly
                // re-crosses the shared edge (production: ghost-controller
                // bounce loop — rescue docked inside the redundancy band,
                // the rescue-warp echo re-crossed Top, and the Windows
                // target's physical mouse was swallowed every ~13s). A real
                // hand cannot span center-to-edge in one event; drop it.
                if hook_delta_is_bogus(dx, dy, ctx.screen_w, ctx.screen_h) {
                    dx = 0;
                    dy = 0;
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
                    match armed_key_decision(keycode) {
                        ArmedKeyDecision::Forward { vk } => {
                            let _ = ctx.tx.send(RawInput::Key {
                                code: vk,
                                down: true,
                            });
                            std::ptr::null_mut()
                        }
                        ArmedKeyDecision::PassThrough => event.as_ptr(),
                    }
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
                    match armed_key_decision(keycode) {
                        ArmedKeyDecision::Forward { vk } => {
                            let _ = ctx.tx.send(RawInput::Key {
                                code: vk,
                                down: false,
                            });
                            std::ptr::null_mut()
                        }
                        ArmedKeyDecision::PassThrough => event.as_ptr(),
                    }
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
                    match armed_key_decision(keycode) {
                        ArmedKeyDecision::Forward { vk } => {
                            let _ = ctx.tx.send(RawInput::Key {
                                code: vk,
                                down: is_down,
                            });
                            std::ptr::null_mut()
                        }
                        // Same unmapped-key rule as KeyDown/KeyUp: Fn flags
                        // transitions must not be swallowed locally, or the
                        // OS keeps the Fn latch asserted.
                        ArmedKeyDecision::PassThrough => event.as_ptr(),
                    }
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
            let (ctx_w, ctx_h) = screen_size();
            let ctx = Box::new(MacTapContext {
                tx,
                keyboard_armed: keyboard_armed_thread,
                pressed_modifiers: pressed_modifiers_thread,
                tap_port: tap_port_thread,
                screen_w: ctx_w,
                screen_h: ctx_h,
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

    /// v0.2.32: Windows cursor hiding (Deskflow MSWindowsScreen idiom —
    /// `SetSystemCursor(CopyIcon(blank), OCR_*)` for every system cursor id,
    /// restored by `SystemParametersInfo(SPI_SETCURSORS)`). The id table and
    /// the blank cursor planes are pure data, so they are pinned here on
    /// every host: every OEM cursor the shell can show must be covered, the
    /// table must hold no duplicates, and the blank planes must describe a
    /// fully transparent 32x32 monochrome cursor (AND mask all ones, XOR
    /// plane all zeros).
    #[test]
    fn system_cursor_ids_cover_every_oem_cursor_and_blank_planes_are_transparent() {
        use super::system_cursor::{blank_cursor_planes, BLANK_CURSOR_SIZE, SYSTEM_CURSOR_IDS};

        let ids = SYSTEM_CURSOR_IDS;
        assert_eq!(ids.len(), 14, "all fourteen OCR_* ids must be covered");
        let mut sorted = ids.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), ids.len(), "duplicate ids would double-copy");
        for id in ids {
            assert!((32512..=32651).contains(&id), "{id} is not an OCR_* id");
        }
        for must in [
            32512u32, 32513, 32514, 32515, 32516, 32642, 32649, 32650, 32651,
        ] {
            assert!(ids.contains(&must), "OCR id {must} missing");
        }

        assert_eq!(BLANK_CURSOR_SIZE, 32);
        let (and_plane, xor_plane) = blank_cursor_planes();
        assert_eq!(and_plane.len(), 32 * 32 / 8);
        assert_eq!(xor_plane.len(), 32 * 32 / 8);
        assert!(
            and_plane.iter().all(|b| *b == 0xFF),
            "AND mask must be opaque"
        );
        assert!(
            xor_plane.iter().all(|b| *b == 0x00),
            "XOR plane must be empty"
        );
    }

    /// v0.2.31: an injected move while a button is held must be posted as
    /// the matching *Dragged event, else AppKit never sees a drag (mac
    /// target: no text selection / window drag from the Windows mouse).
    #[test]
    fn injected_move_kind_follows_held_buttons() {
        assert_eq!(injected_move_kind(0), InjectedMoveKind::Moved);
        assert_eq!(
            injected_move_kind(tetherly_core::BTN_LEFT),
            InjectedMoveKind::LeftDragged
        );
        assert_eq!(
            injected_move_kind(tetherly_core::BTN_RIGHT),
            InjectedMoveKind::RightDragged
        );
        assert_eq!(
            injected_move_kind(tetherly_core::BTN_MIDDLE),
            InjectedMoveKind::OtherDragged
        );
        // Left wins when several are held (same precedence as button()).
        assert_eq!(
            injected_move_kind(tetherly_core::BTN_LEFT | tetherly_core::BTN_RIGHT),
            InjectedMoveKind::LeftDragged
        );
    }

    #[test]
    fn injected_button_tracker_sets_and_clears_masks() {
        let s = next_injected_buttons(0, tetherly_core::BTN_LEFT, true);
        assert_eq!(s, tetherly_core::BTN_LEFT);
        let s = next_injected_buttons(s, tetherly_core::BTN_RIGHT, true);
        assert_eq!(s, tetherly_core::BTN_LEFT | tetherly_core::BTN_RIGHT);
        let s = next_injected_buttons(s, tetherly_core::BTN_LEFT, false);
        assert_eq!(s, tetherly_core::BTN_RIGHT);
        // Releasing an un-held button is a no-op.
        let s = next_injected_buttons(s, tetherly_core::BTN_MIDDLE, false);
        assert_eq!(s, tetherly_core::BTN_RIGHT);
        let s = next_injected_buttons(s, tetherly_core::BTN_RIGHT, false);
        assert_eq!(s, 0);
    }

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

    #[test]
    fn fn_globe_key_is_unmapped_by_design() {
        // The Fn / globe key (0x3F) intentionally has no Windows VK mapping:
        // it is a local-only key. The tap must therefore PASS IT THROUGH when
        // armed (regression guard for the stuck-Fn bug where the callback
        // swallowed its FlagsChanged and the OS kept the Fn latch asserted).
        // If someone ever adds a 0x3F mapping, the pass-through rule for it
        // must be re-evaluated together with the remote Fn semantics.
        assert_eq!(mac_keycode_to_win_vk(0x3F), None);
        assert_eq!(mac_keycode_to_win_vk(0x34), None); // globe/Fn on some layouts
    }

    #[test]
    fn armed_key_decision_matrix() {
        use ArmedKeyDecision::{Forward, PassThrough};

        // Mapped keys forward with the right VK, for every branch the tap
        // consults the decision (the branch itself does not re-filter).
        assert_eq!(armed_key_decision(0x00), Forward { vk: 0x41 }); // A
        assert_eq!(armed_key_decision(0x37), Forward { vk: 0x5B }); // Left Cmd
        assert_eq!(armed_key_decision(0x3B), Forward { vk: 0xA2 }); // Left Ctrl
        assert_eq!(armed_key_decision(0x7A), Forward { vk: 0x70 }); // F1

        // Unmapped keys pass through — the stuck-Fn regression contract.
        assert_eq!(armed_key_decision(0x3F), PassThrough); // Fn / globe
        assert_eq!(armed_key_decision(0x34), PassThrough);

        // Caps Lock is decided by the dedicated 0x39 branch BEFORE the
        // decision fn (synthetic press+release pair, not Forward semantics) —
        // pin that the decision fn itself would forward it, so the special
        // branch stays necessary and visible.
        assert_eq!(armed_key_decision(0x39), Forward { vk: 0x14 });
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

    // Runs on macOS only under `cfg(test)` (hook_move_state is compiled
    // outside Windows just for this): proves the Windows hook's warp echo
    // swallows the teleport displacement and ANCHORS the delta baseline at
    // the warp target (Deskflow PRE_WARP saveMousePosition alignment) — the
    // next real motion is measured from wherever the cursor actually is, so
    // per-batch roaming recenters never lose motion. The one hazard of the
    // anchored baseline (a pre-warp in-flight event consumed after the echo
    // reporting a teleport-spanning delta) is covered by the bogus filter in
    // `hook_delta_is_bogus`.
    #[cfg(all(target_os = "macos", test))]
    #[test]
    fn warp_echo_resets_delta_baseline() {
        use super::hook_move_state;

        // 1. Real user motion: baseline established at (500, 400).
        let _gate = super::warp_state_test_gate();
        assert_eq!(hook_move_state::advance(500, 400), (0, 0)); // cold start: no baseline
        assert_eq!(hook_move_state::advance(510, 404), (10, 4));

        // 2. Programmatic warp to screen center (roaming recenter); the hook
        //    sees the echo at the warp target and must swallow it (not
        //    (dx, dy) it).
        super::mark_pending_warp(960, 540);
        assert!(hook_move_state::take_warp_echo_reset_baseline(960, 540));

        // 3. The next REAL event (hand moved on to (966, 544)) is measured
        //    from the anchored baseline at the warp target: delta = the
        //    genuine hand motion since the warp — not zero, and not the
        //    teleport displacement.
        assert_eq!(hook_move_state::advance(966, 544), (6, 4));

        // 4. Baseline re-established: deltas track again.
        assert_eq!(hook_move_state::advance(976, 548), (10, 4));

        // 5. A position that is NOT the warp target never matches the
        //    marker: no echo, and the baseline is left alone (anchor happens
        //    only when an echo is actually consumed).
        super::mark_pending_warp(100, 100);
        assert!(!hook_move_state::take_warp_echo_reset_baseline(5000, 5000));
        assert_eq!(hook_move_state::advance(5000, 5004), (4024, 4456));

        // 6. Only the echo at the marker's target consumes it and anchors
        //    the baseline; misses in between don't.
        super::mark_pending_warp(20, 30);
        assert!(!hook_move_state::take_warp_echo_reset_baseline(5020, 5030));
        assert_eq!(hook_move_state::advance(5030, 5040), (30, 36));
        assert!(hook_move_state::take_warp_echo_reset_baseline(20, 30));
        assert_eq!(hook_move_state::advance(26, 33), (6, 3));
    }

    /// Deskflow bogus-motion filter (MSWindowsComputer::onMouseMove): with
    /// the delta baseline anchored at screen center by the roaming recenter,
    /// a delta that spans roughly the center-to-edge distance cannot be a
    /// hand motion — it is the leaked displacement of a warp racing the hook
    /// FIFO (in-flight pre-warp event consumed after the echo anchored the
    /// baseline at center). The filter must drop exactly those and keep all
    /// plausible hand motions. Pure geometry, replayed synthetically.
    #[cfg(all(target_os = "macos", test))]
    #[test]
    fn hook_bogus_filter_drops_teleport_span_deltas() {
        let (w, h) = (1920_i32, 1080_i32);
        let (cx, cy) = (w / 2, h / 2);

        // Ordinary hand motions from the center baseline: never bogus.
        assert!(!hook_delta_is_bogus(6, 4, w, h));
        assert!(!hook_delta_is_bogus(-200, 150, w, h));
        assert!(!hook_delta_is_bogus(0, 0, w, h));

        // A violent flick stays far below the center-to-edge distance.
        assert!(!hook_delta_is_bogus(-300, 250, w, h));

        // A leaked teleport displacement (edge position consumed after the
        // center anchor): spans the FULL center-to-edge distance — only
        // deltas that reach past (span - 10px) are bogus, so the fixture
        // must go beyond the span itself, exactly like the in-flight
        // pre-warp event it models (edge → center anchor).
        assert!(hook_delta_is_bogus(cx - 5, 0, w, h));
        assert!(hook_delta_is_bogus(-(cx - 5), 0, w, h));
        assert!(hook_delta_is_bogus(0, cy - 5, w, h));
        assert!(hook_delta_is_bogus(0, -(cy - 5), w, h));

        // Mixed motion: one axis spanning, the other genuine — still bogus.
        assert!(hook_delta_is_bogus(cx - 5, 4, w, h));

        // Tiny screens keep the margin: a flick-size delta stays legit.
        assert!(!hook_delta_is_bogus(300, 250, 800, 600));
    }

    /// Per-batch roaming recenter (Deskflow MSWindowsComputer::onMouseMove):
    /// while roaming the Remote seat, the physical cursor is pulled back to
    /// screen center after every consumed motion batch, so the delta chain —
    /// anchored at center by the echo gate — always measures genuine hand
    /// motion and can never saturate at a screen boundary. Pure geometry —
    /// the target is the screen center; the caller owns the seat check and
    /// the actual warp.
    #[cfg(all(target_os = "macos", test))]
    #[test]
    fn roaming_recenter_target_is_screen_center() {
        let (w, h) = (1920_i32, 1080_i32);
        assert_eq!(roaming_recenter_target(w, h), (960, 540));
        assert_eq!(roaming_recenter_target(800, 600), (400, 300));
        // Degenerate dimensions stay inside the screen.
        assert_eq!(roaming_recenter_target(1, 1), (0, 0));
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

/// Spawn the remote-session viewer window (ADR-138 §2.6 step 5). AppKit
/// windows must live in the `tetherly-viewer` subprocess (installed next to
/// this binary); a headless host has no viewer and reports honestly — the
/// session still works wire-side without it.
/// Spawn the remote-session viewer window (ADR-138 §2.6 step 5). AppKit
/// windows must live in the `tetherly-viewer` subprocess (installed next to
/// this binary); a headless host has no viewer and reports honestly — the
/// session still works wire-side without it.
///
/// A viewer that exits immediately (not installed, or a stale binary that
/// does not understand `--remote-session`) is reported as absent instead of
/// returning a PID for a process that is already gone; a long-lived viewer
/// is handed to a reaper thread so the daemon never accumulates zombies.
#[cfg(target_os = "macos")]
pub fn spawn_remote_session_viewer() -> Result<u32, tetherly_core::CoreError> {
    let exe = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("tetherly-viewer")))
        .unwrap_or_else(|| std::path::PathBuf::from("tetherly-viewer"));
    let mut child = std::process::Command::new(exe)
        .arg("--remote-session")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| {
            tetherly_core::CoreError::ScreenRefused(format!(
                "spawning tetherly-viewer failed ({e}); macOS session viewer needs the AppKit subprocess"
            ))
        })?;
    let pid = child.id();
    // Give the subprocess a moment to fail fast, then report honestly: a
    // viewer that already exited must not be advertised as a live window.
    std::thread::sleep(std::time::Duration::from_millis(400));
    if let Ok(Some(status)) = child.try_wait() {
        return Err(tetherly_core::CoreError::ScreenRefused(format!(
            "tetherly-viewer exited immediately (status: {status}); session viewer window unavailable — session continues wire-side only"
        )));
    }
    // Still alive: detach with a reaper so the exit is collected (no zombie)
    // once the operator closes the window.
    std::thread::Builder::new()
        .name("session-viewer-reap".into())
        .spawn(move || {
            let _ = child.wait();
        })
        .ok();
    Ok(pid)
}

#[cfg(not(target_os = "macos"))]
pub fn spawn_remote_session_viewer() -> Result<u32, tetherly_core::CoreError> {
    Err(tetherly_core::CoreError::ScreenRefused(
        "no session viewer on this platform; session continues wire-side only".into(),
    ))
}

#[cfg(test)]
mod vk_translation_tests {
    use super::*;

    #[test]
    fn win_vk_to_mac_keycode_resolves_every_gesture_vk() {
        // The gesture backends ship these wire VKs; each must translate or
        // the sequence silently injects nothing on a mac target.
        assert_eq!(win_vk_to_mac_keycode(0x09), Some(0x30)); // Tab
        assert_eq!(win_vk_to_mac_keycode(0x5B), Some(0x37)); // LWIN -> Command
        assert_eq!(win_vk_to_mac_keycode(0xA2), Some(0x3B)); // LCONTROL -> Control
        assert_eq!(win_vk_to_mac_keycode(0xC0), Some(0x32)); // OEM_3 -> grave
        assert_eq!(win_vk_to_mac_keycode(0x25), Some(0x7B)); // Left arrow
        assert_eq!(win_vk_to_mac_keycode(0x27), Some(0x7C)); // Right arrow
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn injected_modifier_flags_track_down_up_cycles() {
        use objc2_core_graphics::CGEventFlags;

        // Cmd down: the keydown carries the mask and the tracker holds it.
        let (post, state) = next_injected_flags(0, 0x5B, true);
        assert_eq!(post, CGEventFlags::MaskCommand);
        assert_eq!(state, CGEventFlags::MaskCommand.bits());
        // Grave tap while Cmd is held: carries the mask, tracker unchanged.
        let (post, state) = next_injected_flags(state, 0xC0, true);
        assert_eq!(post, CGEventFlags::MaskCommand);
        assert_eq!(state, CGEventFlags::MaskCommand.bits());
        // Cmd up: the keyup still carries the mask; the tracker drops it.
        let (post, state) = next_injected_flags(state, 0x5B, false);
        assert_eq!(post, CGEventFlags::MaskCommand);
        assert_eq!(state, 0);

        // The Shift+Alt+Tab shape: masks accumulate and release independently
        // (downs Shift, Alt; ups Alt, Shift — the tap helper's exact order).
        let (_, s) = next_injected_flags(0, 0xA0, true);
        let (post, s) = next_injected_flags(s, 0xA4, true);
        assert_eq!(post, CGEventFlags::MaskShift | CGEventFlags::MaskAlternate);
        let (_, s) = next_injected_flags(s, 0xA4, false);
        let (post, s) = next_injected_flags(s, 0xA0, false);
        assert_eq!(post, CGEventFlags::MaskShift);
        assert_eq!(s, 0);
    }
}

#[cfg(test)]
mod daemon_responsiveness_tests {
    use super::*;

    // Production 2026-11-13 (v0.2.34): the scheduled-task launcher hands the
    // daemon BelowNormal — the same class as the 35-thread SRBMiner on a
    // 12-core host — and the idle trimmer had cut its working set to 6.5 MB,
    // so the first edge push after a quiet spell paged the takeover path
    // back in under CPU contention: 2.9 s between the fast-push decision
    // and the takeover request. Anything below AboveNormal must be lifted.
    #[test]
    fn daemon_priority_lifts_every_class_below_above_normal() {
        for class in [
            WIN_IDLE_PRIORITY_CLASS,
            WIN_BELOW_NORMAL_PRIORITY_CLASS,
            WIN_NORMAL_PRIORITY_CLASS,
        ] {
            assert!(
                daemon_priority_needs_raise(class),
                "class {class:#x} must be raised"
            );
        }
    }

    // An operator who already runs the daemon High/Realtime must never be
    // demoted, and AboveNormal is already the target.
    #[test]
    fn daemon_priority_never_lowers_an_equal_or_higher_class() {
        for class in [
            WIN_ABOVE_NORMAL_PRIORITY_CLASS,
            WIN_HIGH_PRIORITY_CLASS,
            WIN_REALTIME_PRIORITY_CLASS,
        ] {
            assert!(
                !daemon_priority_needs_raise(class),
                "class {class:#x} must be left alone"
            );
        }
    }

    // Real effect on the Windows CI runner: after the call the process
    // class is at least AboveNormal and the report says so.
    #[cfg(windows)]
    #[test]
    fn raise_daemon_responsiveness_lifts_this_process_priority() {
        let report = raise_daemon_responsiveness();
        let class = unsafe {
            windows_sys::Win32::System::Threading::GetPriorityClass(
                windows_sys::Win32::System::Threading::GetCurrentProcess(),
            )
        };
        assert!(
            !daemon_priority_needs_raise(class),
            "priority class after raise: {class:#x}"
        );
        assert!(report.priority_ok, "{report:?}");
    }

    // macOS/Linux have no miner-starvation analogue; the call is an honest
    // no-op that reports nothing applied.
    #[cfg(not(windows))]
    #[test]
    fn raise_daemon_responsiveness_is_a_no_op_off_windows() {
        assert_eq!(
            raise_daemon_responsiveness(),
            DaemonResponsiveness::default()
        );
    }
}
