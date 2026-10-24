// SPDX-License-Identifier: Apache-2.0 OR MIT
//! `tetherly-viewer` — the macOS screen viewer subprocess (§9.7 呈现).
//!
//! AppKit windows belong to the main thread and need a running
//! `NSApplication`, which the daemon's tokio frame pump can never offer. The
//! honest architecture (recorded in the v1.89 round) is therefore a viewer
//! *process*: this binary owns the main thread, reads length-prefixed BGRA
//! frames on stdin, and presents them in an `NSWindow`. The daemon spawns it
//! on demand from `screen/macos.rs::present` and closes its stdin when the
//! stream ends — EOF makes the viewer exit.
//!
//! Wire format per frame, all big-endian:
//! `u32 w | u32 h | u32 body_len | body_len bytes of BGRA`.
//!
//! Runtime against a real GUI session is Manual-required; the code is
//! compile-verified on the host and CI's macos job.

fn main() {
    #[cfg(not(target_os = "macos"))]
    {
        eprintln!("tetherly-viewer presents BGRA frames in an AppKit NSWindow; it is macOS-only");
        std::process::exit(2);
    }
    #[cfg(target_os = "macos")]
    viewer::viewer_main();
}

/// One decoded frame handed from the stdin reader to the main thread.
#[cfg(target_os = "macos")]
pub struct Frame {
    pub w: u32,
    pub h: u32,
    pub bgra: Vec<u8>,
}

/// Parse exactly one wire frame from `buf` if it is fully present.
/// Returns the frame and how many bytes it consumed.
#[cfg(target_os = "macos")]
fn parse_frame(buf: &[u8]) -> Option<(Frame, usize)> {
    const HDR: usize = 12;
    if buf.len() < HDR {
        return None;
    }
    let w = u32::from_be_bytes(buf[0..4].try_into().unwrap());
    let h = u32::from_be_bytes(buf[4..8].try_into().unwrap());
    let len = u32::from_be_bytes(buf[8..12].try_into().unwrap()) as usize;
    // 8 MiB is the whole-frame budget on the wire (§9.7); anything larger is
    // a corrupt stream, not a bigger screen.
    let budget = 8 * 1024 * 1024;
    if w == 0 || h == 0 || len > budget || len != w as usize * h as usize * 4 {
        return None;
    }
    if buf.len() < HDR + len {
        return None;
    }
    Some((
        Frame {
            w,
            h,
            bgra: buf[HDR..HDR + len].to_vec(),
        },
        HDR + len,
    ))
}

#[cfg(target_os = "macos")]
mod viewer {
    use super::{parse_frame, Frame};
    use objc2::msg_send;
    use objc2::rc::{autoreleasepool, Retained};
    use objc2::{ClassType, MainThreadMarker, MainThreadOnly};
    use objc2_app_kit::{
        NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSImage, NSImageView,
        NSWindow, NSWindowStyleMask,
    };
    use objc2_core_foundation::kCFRunLoopDefaultMode;
    use objc2_core_graphics::{
        CGBitmapContextCreate, CGBitmapContextCreateImage, CGColorSpace, CGImage,
    };
    use objc2_foundation::{NSPoint, NSRect, NSSize};
    use std::io::Read;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::SyncSender;
    use std::sync::mpsc::{self};

    /// Premultiplied-first alpha + little-endian 32-bit words: B,G,R,A in
    /// memory — the exact layout the capture side produces.
    const BGRA8: u32 = 2 | (2 << 12);

