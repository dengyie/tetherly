// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Tray Popover panel (ADR-137 §4.3): click the tray icon to reveal a
//! compact panel showing live peer status and real-time file transfers.
//!
//! Data comes exclusively from the loopback REST plane (`/api/peers`,
//! `/api/file/transfers`), polled on a run-loop timer and re-rendered as
//! text rows — no direct Node handle crosses into UI land, mirroring the
//! tray dropzone architecture.
//!
//! macOS: NSPopover anchored to the status item's button (this module).
//! Windows: a topmost popup window owned by the tray message pump (tray.rs
//! windows_tray) renders the same rows.

#[cfg(target_os = "macos")]
pub mod macos {
    use objc2::rc::{Allocated, Retained};
    use objc2::runtime::NSObjectProtocol;
    use objc2::{define_class, msg_send, sel, DefinedClass, MainThreadOnly};
    use objc2_app_kit::{
        NSFont, NSPopover, NSPopoverBehavior, NSStackView, NSStackViewGravity, NSStatusBarButton,
        NSTextField, NSUserInterfaceLayoutOrientation, NSView, NSViewController,
    };
    use objc2_core_foundation::CGFloat;
    use objc2_foundation::{
        MainThreadMarker, NSPoint, NSRect, NSRunLoopCommonModes, NSSize, NSString, NSTimeInterval,
        NSTimer,
    };

    /// Compact panel geometry (ADR-137 §4.3).
    const PANEL_W: CGFloat = 320.0;
    const PANEL_H: CGFloat = 240.0;
    const MARGIN: CGFloat = 10.0;
    /// Data refresh cadence while the panel is visible.
    const REFRESH_SECS: NSTimeInterval = 1.0;

    pub(crate) struct PopoverIvars {
        ui_port: u16,
        peers_line: Retained<NSTextField>,
        transfers_line: Retained<NSTextField>,
    }

    define_class!(
        #[unsafe(super(NSViewController))]
        #[thread_kind = MainThreadOnly]
        #[name = "TetherlyPopoverVC"]
        #[ivars = PopoverIvars]
        struct PopoverVC;

        impl PopoverVC {
            #[unsafe(method_id(initWithNibName:bundle:))]
            fn init_nib(
                this: Allocated<Self>,
                nib_name_or_nil: Option<&NSString>,
                nib_bundle_or_nil: Option<&objc2_foundation::NSBundle>,
            ) -> Retained<Self> {
                // Our class is MainThreadOnly and only ever instantiated from
                // `toggle` on the main thread, so the allocation is
                // main-thread by construction.
                let mtm = unsafe { MainThreadMarker::new_unchecked() };
                let this = this.set_ivars(PopoverIvars {
                    ui_port: 0,
                    peers_line: make_line(12.0, true, mtm),
                    transfers_line: make_line(11.0, false, mtm),
                });
                unsafe {
                    msg_send![
                        super(this),
                        initWithNibName: nib_name_or_nil,
                        bundle: nib_bundle_or_nil
                    ]
                }
            }

            /// Timer callback: refresh rows from the loopback plane.
            #[unsafe(method(refreshData))]
            fn refresh_data(&self) {
                let ivars = self.ivars();
                let port = ivars.ui_port;
                set_peers_line(&ivars.peers_line, port);
                set_transfers_line(&ivars.transfers_line, port);
            }
        }

        unsafe impl NSObjectProtocol for PopoverVC {}
    );

    fn make_line(size: CGFloat, bold: bool, mtm: MainThreadMarker) -> Retained<NSTextField> {
        let label = NSTextField::labelWithString(&NSString::from_str("…"), mtm);
        let font = if bold {
            NSFont::boldSystemFontOfSize(size)
        } else {
            NSFont::monospacedDigitSystemFontOfSize_weight(size, 0.0)
        };
        label.setFont(Some(&font));
        label
    }

