// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Benchmark/streaming recommendations and sessions.

use super::*;
use crate::events::UiEvent;
use crate::store::{
    save_bandwidth_profiles, save_bench_policy, BenchPolicy, DeviceType, PeerBandwidthProfile,
};
use tetherly_core::{
    recommend_streaming_params, BenchAcceptFrame, BenchDataFrame, BenchDirection, BenchReportFrame,
    BenchRequestFrame, BenchRole, BenchValidator, CoreError, DeviceId,
};
use tracing::warn;

/// Active link throughput benchmark session metadata.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ActiveBenchSession {
    pub session_id: String,
    pub peer_id: String,
    pub role: BenchRole,
    pub direction: BenchDirection,
    pub duration_secs: u32,
    pub chunk_size: u32,
    pub chunks_processed: u64,
    pub bytes_transferred: u64,
    pub started_at_ms: u64,
    pub last_activity_ms: u64,
}

/// Status report for the link benchmark engine and governance policy.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct BenchStatusReport {
    pub ok: bool,
    pub active: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<ActiveBenchSession>,
    pub policy: BenchPolicy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_report: Option<BenchReportFrame>,
    pub timestamp_ms: u64,
}

/// Decision report when starting or stopping a benchmark session.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct BenchDecisionReport {
    pub ok: bool,
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peer_id: Option<String>,
    pub message: String,
    pub timestamp_ms: u64,
}

/// Governance policy report for link benchmarking.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct BenchPolicyReport {
    pub ok: bool,
    pub policy: BenchPolicy,
    pub message: String,
}

/// Streaming parameter recommendation report for a single peer (ADR-078).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct StreamingRecommendationReport {
    pub ok: bool,
    pub peer_id: String,
    pub peer_alias: String,
    pub has_profile: bool,
    pub measured_mbps: f64,
    pub direction: String,
    pub measured_at_ms: u64,
    pub recommendation: tetherly_core::recommend::StreamingRecommendation,
    pub timestamp_ms: u64,
}

/// One peer's recommendation summary line for the aggregate status view.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct PeerRecommendationSummary {
    pub peer_id: String,
    pub peer_alias: String,
    pub measured_mbps: f64,
    pub direction: String,
    pub measured_at_ms: u64,
    pub tier: String,
}

/// Aggregate recommendation status across all measured peers.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct RecommendationStatusReport {
    pub ok: bool,
    pub peers: Vec<PeerRecommendationSummary>,
    pub timestamp_ms: u64,
}

/// Partial update payload for bench policy.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct BenchPolicyUpdate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_inbound_bench: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_duration_secs: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_bandwidth_mbps: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_groups: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_tags: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_types: Option<Vec<DeviceType>>,
}

