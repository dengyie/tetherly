// SPDX-License-Identifier: Apache-2.0 OR MIT
//! EasyTier overlay sidecar status and lifecycle.

use super::*;
use crate::events::UiEvent;
use crate::lan::bind_overlay;
use crate::sidecar::{self, OverlayStatus};
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;
use tetherly_core::OVERLAY_PROBE_MS;
use tracing::{debug, info};

impl Node {
    pub fn overlay_enabled(&self) -> bool {
        self.shared.cfg.overlay.enabled
    }

    pub fn overlay_status(&self) -> OverlayStatus {
        self.shared.overlay.lock().expect("overlay").clone()
    }

    pub(super) fn spawn_overlay(&self) {
        if !self.shared.cfg.loopback_only {
            let overlay_addrs =
                sidecar::overlay_bind_addrs(&self.shared.cfg.overlay, self.shared.cfg.control_port);
            let overlay_file =
                sidecar::overlay_bind_addrs(&self.shared.cfg.overlay, self.shared.cfg.file_port);
            let overlay_input =
                sidecar::overlay_bind_addrs(&self.shared.cfg.overlay, self.shared.cfg.input_port);
            let overlay_screen =
                sidecar::overlay_bind_addrs(&self.shared.cfg.overlay, self.shared.cfg.screen_port);
            let node = self.clone_handle();
            tokio::spawn(async move {
                for l in bind_overlay(&overlay_addrs).await {
                    node_accept_loop(
                        node.clone_handle(),
                        l,
                        "overlay control",
                        |n, s, a| async move { n.accept_one(s, a).await },
                    );
                }
                for l in bind_overlay(&overlay_file).await {
                    node_accept_loop(
                        node.clone_handle(),
                        l,
                        "overlay file",
                        |n, s, a| async move { n.accept_file(s, a).await },
                    );
                }
                for l in bind_overlay(&overlay_input).await {
                    node_accept_loop(
                        node.clone_handle(),
                        l,
                        "overlay input",
                        |n, s, a| async move { n.accept_input(s, a).await },
                    );
                }
                for l in bind_overlay(&overlay_screen).await {
                    node_accept_loop(
                        node.clone_handle(),
                        l,
                        "overlay screen",
                        |n, s, a| async move { n.accept_screen(s, a).await },
                    );
                }
            });
        }
        let node = self.clone_handle();
        tokio::spawn(async move {
            node.overlay_scan_loop().await;
        });
    }

    pub(super) async fn overlay_scan_loop(&self) {
        let mut stop = self.shared.shutdown.subscribe();
        let mut last_present = false;
        loop {
            if *stop.borrow() {
                break;
            }
            let st = sidecar::discover(&self.shared.cfg.overlay).await;
            let present = st.present;
            {
                *self.shared.overlay.lock().expect("overlay") = st.clone();
            }
            if present != last_present {
                last_present = present;
                self.emit(UiEvent::Overlay {
                    present,
                    lan_only: !present,
                });
                self.emit(UiEvent::OverlayStateChanged {
                    present,
                    source: format!("{:?}", st.source),
                    local_ips: st.local_ips.iter().map(|ip| ip.to_string()).collect(),
                    peer_count: st.peers.len(),
                });
                if !present {
                    info!("overlay down; LAN only");
                } else {
                    info!(peers = st.peers.len(), "overlay peers");
                }
            }
            let live = self.live_peers();
            let live_ips: Vec<IpAddr> = live.iter().map(|p| p.addr.ip()).collect();
            let port = self.control_port();
            let mut targets: Vec<SocketAddr> = self.shared.cfg.overlay.extra_peers.clone();
            for ip in &st.peers {
                if live_ips.contains(&IpAddr::V4(*ip)) {
                    continue;
                }
                targets.push(SocketAddr::from((*ip, port)));
            }
            for addr in targets {
                if self.already_live_on_ip(addr) {
                    continue;
                }
                if let Err(e) = self.dial(addr).await {
                    debug!(%e, %addr, "overlay dial");
                }
            }
            tokio::select! {
                _ = stop.changed() => {
                    if *stop.borrow() {
                        break;
                    }
                }
                _ = tokio::time::sleep(Duration::from_millis(OVERLAY_PROBE_MS.max(5_000))) => {}
            }
        }
    }
}
