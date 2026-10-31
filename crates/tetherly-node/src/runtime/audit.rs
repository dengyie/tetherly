// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Audit log recording and reports.

use super::*;
use crate::events::UiEvent;

/// Category of audit event for security classification and filtering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditCategory {
    Security,
    Peer,
    File,
    Clipboard,
    Notify,
    Route,
    Config,
    Focus,
}

impl std::fmt::Display for AuditCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuditCategory::Security => write!(f, "security"),
            AuditCategory::Peer => write!(f, "peer"),
            AuditCategory::File => write!(f, "file"),
            AuditCategory::Clipboard => write!(f, "clipboard"),
            AuditCategory::Notify => write!(f, "notify"),
            AuditCategory::Route => write!(f, "route"),
            AuditCategory::Config => write!(f, "config"),
            AuditCategory::Focus => write!(f, "focus"),
        }
    }
}

impl std::str::FromStr for AuditCategory {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "security" => Ok(AuditCategory::Security),
            "peer" | "peers" => Ok(AuditCategory::Peer),
            "file" | "files" => Ok(AuditCategory::File),
            "clipboard" | "clip" => Ok(AuditCategory::Clipboard),
            "notify" | "notification" => Ok(AuditCategory::Notify),
            "route" | "routes" => Ok(AuditCategory::Route),
            "config" | "configuration" => Ok(AuditCategory::Config),
            "focus" | "dnd" => Ok(AuditCategory::Focus),
            _ => Err(format!(
                "unknown audit category '{s}', expected: security, peer, file, clipboard, notify, route, config, focus"
            )),
        }
    }
}

/// Severity level of audit event.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum AuditSeverity {
    Info,
    Notice,
    Warning,
    Alert,
}

impl std::fmt::Display for AuditSeverity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuditSeverity::Info => write!(f, "info"),
            AuditSeverity::Notice => write!(f, "notice"),
            AuditSeverity::Warning => write!(f, "warning"),
            AuditSeverity::Alert => write!(f, "alert"),
        }
    }
}

impl std::str::FromStr for AuditSeverity {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "info" => Ok(AuditSeverity::Info),
            "notice" => Ok(AuditSeverity::Notice),
            "warning" | "warn" => Ok(AuditSeverity::Warning),
            "alert" | "critical" => Ok(AuditSeverity::Alert),
            _ => Err(format!(
                "unknown audit severity '{s}', expected: info, notice, warning, alert"
            )),
        }
    }
}

/// Structured security audit entry. Zero-plaintext compliant.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AuditEntry {
    pub id: u64,
    pub timestamp_ms: u64,
    pub category: AuditCategory,
    pub severity: AuditSeverity,
    pub action: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    pub summary: String,
    pub success: bool,
}

/// Paginated and filtered report of audit entries.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AuditLogReport {
    pub ok: bool,
    pub total: usize,
    pub returned: usize,
    pub capacity: usize,
    pub entries: Vec<AuditEntry>,
}

/// Confirmation report when clearing audit log.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AuditClearReport {
    pub ok: bool,
    pub cleared_count: usize,
    pub message: String,
}

/// In-memory bounded circular ring buffer for zero-plaintext audit logs.
#[derive(Debug)]
pub struct AuditEngine {
    pub(super) max_capacity: usize,
    pub(super) next_id: u64,
    pub(super) entries: std::collections::VecDeque<AuditEntry>,
}

impl AuditEngine {
    pub fn new(max_capacity: usize) -> Self {
        Self {
            max_capacity: max_capacity.max(10),
            next_id: 1,
            entries: std::collections::VecDeque::new(),
        }
    }

    pub fn capacity(&self) -> usize {
        self.max_capacity
    }

    #[allow(clippy::too_many_arguments)]
    pub fn push(
        &mut self,
        now_ms: u64,
        category: AuditCategory,
        severity: AuditSeverity,
        action: impl Into<String>,
        target: Option<String>,
        summary: impl Into<String>,
        success: bool,
    ) -> AuditEntry {
        let entry = AuditEntry {
            id: self.next_id,
            timestamp_ms: now_ms,
            category,
            severity,
            action: action.into(),
            target,
            summary: summary.into(),
            success,
        };
        self.next_id += 1;
        if self.entries.len() >= self.max_capacity {
            self.entries.pop_front();
        }
        self.entries.push_back(entry.clone());
        entry
    }

    pub fn query(
        &self,
        category: Option<AuditCategory>,
        severity: Option<AuditSeverity>,
        limit: Option<usize>,
        since_ms: Option<u64>,
    ) -> (usize, Vec<AuditEntry>) {
        let total = self.entries.len();
        let mut filtered: Vec<AuditEntry> = self
            .entries
            .iter()
            .rev()
            .filter(|e| {
                if let Some(c) = category {
                    if e.category != c {
                        return false;
                    }
                }
                if let Some(s) = severity {
                    if e.severity < s {
                        return false;
                    }
                }
                if let Some(since) = since_ms {
                    if e.timestamp_ms < since {
                        return false;
                    }
                }
                true
            })
            .cloned()
            .collect();

        if let Some(lim) = limit {
            if filtered.len() > lim {
                filtered.truncate(lim);
            }
        }
        (total, filtered)
    }

    pub fn clear(&mut self) -> usize {
        let count = self.entries.len();
        self.entries.clear();
        count
    }
}

impl Node {
    /// Record a structured, zero-plaintext audit entry into the circular buffer and emit over SSE.
    pub fn record_audit(
        &self,
        category: AuditCategory,
        severity: AuditSeverity,
        action: impl Into<String>,
        target: Option<impl Into<String>>,
        summary: impl Into<String>,
        success: bool,
    ) -> AuditEntry {
        let now_ms = self.shared.clock.unix_ms();
        let target_str = target.map(|t| t.into());
        let entry = {
            let mut engine = self.shared.audit_engine.lock().expect("audit_engine lock");
            engine.push(
                now_ms, category, severity, action, target_str, summary, success,
            )
        };
        // Broadcast over SSE event bus (Zero-Plaintext)
        self.emit(UiEvent::audit_event_recorded(
            entry.id,
            entry.category.to_string(),
            entry.severity.to_string(),
            &entry.action,
            entry.target.as_deref(),
            &entry.summary,
            entry.success,
            entry.timestamp_ms,
        ));
        entry
    }

    /// Query and filter security audit log entries.
    pub fn audit_report(
        &self,
        category: Option<AuditCategory>,
        severity: Option<AuditSeverity>,
        limit: Option<usize>,
        since_ms: Option<u64>,
    ) -> AuditLogReport {
        let engine = self.shared.audit_engine.lock().expect("audit_engine lock");
        let capacity = engine.capacity();
        let (total, entries) = engine.query(category, severity, limit, since_ms);
        let returned = entries.len();
        AuditLogReport {
            ok: true,
            total,
            returned,
            capacity,
            entries,
        }
    }

    /// Purge audit log entries and log a security audit event for the clear operation.
    pub fn clear_audit_log(&self) -> AuditClearReport {
        let count = {
            let mut engine = self.shared.audit_engine.lock().expect("audit_engine lock");
            engine.clear()
        };
        // Record a Notice audit event for the clear action itself
        self.record_audit(
            AuditCategory::Security,
            AuditSeverity::Notice,
            "audit_log_cleared",
            None::<String>,
            format!("security audit log cleared ({count} entries purged)"),
            true,
        );
        AuditClearReport {
            ok: true,
            cleared_count: count,
            message: format!("successfully cleared {count} audit log entries"),
        }
    }
}
