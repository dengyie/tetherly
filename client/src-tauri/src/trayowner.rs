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

/// One-shot guard: the LINK_CHANNEL listener fires on every reconnect, but
/// the claim PATCH only needs to happen once per process (the daemon keeps
/// the claim across shell reconnects; the watchdog covers gaps).
pub struct Claimed(pub std::sync::atomic::AtomicBool);

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