impl Node {
    /// Ingest and process an inbound BenchRequestFrame: this node becomes the responder.
    pub async fn receive_bench_request(&self, peer: &DeviceId, frame: BenchRequestFrame) {
        let now_ms = self.shared.clock.unix_ms();
        let peer_str = peer.to_string();

        if BenchValidator::validate_request(&frame).is_err() {
            warn!(peer = %peer, session_id = %frame.session_id, "invalid bench request frame rejected");
            return;
        }

        let policy = self.bench_policy();
        let profile = self.peer_profile(peer);

        let rejection_reason = if !policy.enabled {
            Some("link benchmarking disabled".to_string())
        } else if !policy.allow_inbound_bench {
            Some("inbound benchmark serving disabled by policy".to_string())
        } else if frame.duration_secs > policy.max_duration_secs {
            Some("duration exceeds policy limit".to_string())
        } else if !policy.is_peer_eligible(&profile, false) {
            Some("peer ineligible under bench policy filters".to_string())
        } else {
            None
        };

        if let Some(reason) = rejection_reason {
            let accept = BenchAcceptFrame {
                session_id: frame.session_id.clone(),
                accepted: false,
                reject_reason: Some(reason.clone()),
                direction: frame.direction,
                duration_secs: frame.duration_secs,
                chunk_size: frame.chunk_size,
                timestamp_ms: now_ms,
            };
            if let Ok(inner) = accept.to_frame() {
                let _ = self.send_to(peer, inner).await;
            }
            self.record_audit(
                AuditCategory::Security,
                AuditSeverity::Warning,
                "bench_request_rejected",
                Some(peer_str.clone()),
                format!("Rejected bench request from {}: {}", peer_str, reason),
                false,
            );
            return;
        }

        // Responder clamps to its own policy envelope before agreeing.
        let duration_secs = frame.duration_secs.min(policy.max_duration_secs);
        let chunk_size = BenchValidator::clamp_chunk_size(frame.chunk_size);

        // Reject if a bench session is already active on this node.
        let already_active = {
            let guard = self.shared.active_bench.lock().expect("active_bench");
            guard.is_some()
        };
        if already_active {
            let accept = BenchAcceptFrame {
                session_id: frame.session_id.clone(),
                accepted: false,
                reject_reason: Some("a benchmark session is already active".to_string()),
                direction: frame.direction,
                duration_secs: frame.duration_secs,
                chunk_size: frame.chunk_size,
                timestamp_ms: now_ms,
            };
            if let Ok(inner) = accept.to_frame() {
                let _ = self.send_to(peer, inner).await;
            }
            return;
        }

        let session = ActiveBenchSession {
            session_id: frame.session_id.clone(),
            peer_id: peer_str.clone(),
            role: BenchRole::Responder,
            direction: frame.direction,
            duration_secs,
            chunk_size,
            chunks_processed: 0,
            bytes_transferred: 0,
            started_at_ms: now_ms,
            last_activity_ms: now_ms,
        };
        *self.shared.active_bench.lock().expect("active_bench") = Some(session);

        let alias = self.peer_alias(peer).unwrap_or_else(|| peer_str.clone());
        self.emit(UiEvent::bench_session_started(
            &frame.session_id,
            &peer_str,
            &alias,
            "responder",
            frame.direction.as_str(),
            duration_secs,
            chunk_size,
            now_ms,
        ));
        self.record_audit(
            AuditCategory::Peer,
            AuditSeverity::Info,
            "bench_responder_started",
            Some(peer_str.clone()),
            format!(
                "Serving benchmark {} from peer {} (direction={}, duration={}s, chunk={}B)",
                frame.session_id,
                peer_str,
                frame.direction.as_str(),
                duration_secs,
                chunk_size
            ),
            true,
        );

        let accept = BenchAcceptFrame {
            session_id: frame.session_id.clone(),
            accepted: true,
            reject_reason: None,
            direction: frame.direction,
            duration_secs,
            chunk_size,
            timestamp_ms: now_ms,
        };
        if let Ok(inner) = accept.to_frame() {
            let _ = self.send_to(peer, inner).await;
        }

        // Pull direction: the responder is the payload sender; start its engine now.
        if frame.direction == BenchDirection::Pull {
            self.spawn_bench_sender(
                peer.clone(),
                frame.session_id.clone(),
                frame.direction,
                duration_secs,
                chunk_size,
                policy.max_bandwidth_mbps,
            );
        }
    }

    /// Ingest and process an inbound BenchAcceptFrame: this node is the initiator.
    pub async fn receive_bench_accept(&self, peer: &DeviceId, frame: BenchAcceptFrame) {
        let now_ms = self.shared.clock.unix_ms();
        let peer_str = peer.to_string();

        if BenchValidator::validate_session_id(&frame.session_id).is_err() {
            return;
        }

        if !frame.accepted {
            // Responder refused; tear down the local half of the session.
            let removed = {
                let mut guard = self.shared.active_bench.lock().expect("active_bench");
                guard
                    .as_ref()
                    .map(|s| s.session_id == frame.session_id && s.peer_id == peer_str)
                    .unwrap_or(false)
                    .then(|| guard.take())
                    .flatten()
            };
            if let Some(session) = removed {
                let alias = self.peer_alias(peer).unwrap_or_else(|| peer_str.clone());
                self.emit(UiEvent::bench_session_finished(
                    &session.session_id,
                    &peer_str,
                    &alias,
                    session.direction.as_str(),
                    now_ms.saturating_sub(session.started_at_ms),
                    session.bytes_transferred,
                    session.chunks_processed,
                    0.0,
                    0,
                    0,
                    now_ms,
                ));
                self.record_audit(
                    AuditCategory::Peer,
                    AuditSeverity::Warning,
                    "bench_rejected_by_peer",
                    Some(peer_str.clone()),
                    format!(
                        "Benchmark {} refused by peer: {:?}",
                        frame.session_id, frame.reject_reason
                    ),
                    true,
                );
            }
            return;
        }

        // Only the initiator consumes accepts for a session it created; the
        // role check and parameter adoption happen in one critical section.
        let adopted = {
            let mut guard = self.shared.active_bench.lock().expect("active_bench");
            let is_initiator = guard
                .as_ref()
                .map(|s| {
                    s.session_id == frame.session_id
                        && s.peer_id == peer_str
                        && s.role == BenchRole::Initiator
                })
                .unwrap_or(false);
            if is_initiator {
                if let Some(ref mut session) = *guard {
                    session.duration_secs = frame.duration_secs;
                    session.chunk_size = frame.chunk_size;
                }
            }
            is_initiator
        };
        if !adopted {
            return;
        }

        if frame.direction == BenchDirection::Push {
            let policy = self.bench_policy();
            self.spawn_bench_sender(
                peer.clone(),
                frame.session_id.clone(),
                frame.direction,
                frame.duration_secs,
                frame.chunk_size,
                policy.max_bandwidth_mbps,
            );
        }
    }

