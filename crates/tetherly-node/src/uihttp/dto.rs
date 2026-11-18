// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Legacy-surviving DTOs: only the symbols still referenced after the
//! M5 convergence (§169) — DaemonRestartBody/Report (runtime lifecycle +
//! lib re-export), parse_hh_mm (v1 settings scheduling) and node_clock_ms
//! (v1 locally-originated pushes).

#[derive(serde::Deserialize, Default, Debug, Clone)]
pub struct DaemonRestartBody {
    #[serde(default, alias = "replacement", alias = "new_binary")]
    pub replacement_path: Option<String>,
    #[serde(default, alias = "delay")]
    pub delay_ms: Option<u64>,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct DaemonRestartReport {
    pub ok: bool,
    pub action: String,
    pub old_pid: u32,
    pub new_pid: u32,
    pub message: String,
    pub timestamp_ms: u64,
}

pub(super) fn parse_hh_mm(s: &str) -> Option<u16> {
    let parts: Vec<&str> = s.trim().split(':').collect();
    if parts.len() != 2 {
        return None;
    }
    let h: u16 = parts[0].trim().parse().ok()?;
    let m: u16 = parts[1].trim().parse().ok()?;
    if h < 24 && m < 60 {
        Some(h * 60 + m)
    } else {
        None
    }
}

/// Wall clock for a locally originated push. The node clock is the handshake
/// clock; a notify timestamp only has to be monotone enough for the peer's TTL.
pub(super) fn node_clock_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
