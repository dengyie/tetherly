// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Peer trust, PIN pairing, connect/disconnect, block/unblock.

use super::*;
use crate::events::UiEvent;
use crate::store::save_trust;
use std::net::SocketAddr;
use std::sync::atomic::Ordering;
use tetherly_core::pin::{format_pin, generate_pin};
use tetherly_core::session::PIN_TTL;
use tetherly_core::TrustStore;
use tetherly_core::{CoreError, CursorSeat, DeviceId, PathKind};
use tetherly_crypto::Identity;
use tetherly_net::session::{SessionConfig, HANDSHAKE_TIMEOUT};
use tokio::sync::broadcast;
use tracing::warn;

#[derive(Clone)]
pub struct LivePeer {
    pub device_id: DeviceId,
    pub name: String,
    pub platform: String,
    pub addr: SocketAddr,
    pub handshake_hash: [u8; 32],
    /// Advertised via caps.update `fileport=`. None until the peer tells us.
    pub file_port: Option<u16>,
    /// Advertised via caps.update `inputport=`. None until the peer tells us.
    pub input_port: Option<u16>,
    /// Advertised via caps.update `screenport=`. None until the peer tells us.
    pub screen_port: Option<u16>,
    /// Advertised capability tokens from the peer.
    pub caps: Vec<String>,
    pub path: PathKind,
    pub rtt_ms: Option<u64>,
    pub last_seen_ms: u64,
}

/// Status report for the node's currently armed pairing PIN and its lifecycle.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct PinReport {
    pub ok: bool,
    pub active: bool,
    pub pin: Option<String>,
    pub expires_in_secs: u64,
    pub ttl_secs: u64,
    pub created_at_ms: u64,
}

/// Report returned when actively connecting to a trusted peer on demand.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct PeerConnectReport {
    pub ok: bool,
    pub device_id: String,
    pub name: Option<String>,
    pub alias: Option<String>,
    pub addr: Option<String>,
    pub connected: bool,
    pub already_connected: bool,
}

/// Report returned when actively disconnecting a live peer session on demand.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct PeerDisconnectReport {
    pub ok: bool,
    pub device_id: String,
    pub name: Option<String>,
    pub alias: Option<String>,
    pub disconnected: bool,
    pub already_disconnected: bool,
}

/// Report returned when blocking a peer device.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct PeerBlockReport {
    pub ok: bool,
    pub device_id: String,
    pub name: Option<String>,
    pub alias: Option<String>,
    pub reason: Option<String>,
    pub blocked: bool,
    pub already_blocked: bool,
    pub disconnected_active: bool,
}

/// Report returned when unblocking a peer device.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct PeerUnblockReport {
    pub ok: bool,
    pub device_id: String,
    pub unblocked: bool,
    pub was_blocked: bool,
}

/// Summary report of all blocked peer devices.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct BlockedPeersReport {
    pub ok: bool,
    pub count: usize,
    pub peers: Vec<crate::store::BlockedPeer>,
}

impl Node {
    pub fn identity(&self) -> &Identity {
        &self.shared.identity
    }

    pub fn subscribe(&self) -> broadcast::Receiver<UiEvent> {
        self.shared.events.subscribe()
    }

    pub fn session_config(&self) -> SessionConfig {
        self.session_config_inner(false)
    }

    pub(super) fn session_config_inner(&self, resume_only: bool) -> SessionConfig {
        let pin = *self.shared.pin.lock().expect("pin");
        SessionConfig {
            identity: self.shared.identity.clone(),
            name: self.name(),
            platform: self.shared.cfg.platform.clone(),
            trust: self.shared.trust.clone(),
            lockout: self.shared.lockout.clone(),
            tcp_limiter: self.shared.tcp_limiter.clone(),
            pin: pin.map(|(p, _)| p),
            pin_created_ms: pin.map(|(_, t)| t).unwrap_or(0),
            clock: self.shared.clock.clone(),
            handshake_timeout: HANDSHAKE_TIMEOUT,
            hello_override: None,
            resume_only,
            screen_only: false,
            caps: None,
        }
    }

    pub fn persist_trust(&self) {
        let store = self.shared.trust.lock().expect("trust");
        if let Err(e) = save_trust(&self.shared.cfg.data_dir, &store) {
            warn!(%e, "trust persist failed");
        }
    }