    /// Ingest a benchmark payload chunk: the local node is the receiving side.
    /// Payload bytes are engine-generated random data and are never copied into
    /// events, logs, or audit records (Zero-Plaintext).
    pub fn receive_bench_data(&self, peer: &DeviceId, frame: BenchDataFrame) {
        let now_ms = self.shared.clock.unix_ms();
        let peer_str = peer.to_string();

        if BenchValidator::validate_data_frame(&frame).is_err() {
            return;
        }

        let mut guard = self.shared.active_bench.lock().expect("active_bench");
        if let Some(ref mut session) = *guard {
            if session.session_id == frame.session_id && session.peer_id == peer_str {
                session.last_activity_ms = now_ms;
                session.chunks_processed = session.chunks_processed.saturating_add(1);
                session.bytes_transferred = session
                    .bytes_transferred
                    .saturating_add(frame.data.len() as u64);
            }
        }
    }

    /// Ingest the final benchmark report from the sending side and finalize the
    /// local (receiving) half with authoritative link measurements.
    pub async fn receive_bench_report(&self, peer: &DeviceId, frame: BenchReportFrame) {
        let now_ms = self.shared.clock.unix_ms();
        let peer_str = peer.to_string();
        let alias = self.peer_alias(peer).unwrap_or_else(|| peer_str.clone());

        if BenchValidator::validate_session_id(&frame.session_id).is_err() {
            return;
        }

        let removed = {
            let mut guard = self.shared.active_bench.lock().expect("active_bench");
            guard
                .as_ref()
                .map(|s| s.session_id == frame.session_id && s.peer_id == peer_str)
                .unwrap_or(false)
                .then(|| guard.take())
                .flatten()
        };

        if let Some(session) = removed {
            // Receiving side numbers are the authoritative link measurement.
            let duration_ms = now_ms.saturating_sub(session.started_at_ms).max(1);
            let throughput_mbps =
                (session.bytes_transferred as f64 * 8.0) / (duration_ms as f64 / 1000.0) / 1e6;

            // Authoritative profile report: receiver-side bytes/chunks/throughput,
            // sender-side pacing latency/jitter retained for reference. Stored in
            // last_bench_report and distilled into the persistent peer profile.
            let authoritative = BenchReportFrame {
                session_id: session.session_id.clone(),
                direction: session.direction,
                duration_ms,
                bytes_transferred: session.bytes_transferred,
                chunks: session.chunks_processed,
                throughput_mbps,
                avg_chunk_latency_us: frame.avg_chunk_latency_us,
                jitter_us: frame.jitter_us,
                stopped_early: frame.stopped_early,
                timestamp_ms: now_ms,
            };
            *self
                .shared
                .last_bench_report
                .lock()
                .expect("last_bench_report") = Some(authoritative);

            self.record_bandwidth_profile(
                &session.peer_id,
                &alias,
                throughput_mbps,
                session.direction.as_str(),
                &session.session_id,
                now_ms,
            );

            self.emit(UiEvent::bench_session_finished(
                &session.session_id,
                &session.peer_id,
                &alias,
                session.direction.as_str(),
                duration_ms,
                session.bytes_transferred,
                session.chunks_processed,
                throughput_mbps,
                frame.avg_chunk_latency_us,
                frame.jitter_us,
                now_ms,
            ));
            self.record_audit(
                AuditCategory::Peer,
                AuditSeverity::Info,
                "bench_finished",
                Some(session.peer_id.clone()),
                format!(
                    "Benchmark {} finished: {:.2} Mbps received ({} bytes / {} chunks in {} ms)",
                    session.session_id,
                    throughput_mbps,
                    session.bytes_transferred,
                    session.chunks_processed,
                    duration_ms
                ),
                true,
            );
        }
    }

