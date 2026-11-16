// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Node identity, runtime config mutation and config reports.

use super::*;
use crate::events::UiEvent;
use std::sync::atomic::Ordering;
use tetherly_core::CoreError;

impl Node {
    pub fn name(&self) -> String {
        self.shared.node_name.lock().expect("node_name").clone()
    }

    pub fn platform(&self) -> &str {
        &self.shared.cfg.platform
    }

    pub fn set_node_name(&self, name: &str) -> Result<String, CoreError> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err(CoreError::InvalidConfig(
                "node name cannot be empty".to_string(),
            ));
        }
        if trimmed.len() > 64 {
            return Err(CoreError::InvalidConfig(
                "node name exceeds maximum length of 64 bytes".to_string(),
            ));
        }
        if trimmed.chars().any(|c| c.is_control()) {
            return Err(CoreError::InvalidConfig(
                "node name contains disallowed control characters".to_string(),
            ));
        }
        {
            let mut n = self.shared.node_name.lock().expect("node_name");
            *n = trimmed.to_string();
        }
        if self.is_advertise_enabled() {
            if let Some(tx) = self
                .shared
                .advertise_stop
                .lock()
                .expect("advertise_stop")
                .take()
            {
                let _ = tx.send(true);
            }
            self.advertise_mdns();
        }
        self.emit_config_changed();
        Ok(trimmed.to_string())
    }

    pub fn update_config(
        &self,
        name: Option<&str>,
        advertise: Option<bool>,
        reconnect: Option<bool>,
    ) -> Result<NodeConfigReport, CoreError> {
        if let Some(n) = name {
            let trimmed = n.trim();
            if trimmed.is_empty() {
                return Err(CoreError::InvalidConfig(
                    "node name cannot be empty".to_string(),
                ));
            }
            if trimmed.len() > 64 {
                return Err(CoreError::InvalidConfig(
                    "node name exceeds maximum length of 64 bytes".to_string(),
                ));
            }
            if trimmed.chars().any(|c| c.is_control()) {
                return Err(CoreError::InvalidConfig(
                    "node name contains disallowed control characters".to_string(),
                ));
            }
            *self.shared.node_name.lock().expect("node_name") = trimmed.to_string();
        }
        if let Some(adv) = advertise {
            let prev = self.shared.advertise.swap(adv, Ordering::SeqCst);
            if prev != adv {
                if adv {
                    self.advertise_mdns();
                } else if let Some(tx) = self
                    .shared
                    .advertise_stop
                    .lock()
                    .expect("advertise_stop")
                    .take()
                {
                    let _ = tx.send(true);
                }
            } else if adv && name.is_some() {
                if let Some(tx) = self
                    .shared
                    .advertise_stop
                    .lock()
                    .expect("advertise_stop")
                    .take()
                {
                    let _ = tx.send(true);
                }
                self.advertise_mdns();
            }
        } else if name.is_some() && self.is_advertise_enabled() {
            if let Some(tx) = self
                .shared
                .advertise_stop
                .lock()
                .expect("advertise_stop")
                .take()
            {
                let _ = tx.send(true);
            }
            self.advertise_mdns();
        }
        if let Some(rec) = reconnect {
            self.shared.reconnect.store(rec, Ordering::SeqCst);
        }
        self.emit_config_changed();
        Ok(self.config_report())
    }

    pub fn config_report(&self) -> NodeConfigReport {
        let bound_ctrl = self.shared.bound_control.load(Ordering::SeqCst);
        let bound_file = self.shared.bound_file.load(Ordering::SeqCst);
        let bound_input = self.shared.bound_input.load(Ordering::SeqCst);
        let bound_screen = self.shared.bound_screen.load(Ordering::SeqCst);
        let bound_ui = self.shared.bound_ui.load(Ordering::SeqCst);

        let ctrl_port = if bound_ctrl != 0 {
            bound_ctrl
        } else {
            self.shared.cfg.control_port
        };
        let file_port = if bound_file != 0 {
            bound_file
        } else {
            self.shared.cfg.file_port
        };
        let input_port = if bound_input != 0 {
            bound_input
        } else {
            self.shared.cfg.input_port
        };
        let screen_port = if bound_screen != 0 {
            bound_screen
        } else {
            self.shared.cfg.screen_port
        };
        let ui_port = if bound_ui != 0 {
            bound_ui
        } else {
            self.shared.cfg.ui_port
        };

        let ancs_attached = self.shared.ancs.lock().expect("ancs").is_some();

        NodeConfigReport {
            ok: true,
            device_id: self.shared.identity.device_id().to_string(),
            name: self.name(),
            platform: self.platform().to_string(),
            data_dir: self.shared.cfg.data_dir.display().to_string(),
            loopback_only: self.shared.cfg.loopback_only,
            advertise: self.is_advertise_enabled(),
            reconnect: self.is_reconnect_enabled(),
            memory_clip: self.shared.cfg.memory_clip,
            memory_notifier: self.shared.cfg.memory_notifier,
            ui_port,
            control_port: ctrl_port,
            file_port,
            input_port,
            screen_port,
            overlay_enabled: self.shared.cfg.overlay.enabled,
            ancs_attached,
            ancs_peripheral: self.shared.cfg.ancs_peripheral.clone(),
        }
    }

    pub(super) fn emit_config_changed(&self) {
        let name = self.name();
        let advertise = self.is_advertise_enabled();
        let reconnect = self.is_reconnect_enabled();
        self.emit(UiEvent::ConfigChanged {
            name,
            advertise,
            reconnect,
        });
    }

    /// Current tray owner (§6.3).
    pub fn tray_owner(&self) -> tetherly_api::TrayOwner {
        *self.shared.tray_owner.lock().expect("tray_owner")
    }

    /// Claim or release the tray surface (§6.3). A `client` claim hides the
    /// built-in tray and starts the keepalive window; a `daemon` value
    /// restores the built-in tray immediately. Returns the new owner and
    /// whether the visible state actually changed (idempotent claims are a
    /// no-op). `emit` mirrors `update_config`'s settings.changed push so
    /// every subscriber sees the new `tray_owner`.
    pub fn set_tray_owner(
        &self,
        owner: tetherly_api::TrayOwner,
    ) -> Result<(tetherly_api::TrayOwner, bool), tetherly_core::CoreError> {
        let mut slot = self.shared.tray_owner.lock().expect("tray_owner");
        if *slot == owner {
            return Ok((owner, false));
        }
        *slot = owner;
        match owner {
            // Fresh claim: the keepalive window starts now.
            tetherly_api::TrayOwner::Client => {
                *self.shared.tray_keepalive.lock().expect("tray_keepalive") =
                    Some(self.shared.clock.unix_ms());
            }
            // Release or reclaim: forget any stale keepalive.
            tetherly_api::TrayOwner::Daemon => {
                *self.shared.tray_keepalive.lock().expect("tray_keepalive") = None;
                self.shared.tray_sse_seen.store(false, Ordering::Release);
            }
        }
        drop(slot);
        self.emit_config_changed();
        Ok((owner, true))
    }

    /// The shell (or the v1 SSE layer) feeds this while it holds the tray:
    /// any activity restarts the reclaim window.
    pub fn tray_keepalive(&self) {
        if self.tray_owner() == tetherly_api::TrayOwner::Client {
            *self.shared.tray_keepalive.lock().expect("tray_keepalive") =
                Some(self.shared.clock.unix_ms());
        }
    }

    /// Record that at least one v1 SSE subscriber connected. The daemon
    /// binary's tray watchdog reads this to distinguish "shell never showed
    /// up after claiming" from "shell went silent mid-run".
    pub fn mark_tray_sse_seen(&self) {
        self.shared.tray_sse_seen.store(true, Ordering::Release);
    }

    /// Whether the claimed tray's keepalive window has expired: the claim
    /// (or last keepalive) is older than `TRAY_TAKEOVER_RECLAIM_MS` while
    /// `tray_owner == client`. `false` while unclaimed.
    pub fn tray_keepalive_expired(&self) -> bool {
        if self.tray_owner() != tetherly_api::TrayOwner::Client {
            return false;
        }
        let now = self.shared.clock.unix_ms();
        self.shared
            .tray_keepalive
            .lock()
            .expect("tray_keepalive")
            .is_some_and(|last_ms| {
                now.saturating_sub(last_ms) >= tetherly_api::TRAY_TAKEOVER_RECLAIM_MS
            })
    }

    pub fn lan_only(&self) -> bool {
        !self.overlay_status().present
    }
}