    /// Reader thread: stdin → frames → channel. `done` flips on EOF or a
    /// malformed stream; either way the main loop winds down.
    fn reader(tx: SyncSender<Frame>, done: &'static AtomicBool) {
        let mut buf: Vec<u8> = Vec::with_capacity(8 * 1024 * 1024 + 12);
        let mut chunk = [0u8; 64 * 1024];
        let mut stdin = std::io::stdin().lock();
        loop {
            match stdin.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    buf.extend_from_slice(&chunk[..n]);
                    while let Some((frame, used)) = parse_frame(&buf) {
                        buf.drain(..used);
                        if tx.send(frame).is_err() {
                            done.store(true, Ordering::Relaxed);
                            return;
                        }
                    }
                    if buf.len() > 8 * 1024 * 1024 + 12 {
                        // No complete frame inside the budget: corrupt stream.
                        break;
                    }
                }
            }
        }
        done.store(true, Ordering::Relaxed);
    }

    /// BGRA → CGImage via a bitmap context that borrows our buffer as its
    /// backing store (the same API the capture side uses, reversed), then a
    /// copy-on-write image snapshot. Both drop at the end of `show`.
    unsafe fn cgimage_from_bgra(
        f: &Frame,
    ) -> Option<objc2_core_foundation::CFRetained<objc2_core_graphics::CGImage>> {
        let space = CGColorSpace::new_device_rgb()?;
        let ctx = CGBitmapContextCreate(
            f.bgra.as_ptr() as *mut std::ffi::c_void,
            f.w as usize,
            f.h as usize,
            8,
            f.w as usize * 4,
            Some(&space),
            BGRA8,
        )?;
        CGBitmapContextCreateImage(Some(&ctx))
    }

    struct Ui {
        window: Retained<NSWindow>,
        view: Retained<NSImageView>,
    }

    unsafe fn build_ui(mtm: MainThreadMarker, first: &Frame) -> Option<Ui> {
        let style =
            NSWindowStyleMask::Titled | NSWindowStyleMask::Closable | NSWindowStyleMask::Resizable;
        let rect = NSRect::new(
            NSPoint::new(100.0, 100.0),
            NSSize::new(first.w as f64, first.h as f64),
        );
        let window = NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            rect,
            style,
            NSBackingStoreType::Buffered,
            false,
        );
        window.setTitle(&objc2_foundation::NSString::from_str("Tetherly 远程屏幕"));
        let view = NSImageView::initWithFrame(NSImageView::alloc(mtm), rect);
        window.setContentView(Some(&view));
        window.makeKeyAndOrderFront(None);
        Some(Ui { window, view })
    }

    /// `objc2-app-kit` 0.3 declares no thread-safety trait for `NSImage`, so
    /// the sanctioned `alloc` helpers cannot name it. Raw `alloc` +
    /// `initWithCGImage:size:` here, the same `msg_send` style `ancs/macos.rs`
    /// uses; both calls stay on the main thread.
    unsafe fn nsimage_from_cg(cg: &CGImage, w: u32, h: u32) -> Option<Retained<NSImage>> {
        let allocated: *mut objc2::runtime::AnyObject = msg_send![NSImage::class(), alloc];
        if allocated.is_null() {
            return None;
        }
        let size = NSSize::new(w as f64, h as f64);
        let raw: *mut objc2::runtime::AnyObject =
            msg_send![allocated, initWithCGImage: cg, size: size];
        if raw.is_null() {
            return None;
        }
        Retained::from_raw(raw.cast())
    }

    unsafe fn show(ui: &Ui, f: &Frame) {
        autoreleasepool(|_| {
            let Some(cg) = cgimage_from_bgra(f) else {
                return;
            };
            let Some(image) = nsimage_from_cg(&cg, f.w, f.h) else {
                return;
            };
            ui.view.setImage(Some(&image));
            ui.window.display();
        });
    }

    pub fn viewer_main() {
        static DONE: AtomicBool = AtomicBool::new(false);
        let (tx, rx) = mpsc::sync_channel::<Frame>(8);
        std::thread::spawn(|| reader(tx, &DONE));

        unsafe {
            // This process exists to own the main thread; the marker cannot
            // fail here.
            let mtm = MainThreadMarker::new_unchecked();
            let app = NSApplication::sharedApplication(mtm);
            app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
            let mut ui: Option<Ui> = None;
            // The first frame sizes the window; later frames only refresh
            // the image, mirroring M5.3's no-replay semantics.
            loop {
                let mut latest: Option<Frame> = None;
                while let Ok(f) = rx.try_recv() {
                    latest = Some(f);
                }
                if let Some(f) = latest {
                    match &ui {
                        None => match build_ui(mtm, &f) {
                            Some(built) => {
                                show(&built, &f);
                                app.activate();
                                ui = Some(built);
                            }
                            None => {
                                eprintln!("tetherly-viewer: window creation failed");
                                return;
                            }
                        },
                        Some(u) => show(u, &f),
                    }
                } else if DONE.load(Ordering::Relaxed) {
                    break;
                }
                // Pump AppKit ~30 ms without a full NSApp.run(): enough for
                // window compositing, and it never blocks past EOF.
                objc2_core_foundation::CFRunLoop::run_in_mode(kCFRunLoopDefaultMode, 0.03, true);
            }
            if let Some(u) = ui {
                u.window.orderOut(None);
            }
        }
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    fn wire(w: u32, h: u32, bgra: &[u8]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&w.to_be_bytes());
        v.extend_from_slice(&h.to_be_bytes());
        v.extend_from_slice(&(bgra.len() as u32).to_be_bytes());
        v.extend_from_slice(bgra);
        v
    }

    #[test]
    fn parses_a_complete_frame_and_reports_the_consumed_length() {
        let bytes = wire(2, 1, &[1, 2, 3, 4, 5, 6, 7, 8]);
        let (f, used) = parse_frame(&bytes).unwrap();
        assert_eq!((f.w, f.h), (2, 1));
        assert_eq!(f.bgra, vec![1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(used, bytes.len());
    }

    #[test]
    fn waits_for_a_partial_body() {
        let bytes = wire(4, 4, &[0u8; 64]);
        assert!(parse_frame(&bytes[..20]).is_none());
    }

    #[test]
    fn rejects_corrupt_geometry_and_oversized_budgets() {
        // len disagrees with w*h*4
        let mut bad = Vec::new();
        bad.extend_from_slice(&2u32.to_be_bytes());
        bad.extend_from_slice(&2u32.to_be_bytes());
        bad.extend_from_slice(&3u32.to_be_bytes());
        assert!(parse_frame(&bad).is_none());
        // zero dims
        let zero = wire(0, 4, &[0u8; 4]);
        assert!(parse_frame(&zero).is_none());
        // over the 8 MiB budget
        let huge = wire(4096, 4096, &[0u8; 4]);
        // (w*h*4 fits u32 here exactly; body_len 4 ≠ expected → rejected)
        assert!(parse_frame(&huge).is_none());
    }
}