    /// Distill a completed receiver-side measurement into the persistent
    /// per-peer bandwidth profile, broadcast the update, and audit it.
    pub(super) fn record_bandwidth_profile(
        &self,
        peer_id: &str,
        alias: &str,
        measured_mbps: f64,
        direction: &str,
        session_id: &str,
        now_ms: u64,
    ) {
        let tier = recommend_streaming_params(measured_mbps).tier;
        // v1.92: a fresh link measurement re-tiers a live H.264 stream hot
        // (the encoder rebuilds with an IDR at the new rate).
        if self.shared.screen_bitrate_auto {
            if let Some(sel) = &self.shared.screen_encode_codec {
                if self
                    .shared
                    .screen_engine
                    .lock()
                    .expect("screen")
                    .state()
                    .is_streaming()
                {
                    sel.set_bitrate_bps(crate::screen::screen_bitrate_from_profile(Some(
                        measured_mbps,
                    )));
                }
            }
        }
        {
            let mut table = self
                .shared
                .bandwidth_profiles
                .lock()
                .expect("bandwidth_profiles");
            table.upsert(PeerBandwidthProfile {
                peer_id: peer_id.to_string(),
                measured_mbps,
                direction: direction.to_string(),
                session_id: session_id.to_string(),
                measured_at_ms: now_ms,
            });
            if let Err(e) = save_bandwidth_profiles(&self.shared.cfg.data_dir, &table) {
                warn!(error = %e, "failed to persist bandwidth_profiles.json");
            }
        }
        self.emit(UiEvent::bandwidth_profile_updated(
            peer_id,
            alias,
            measured_mbps,
            direction,
            &tier,
            now_ms,
        ));
        self.record_audit(
            AuditCategory::Peer,
            AuditSeverity::Info,
            "bandwidth_profile_updated",
            Some(peer_id.to_string()),
            format!(
                "Bandwidth profile updated for peer {} ({}, session {}): {:.2} Mbps -> tier {}",
                peer_id, direction, session_id, measured_mbps, tier
            ),
            true,
        );
    }

