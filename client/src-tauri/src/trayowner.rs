// SPDX-License-Identifier: Apache-2.0 OR MIT
//! §6.3 tray takeover: the shell claims the tray surface from the daemon.
//!
//! Once the SSE link goes live, the shell PATCHes
//! `{"tray_owner":"client"}` — the daemon hides its built-in tray. The
//! daemon's 10s keepalive watchdog is the fallback for crashes and lost
//! links; the shell additionally best-effort releases the claim on clean
//! exit (menu quit / window close) so the built-in tray returns
//! immediately instead of after the timeout.

use crate::proxy;

/// Link-up edge tracker. The claim must be re-sent on EVERY down→up edge,
/// not once per process: a daemon restart (deploy) resets `tray_owner` to
/// `daemon`, and the reclaim watchdog flips it back after a keepalive gap
/// (sleep/wake) — a one-shot claim left the built-in icon showing next to
/// the shell's forever (production 2026-11-23: two Tetherly menu-bar icons).
pub struct Claimed(pub std::sync::atomic::AtomicBool);

impl Claimed {
    /// Record the link state; `true` exactly on a down→up edge (claim now).
    pub fn on_link(&self, up: bool) -> bool {
        let was = self.0.swap(up, std::sync::atomic::Ordering::SeqCst);
        up && !was
    }
}

/// Claim the tray surface. Call once the SSE link is live (or right after
/// startup — a claim before any subscriber exists is still valid because
/// the daemon's watchdog only reclaims after the grace window).
pub async fn claim(port: u16, token: &str) {
    release_or_claim(port, token, true).await;
}

/// Release the tray surface (clean exit). Best-effort: failures fall back
/// to the daemon's reclaim timeout.
pub async fn release(port: u16, token: &str) {
    release_or_claim(port, token, false).await;
}

/// `GET /api/v1/settings` body → the daemon took the tray back (watchdog
/// reclaim after a sleep/wake keepalive gap with the SSE socket surviving,
/// so no link edge fires). Unknown shapes never trigger a claim.
pub fn needs_reclaim(settings: &serde_json::Value) -> bool {
    settings
        .pointer("/general/tray_owner")
        .and_then(|v| v.as_str())
        == Some("daemon")
}

/// While the link is up, re-assert the claim if the daemon reclaimed it.
/// Cheap loopback GET every [`RECHECK`]; steady state sends no PATCH.
pub const RECHECK: std::time::Duration = std::time::Duration::from_secs(15);

pub async fn recheck(port: u16, token: &str) {
    if let Ok(r) = proxy::request(port, token, "GET", "/api/v1/settings", None).await {
        if r.status == 200 && needs_reclaim(&r.body) {
            tracing::info!("tray_owner was reclaimed by the daemon; re-claiming");
            claim(port, token).await;
        }
    }
}

async fn release_or_claim(port: u16, token: &str, claim: bool) {
    let body = serde_json::json!({"tray_owner": if claim { "client" } else { "daemon" }});
    match proxy::request(port, token, "PATCH", "/api/v1/settings", Some(&body)).await {
        Ok(r) if r.status == 200 => {
            tracing::info!("tray_owner -> {}", if claim { "client" } else { "daemon" });
        }
        Ok(r) => tracing::warn!("tray_owner patch answered {}", r.status),
        Err(e) => tracing::warn!("tray_owner patch failed: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::{needs_reclaim, Claimed};

    #[test]
    fn reclaim_only_when_daemon_owns_tray() {
        let s = |o: &str| serde_json::json!({"general": {"tray_owner": o}});
        assert!(needs_reclaim(&s("daemon")));
        assert!(!needs_reclaim(&s("client")));
        assert!(!needs_reclaim(&serde_json::json!({"general": {}})));
    }

    /// Every reconnect after a drop re-claims (daemon restart / reclaim
    /// resets the owner); steady-state up frames do not spam PATCHes.
    #[test]
    fn claims_on_every_down_to_up_edge() {
        let c = Claimed(std::sync::atomic::AtomicBool::new(false));
        assert!(c.on_link(true), "first live link claims");
        assert!(!c.on_link(true), "still up: no re-claim");
        assert!(!c.on_link(false), "down never claims");
        assert!(c.on_link(true), "reconnect after daemon restart re-claims");
    }
    /// The claim/release bodies are strict-contract PATCHes: exactly the
    /// `tray_owner` field, enum values the daemon parses.
    #[test]
    fn claim_and_release_bodies_match_contract() {
        let claim = serde_json::json!({"tray_owner": "client"});
        let release = serde_json::json!({"tray_owner": "daemon"});
        let parsed: tetherly_contract_shim::SettingsPatch =
            serde_json::from_value(claim).expect("claim parses");
        assert_eq!(parsed.tray_owner.map(|o| o as i32), Some(1));
        let parsed: tetherly_contract_shim::SettingsPatch =
            serde_json::from_value(release).expect("release parses");
        assert_eq!(parsed.tray_owner.map(|o| o as i32), Some(0));
    }

    /// Minimal mirror of the wire contract (kept local so the shell crate
    /// does not depend on daemon internals; drift is caught by the real
    /// daemon's deny_unknown_fields parse in integration). Only the fields
    /// the shim needs; the rest parse via serde defaults.
    mod tetherly_contract_shim {
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        pub struct SettingsPatch {
            #[serde(default)]
            pub tray_owner: Option<TrayOwner>,
        }
        #[derive(serde::Deserialize, Clone, Copy, PartialEq)]
        #[serde(rename_all = "lowercase")]
        pub enum TrayOwner {
            Daemon = 0,
            Client = 1,
        }
    }
}