    pub fn generate_pin(&self) -> Result<String, CoreError> {
        let pin = generate_pin(&OsRng)?;
        let now = self.shared.clock.unix_ms();
        *self.shared.pin.lock().expect("pin") = Some((pin, now));
        let _ = self
            .shared
            .events
            .send(UiEvent::PinStateChanged { active: true });
        self.record_audit(
            AuditCategory::Security,
            AuditSeverity::Info,
            "pin_generated",
            None::<String>,
            "new pairing PIN generated for incoming connection",
            true,
        );
        Ok(format_pin(&pin))
    }

    pub fn set_pin(&self, digits: [u8; 8]) {
        *self.shared.pin.lock().expect("pin") = Some((digits, self.shared.clock.unix_ms()));
        let _ = self
            .shared
            .events
            .send(UiEvent::PinStateChanged { active: true });
        self.record_audit(
            AuditCategory::Security,
            AuditSeverity::Info,
            "pin_set",
            None::<String>,
            "pairing PIN set by runtime",
            true,
        );
    }

    pub fn pin_if_fresh(&self) -> Option<String> {
        let guard = self.shared.pin.lock().expect("pin");
        let (pin, created) = guard.as_ref()?;
        let age = self.shared.clock.unix_ms().saturating_sub(*created);
        if age > PIN_TTL.as_millis() as u64 {
            return None;
        }
        Some(format_pin(pin))
    }

    pub fn pin_report(&self) -> PinReport {
        let ttl_secs = PIN_TTL.as_secs();
        let ttl_ms = PIN_TTL.as_millis() as u64;
        let guard = self.shared.pin.lock().expect("pin");
        match guard.as_ref() {
            Some((digits, created)) => {
                let age = self.shared.clock.unix_ms().saturating_sub(*created);
                if age <= ttl_ms {
                    let rem_ms = ttl_ms.saturating_sub(age);
                    let rem_secs = rem_ms / 1000;
                    PinReport {
                        ok: true,
                        active: true,
                        pin: Some(format_pin(digits)),
                        expires_in_secs: rem_secs,
                        ttl_secs,
                        created_at_ms: *created,
                    }
                } else {
                    PinReport {
                        ok: true,
                        active: false,
                        pin: None,
                        expires_in_secs: 0,
                        ttl_secs,
                        created_at_ms: *created,
                    }
                }
            }
            None => PinReport {
                ok: true,
                active: false,
                pin: None,
                expires_in_secs: 0,
                ttl_secs,
                created_at_ms: 0,
            },
        }
    }

    pub fn clear_pin(&self) -> bool {
        let mut guard = self.shared.pin.lock().expect("pin");
        let was_active = if let Some((_, created)) = guard.as_ref() {
            let age = self.shared.clock.unix_ms().saturating_sub(*created);
            age <= PIN_TTL.as_millis() as u64
        } else {
            false
        };
        *guard = None;
        drop(guard);
        if was_active {
            let _ = self
                .shared
                .events
                .send(UiEvent::PinStateChanged { active: false });
        }
        was_active
    }

    pub fn is_advertise_enabled(&self) -> bool {
        self.shared.advertise.load(Ordering::SeqCst)
    }