    /// Spawn the paced payload-sending engine for the given benchmark session.
    /// Chunks carry engine-generated random bytes at a rate capped by the
    /// policy's `max_bandwidth_mbps` so coexisting streams stay usable.
    pub(super) fn spawn_bench_sender(
        &self,
        peer: DeviceId,
        session_id: String,
        direction: BenchDirection,
        duration_secs: u32,
        chunk_size: u32,
        max_bandwidth_mbps: u32,
    ) {
        let node = self.clone();
        tokio::spawn(async move {
            let started_ms = node.shared.clock.unix_ms();
            let deadline = started_ms + (duration_secs as u64) * 1000;

            // One random payload template per session; chunk bodies are engine
            // data only and never contain user content.
            let mut template = vec![0u8; chunk_size as usize];
            let _ = getrandom::getrandom(&mut template);

            let bytes_per_sec = (max_bandwidth_mbps as u64).saturating_mul(125_000);
            let pacing_us = (chunk_size as u64 * 1_000_000)
                .checked_div(bytes_per_sec)
                .unwrap_or(0);
            let pacing = std::time::Duration::from_micros(pacing_us);

            let mut chunk_seq: u64 = 0;
            let mut last_send_ms: Option<u64> = None;
            // Send-side pacing metrics: mean inter-send gap and its mean absolute
            // deviation (jitter proxy). Receiver-side numbers stay authoritative.
            let mut gap_sum_us: u64 = 0;
            let mut mad_sum_us: u64 = 0;
            let mut gap_samples: u64 = 0;

            loop {
                let now_ms = node.shared.clock.unix_ms();
                if now_ms >= deadline {
                    break;
                }
                {
                    let guard = node.shared.active_bench.lock().expect("active_bench");
                    let still_active = guard
                        .as_ref()
                        .map(|s| s.session_id == session_id)
                        .unwrap_or(false);
                    if !still_active {
                        return; // stopped remotely or locally
                    }
                }

                if let Some(last) = last_send_ms {
                    let gap_us = now_ms.saturating_sub(last).saturating_mul(1000);
                    gap_sum_us = gap_sum_us.saturating_add(gap_us);
                    let running_mean = gap_sum_us / gap_samples.max(1);
                    mad_sum_us = mad_sum_us.saturating_add(gap_us.abs_diff(running_mean));
                    gap_samples = gap_samples.saturating_add(1);
                }
                last_send_ms = Some(now_ms);

                let inner =
                    match BenchDataFrame::to_frame_for(&session_id, chunk_seq, &template, now_ms) {
                        Ok(inner) => inner,
                        Err(_) => break, // unrecoverable codec failure; finalize below
                    };
                if node.send_to(&peer, inner).await.is_err() {
                    break;
                }
                chunk_seq = chunk_seq.saturating_add(1);

                {
                    let mut guard = node.shared.active_bench.lock().expect("active_bench");
                    if let Some(ref mut session) = *guard {
                        if session.session_id == session_id {
                            session.chunks_processed = session.chunks_processed.saturating_add(1);
                            session.bytes_transferred =
                                session.bytes_transferred.saturating_add(chunk_size as u64);
                            session.last_activity_ms = now_ms;
                        }
                    }
                }

                if !pacing.is_zero() {
                    tokio::time::sleep(pacing).await;
                }
            }

            // Sending window elapsed (or link dropped): report and finalize locally.
            let end_ms = node.shared.clock.unix_ms();
            let duration_ms = end_ms.saturating_sub(started_ms).max(1);
            let (bytes_sent, chunks_sent) = {
                let mut guard = node.shared.active_bench.lock().expect("active_bench");
                let stats = guard
                    .as_ref()
                    .filter(|s| s.session_id == session_id)
                    .map(|s| (s.bytes_transferred, s.chunks_processed))
                    .unwrap_or((0, 0));
                guard.take();
                stats
            };
            let throughput_mbps = (bytes_sent as f64 * 8.0) / (duration_ms as f64 / 1000.0) / 1e6;
            let avg_gap_us = gap_sum_us.checked_div(gap_samples).unwrap_or(0);
            let jitter_us = mad_sum_us.checked_div(gap_samples).unwrap_or(0);

            let report = BenchReportFrame {
                session_id: session_id.clone(),
                direction,
                duration_ms,
                bytes_transferred: bytes_sent,
                chunks: chunks_sent,
                throughput_mbps,
                avg_chunk_latency_us: avg_gap_us,
                jitter_us,
                stopped_early: Some(end_ms < deadline),
                timestamp_ms: end_ms,
            };

            let alias = DeviceId::parse(&peer.to_string())
                .ok()
                .and_then(|id| node.peer_alias(&id))
                .unwrap_or_else(|| peer.to_string());
            node.emit(UiEvent::bench_session_finished(
                &session_id,
                peer.to_string(),
                &alias,
                direction.as_str(),
                duration_ms,
                bytes_sent,
                chunks_sent,
                throughput_mbps,
                avg_gap_us,
                jitter_us,
                end_ms,
            ));
            node.record_audit(
                AuditCategory::Peer,
                AuditSeverity::Info,
                "bench_sender_finished",
                Some(peer.to_string()),
                format!(
                    "Benchmark {} send window closed: {:.2} Mbps sent ({} bytes / {} chunks in {} ms)",
                    session_id, throughput_mbps, bytes_sent, chunks_sent, duration_ms
                ),
                true,
            );

            *node
                .shared
                .last_bench_report
                .lock()
                .expect("last_bench_report") = Some(report.clone());
            if let Ok(inner) = report.to_frame() {
                let _ = node.send_to(&peer, inner).await;
            }
        });
    }

