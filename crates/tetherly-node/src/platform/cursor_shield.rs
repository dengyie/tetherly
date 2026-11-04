// SPDX-License-Identifier: Apache-2.0 OR MIT
//! macOS Hardware Cursor Shield Window and Blank Cursor Overlay.
//!
//! Aligns with Deskflow PR #10231 architecture to permanently eradicate
//! hardware cursor ghosts, imprints, or hover artifacts on macOS when
//! roaming to a secondary screen (e.g. Windows).
//!
//! 1) Transparent borderless `NSWindow` shields at `NSScreenSaverWindowLevel` (level 1000)
//!    placed over all active displays (`NSScreen.screens`).
//! 2) `ignoresMouseEvents: NO` ensures no underlying application or system UI
//!    (Dock, Menu Bar, browser, terminal) can receive mouse tracking rects or call `[NSCursor set]`.
//! 3) 16x16 transparent blank `NSCursor` deployed across AppKit cursor stack.
//! 4) Immediate destruction and cursor restoration upon entering local seat.

#[cfg(target_os = "macos")]
mod macos_impl {
    use dispatch2::DispatchQueue;
    use objc2::rc::Retained;
    use objc2::MainThreadMarker;
    use objc2_app_kit::{
        NSBackingStoreType, NSColor, NSCursor, NSImage, NSScreen, NSScreenSaverWindowLevel,
        NSWindow, NSWindowCollectionBehavior, NSWindowStyleMask,
    };
    use objc2_foundation::{NSPoint, NSSize};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Mutex;

    // Desired display state of the cursor shield overlay.
    // Prevents async/sync race conditions where hide_cursor_shields() finishes before
    // an asynchronously queued show_cursor_shields() executes, which would leave a
    // 1000-level transparent window permanently intercepting macOS mouse clicks.
    static SHIELDS_ACTIVE: AtomicBool = AtomicBool::new(false);

    // Raw pointers to active NSWindow instances (stored as usize) so they can be held across threads,
    // while all creation, presentation, and destruction are strictly dispatched to macOS's main queue.
    static SHIELD_WINDOWS: Mutex<Vec<usize>> = Mutex::new(Vec::new());

    fn set_blank_cursor_main(mtm: MainThreadMarker) {
        let blank_img = NSImage::initWithSize(mtm.alloc(), NSSize::new(16.0, 16.0));
        let blank_cursor =
            NSCursor::initWithImage_hotSpot(mtm.alloc(), &blank_img, NSPoint::new(0.0, 0.0));
        blank_cursor.set();
    }

    /// Sets blank cursor immediately if already on the main thread, or asynchronously on the main queue.
    /// This ensures the cursor image is transparent without blocking background event loops.
    pub fn set_blank_cursor_now() {
        if let Some(mtm) = MainThreadMarker::new() {
            set_blank_cursor_main(mtm);
        } else {
            DispatchQueue::main().exec_async(|| {
                if let Some(mtm) = MainThreadMarker::new() {
                    set_blank_cursor_main(mtm);
                }
            });
        }
    }

    /// Safely invokes NSCursor::hide() on macOS's main thread.
    pub fn hide_appkit_cursor() {
        if MainThreadMarker::new().is_some() {
            NSCursor::hide();
        } else {
            DispatchQueue::main().exec_async(|| {
                NSCursor::hide();
            });
        }
    }

    /// Safely invokes NSCursor::unhide() on macOS's main thread.
    pub fn unhide_appkit_cursor() {
        if MainThreadMarker::new().is_some() {
            NSCursor::unhide();
        } else {
            DispatchQueue::main().exec_async(|| {
                NSCursor::unhide();
            });
        }
    }

    /// Destroys and removes any active shield windows (strictly on the main thread).
    fn remove_cursor_shields_main(_mtm: MainThreadMarker) {
        let mut shields = SHIELD_WINDOWS.lock().expect("lock");
        for &ptr in shields.iter() {
            if ptr != 0 {
                if let Some(window) = unsafe { Retained::from_raw(ptr as *mut NSWindow) } {
                    window.orderOut(None);
                    // window drops here, releasing the retained NSWindow
                }
            }
        }
        shields.clear();
    }

