// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Cross-platform system tray icon and background status presence.
//!
//! macOS: NSStatusBar template glyph; left click opens the Magpie-style
//! panel (frosted NSPanel + WKWebView on the SPA `#tray` route), right
//! click pops the compact context menu.
//! Windows: Shell_NotifyIconW notification area icon with popup menu and dashboard launcher.
//!
//! §6.3 tray takeover: the daemon's built-in tray hides itself while a
//! Tauri shell claims `tray_owner=client` and reappears on release/reclaim.
//! Both platform loops poll [`VISIBLE`] (the `tray_set_visible` write side)
//! on a cheap timer — no cross-thread AppKit calls, no extra sync surface.

use std::sync::atomic::{AtomicBool, Ordering};

static VISIBLE: AtomicBool = AtomicBool::new(true);

/// Show or hide the built-in tray icon (idempotent). Platform loops pick
/// the change up within one poll tick.
pub fn tray_set_visible(visible: bool) {
    VISIBLE.store(visible, Ordering::Release);
}

/// Read-side used by the platform loops. On platforms without a tray loop
/// (Linux stub) the only consumer disappears, hence the allow.
#[cfg_attr(not(any(windows, target_os = "macos")), allow(dead_code))]
fn tray_visible() -> bool {
    VISIBLE.load(Ordering::Acquire)
}

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
    use std::cell::RefCell;
    use std::sync::atomic::{AtomicU64, Ordering};

    use objc2::rc::{Allocated, Retained};
    use objc2::runtime::{AnyObject, Bool, NSObject, NSObjectProtocol, ProtocolObject};
    use objc2::{define_class, msg_send, sel, AnyThread, ClassType, DefinedClass, MainThreadOnly};
    use objc2_app_kit::{
        NSApplication, NSApplicationActivationPolicy, NSAutoresizingMaskOptions,
        NSBackingStoreType, NSCellImagePosition, NSColor, NSDragOperation, NSDraggingDestination,
        NSDraggingInfo, NSEventMask, NSEventModifierFlags, NSEventType, NSImage, NSMenu,
        NSMenuItem, NSPanel, NSPasteboardTypeFileURL, NSPopUpMenuWindowLevel, NSScreen,
        NSStatusBar, NSStatusBarButton, NSStatusItem, NSView, NSVisualEffectBlendingMode,
        NSVisualEffectMaterial, NSVisualEffectState, NSVisualEffectView,
        NSWindowCollectionBehavior, NSWindowStyleMask,
    };
    use objc2_foundation::{
        MainThreadMarker, NSArray, NSData, NSDictionary, NSNumber, NSPoint, NSRect, NSSize,
        NSString, NSURLRequest, NSURL,
    };
    use objc2_web_kit::{
        WKScriptMessage, WKScriptMessageHandler, WKUserContentController, WKWebView,
        WKWebViewConfiguration,
    };

    /// Panel geometry (§225): Magpie's menubar panel is 380pt wide; the
    /// height fits status + devices + transfers cards plus the footer.
    const PANEL_W: f64 = 380.0;
    const PANEL_H: f64 = 468.0;
    /// The page reports its content height (`resize` action); clamp it.
    const PANEL_MIN_H: f64 = 200.0;
    const PANEL_MAX_H: f64 = 640.0;
    /// Gap between the menubar bottom edge and the panel.
    const PANEL_GAP: f64 = 6.0;
    /// A click on the status item while the panel is open first resigns key
    /// (hides the panel) and then fires the button action; swallow a re-show
    /// inside this window so the click reads as "close", not "close+reopen".
    const RETOGGLE_GUARD_MS: u64 = 300;
    /// JS bridge name — `window.webkit.messageHandlers.tetherlyTray`.
    const BRIDGE_NAME: &str = "tetherlyTray";

    pub(crate) struct TrayIvars {
        ui_port: u16,
        device_alias: String,
        /// Status-item button (visibility fold + panel anchor). Only the
        /// tray target's ivars carry a value; the drop view keeps `None`.
        button: Option<Retained<NSStatusBarButton>>,
        /// Status item (right-click menu attach/detach).
        item: Option<Retained<NSStatusItem>>,
    }

    fn now_ms() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
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
                let this = this.set_ivars(TrayIvars {
                    ui_port: 0,
                    device_alias: String::new(),
                    button: None,
                    item: None,
                });
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

    // ---------------------------------------------------------------
    // Menubar panel (§225, Magpie-style): borderless non-activating
    // NSPanel → NSVisualEffectView (popover material, 12pt corners) →
    // transparent WKWebView loading the SPA's `#tray` route. The page
    // talks back through the `tetherlyTray` script-message bridge.
    // ---------------------------------------------------------------

    static LAST_HIDE_MS: AtomicU64 = AtomicU64::new(0);

    struct PanelState {
        panel: Retained<TrayPanel>,
        web: Retained<WKWebView>,
        loaded: bool,
        /// Content-fitted height last reported by the page.
        height: f64,
    }

    thread_local! {
        static PANEL: RefCell<Option<PanelState>> = const { RefCell::new(None) };
    }

    define_class!(
        #[unsafe(super(NSPanel))]
        #[thread_kind = MainThreadOnly]
        #[name = "TetherlyTrayPanel"]
        pub(crate) struct TrayPanel;

        impl TrayPanel {
            /// Borderless panels refuse key status by default; the web view
            /// needs it for focus/typing and for the resign-key dismissal.
            #[unsafe(method(canBecomeKeyWindow))]
            fn can_become_key_window(&self) -> bool {
                true
            }

            /// Click-outside dismissal: losing key status hides the panel
            /// (same model as NSPopover transient behaviour, without the
            /// arrow chrome Magpie also avoids).
            #[unsafe(method(resignKeyWindow))]
            fn resign_key_window(&self) {
                unsafe {
                    let _: () = msg_send![super(self), resignKeyWindow];
                }
                self.orderOut(None);
                LAST_HIDE_MS.store(now_ms(), Ordering::Relaxed);
            }
        }
    );

    pub(crate) struct BridgeIvars {
        ui_port: u16,
    }

    define_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "TetherlyTrayBridge"]
        #[ivars = BridgeIvars]
        pub(crate) struct TrayBridge;

        unsafe impl NSObjectProtocol for TrayBridge {}

        unsafe impl WKScriptMessageHandler for TrayBridge {
            /// `window.webkit.messageHandlers.tetherlyTray.postMessage(json)`
            /// from the SPA tray bridge. The body is a JSON string with an
            /// `action` field (traybridge.ts `TrayAction`).
            #[unsafe(method(userContentController:didReceiveScriptMessage:))]
            fn did_receive_script_message(
                &self,
                _controller: &WKUserContentController,
                message: &WKScriptMessage,
            ) {
                let body = unsafe { message.body() };
                let Ok(text) = body.downcast::<NSString>() else {
                    return;
                };
                handle_panel_action(self.ivars().ui_port, &text.to_string());
            }
        }
    );

    /// Dispatch one SPA tray action. `toggled`/`error` are informational
    /// (the page already reflects them); the host-side ones are open-main,
    /// close and quit.
    fn handle_panel_action(ui_port: u16, raw: &str) {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(raw) else {
            tracing::debug!(raw, "tray panel: non-JSON bridge message ignored");
            return;
        };
        match value["action"].as_str().unwrap_or_default() {
            "open-main" => {
                hide_panel();
                crate::apitoken::open_client_or_dashboard(
                    ui_port,
                    tetherly_loopback::api_token_for(ui_port).as_deref(),
                );
            }
            "close" => hide_panel(),
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

    /// Fit the panel to the page's content height, keeping its top edge
    /// under the status item. The borrow is released before the (possibly
    /// run-loop-pumping) animated frame change.
    fn resize_panel(height: f64) {
        let target = PANEL.with(|slot| {
            let mut slot = slot.borrow_mut();
            let state = slot.as_mut()?;
            if (state.height - height).abs() < 0.5 {
                return None;
            }
            state.height = height;
            state.panel.isVisible().then(|| state.panel.clone())
        });
        if let Some(panel) = target {
            let f = panel.frame();
            let top = f.origin.y + f.size.height;
            let frame = NSRect::new(
                NSPoint::new(f.origin.x, (top - height).round()),
                NSSize::new(f.size.width, height),
            );
            panel.setFrame_display_animate(frame, true, true);
        }
    }

    fn hide_panel() {
        PANEL.with(|slot| {
            if let Some(state) = slot.borrow().as_ref() {
                if state.panel.isVisible() {
                    state.panel.orderOut(None);
                    LAST_HIDE_MS.store(now_ms(), Ordering::Relaxed);
                }
            }
        });
    }

    /// The SPA tray route with the per-port API token in the fragment
    /// (`#tray&token=…`, same handoff as the dashboard URL — App.svelte
    /// strips the token and keeps `#tray`).
    fn panel_url(ui_port: u16) -> String {
        match tetherly_loopback::api_token_for(ui_port) {
            Some(token) => format!("http://127.0.0.1:{ui_port}/#tray&token={token}"),
            None => format!("http://127.0.0.1:{ui_port}/#tray"),
        }
    }

    fn load_panel_page(web: &WKWebView, ui_port: u16) {
        let url = NSString::from_str(&panel_url(ui_port));
        if let Some(url) = NSURL::URLWithString(&url) {
            let request = NSURLRequest::requestWithURL(&url);
            let _ = unsafe { web.loadRequest(&request) };
        }
    }

    fn build_panel(mtm: MainThreadMarker, ui_port: u16) -> PanelState {
        let rect = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(PANEL_W, PANEL_H));
        let mask = NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel;
        let panel: Retained<TrayPanel> = unsafe {
            msg_send![
                TrayPanel::alloc(mtm),
                initWithContentRect: rect,
                styleMask: mask,
                backing: NSBackingStoreType::Buffered,
                defer: false,
            ]
        };
        panel.setLevel(NSPopUpMenuWindowLevel);
        panel.setOpaque(false);
        panel.setBackgroundColor(Some(&NSColor::clearColor()));
        panel.setHasShadow(true);
        panel.setMovable(false);
        panel.setHidesOnDeactivate(false);
        panel.setCollectionBehavior(
            NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::FullScreenAuxiliary
                | NSWindowCollectionBehavior::Transient
                | NSWindowCollectionBehavior::IgnoresCycle,
        );

        // Frosted card: popover material behind the window, rounded by the
        // backing layer so the web content is clipped to the same radius.
        let fx: Retained<NSVisualEffectView> =
            unsafe { msg_send![NSVisualEffectView::alloc(mtm), initWithFrame: rect] };
        fx.setMaterial(NSVisualEffectMaterial::Popover);
        fx.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
        fx.setState(NSVisualEffectState::Active);
        fx.setWantsLayer(true);
        unsafe {
            let layer: Option<Retained<AnyObject>> = msg_send![&*fx, layer];
            if let Some(layer) = layer {
                let _: () = msg_send![&*layer, setCornerRadius: 12.0f64];
                let _: () = msg_send![&*layer, setMasksToBounds: true];
            }
        }
        fx.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable
                | NSAutoresizingMaskOptions::ViewHeightSizable,
        );

        // Web view: transparent so the material shows through the SPA's
        // `color-mix(var(--bg) 93%, transparent)` panel background.
        let config = unsafe { WKWebViewConfiguration::new(mtm) };
        let bridge = TrayBridge::alloc(mtm).set_ivars(BridgeIvars { ui_port });
        let bridge: Retained<TrayBridge> = unsafe { msg_send![super(bridge), init] };
        unsafe {
            config.userContentController().addScriptMessageHandler_name(
                ProtocolObject::from_ref(&*bridge),
                &NSString::from_str(BRIDGE_NAME),
            );
        }
        let web: Retained<WKWebView> =
            unsafe { WKWebView::initWithFrame_configuration(WKWebView::alloc(mtm), rect, &config) };
        web.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable
                | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        unsafe {
            // Private-but-stable KVC key WebKit honours for transparent pages.
            let no = NSNumber::numberWithBool(false);
            let _: () =
                msg_send![&*web, setValue: &*no, forKey: &*NSString::from_str("drawsBackground")];
            web.setUnderPageBackgroundColor(Some(&NSColor::clearColor()));
        }
        fx.addSubview(&web);
        panel.setContentView(Some(&fx));
        // The user content controller keeps the handler alive, but pin it
        // for the process lifetime regardless — the panel never goes away.
        std::mem::forget(bridge);

        PanelState {
            panel,
            web,
            loaded: false,
            height: PANEL_H,
        }
    }

    /// Anchor the panel centred under the status item, clamped to the
    /// screen's visible frame (origin is bottom-left in AppKit coordinates).
    fn anchored_frame(button: &NSStatusBarButton, height: f64, mtm: MainThreadMarker) -> NSRect {
        let bounds = button.bounds();
        let in_window = button.convertRect_toView(bounds, None);
        let window = button.window();
        let on_screen = window
            .as_ref()
            .map(|w| w.convertRectToScreen(in_window))
            .unwrap_or(in_window);
        let mut x = on_screen.origin.x + on_screen.size.width / 2.0 - PANEL_W / 2.0;
        let mut y = on_screen.origin.y - height - PANEL_GAP;
        let screen = window
            .as_ref()
            .and_then(|w| w.screen())
            .or_else(|| NSScreen::mainScreen(mtm));
        if let Some(screen) = screen {
            let vf = screen.visibleFrame();
            let max_x = vf.origin.x + vf.size.width - PANEL_W - 8.0;
            x = x.min(max_x).max(vf.origin.x + 8.0);
            y = y.max(vf.origin.y + 8.0);
        }
        NSRect::new(
            NSPoint::new(x.round(), y.round()),
            NSSize::new(PANEL_W, height),
        )
    }

    /// Build the panel (if needed) and start loading the SPA page without
    /// showing it. The first build spawns the WebContent process and the
    /// page load runs the whole SPA bootstrap — hundreds of ms, far more on
    /// a loaded host — which used to land on the user's click (§232).
    fn ensure_panel_loaded(
        slot: &mut Option<PanelState>,
        ui_port: u16,
        mtm: MainThreadMarker,
    ) -> &mut PanelState {
        let state = slot.get_or_insert_with(|| build_panel(mtm, ui_port));
        if !state.loaded {
            load_panel_page(&state.web, ui_port);
            state.loaded = true;
        }
        state
    }

    /// §232: pre-warm the panel shortly after the status item appears so the
    /// click only has to order an already-rendered window front. Skipped
    /// while the client owns the tray (the built-in item is hidden then).
    fn prewarm_panel(ui_port: u16, mtm: MainThreadMarker) {
        if !super::tray_visible() {
            return;
        }
        let started = std::time::Instant::now();
        PANEL.with(|slot| {
            ensure_panel_loaded(&mut slot.borrow_mut(), ui_port, mtm);
        });
        tracing::info!(
            elapsed_ms = started.elapsed().as_millis() as u64,
            "tray panel pre-warmed"
        );
    }

    fn toggle_panel(button: &NSStatusBarButton, ui_port: u16, mtm: MainThreadMarker) {
        let now = now_ms();
        PANEL.with(|slot| {
            let mut slot = slot.borrow_mut();
            let state = ensure_panel_loaded(&mut slot, ui_port, mtm);
            if state.panel.isVisible() {
                state.panel.orderOut(None);
                LAST_HIDE_MS.store(now, Ordering::Relaxed);
                return;
            }
            if now.saturating_sub(LAST_HIDE_MS.load(Ordering::Relaxed)) < RETOGGLE_GUARD_MS {
                return;
            }
            state
                .panel
                .setFrame_display(anchored_frame(button, state.height, mtm), false);
            state.panel.makeKeyAndOrderFront(None);
        });
    }

    define_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "TetherlyTrayTarget"]
        #[ivars = TrayIvars]
        pub(crate) struct TetherlyTrayTarget;

        impl TetherlyTrayTarget {
            /// Status-item click. Left click toggles the panel; right click
            /// (or ctrl-click) pops the compact context menu, Magpie-style.
            #[unsafe(method(statusClicked:))]
            fn status_clicked(&self, _sender: &NSStatusBarButton) {
                let mtm = self.mtm();
                let Some(button) = self.ivars().button.as_ref() else {
                    return;
                };
                let secondary = NSApplication::sharedApplication(mtm)
                    .currentEvent()
                    .map(|ev| {
                        ev.r#type() == NSEventType::RightMouseUp
                            || ev.modifierFlags().contains(NSEventModifierFlags::Control)
                    })
                    .unwrap_or(false);
                if secondary {
                    hide_panel();
                    self.show_context_menu(button, mtm);
                } else {
                    toggle_panel(button, self.ivars().ui_port, mtm);
                }
            }

            #[unsafe(method(openDashboard:))]
            fn open_dashboard(&self, _sender: &NSMenuItem) {
                let port = self.ivars().ui_port;
                // M4: prefer the Tauri client (deep link), fall back to the
                // browser console when no client is installed. §146: the
                // token lives in the per-port entry spawn_ui minted, not the
                // process global (the CLI's foreign-daemon fallback).
                crate::apitoken::open_client_or_dashboard(
                    port,
                    tetherly_loopback::api_token_for(port).as_deref(),
                );
            }

            #[unsafe(method(openAccessibility:))]
            fn open_accessibility(&self, _sender: &NSMenuItem) {
                let _ = crate::platform::open_accessibility_settings();
            }

            #[unsafe(method(quitTetherly:))]
            fn quit_tetherly(&self, _sender: &NSMenuItem) {
                std::process::exit(0);
            }

            /// §232: one-shot timer shortly after launch — build and load
            /// the panel off the click path.
            #[unsafe(method(prewarmPanel:))]
            fn prewarm_panel_tick(&self, _timer: &objc2_foundation::NSTimer) {
                prewarm_panel(self.ivars().ui_port, self.mtm());
            }

            /// §6.3 tray takeover: run-loop timer tick — fold the shared
            /// visibility flag into the status item. alphaValue=0 keeps the
            /// item, menu and dropzone alive while visually hidden; restore
            /// is alpha back to 1, no NSStatusBar surgery.
            #[unsafe(method(trayVisibilityTick:))]
            fn tray_visibility_tick(&self, _timer: &objc2_foundation::NSTimer) {
                static SHOWN: std::sync::atomic::AtomicBool =
                    std::sync::atomic::AtomicBool::new(true);
                let want = super::tray_visible();
                if SHOWN.swap(want, std::sync::atomic::Ordering::SeqCst) != want {
                    if let Some(button) = self.ivars().button.as_ref() {
                        button.setAlphaValue(if want { 1.0 } else { 0.0 });
                    }
                    // Alpha alone left an invisible, still-clickable gap in
                    // the menu bar; zero length collapses the slot. Not
                    // setVisible(false): AppKit persists that per autosave
                    // name and would keep the item hidden after a restart.
                    if let Some(item) = self.ivars().item.as_ref() {
                        item.setLength(if want { -1.0 } else { 0.0 });
                    }
                    if !want {
                        hide_panel();
                    }
                }
            }
        }

        unsafe impl NSObjectProtocol for TetherlyTrayTarget {}
    );

    impl TetherlyTrayTarget {
        /// Build the context menu fresh (accessibility state is live) and
        /// pop it from the status item: attach → synthetic click → detach,
        /// so left clicks keep reaching `statusClicked:` instead of the menu.
        fn show_context_menu(&self, button: &NSStatusBarButton, mtm: MainThreadMarker) {
            let Some(item) = self.ivars().item.as_ref() else {
                return;
            };
            let menu = build_context_menu(self, mtm);
            item.setMenu(Some(&menu));
            unsafe { button.performClick(None) };
            item.setMenu(None);
        }
    }

    fn menu_item(
        mtm: MainThreadMarker,
        title: &str,
        action: Option<objc2::runtime::Sel>,
        target: Option<&AnyObject>,
    ) -> Retained<NSMenuItem> {
        let item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                mtm.alloc(),
                &NSString::from_str(title),
                action,
                &NSString::from_str(""),
            )
        };
        if action.is_some() {
            unsafe { item.setTarget(target) };
            item.setEnabled(true);
        } else {
            item.setEnabled(false);
        }
        item
    }

    /// Compact right-click menu (no emoji, no chatter — Magpie's menubar
    /// keeps the text to status / console / permission / quit).
    fn build_context_menu(target: &TetherlyTrayTarget, mtm: MainThreadMarker) -> Retained<NSMenu> {
        let ivars = target.ivars();
        let target_ref: &AnyObject = target.as_ref();
        let menu = NSMenu::new(mtm);
        menu.addItem(&menu_item(
            mtm,
            &format!("Tetherly · 运行中 ({})", ivars.device_alias),
            None,
            None,
        ));
        menu.addItem(&NSMenuItem::separatorItem(mtm));
        menu.addItem(&menu_item(
            mtm,
            "打开控制台",
            Some(sel!(openDashboard:)),
            Some(target_ref),
        ));
        if crate::platform::is_accessibility_trusted() {
            menu.addItem(&menu_item(mtm, "辅助功能权限：已授权", None, None));
        } else {
            menu.addItem(&menu_item(
                mtm,
                "辅助功能权限：未授权…",
                Some(sel!(openAccessibility:)),
                Some(target_ref),
            ));
        }
        menu.addItem(&NSMenuItem::separatorItem(mtm));
        menu.addItem(&menu_item(
            mtm,
            "退出 Tetherly",
            Some(sel!(quitTetherly:)),
            Some(target_ref),
        ));
        menu
    }

    /// Build the tray dropzone view covering the full bounds of the status
    /// item's button. The view is the registered dragging destination; the
    /// button stays clickable underneath (the view is non-interfering for
    /// mouse events because a plain NSView passes clicks through its own
    /// hit-testing only when it has no event handlers — the menu still works
    /// since dragging is the only gesture this view intercepts).
    fn attach_drop_view(button: &NSStatusBarButton, ui_port: u16, mtm: MainThreadMarker) {
        let frame = button.bounds();
        let drop = TrayDropView::alloc(mtm).set_ivars(TrayIvars {
            ui_port,
            device_alias: String::new(),
            button: None,
            item: None,
        });
        let drop: Retained<TrayDropView> = unsafe { msg_send![super(drop), initWithFrame: frame] };
        let file_type = unsafe { NSPasteboardTypeFileURL };
        drop.registerForDraggedTypes(&NSArray::from_slice(&[file_type]));
        // Auto-resize with the button so the dropzone never shrinks away.
        drop.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable
                | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        button.addSubview(&drop);
        // Keep the drop view alive for the process lifetime.
        std::mem::forget(drop);
    }

    /// Monochrome template glyph (assets/brand/menubar-template-36x36.png,
    /// the mac47054639 mark at 18pt@2x). `setTemplate` lets AppKit tint it
    /// for light/dark menubars and the pressed state — Magpie's menubar
    /// presence is exactly this: one glyph, no title.
    fn menubar_image() -> Option<Retained<NSImage>> {
        let png = NSData::with_bytes(include_bytes!(
            "../../../../assets/brand/menubar-template-36x36.png"
        ));
        let image = NSImage::initWithData(NSImage::alloc(), &png)?;
        image.setSize(NSSize::new(18.0, 18.0));
        image.setTemplate(true);
        Some(image)
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
        let button_ref = item.button(mtm);
        let target = TetherlyTrayTarget::alloc(mtm).set_ivars(TrayIvars {
            ui_port,
            device_alias,
            button: button_ref.clone(),
            item: Some(item.clone()),
        });
        let target: Retained<TetherlyTrayTarget> = unsafe { msg_send![super(target), init] };
        let target_ref: &AnyObject = target.as_ref();

        if let Some(button) = button_ref.as_ref() {
            match menubar_image() {
                Some(image) => {
                    button.setImage(Some(&image));
                    button.setImagePosition(NSCellImagePosition::ImageOnly);
                    button.setTitle(&NSString::from_str(""));
                }
                // Glyph title fallback if AppKit cannot decode the PNG.
                None => button.setTitle(&NSString::from_str("⇄")),
            }
            button.setToolTip(Some(&NSString::from_str("Tetherly")));
            // Left click → panel, right click → menu (`statusClicked:`
            // inspects the current event). No menu is attached to the item
            // at rest, otherwise AppKit would swallow the action.
            unsafe {
                button.setTarget(Some(target_ref));
                button.setAction(Some(sel!(statusClicked:)));
            }
            button.sendActionOn(NSEventMask::LeftMouseUp | NSEventMask::RightMouseUp);
            // ADR-137 Engine I: the tray icon is a live dropzone — dragging
            // files onto it sends them to the connected peer via the loopback
            // drag REST chain (intent → confirm → stream → MOTW settle).
            attach_drop_view(button, ui_port, mtm);
        }

        // §6.3 tray takeover: run-loop timer polls the shared visibility
        // flag on the main thread and folds it into the status item (see
        // `trayVisibilityTick:`).
        unsafe {
            let timer =
                objc2_foundation::NSTimer::timerWithTimeInterval_target_selector_userInfo_repeats(
                    0.5,
                    &target,
                    sel!(trayVisibilityTick:),
                    None,
                    true,
                );
            objc2_foundation::NSRunLoop::currentRunLoop()
                .addTimer_forMode(&timer, objc2_foundation::NSRunLoopCommonModes);
            // §232: pre-warm the panel once the run loop is up (1.5 s keeps
            // it off the daemon's own startup path).
            let warm =
                objc2_foundation::NSTimer::timerWithTimeInterval_target_selector_userInfo_repeats(
                    1.5,
                    &target,
                    sel!(prewarmPanel:),
                    None,
                    false,
                );
            objc2_foundation::NSRunLoop::currentRunLoop()
                .addTimer_forMode(&warm, objc2_foundation::NSRunLoopCommonModes);
        }

        // The status item and target must outlive `app.run()` (which never
        // returns); pin them explicitly rather than relying on drop order.
        std::mem::forget(item);
        std::mem::forget(target);

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
pub fn spawn_tray(
    _ui_port: u16,
    _device_alias: String,
    _data_dir: std::path::PathBuf,
) -> Option<TrayHandle> {
    None
}

/// Windows left-click panel (§226/§227): WebView2 host for the SPA `#tray`
/// route, run in a user-context child process (`tetherly tray-panel`) because
/// WebView2 refuses to start under the LocalSystem daemon. A sibling of
/// `windows_tray` rather than a child so the `#[path]` resolves next to this
/// file; it reaches the tray's shared items via `super::windows_tray`.
#[cfg(windows)]
#[path = "tray_win_panel.rs"]
mod windows_tray_panel;

/// Entry point of the `tetherly tray-panel` child (§227).
#[cfg(windows)]
pub use windows_tray_panel::run_tray_panel_process;

#[cfg(windows)]
mod windows_tray {
    use super::windows_tray_panel::launcher as panel;
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use std::sync::atomic::{AtomicBool, AtomicU16, AtomicUsize, Ordering};
    use windows_sys::Win32::Foundation::{ERROR_SUCCESS, HWND, LPARAM, LRESULT, POINT, WPARAM};
    use windows_sys::Win32::Graphics::Gdi::{
        CreateBitmap, CreateDIBSection, DeleteObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
        DIB_RGB_COLORS,
    };
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, HKEY_USERS,
        KEY_READ, REG_DWORD,
    };
    use windows_sys::Win32::UI::HiDpi::{
        SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
    };
    use windows_sys::Win32::UI::Shell::{
        Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY,
        NOTIFYICONDATAW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        AppendMenuW, CreateIconIndirect, CreatePopupMenu, CreateWindowExW, DefWindowProcW,
        DestroyIcon, DestroyMenu, DestroyWindow, DispatchMessageW, GetCursorPos, GetMessageW,
        GetSystemMetrics, LoadIconW, PostQuitMessage, RegisterClassExW, SetForegroundWindow,
        SetTimer, TrackPopupMenuEx, TranslateMessage, HICON, HMENU, ICONINFO, IDI_APPLICATION,
        MF_GRAYED, MF_SEPARATOR, MF_STRING, MSG, SM_CXSMICON, TPM_RIGHTBUTTON, WM_APP, WM_CLOSE,
        WM_COMMAND, WM_DESTROY, WM_LBUTTONDBLCLK, WM_LBUTTONUP, WM_RBUTTONUP, WM_SETTINGCHANGE,
        WM_TIMER, WNDCLASSEXW,
    };

    const WM_TRAYICON: u32 = WM_APP + 42;
    const IDM_STATUS: usize = 1001;
    const IDM_DASHBOARD: usize = 1002;
    const IDM_ROAMING: usize = 1003;
    const IDM_EXIT: usize = 1004;
    const IDM_PANEL: usize = 1005;
    /// The notification-area icon's id (matches `nid.uID` below).
    pub(super) const TRAY_ICON_ID: u32 = 100;

    /// Panel geometry in logical (96-dpi) pixels — the macOS panel's numbers,
    /// so the SPA tray route lays out identically on both desktops.
    pub(super) const PANEL_W: f64 = 380.0;
    pub(super) const PANEL_H: f64 = 468.0;
    pub(super) const PANEL_MIN_H: f64 = 200.0;
    pub(super) const PANEL_MAX_H: f64 = 640.0;
    /// A tray click while the panel is open deactivates (hides) it before
    /// the click is delivered; a toggle inside this window stays a close.
    pub(super) const RETOGGLE_GUARD_MS: u64 = 300;
    /// Visibility-poll timer id (§6.3 tray takeover) and cadence.
    const VIS_TIMER_ID: usize = 0xC0FF_EE02;
    const VIS_POLL_MS: u32 = 500;

    pub(super) fn to_wide(s: &str) -> Vec<u16> {
        OsStr::new(s).encode_wide().chain(Some(0)).collect()
    }

    /// Open the Tauri client (deep link) or, failing that, the browser
    /// console — ADR-139 M4, with the §146 per-port token.
    fn open_main(port: u16) {
        crate::apitoken::open_client_or_dashboard(
            port,
            tetherly_loopback::api_token_for(port).as_deref(),
        );
    }

    /// The mac47054639 brand mark as a monochrome template `HICON` at the
    /// shell's small-icon size — the Windows twin of the macOS
    /// `isTemplate` status item (§227). Each embedded
    /// `assets/brand/tray-mono-{16,20,24,32,48}.a8` is the glyph's coverage
    /// (alpha only, 1 byte/pixel) rasterised for its own size, so the nearest
    /// asset at or above `SM_CXSMICON` is crisp instead of a blurred
    /// thumbnail. The ink is white on a dark taskbar and black on a light one
    /// (`light_taskbar`), like every system tray glyph. GDI gets a top-down
    /// 32bpp DIB section holding premultiplied BGRA (`CreateIconIndirect`
    /// honours the alpha channel) plus an all-zero 1bpp mask. Falls back to
    /// the stock application icon if GDI refuses, so the tray never vanishes
    /// over a cosmetic failure.
    pub(super) fn brand_icon(light_taskbar: bool) -> HICON {
        const ASSETS: [(usize, &[u8]); 5] = [
            (
                16,
                include_bytes!("../../../../assets/brand/tray-mono-16.a8"),
            ),
            (
                20,
                include_bytes!("../../../../assets/brand/tray-mono-20.a8"),
            ),
            (
                24,
                include_bytes!("../../../../assets/brand/tray-mono-24.a8"),
            ),
            (
                32,
                include_bytes!("../../../../assets/brand/tray-mono-32.a8"),
            ),
            (
                48,
                include_bytes!("../../../../assets/brand/tray-mono-48.a8"),
            ),
        ];
        let fallback = || unsafe { LoadIconW(std::ptr::null_mut(), IDI_APPLICATION) };

        let want = unsafe { GetSystemMetrics(SM_CXSMICON) }.max(16) as usize;
        let (side, alpha) = ASSETS
            .iter()
            .copied()
            .find(|(s, _)| *s >= want)
            .unwrap_or(ASSETS[ASSETS.len() - 1]);
        if alpha.len() != side * side {
            return fallback();
        }

        let mut info: BITMAPINFO = unsafe { std::mem::zeroed() };
        info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        info.bmiHeader.biWidth = side as i32;
        // Negative height = top-down rows, matching the asset layout.
        info.bmiHeader.biHeight = -(side as i32);
        info.bmiHeader.biPlanes = 1;
        info.bmiHeader.biBitCount = 32;
        info.bmiHeader.biCompression = BI_RGB;
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let color = unsafe {
            CreateDIBSection(
                std::ptr::null_mut(),
                &info,
                DIB_RGB_COLORS,
                &mut bits,
                std::ptr::null_mut(),
                0,
            )
        };
        if color.is_null() || bits.is_null() {
            return fallback();
        }
        // SAFETY: the section was just allocated for `side*side` 32bpp pixels
        // and is owned by this thread until `DeleteObject` below.
        let dst = unsafe { std::slice::from_raw_parts_mut(bits as *mut u8, side * side * 4) };
        // Premultiplied BGRA: for white ink every channel equals the
        // coverage; for black ink the colour channels stay zero.
        let ink = if light_taskbar { 0u8 } else { 255u8 };
        // `chunks_exact` rather than `as_chunks`: the workspace MSRV is 1.82.
        for (d, &a) in dst.chunks_exact_mut(4).zip(alpha.iter()) {
            let c = ((ink as u32 * a as u32 + 127) / 255) as u8;
            d[0] = c;
            d[1] = c;
            d[2] = c;
            d[3] = a;
        }

        // 1bpp mask, rows padded to a WORD; all zero = colour bitmap decides.
        let stride = side.div_ceil(16) * 2;
        let zeros = vec![0u8; stride * side];
        let mask = unsafe { CreateBitmap(side as i32, side as i32, 1, 1, zeros.as_ptr().cast()) };
        if mask.is_null() {
            unsafe {
                DeleteObject(color);
            }
            return fallback();
        }

        let mut icon_info: ICONINFO = unsafe { std::mem::zeroed() };
        icon_info.fIcon = 1;
        icon_info.hbmMask = mask;
        icon_info.hbmColor = color;
        let icon = unsafe { CreateIconIndirect(&icon_info) };
        unsafe {
            DeleteObject(mask);
            DeleteObject(color);
        }
        if icon.is_null() {
            fallback()
        } else {
            icon
        }
    }

    /// `SystemUsesLightTheme` under `…\Themes\Personalize` — the taskbar
    /// (not the apps) theme, which is what decides the tray glyph's ink.
    /// Read from `HKCU` for a user-context daemon; a LocalSystem daemon has
    /// no meaningful HKCU, so it reads the console user's hive under
    /// `HKEY_USERS\<SID>` instead. Anything unreadable counts as dark (the
    /// Windows default and what every production host here runs).
    pub(super) fn taskbar_is_light() -> bool {
        const SUBKEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize";
        let (root, path): (HKEY, String) = if crate::cred::process_runs_as_local_system() {
            match crate::cred::console_user_sid() {
                Some(sid) => (HKEY_USERS, format!(r"{sid}\{SUBKEY}")),
                None => return false,
            }
        } else {
            (HKEY_CURRENT_USER, SUBKEY.to_string())
        };
        let path_w = to_wide(&path);
        let value_w = to_wide("SystemUsesLightTheme");
        let mut key: HKEY = std::ptr::null_mut();
        if unsafe { RegOpenKeyExW(root, path_w.as_ptr(), 0, KEY_READ, &mut key) } != ERROR_SUCCESS {
            return false;
        }
        let mut kind: u32 = 0;
        let mut data: u32 = 0;
        let mut len: u32 = std::mem::size_of::<u32>() as u32;
        let rc = unsafe {
            RegQueryValueExW(
                key,
                value_w.as_ptr(),
                std::ptr::null_mut(),
                &mut kind,
                (&mut data as *mut u32).cast(),
                &mut len,
            )
        };
        unsafe {
            RegCloseKey(key);
        }
        rc == ERROR_SUCCESS && kind == REG_DWORD && data != 0
    }

    /// The icon currently installed in the notification area and the theme
    /// it was drawn for, so `WM_SETTINGCHANGE` can swap ink without a redraw
    /// storm (only when the taskbar theme actually flipped).
    static CURRENT_ICON: AtomicUsize = AtomicUsize::new(0);
    static CURRENT_LIGHT: AtomicBool = AtomicBool::new(false);

    /// Re-read the taskbar theme and, if it flipped, rebuild the glyph and
    /// hand it to the shell (`NIM_MODIFY`, icon only — the tip and callback
    /// stay). Called on the tray thread from `WM_SETTINGCHANGE`.
    unsafe fn refresh_icon_for_theme(hwnd: HWND) {
        let light = taskbar_is_light();
        if CURRENT_LIGHT.swap(light, Ordering::SeqCst) == light {
            return;
        }
        let icon = brand_icon(light);
        let mut nid = std::mem::zeroed::<NOTIFYICONDATAW>();
        nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = hwnd;
        nid.uID = TRAY_ICON_ID;
        nid.uFlags = NIF_ICON;
        nid.hIcon = icon;
        Shell_NotifyIconW(NIM_MODIFY, &nid);
        let old = CURRENT_ICON.swap(icon as usize, Ordering::SeqCst) as HICON;
        // The shell copies the icon on NIM_ADD/NIM_MODIFY, and the hidden
        // tray window never paints its class icon, so the old handle is
        // free to go.
        if !old.is_null() {
            DestroyIcon(old);
        }
        tracing::info!(light, "tray icon re-inked for the taskbar theme");
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
                let port = GLOBAL_UI_PORT.load(Ordering::Relaxed);
                if event == WM_LBUTTONUP {
                    // §226/§227: left click toggles the WebView2 panel (the
                    // SPA `#tray` route, like the macOS NSPanel), hosted by a
                    // user-context child process. If no child can be spawned
                    // it degrades to the ADR-139 M4 path: Tauri client deep
                    // link, else the browser console.
                    if !panel::toggle(hwnd, port) {
                        open_main(port);
                    }
                } else if event == WM_LBUTTONDBLCLK {
                    panel::hide();
                    open_main(port);
                } else if event == WM_RBUTTONUP {
                    panel::hide();
                    let mut pt = POINT { x: 0, y: 0 };
                    GetCursorPos(&mut pt);
                    SetForegroundWindow(hwnd);

                    let menu: HMENU = CreatePopupMenu();
                    let status_str = to_wide("Tetherly · 运行中");
                    let panel_str = to_wide("打开面板");
                    let dash_str = to_wide(&format!("打开控制台 (127.0.0.1:{port})"));
                    let roam_str = to_wide("键鼠漫游：活跃");
                    let exit_str = to_wide("退出 Tetherly");

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
                let port = GLOBAL_UI_PORT.load(Ordering::Relaxed);
                if id == IDM_DASHBOARD {
                    open_main(port);
                } else if id == IDM_PANEL {
                    if !panel::toggle(hwnd, port) {
                        open_main(port);
                    }
                } else if id == IDM_EXIT {
                    std::process::exit(0);
                }
                0
            }
            WM_CLOSE => {
                DestroyWindow(hwnd);
                0
            }
            // §227: the taskbar theme flipped (Settings → Colours); re-ink
            // the monochrome glyph. Any WM_SETTINGCHANGE is cheap to check.
            WM_SETTINGCHANGE => {
                refresh_icon_for_theme(hwnd);
                0
            }
            // §6.3 tray takeover: poll the shared visibility flag and add /
            // delete the notification-area icon accordingly.
            WM_TIMER if wparam as u32 as usize == VIS_TIMER_ID => {
                let want = super::tray_visible();
                static SHOWN: AtomicBool = AtomicBool::new(true);
                if SHOWN.swap(want, Ordering::SeqCst) != want {
                    let mut nid = std::mem::zeroed::<NOTIFYICONDATAW>();
                    nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
                    nid.hWnd = hwnd;
                    nid.uID = TRAY_ICON_ID;
                    if want {
                        nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
                        nid.uCallbackMessage = WM_TRAYICON;
                        nid.hIcon = CURRENT_ICON.load(Ordering::SeqCst) as HICON;
                        let tip = to_wide("Tetherly · 运行中");
                        let len = tip.len().min(nid.szTip.len() - 1);
                        nid.szTip[..len].copy_from_slice(&tip[..len]);
                        Shell_NotifyIconW(NIM_ADD, &nid);
                    } else {
                        Shell_NotifyIconW(NIM_DELETE, &nid);
                    }
                }
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
            // §226: the panel sizes itself from `GetDpiForWindow`, which only
            // reports the real monitor DPI on a per-monitor-aware thread. The
            // daemon exe carries no DPI manifest, so opt this thread in.
            SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);

            let class_name = to_wide("TetherlyTrayClass");
            let light = taskbar_is_light();
            let icon: HICON = brand_icon(light);
            CURRENT_LIGHT.store(light, Ordering::SeqCst);
            CURRENT_ICON.store(icon as usize, Ordering::SeqCst);
            tracing::info!(light, "tray icon inked for the taskbar theme");

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
            nid.uID = TRAY_ICON_ID;
            nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
            nid.uCallbackMessage = WM_TRAYICON;
            nid.hIcon = CURRENT_ICON.load(Ordering::SeqCst) as HICON;

            let tip = to_wide("Tetherly · 运行中");
            let len = tip.len().min(nid.szTip.len() - 1);
            nid.szTip[..len].copy_from_slice(&tip[..len]);

            Shell_NotifyIconW(NIM_ADD, &nid);

            // §6.3 tray takeover: poll the shared visibility flag so a
            // client claim / watchdog reclaim swaps the icon within ~0.5s.
            SetTimer(hwnd, VIS_TIMER_ID, VIS_POLL_MS, None);

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

/// `data_dir` is forwarded to the §227 panel child (`--data-dir`) so it logs
/// beside the daemon and can read the token file if the env var is lost.
#[cfg(windows)]
pub fn spawn_tray(
    ui_port: u16,
    _device_alias: String,
    data_dir: std::path::PathBuf,
) -> Option<TrayHandle> {
    windows_tray_panel::launcher::set_data_dir(data_dir);
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
pub fn spawn_tray(
    _ui_port: u16,
    _device_alias: String,
    _data_dir: std::path::PathBuf,
) -> Option<TrayHandle> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tray_spawn_and_stop() {
        let handle = spawn_tray(45716, "TEST-PEER".into(), std::env::temp_dir());
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