    /// Query status of the active benchmark session and governance policy.
    pub fn bench_status(&self) -> BenchStatusReport {
        let now_ms = self.shared.clock.unix_ms();
        let policy = self.bench_policy();
        let mut guard = self.shared.active_bench.lock().expect("active_bench");

        // Timeout fallback: finalize a session whose send window plus grace has
        // elapsed without a report (e.g. peer disconnected mid-run).
        let expired = guard
            .as_ref()
            .map(|s| {
                now_ms.saturating_sub(s.started_at_ms) > (s.duration_secs as u64) * 1000 + 10_000
            })
            .unwrap_or(false);

        if expired {
            if let Some(session) = guard.take() {
                drop(guard);
                let alias = DeviceId::parse(&session.peer_id)
                    .ok()
                    .and_then(|id| self.peer_alias(&id))
                    .unwrap_or_else(|| session.peer_id.clone());
                let duration_ms = now_ms.saturating_sub(session.started_at_ms).max(1);
                let throughput_mbps =
                    (session.bytes_transferred as f64 * 8.0) / (duration_ms as f64 / 1000.0) / 1e6;
                self.emit(UiEvent::bench_session_finished(
                    &session.session_id,
                    &session.peer_id,
                    &alias,
                    session.direction.as_str(),
                    duration_ms,
                    session.bytes_transferred,
                    session.chunks_processed,
                    throughput_mbps,
                    0,
                    0,
                    now_ms,
                ));
                self.record_audit(
                    AuditCategory::Peer,
                    AuditSeverity::Notice,
                    "bench_timeout",
                    Some(session.peer_id.clone()),
                    format!(
                        "Benchmark {} finalized by timeout without peer report",
                        session.session_id
                    ),
                    true,
                );
                let last_report = self
                    .shared
                    .last_bench_report
                    .lock()
                    .expect("last_bench_report")
                    .clone();
                return BenchStatusReport {
                    ok: true,
                    active: false,
                    session: None,
                    policy,
                    last_report,
                    timestamp_ms: now_ms,
                };
            }
        }

        let (active, session) = match *guard {
            Some(ref s) => (true, Some(s.clone())),
            None => (false, None),
        };
        let last_report = self
            .shared
            .last_bench_report
            .lock()
            .expect("last_bench_report")
            .clone();

        BenchStatusReport {
            ok: true,
            active,
            session,
            policy,
            last_report,
            timestamp_ms: now_ms,
        }
    }

    /// Start a link throughput benchmark against a remote peer.
    pub async fn bench_start(
        &self,
        peer_query: &str,
        direction: BenchDirection,
        duration_secs: Option<u32>,
        chunk_size: Option<u32>,
    ) -> Result<BenchDecisionReport, CoreError> {
        let now_ms = self.shared.clock.unix_ms();
        let peer_id = self.resolve_peer(peer_query)?;
        let peer_str = peer_id.to_string();
        let alias = self
            .peer_alias(&peer_id)
            .unwrap_or_else(|| peer_str.clone());

        let policy = self.bench_policy();
        if !policy.enabled {
            return Err(CoreError::InvalidPayload(
                "link benchmarking disabled".to_string(),
            ));
        }

        let profile = self.peer_profile(&peer_id);
        if !policy.is_peer_eligible(&profile, true) {
            return Err(CoreError::InvalidPayload(
                "peer is ineligible under bench policy filters".to_string(),
            ));
        }

        let req_duration = BenchValidator::clamp_duration(
            duration_secs.unwrap_or(5).min(policy.max_duration_secs),
        );
        let req_chunk = BenchValidator::clamp_chunk_size(chunk_size.unwrap_or(16384));

        let session_id = format!("bench-sess-{}", &now_ms.to_string()[6..]);
        let request = BenchRequestFrame {
            session_id: session_id.clone(),
            peer_id: self.identity().device_id().to_string(),
            direction,
            duration_secs: req_duration,
            chunk_size: req_chunk,
            timestamp_ms: now_ms,
        };

        BenchValidator::validate_request(&request)?;

        // Atomically reserve the single session slot before the await, so two
        // concurrent starts cannot both pass the emptiness check (TOCTOU).
        {
            let mut guard = self.shared.active_bench.lock().expect("active_bench");
            if guard.is_some() {
                return Err(CoreError::InvalidPayload(
                    "a benchmark session is already active".to_string(),
                ));
            }
            *guard = Some(ActiveBenchSession {
                session_id: session_id.clone(),
                peer_id: peer_str.clone(),
                role: BenchRole::Initiator,
                direction,
                duration_secs: req_duration,
                chunk_size: req_chunk,
                chunks_processed: 0,
                bytes_transferred: 0,
                started_at_ms: now_ms,
                last_activity_ms: now_ms,
            });
        }

        let inner = request.to_frame()?;
        if let Err(e) = self.send_to(&peer_id, inner).await {
            // Roll back the reservation so a later start is not blocked forever.
            let mut guard = self.shared.active_bench.lock().expect("active_bench");
            if guard
                .as_ref()
                .map(|s| s.session_id == session_id)
                .unwrap_or(false)
            {
                *guard = None;
            }
            return Err(e);
        }

        self.emit(UiEvent::bench_session_started(
            &session_id,
            &peer_str,
            &alias,
            "initiator",
            direction.as_str(),
            req_duration,
            req_chunk,
            now_ms,
        ));
        self.record_audit(
            AuditCategory::Peer,
            AuditSeverity::Info,
            "bench_started",
            Some(peer_str.clone()),
            format!(
                "Started benchmark {} against peer {} (direction={}, duration={}s, chunk={}B)",
                session_id,
                peer_str,
                direction.as_str(),
                req_duration,
                req_chunk
            ),
            true,
        );

        Ok(BenchDecisionReport {
            ok: true,
            action: "start".to_string(),
            session_id: Some(session_id),
            peer_id: Some(peer_str),
            message: "link benchmark session requested".to_string(),
            timestamp_ms: now_ms,
        })
    }