    /// Creates and orders front full-screen borderless transparent shield windows on all displays at NSScreenSaverWindowLevel.
    pub fn show_cursor_shields() {
        SHIELDS_ACTIVE.store(true, Ordering::SeqCst);
        DispatchQueue::main().exec_async(|| {
            // If hide was requested in the interim, abort immediately without creating windows
            if !SHIELDS_ACTIVE.load(Ordering::SeqCst) {
                if let Some(mtm) = MainThreadMarker::new() {
                    remove_cursor_shields_main(mtm);
                }
                return;
            }

            let Some(mtm) = MainThreadMarker::new() else {
                return;
            };
            remove_cursor_shields_main(mtm);

            let screens = NSScreen::screens(mtm);
            let mut new_shields = Vec::with_capacity(screens.len());

            for screen in screens.iter() {
                let frame = screen.frame();
                let shield: Retained<NSWindow> = unsafe {
                    NSWindow::initWithContentRect_styleMask_backing_defer(
                        mtm.alloc(),
                        frame,
                        NSWindowStyleMask::Borderless,
                        NSBackingStoreType::Buffered,
                        false,
                    )
                };
                unsafe {
                    shield.setReleasedWhenClosed(false);
                }
                shield.setOpaque(false);
                shield.setBackgroundColor(Some(&NSColor::clearColor()));
                shield.setHasShadow(false);
                shield.setIgnoresMouseEvents(false);
                shield.setLevel(NSScreenSaverWindowLevel);
                shield.setCollectionBehavior(
                    NSWindowCollectionBehavior::CanJoinAllSpaces
                        | NSWindowCollectionBehavior::Stationary
                        | NSWindowCollectionBehavior::FullScreenAuxiliary
                        | NSWindowCollectionBehavior::IgnoresCycle,
                );
                shield.setFrame_display(frame, false);
                shield.orderFrontRegardless();

                let raw = Retained::into_raw(shield) as usize;
                new_shields.push(raw);
            }

            // Check again before committing to storage: if hide was called while creating windows, tear down immediately
            if !SHIELDS_ACTIVE.load(Ordering::SeqCst) {
                for &ptr in &new_shields {
                    if ptr != 0 {
                        if let Some(window) = unsafe { Retained::from_raw(ptr as *mut NSWindow) } {
                            window.orderOut(None);
                        }
                    }
                }
                return;
            }

            set_blank_cursor_main(mtm);

            let mut guard = SHIELD_WINDOWS.lock().expect("lock");
            *guard = new_shields;
        });
    }

    /// Hides and destroys all cursor shield windows, restoring underlying app interaction.
    pub fn hide_cursor_shields() {
        SHIELDS_ACTIVE.store(false, Ordering::SeqCst);
        if let Some(mtm) = MainThreadMarker::new() {
            remove_cursor_shields_main(mtm);
        } else {
            DispatchQueue::main().exec_async(|| {
                if let Some(mtm) = MainThreadMarker::new() {
                    remove_cursor_shields_main(mtm);
                }
            });
        }
    }
}

#[cfg(target_os = "macos")]
pub use macos_impl::*;

#[cfg(not(target_os = "macos"))]
pub fn set_blank_cursor_now() {}

#[cfg(not(target_os = "macos"))]
pub fn hide_appkit_cursor() {}

#[cfg(not(target_os = "macos"))]
pub fn unhide_appkit_cursor() {}

#[cfg(not(target_os = "macos"))]
pub fn show_cursor_shields() {}

#[cfg(not(target_os = "macos"))]
pub fn hide_cursor_shields() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cursor_shield_api_lifecycle() {
        set_blank_cursor_now();
        show_cursor_shields();
        hide_cursor_shields();
    }
}