    pub fn set_advertise_enabled(&self, enabled: bool) -> bool {
        let prev = self.shared.advertise.swap(enabled, Ordering::SeqCst);
        if prev != enabled {
            if enabled {
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
            self.emit_config_changed();
        }
        enabled
    }

    pub fn is_reconnect_enabled(&self) -> bool {
        self.shared.reconnect.load(Ordering::SeqCst)
    }

    pub fn set_reconnect_enabled(&self, enabled: bool) -> bool {
        let prev = self.shared.reconnect.swap(enabled, Ordering::SeqCst);
        if prev != enabled {
            self.emit_config_changed();
        }
        enabled
    }

    pub fn trusted(&self) -> Vec<tetherly_core::TrustedPeer> {
        self.shared.trust.lock().expect("trust").all()
    }

    pub fn is_trusted(&self, id: &DeviceId) -> bool {
        let store = self.shared.trust.lock().expect("trust");
        store.peers.get(id).is_some_and(|p| !p.revoked)
    }

    /// Resolve a user-supplied peer query string (device_id, alias, or peer name)
    /// into a concrete DeviceId.
    pub fn resolve_peer(&self, query: &str) -> Result<DeviceId, CoreError> {
        let q = query.trim();
        if q.is_empty() {
            return Err(CoreError::PeerNotFound("empty query".to_string()));
        }

        // 1. Direct exact DeviceId parse
        if let Ok(id) = DeviceId::parse(q) {
            return Ok(id);
        }

        let live = self.live_peers();
        let trusted = self.trusted();
        let q_lower = q.to_lowercase();
        let mut matches = Vec::new();

        // 2. Exact match on alias or name (case-insensitive)
        for t in &trusted {
            if t.alias.to_lowercase() == q_lower && !matches.contains(&t.device_id) {
                matches.push(t.device_id.clone());
            }
        }
        for l in &live {
            if l.name.to_lowercase() == q_lower && !matches.contains(&l.device_id) {
                matches.push(l.device_id.clone());
            }
        }

        // 3. Substring / prefix match if no exact match
        if matches.is_empty() {
            for t in &trusted {
                if (t.alias.to_lowercase().contains(&q_lower)
                    || t.device_id.as_str().to_lowercase().contains(&q_lower))
                    && !matches.contains(&t.device_id)
                {
                    matches.push(t.device_id.clone());
                }
            }
            for l in &live {
                if (l.name.to_lowercase().contains(&q_lower)
                    || l.device_id.as_str().to_lowercase().contains(&q_lower))
                    && !matches.contains(&l.device_id)
                {
                    matches.push(l.device_id.clone());
                }
            }
        }

        match matches.len() {
            1 => Ok(matches.remove(0)),
            0 => Err(CoreError::PeerNotFound(format!(
                "no peer matching '{q}' found"
            ))),
            _ => {
                let candidate_list = matches
                    .iter()
                    .map(|id| id.to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                Err(CoreError::PeerNotFound(format!(
                    "multiple peers match '{q}': {candidate_list}"
                )))
            }
        }
    }

    pub fn forget(&self, device_id: &DeviceId) {
        let mut store = self.shared.trust.lock().expect("trust");
        let _ = store.forget(device_id);
        drop(store);
        self.persist_trust();
        self.shared
            .ping_waiters
            .lock()
            .expect("ping_waiters")
            .remove(device_id);
        // A forgotten device must not keep its live session, its capability
        // state, or (if it held input) the remote seat. Dropping the outbound
        // sender closes the channel, which ends the session task and its TCP.
        let had_session = match self.shared.sessions.try_lock() {
            Ok(mut sessions) => sessions.remove(device_id).is_some(),
            Err(_) => false,
        };
        let had_live = self
            .shared
            .live
            .lock()
            .expect("live")
            .remove(device_id)
            .is_some();
        if *self.shared.input_seat.lock().expect("seat") == Some(CursorSeat::Remote) {
            *self.shared.input_seat.lock().expect("seat") = Some(CursorSeat::Local);
        }
        {
            let mut sp = self.shared.screen_peer.lock().expect("screen_peer");
            if sp.as_ref() == Some(device_id) {
                *sp = None;
            }
        }
        {
            let mut ip = self.shared.input_peer.lock().expect("input_peer");
            if ip.as_ref() == Some(device_id) {
                *ip = None;
            }
        }
        if had_session || had_live {
            self.emit(UiEvent::PeerDown {
                device_id: device_id.to_string(),
            });
        }
    }

    /// Revoke trust and actively sever any connection with `device_id`.
    ///
    /// Purges the peer from the in-memory `TrustStore`, atomically re-persists
    /// `trusted.json`, forgets remembered routes in `routes.json`, terminates
    /// active session streaming by dropping its outbound sender, evicts the peer
    /// from live state, and emits `PeerDown`. Returns `true` if the peer was known
    /// and removed, `false` otherwise.
    pub async fn unpair(&self, device_id: &DeviceId) -> bool {
        let had_trust = {
            let mut store = self.shared.trust.lock().expect("trust");
            store.peers.remove(device_id).is_some()
        };
        self.persist_trust();
        let _ = crate::store::forget_route(&self.shared.cfg.data_dir, device_id.as_str());

        let had_session = self
            .shared
            .sessions
            .lock()
            .await
            .remove(device_id)
            .is_some();
        let had_live = self
            .shared
            .live
            .lock()
            .expect("live")
            .remove(device_id)
            .is_some();
        if *self.shared.input_seat.lock().expect("seat") == Some(CursorSeat::Remote) {
            *self.shared.input_seat.lock().expect("seat") = Some(CursorSeat::Local);
        }
        {
            let mut sp = self.shared.screen_peer.lock().expect("screen_peer");
            if sp.as_ref() == Some(device_id) {
                *sp = None;
            }
        }
        {
            let mut ip = self.shared.input_peer.lock().expect("input_peer");
            if ip.as_ref() == Some(device_id) {
                *ip = None;
            }
        }
        if had_session || had_live {
            self.emit(UiEvent::PeerDown {
                device_id: device_id.to_string(),
            });
        }
        if had_trust || had_session || had_live {
            self.record_audit(
                AuditCategory::Security,
                AuditSeverity::Notice,
                "peer_unpaired",
                Some(device_id.to_string()),
                format!("peer '{device_id}' trust revoked and connection terminated"),
                true,
            );
        }
        had_trust || had_session || had_live
    }

    /// Actively connect to a trusted peer (specified by device ID, alias, or name).
    ///
    /// If `direct_addr` is provided, parses and validates the socket address, remembers it
    /// in `routes.json`, and attempts to dial it. Otherwise looks up remembered routes.
    /// If the peer is already live, returns immediately with `already_connected: true`.
    pub async fn connect_peer(
        &self,
        query: &str,
        direct_addr: Option<&str>,
    ) -> Result<PeerConnectReport, CoreError> {
        let target_id = self.resolve_peer(query)?;
        let parsed_direct: Option<SocketAddr> = if let Some(raw_addr) = direct_addr {
            let parsed: SocketAddr = raw_addr.trim().parse().map_err(|e| {
                CoreError::InvalidRoute(format!("invalid socket address '{raw_addr}': {e}"))
            })?;
            if parsed.port() == 0 {
                return Err(CoreError::InvalidRoute(
                    "socket port must be non-zero".to_string(),
                ));
            }
            Some(parsed)
        } else {
            None
        };

        if self.is_blocked(target_id.as_str()) {
            return Err(CoreError::PeerBlocked(target_id.to_string()));
        }

        let trusted = self.trusted();
        let matched = trusted.iter().find(|t| t.device_id == target_id);
        let alias = matched.and_then(|t| {
            if t.alias.is_empty() {
                None
            } else {
                Some(t.alias.clone())
            }
        });

        // Check if already live
        {
            let live = self.shared.live.lock().expect("live");
            if let Some(lp) = live.get(&target_id) {
                return Ok(PeerConnectReport {
                    ok: true,
                    device_id: target_id.to_string(),
                    name: Some(lp.name.clone()),
                    alias,
                    addr: Some(lp.addr.to_string()),
                    connected: true,
                    already_connected: true,
                });
            }
        }

        let addr = if let Some(parsed) = parsed_direct {
            let _ = crate::store::remember_route(
                &self.shared.cfg.data_dir,
                target_id.as_str(),
                &parsed.to_string(),
            );
            parsed
        } else {
            let routes = crate::store::load_routes(&self.shared.cfg.data_dir);
            route_addr(&routes, target_id.as_str()).ok_or_else(|| {
                CoreError::InvalidRoute(format!("no known route for peer {}", target_id))
            })?
        };

        match self.dial(addr).await {
            Ok(()) => {
                let live_name = {
                    let live = self.shared.live.lock().expect("live");
                    live.get(&target_id).map(|lp| lp.name.clone())
                };
                let name = live_name.or_else(|| alias.clone());
                Ok(PeerConnectReport {
                    ok: true,
                    device_id: target_id.to_string(),
                    name,
                    alias,
                    addr: Some(addr.to_string()),
                    connected: true,
                    already_connected: false,
                })
            }
            Err(e) => Err(CoreError::InvalidRoute(format!(
                "failed to dial {} at {}: {}",
                target_id, addr, e
            ))),
        }
    }

    /// Actively disconnect an active session with the specified peer (by id, alias, or name)
    /// without revoking trust or forgetting its route.
    ///
    /// Drops the outbound session channel, evicts the peer from live state,
    /// emits `UiEvent::PeerDown`, and returns a `PeerDisconnectReport`.
    pub async fn disconnect_peer(&self, query: &str) -> Result<PeerDisconnectReport, CoreError> {
        let target_id = self.resolve_peer(query)?;
        let trusted = self.trusted();
        let matched = trusted.iter().find(|t| t.device_id == target_id);
        let alias = matched.and_then(|t| {
            if t.alias.is_empty() {
                None
            } else {
                Some(t.alias.clone())
            }
        });
        let live_name = {
            let live = self.shared.live.lock().expect("live");
            live.get(&target_id).map(|lp| lp.name.clone())
        };
        let name = live_name.or_else(|| alias.clone());

        let had_session = self
            .shared
            .sessions
            .lock()
            .await
            .remove(&target_id)
            .is_some();
        let had_live = self
            .shared
            .live
            .lock()
            .expect("live")
            .remove(&target_id)
            .is_some();

        if *self.shared.input_seat.lock().expect("seat") == Some(CursorSeat::Remote) {
            *self.shared.input_seat.lock().expect("seat") = Some(CursorSeat::Local);
        }
        {
            let mut sp = self.shared.screen_peer.lock().expect("screen_peer");
            if sp.as_ref() == Some(&target_id) {
                *sp = None;
            }
        }
        {
            let mut ip = self.shared.input_peer.lock().expect("input_peer");
            if ip.as_ref() == Some(&target_id) {
                *ip = None;
            }
        }

        if had_session || had_live {
            self.emit(UiEvent::PeerDown {
                device_id: target_id.to_string(),
            });
            Ok(PeerDisconnectReport {
                ok: true,
                device_id: target_id.to_string(),
                name,
                alias,
                disconnected: true,
                already_disconnected: false,
            })
        } else {
            Ok(PeerDisconnectReport {
                ok: true,
                device_id: target_id.to_string(),
                name,
                alias,
                disconnected: false,
                already_disconnected: true,
            })
        }
    }

    /// Check if a peer device ID is currently in the local blocklist.
    pub fn is_blocked(&self, device_id: &str) -> bool {
        let blocked = self.shared.blocked.lock().expect("blocked");
        blocked.peers.iter().any(|p| p.device_id == device_id)
    }

    /// Return a summary report of all currently blocked peers.
    pub fn blocked_peers_report(&self) -> BlockedPeersReport {
        let blocked = self.shared.blocked.lock().expect("blocked");
        BlockedPeersReport {
            ok: true,
            count: blocked.peers.len(),
            peers: blocked.peers.clone(),
        }
    }

    /// Block a peer device by device ID, friendly alias, or broadcast name.
    ///
    /// Persists the block entry into `blocked.json`, and if the peer has an active
    /// connection, forcibly terminates it, evicts from live state, resets input seat,
    /// and emits `UiEvent::PeerDown`. Finally emits `UiEvent::PeerBlocked`.
    pub async fn block_peer(
        &self,
        query: &str,
        reason: Option<&str>,
    ) -> Result<PeerBlockReport, CoreError> {
        let trimmed_query = query.trim();
        if trimmed_query.is_empty() {
            return Err(CoreError::PeerNotFound("empty peer identifier".to_string()));
        }
        let target_id = match self.resolve_peer(trimmed_query) {
            Ok(id) => id,
            Err(e) => {
                if let Ok(id) = DeviceId::parse(trimmed_query) {
                    id
                } else {
                    return Err(e);
                }
            }
        };

        if target_id == *self.shared.identity.device_id() {
            return Err(CoreError::InvalidPayload(
                "cannot block local device".to_string(),
            ));
        }

        let sanitized_reason = if let Some(r) = reason {
            let r_trimmed = r.trim();
            if r_trimmed.is_empty() {
                None
            } else {
                if r_trimmed.len() > 128 {
                    return Err(CoreError::InvalidPayload(
                        "reason exceeds maximum length of 128 bytes".to_string(),
                    ));
                }
                if r_trimmed.chars().any(|c| c.is_control()) {
                    return Err(CoreError::InvalidPayload(
                        "reason cannot contain control characters".to_string(),
                    ));
                }
                Some(r_trimmed.to_string())
            }
        } else {
            None
        };

        let now_ms = self.shared.clock.unix_ms();
        let already_blocked = {
            let mut blocked = self.shared.blocked.lock().expect("blocked");
            let is_already = blocked
                .peers
                .iter()
                .any(|p| p.device_id == target_id.as_str());
            if let Some(existing) = blocked
                .peers
                .iter_mut()
                .find(|p| p.device_id == target_id.as_str())
            {
                existing.reason = sanitized_reason.clone();
                existing.blocked_at = now_ms;
            } else {
                blocked.peers.push(crate::store::BlockedPeer {
                    device_id: target_id.to_string(),
                    reason: sanitized_reason.clone(),
                    blocked_at: now_ms,
                });
            }
            let _ = crate::store::save_blocked(&self.shared.cfg.data_dir, &blocked);
            is_already
        };

        let disconnected_active = {
            let mut sessions = self.shared.sessions.lock().await;
            let had_session = sessions.remove(&target_id).is_some();
            let had_live = {
                let mut live = self.shared.live.lock().expect("live");
                live.remove(&target_id).is_some()
            };
            if had_session || had_live {
                if *self.shared.input_seat.lock().expect("seat") == Some(CursorSeat::Remote) {
                    *self.shared.input_seat.lock().expect("seat") = Some(CursorSeat::Local);
                }
                self.emit(UiEvent::PeerDown {
                    device_id: target_id.to_string(),
                });
                true
            } else {
                false
            }
        };

        let trusted = self.trusted();
        let matched = trusted.iter().find(|t| t.device_id == target_id);
        let alias = matched.and_then(|t| {
            if t.alias.is_empty() {
                None
            } else {
                Some(t.alias.clone())
            }
        });
        let name = alias.clone();

        self.emit(UiEvent::PeerBlocked {
            device_id: target_id.to_string(),
            reason: sanitized_reason.clone(),
        });

        self.record_audit(
            AuditCategory::Security,
            AuditSeverity::Warning,
            "peer_blocked",
            Some(target_id.to_string()),
            format!(
                "peer '{target_id}' added to blacklist{}",
                sanitized_reason
                    .as_deref()
                    .map(|r| format!(" (reason: {r})"))
                    .unwrap_or_default()
            ),
            true,
        );

        Ok(PeerBlockReport {
            ok: true,
            device_id: target_id.to_string(),
            name,
            alias,
            reason: sanitized_reason,
            blocked: true,
            already_blocked,
            disconnected_active,
        })
    }

    /// Unblock a previously blocked peer device.
    ///
    /// Removes the peer from `blocked.json` and emits `UiEvent::PeerUnblocked`.
    pub async fn unblock_peer(&self, query: &str) -> Result<PeerUnblockReport, CoreError> {
        let trimmed_query = query.trim();
        if trimmed_query.is_empty() {
            return Err(CoreError::PeerNotFound("empty peer identifier".to_string()));
        }
        let target_id = match self.resolve_peer(trimmed_query) {
            Ok(id) => id,
            Err(e) => {
                if let Ok(id) = DeviceId::parse(trimmed_query) {
                    id
                } else {
                    return Err(e);
                }
            }
        };

        if target_id == *self.shared.identity.device_id() {
            return Err(CoreError::InvalidPayload(
                "cannot unblock local device".to_string(),
            ));
        }

        let was_blocked = {
            let mut blocked = self.shared.blocked.lock().expect("blocked");
            let initial_len = blocked.peers.len();
            blocked.peers.retain(|p| p.device_id != target_id.as_str());
            let removed = blocked.peers.len() < initial_len;
            if removed {
                let _ = crate::store::save_blocked(&self.shared.cfg.data_dir, &blocked);
            }
            removed
        };

        if was_blocked {
            self.emit(UiEvent::PeerUnblocked {
                device_id: target_id.to_string(),
            });
            self.record_audit(
                AuditCategory::Security,
                AuditSeverity::Notice,
                "peer_unblocked",
                Some(target_id.to_string()),
                format!("peer '{target_id}' removed from blacklist"),
                true,
            );
        }

        Ok(PeerUnblockReport {
            ok: true,
            device_id: target_id.to_string(),
            unblocked: was_blocked,
            was_blocked,
        })
    }

    pub(super) fn already_live_on_ip(&self, addr: SocketAddr) -> bool {
        self.shared
            .live
            .lock()
            .expect("live")
            .values()
            .any(|p| p.addr.ip() == addr.ip() && p.addr.port() == addr.port())
    }
}