    /// Stop the active benchmark session early, notifying the peer.
    pub async fn bench_stop(&self) -> Result<BenchDecisionReport, CoreError> {
        let now_ms = self.shared.clock.unix_ms();

        let active_opt = {
            let mut guard = self.shared.active_bench.lock().expect("active_bench");
            guard.take()
        };

        if let Some(session) = active_opt {
            let peer_id = DeviceId::parse(&session.peer_id).ok();
            let duration_ms = now_ms.saturating_sub(session.started_at_ms).max(1);
            let throughput_mbps =
                (session.bytes_transferred as f64 * 8.0) / (duration_ms as f64 / 1000.0) / 1e6;

            let report = BenchReportFrame {
                session_id: session.session_id.clone(),
                direction: session.direction,
                duration_ms,
                bytes_transferred: session.bytes_transferred,
                chunks: session.chunks_processed,
                throughput_mbps,
                avg_chunk_latency_us: 0,
                jitter_us: 0,
                stopped_early: Some(true),
                timestamp_ms: now_ms,
            };

            if let Some(peer) = peer_id {
                if let Ok(inner) = report.to_frame() {
                    let _ = self.send_to(&peer, inner).await;
                }
            }

            *self
                .shared
                .last_bench_report
                .lock()
                .expect("last_bench_report") = Some(report);

            let alias = DeviceId::parse(&session.peer_id)
                .ok()
                .and_then(|id| self.peer_alias(&id))
                .unwrap_or_else(|| session.peer_id.clone());

            self.emit(UiEvent::bench_session_finished(
                &session.session_id,
                &session.peer_id,
                &alias,
                session.direction.as_str(),
                duration_ms,
                session.bytes_transferred,
                session.chunks_processed,
                throughput_mbps,
                0,
                0,
                now_ms,
            ));
            self.record_audit(
                AuditCategory::Peer,
                AuditSeverity::Notice,
                "bench_stopped",
                Some(session.peer_id.clone()),
                format!("Benchmark {} stopped early by user", session.session_id),
                true,
            );

            Ok(BenchDecisionReport {
                ok: true,
                action: "stop".to_string(),
                session_id: Some(session.session_id),
                peer_id: Some(session.peer_id),
                message: "benchmark session stopped".to_string(),
                timestamp_ms: now_ms,
            })
        } else {
            Ok(BenchDecisionReport {
                ok: true,
                action: "stop".to_string(),
                session_id: None,
                peer_id: None,
                message: "no active benchmark session to stop".to_string(),
                timestamp_ms: now_ms,
            })
        }
    }

    /// Returns the current bench governance policy.
    pub fn bench_policy(&self) -> BenchPolicy {
        self.shared
            .bench_policy
            .lock()
            .expect("bench_policy")
            .clone()
    }

    /// Status and configuration report for bench policy.
    pub fn bench_policy_report(&self) -> BenchPolicyReport {
        BenchPolicyReport {
            ok: true,
            policy: self.bench_policy(),
            message: "bench policy report".to_string(),
        }
    }