    impl PopoverVC {
        fn build_view(&self, mtm: MainThreadMarker) -> Retained<NSView> {
            let root = NSView::initWithFrame(
                NSView::alloc(mtm),
                NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(PANEL_W, PANEL_H)),
            );
            let stack = NSStackView::initWithFrame(
                NSStackView::alloc(mtm),
                NSRect::new(
                    NSPoint::new(MARGIN, MARGIN),
                    NSSize::new(PANEL_W - MARGIN * 2.0, PANEL_H - MARGIN * 2.0),
                ),
            );
            stack.setOrientation(NSUserInterfaceLayoutOrientation::Vertical);
            stack.setSpacing(8.0);
            stack.addView_inGravity(&self.ivars().peers_line, NSStackViewGravity::Top);
            stack.addView_inGravity(&self.ivars().transfers_line, NSStackViewGravity::Top);
            root.addSubview(&stack);
            root
        }
    }

    // ------------------------------------------------------------------
    // Loopback data → row text
    // ------------------------------------------------------------------

    fn get_json(port: u16, path: &str) -> Option<serde_json::Value> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .ok()?;
        let polled = runtime.block_on(async {
            match tetherly_loopback::loopback_get(port, path).await {
                Ok((200, body)) => Some(body),
                _ => None,
            }
        });
        let body = polled?;
        serde_json::from_slice(&body).ok()
    }

    fn set_peers_line(line: &NSTextField, port: u16) {
        let text = match get_json(port, "/api/peers").and_then(|v| v.get("live").cloned()) {
            Some(live) => {
                let rows: Vec<String> = live
                    .as_array()
                    .map(|arr| {
                        arr.iter()
                            .map(|p| {
                                let name = p["alias"]
                                    .as_str()
                                    .or_else(|| p["name"].as_str())
                                    .unwrap_or("?");
                                let platform = p["platform"].as_str().unwrap_or("");
                                format!("🟢 {name} ({platform})")
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                if rows.is_empty() {
                    "未连接对端 — 拖文件到托盘图标可直接投递".to_string()
                } else {
                    format!("Peers:\n  {}", rows.join("\n  "))
                }
            }
            None => "⚠️ 无法连接本地守护进程".to_string(),
        };
        line.setStringValue(&NSString::from_str(&text));
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

    /// Compact progress bar: `█████░░░░░ 52%`.
    fn progress_bar(done: f64) -> String {
        const WIDTH: usize = 10;
        let filled = ((done * WIDTH as f64).round() as usize).clamp(0, WIDTH);
        format!("{}{}", "█".repeat(filled), "░".repeat(WIDTH - filled))
    }

    fn set_transfers_line(line: &NSTextField, port: u16) {
        let text = match get_json(port, "/api/file/transfers") {
            Some(v) => {
                let items = v.get("transfers").and_then(|t| t.as_array());
                let rows: Vec<String> = items
                    .map(|arr| {
                        arr.iter()
                            .map(|t| {
                                let name = t["filename"].as_str().unwrap_or("file");
                                let total = t["total"].as_u64().unwrap_or(0);
                                let done_bytes = t["transferred"].as_u64().unwrap_or(0);
                                let speed = t["speed_bps"].as_u64().unwrap_or(0);
                                let phase = t["phase"].as_str().unwrap_or("");
                                let frac = if total > 0 {
                                    done_bytes as f64 / total as f64
                                } else {
                                    0.0
                                };
                                match phase {
                                    "Completed" => format!("✅ {name} ({})", fmt_bytes(total)),
                                    "Failed" => {
                                        let e = t["error"].as_str().unwrap_or("failed");
                                        format!("❌ {name} — {e}")
                                    }
                                    _ => {
                                        let pct = (frac * 100.0) as u64;
                                        let arrow =
                                            if phase == "Verifying" { "🔍" } else { "⇄" };
                                        format!(
                                            "{arrow} {name}\n  {} {}% ({}/{}, {})",
                                            progress_bar(frac),
                                            pct,
                                            fmt_bytes(done_bytes),
                                            fmt_bytes(total),
                                            fmt_bps(speed)
                                        )
                                    }
                                }
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                if rows.is_empty() {
                    "空 — 拖文件到托盘 ⇄".to_string()
                } else {
                    format!("传输:\n  {}", rows.join("\n  "))
                }
            }
            None => "传输: 无法读取状态".to_string(),
        };
        line.setStringValue(&NSString::from_str(&text));
    }

    // ------------------------------------------------------------------
    // Popover lifecycle
    // ------------------------------------------------------------------

    struct PopoverState {
        popover: Retained<NSPopover>,
        timer: Retained<NSTimer>,
    }

    // The state object lives entirely on the main thread (popover + timer
    // + NSTextField are all main-thread AppKit objects); `toggle` is only
    // ever invoked from AppKit event handling on the main thread, so a
    // Mutex shared across threads serializes the rare non-main-thread
    // early-return path and the state itself is never touched off-thread.
    struct PopoverStateBox(std::sync::Mutex<Option<PopoverState>>);
    unsafe impl Sync for PopoverStateBox {}
    static POPOVER: PopoverStateBox = PopoverStateBox(std::sync::Mutex::new(None));

    /// Called from the tray on status-button click: show the popover (or
    /// close it if already shown). A run-loop timer refreshes rows every
    /// second while visible; the Transient behavior closes it on outside
    /// clicks (we invalidate the timer next time the user toggles, since
    /// wiring popoverDidClose needs the full delegate protocol). The
    /// `button` is the popover's positioning anchor.
    pub fn toggle(ui_port: u16, _button: &NSStatusBarButton) {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let mut guard = POPOVER.0.lock().unwrap();
        // Take the old state unconditionally so a Transient-closed popover
        // still gets its timer invalidated on the next click.
        if let Some(state) = guard.take() {
            state.timer.invalidate();
            if state.popover.isShown() {
                state.popover.close();
            }
            return;
        }

        // Build the content VC with the port baked in, refresh once for a
        // first paint so the panel never flashes stale text.
        let alloc: Allocated<PopoverVC> = PopoverVC::alloc(mtm);
        let vc = alloc.set_ivars(PopoverIvars {
            ui_port,
            peers_line: make_line(12.0, true, mtm),
            transfers_line: make_line(11.0, false, mtm),
        });
        unsafe {
            let vc: Retained<PopoverVC> = msg_send![
                super(vc),
                initWithNibName: Option::<&NSString>::None,
                bundle: Option::<&objc2_foundation::NSBundle>::None
            ];
            let content = vc.build_view(mtm);
            vc.setView(&content);
            // Direct call of the defined method (its Rust form takes the
            // implicit `_cmd` selector argument).
            vc.refresh_data(sel!(refreshData));

            let popover = NSPopover::new(mtm);
            popover.setBehavior(NSPopoverBehavior::Transient);
            // Deref coercion: PopoverVC -> NSViewController.
            popover.setContentViewController(Some(&vc));
            popover.showRelativeToRect_ofView_preferredEdge(
                content.bounds(),
                &content,
                objc2_foundation::NSRectEdge::MaxY,
            );

            let timer = NSTimer::timerWithTimeInterval_target_selector_userInfo_repeats(
                REFRESH_SECS,
                &vc,
                sel!(refreshData),
                None,
                true,
            );
            objc2_foundation::NSRunLoop::currentRunLoop()
                .addTimer_forMode(&timer, NSRunLoopCommonModes);

            *guard = Some(PopoverState { popover, timer });
            // AppKit retains the VC through the popover's content view
            // controller + the timer's target; forget our typed handle so
            // it survives past this function.
            std::mem::forget(vc);
        }
    }
}
