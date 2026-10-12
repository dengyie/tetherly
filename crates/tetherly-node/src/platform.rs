// SPDX-License-Identifier: Apache-2.0 OR MIT
//! OS clipboard, conservative insert, and allowlist-driven app open.
//! insert() and open() only run on a user click.

use std::sync::{Arc, Mutex};
use tetherly_core::{insertion_plan, Clip, CoreError, InsertionPlan, Insertor, Opener};

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
            inject_mouse(MOUSEEVENTF_WHEEL, 0, 0, dy as u32);
        }
        if dx != 0 {
            inject_mouse(MOUSEEVENTF_HWHEEL, 0, 0, dx as u32);
        }
    }

    fn key(&mut self, code: u16, down: bool, mods: u8) {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
            SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP,
        };
        let _ = mods; // modifiers travel as their own key events
        let input = KEYBDINPUT {
            wVk: code,
            wScan: 0,
            dwFlags: if down { 0 } else { KEYEVENTF_KEYUP },
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

#[cfg(not(windows))]
impl tetherly_core::InputSink for SystemInputSink {
    fn move_abs(&mut self, _x: i32, _y: i32) {}
    fn button(&mut self, _mask: u8, _down: bool) {}
    fn wheel(&mut self, _dx: i16, _dy: i16) {}
    fn key(&mut self, _code: u16, _down: bool, _mods: u8) {}
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
        SendInput(
            1,
            [INPUT {
                r#type: INPUT_MOUSE,
                Anonymous: INPUT_0 { mi: input },
            }]
            .as_mut_ptr(),
            std::mem::size_of::<INPUT>() as i32,
        );
    }
}

// ---------------------------------------------------------------------------
// SystemInputCapture — low-level hooks on the controller side
// ---------------------------------------------------------------------------

/// One raw local input observation from the hook thread. Mouse coordinates
/// are physical pixels; keys are Win32 virtual-key codes.
#[cfg(windows)]
#[derive(Debug, Clone, Copy)]
pub enum RawInput {
    Move { x: i32, y: i32 },
    Button { mask: u8, down: bool },
    Wheel { dx: i16, dy: i16 },
    Key { code: u16, down: bool },
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

/// Primary-monitor size in pixels, for the controller's edge state machine.
#[cfg(windows)]
pub fn screen_size() -> (i32, i32) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};
    unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) }
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
                    WM_MOUSEMOVE => push(RawInput::Move {
                        x: mi.pt.x,
                        y: mi.pt.y,
                    }),
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

#[cfg(all(test, windows))]
mod input_tests {
    use super::units_to_abs;

    #[test]
    fn units_map_to_full_absolute_range() {
        assert_eq!(units_to_abs(0), 0);
        assert_eq!(units_to_abs(1000), 65_535);
        assert_eq!(units_to_abs(500), 32_767); // 65535/2
                                               // Out-of-range clamps instead of wrapping.
        assert_eq!(units_to_abs(-5), 0);
        assert_eq!(units_to_abs(2000), 65_535);
        // Monotonic across the whole domain.
        let mut prev = -1;
        for u in 0..=1000 {
            let abs = units_to_abs(u);
            assert!(abs >= prev);
            prev = abs;
        }
    }
}