    /// Update bench policy and persist to disk atomically with 0o600 permissions.
    pub fn set_bench_policy(
        &self,
        mut policy: BenchPolicy,
    ) -> Result<BenchPolicyReport, CoreError> {
        let now = self.shared.clock.unix_ms();
        policy.sanitize();
        save_bench_policy(&self.shared.cfg.data_dir, &policy)
            .map_err(|e| CoreError::File(format!("failed to save bench_policy.json: {e}")))?;
        *self.shared.bench_policy.lock().expect("bench_policy") = policy.clone();

        self.emit(UiEvent::bench_policy_changed(
            policy.enabled,
            policy.allow_inbound_bench,
            policy.max_duration_secs,
            policy.max_bandwidth_mbps,
            policy.allowed_groups.clone(),
            policy.allowed_tags.clone(),
            policy
                .allowed_types
                .iter()
                .map(|t| t.as_str().to_string())
                .collect(),
            now,
        ));

        self.record_audit(
            AuditCategory::Config,
            AuditSeverity::Info,
            "bench_policy_changed",
            None::<String>,
            format!(
                "Updated bench policy: enabled={}, allow_inbound={}, max_duration={}s, max_bandwidth={}Mbps",
                policy.enabled, policy.allow_inbound_bench, policy.max_duration_secs, policy.max_bandwidth_mbps
            ),
            true,
        );

        Ok(BenchPolicyReport {
            ok: true,
            policy,
            message: "bench policy updated".to_string(),
        })
    }

    /// Partially update bench policy and persist to disk.
    pub fn update_bench_policy(
        &self,
        update: BenchPolicyUpdate,
    ) -> Result<BenchPolicyReport, CoreError> {
        let mut policy = self.bench_policy();
        if let Some(v) = update.enabled {
            policy.enabled = v;
        }
        if let Some(v) = update.allow_inbound_bench {
            policy.allow_inbound_bench = v;
        }
        if let Some(v) = update.max_duration_secs {
            policy.max_duration_secs = v;
        }
        if let Some(v) = update.max_bandwidth_mbps {
            policy.max_bandwidth_mbps = v;
        }
        if let Some(v) = update.allowed_groups {
            policy.allowed_groups = v;
        }
        if let Some(v) = update.allowed_tags {
            policy.allowed_tags = v;
        }
        if let Some(v) = update.allowed_types {
            policy.allowed_types = v;
        }
        self.set_bench_policy(policy)
    }

    /// Bandwidth-profile-driven streaming parameter recommendation for one peer
    /// (ADR-078). Falls back to the conservative minimal tier when no
    /// receiver-side measurement exists for the peer yet.
    pub fn streaming_recommendation(
        &self,
        peer_query: &str,
    ) -> Result<StreamingRecommendationReport, CoreError> {
        let now_ms = self.shared.clock.unix_ms();
        let peer_id = self.resolve_peer(peer_query)?;
        let peer_str = peer_id.to_string();
        let alias = self
            .peer_alias(&peer_id)
            .unwrap_or_else(|| peer_str.clone());

        let profile = self
            .shared
            .bandwidth_profiles
            .lock()
            .expect("bandwidth_profiles")
            .get(&peer_str)
            .cloned();

        let (measured_mbps, direction, measured_at_ms, has_profile) = match &profile {
            Some(p) => (p.measured_mbps, p.direction.clone(), p.measured_at_ms, true),
            None => (0.0, "none".to_string(), 0, false),
        };
        let recommendation = recommend_streaming_params(measured_mbps);
        // A recommendation that cannot survive the owning validators is a
        // logic bug; surface it instead of returning unusable tiers.
        recommendation.validate()?;

        Ok(StreamingRecommendationReport {
            ok: true,
            peer_id: peer_str,
            peer_alias: alias,
            has_profile,
            measured_mbps,
            direction,
            measured_at_ms,
            recommendation,
            timestamp_ms: now_ms,
        })
    }

    /// Aggregate recommendation status across all known peer profiles.
    pub fn recommendation_status(&self) -> RecommendationStatusReport {
        let now_ms = self.shared.clock.unix_ms();
        let table = self
            .shared
            .bandwidth_profiles
            .lock()
            .expect("bandwidth_profiles")
            .clone();
        let peers = table
            .peers
            .iter()
            .map(|p| {
                let alias = DeviceId::parse(&p.peer_id)
                    .ok()
                    .and_then(|id| self.peer_alias(&id))
                    .unwrap_or_else(|| p.peer_id.clone());
                PeerRecommendationSummary {
                    peer_id: p.peer_id.clone(),
                    peer_alias: alias,
                    measured_mbps: p.measured_mbps,
                    direction: p.direction.clone(),
                    measured_at_ms: p.measured_at_ms,
                    tier: recommend_streaming_params(p.measured_mbps).tier,
                }
            })
            .collect();
        RecommendationStatusReport {
            ok: true,
            peers,
            timestamp_ms: now_ms,
        }
    }
}
