// SPDX-License-Identifier: Apache-2.0 OR MIT
//! File offers, transfers and auto-accept policy.

use super::*;
use crate::events::UiEvent;
use crate::store::{AutoAcceptPolicy, TargetSelector};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use tetherly_core::frame::{FileDone, FileOffer, IncrementalSha256};
use tetherly_core::{CoreError, DeviceId};
use tetherly_net::filechan::read_header as read_file_header;
use tokio::net::TcpStream;
use tokio::sync::oneshot;
use tracing::{info, warn};

/// Single target dispatch outcome for multi-peer file offer fan-out.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PeerFileSendItem {
    pub peer: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peer_alias: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transfer_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outbox_id: Option<String>,
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Comprehensive aggregated report for single-hash multi-peer file offer dispatch.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MultiFileSendReport {
    pub ok: bool,
    pub selector: TargetSelector,
    pub filename: String,
    pub size: u64,
    pub sha256: String,
    pub target_count: usize,
    pub success_count: usize,
    pub skipped_offline_count: usize,
    pub failed_count: usize,
    pub dispatches: Vec<PeerFileSendItem>,
}

/// Detailed status and configuration report for granular file auto-accept policy.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AutoAcceptPolicyReport {
    pub ok: bool,
    pub policy: AutoAcceptPolicy,
    pub auto_accepted_count: u64,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TransferDirection {
    Inbound,
    Outbound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TransferPhase {
    Transferring,
    Verifying,
    Completed,
    Failed,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct ActiveTransfer {
    pub transfer_id: String,
    pub peer: DeviceId,
    pub direction: TransferDirection,
    pub filename: String,
    pub total: u64,
    pub transferred: u64,
    pub speed_bps: u64,
    pub phase: TransferPhase,
    pub error: Option<String>,
    pub started_ms: u64,
    pub updated_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_offset: Option<u64>,
    #[serde(default)]
    pub is_resumed: bool,
}

/// Comprehensive lightweight report of active and retained file transfers.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct TransfersReport {
    pub ok: bool,
    pub active_count: usize,
    pub terminal_count: usize,
    pub total_count: usize,
    pub transfers: Vec<ActiveTransfer>,
    pub stats: TransferStats,
}

/// Comprehensive report of pending incoming file transfer offers.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct FileOffersReport {
    pub ok: bool,
    pub count: usize,
    pub auto_accept_trusted: bool,
    pub offers: Vec<FileOfferDto>,
}

/// Detailed metadata of a single pending file transfer offer.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct FileOfferDto {
    pub transfer_id: String,
    pub filename: String,
    pub total_bytes: u64,
    pub resume_from: u64,
    pub source: String,
    pub peer: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peer_alias: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peer_name: Option<String>,
    pub size: u64,
    pub sha256: String,
    pub timestamp_ms: u64,
    pub offered_ms: u64,
}

/// Result of accepting or rejecting a file transfer offer.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct FileDecisionReport {
    pub ok: bool,
    pub accepted: bool,
    pub action: String,
    pub transfer_id: String,
    pub peer: String,
    pub filename: String,
    pub total_bytes: u64,
    pub resume_from: u64,
    pub timestamp_ms: u64,
}

/// Result of batch accepting or rejecting file transfer offers.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct FileBatchDecisionReport {
    pub ok: bool,
    pub action: String,
    pub accepted_count: usize,
    pub rejected_count: usize,
    pub total_evaluated: usize,
    pub decisions: Vec<FileDecisionReport>,
}

/// Single item within a batch send request.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct BatchSendItem {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_base64: Option<String>,
}

/// Request payload for batch file sending.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct BatchSendRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peer: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<BatchSendItem>,
}

/// Result report for an individual file within a batch send operation.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct BatchTransferItemReport {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transfer_id: Option<String>,
    pub name: String,
    pub size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Aggregate report returned after submitting a batch of files for sending.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct BatchSendReport {
    pub ok: bool,
    pub batch_id: String,
    pub peer: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peer_alias: Option<String>,
    pub total_files: usize,
    pub successful_files: usize,
    pub failed_files: usize,
    pub total_bytes: u64,
    pub items: Vec<BatchTransferItemReport>,
}

/// Status and configuration of auto-accept policy for trusted peers.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct AutoAcceptReport {
    pub ok: bool,
    pub enabled: bool,
    pub auto_accepted_count: u64,
}

/// Session aggregate transfer metrics across all inbound and outbound files.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct TransferStats {
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub completed_count: u64,
    pub failed_count: u64,
}

impl Node {
    pub async fn accept_file(
        &self,
        mut stream: TcpStream,
        _addr: SocketAddr,
    ) -> Result<(), tetherly_net::NetError> {
        // Header is unauthenticated (id/total/start). It only names the
        // transfer; the token (HKDF over the Noise handshake + transfer_id)
        // gates every chunk, so a forged id simply fails to decrypt.
        let (transfer_id, total, start) = read_file_header(&mut stream).await?;
        let (accepted, source, size, resume_hint, name, expected) = {
            let hubs = self.shared.hubs.lock().expect("hubs");
            let t = hubs
                .files
                .get(&transfer_id)
                .ok_or(tetherly_net::NetError::File("unknown".into()))?;
            (
                t.status == tetherly_core::TransferStatus::Accepted,
                t.source.clone(),
                t.files.iter().map(|f| f.size).sum::<u64>(),
                t.resume_from.unwrap_or(0),
                t.files
                    .first()
                    .map(|f| f.name.clone())
                    .unwrap_or_else(|| "file.bin".into()),
                t.files
                    .first()
                    .map(|f| f.sha256.clone())
                    .unwrap_or_default(),
            )
        };
        if !accepted {
            return Err(tetherly_net::NetError::File("not accepted".into()));
        }
        if total != size || start != resume_hint {
            return Err(tetherly_net::NetError::Handshake);
        }
        // The offer's size is authoritative (already capped by the hub), but the
        // channel header is unauthenticated, so re-assert the cap here and hand
        // the channel the real bound instead of `u64::MAX`: the whole body is
        // buffered in memory below, so an unbounded `total` would be an
        // unbounded allocation.
        if total > tetherly_core::FILE_MAX {
            return Err(tetherly_net::NetError::TooLarge);
        }
        let token = self
            .file_token_for(&source, &transfer_id)
            .ok_or(tetherly_net::NetError::File("no session token".into()))?;
        let dest_dir = self.shared.cfg.data_dir.join("inbox");
        std::fs::create_dir_all(&dest_dir)?;
        let part = dest_dir.join(format!(".{}.part", transfer_id));

        let mut hasher = tetherly_core::IncrementalSha256::new();
        // If resuming, verify that the on-disk partial exists and matches start,
        // feeding the prefix [0..start] into the incremental hasher. The prefix
        // read runs off-thread (a multi-GiB re-read would pin a runtime worker);
        // it returns the seeded hasher so the async receive loop continues the
        // same digest without needing state merging.
        if start > 0 {
            let meta = std::fs::metadata(&part)?;
            let aligned = meta.len() - (meta.len() % tetherly_core::FILE_CHUNK as u64);
            if aligned < start {
                return Err(tetherly_net::NetError::Handshake);
            }
            let prefix_part = part.clone();
            let prefix_len = start;
            let prefix_hasher = tokio::task::spawn_blocking(
                move || -> Result<IncrementalSha256, std::io::Error> {
                    let mut hasher = IncrementalSha256::new();
                    use std::io::Read;
                    let mut f = std::fs::File::open(&prefix_part)?;
                    let mut buf = [0u8; tetherly_core::FILE_CHUNK];
                    let mut read_total = 0u64;
                    while read_total < prefix_len {
                        let want = ((prefix_len - read_total) as usize).min(buf.len());
                        f.read_exact(&mut buf[..want])?;
                        hasher.update(&buf[..want]);
                        read_total += want as u64;
                    }
                    Ok(hasher)
                },
            )
            .await
            .map_err(|e| tetherly_net::NetError::File(format!("hash task panicked: {e}")))??;
            hasher = prefix_hasher;
        }
        let mut written = start;
        let now = self.shared.clock.unix_ms();
        let mut cancel_rx = {
            let mut cancels = self.shared.transfer_cancels.lock().expect("cancels");
            let mut transfers = self.shared.active_transfers.lock().expect("transfers");
            if let Some(t) = transfers.get_mut(&transfer_id) {
                if t.phase == TransferPhase::Failed {
                    return Err(tetherly_net::NetError::File("cancelled by user".into()));
                }
                t.transferred = start;
                if start > 0 {
                    t.resume_offset = Some(start);
                    t.is_resumed = true;
                }
                t.phase = TransferPhase::Transferring;
                t.updated_ms = now;
            } else {
                transfers.insert(
                    transfer_id.clone(),
                    ActiveTransfer {
                        transfer_id: transfer_id.clone(),
                        peer: source.clone(),
                        direction: TransferDirection::Inbound,
                        filename: name.clone(),
                        total,
                        transferred: start,
                        speed_bps: 0,
                        phase: TransferPhase::Transferring,
                        error: None,
                        started_ms: now,
                        updated_ms: now,
                        resume_offset: if start > 0 { Some(start) } else { None },
                        is_resumed: start > 0,
                    },
                );
            }
            let (cancel_tx, cancel_rx) = oneshot::channel::<()>();
            cancels.insert(transfer_id.clone(), cancel_tx);
            cancel_rx
        };
        // Open the .part file for writing chunks at offset `start`.
        let mut part_file = {
            let f = std::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(false)
                .open(&part)?;
            f.set_len(start)?;
            f.sync_all()?;
            tokio::fs::OpenOptions::new()
                .write(true)
                .open(&part)
                .await?
        };
        use tokio::io::AsyncSeekExt;
        part_file.seek(std::io::SeekFrom::Start(start)).await?;

        let node = self.clone_handle();
        let tid = transfer_id.clone();
        let recv_fut = tetherly_net::filechan::recv_chunks_to_sink(
            &mut stream,
            &mut part_file,
            &token,
            &transfer_id,
            total,
            start,
            tetherly_core::FILE_MAX,
            hasher,
            |chan_offset, chunk_len| {
                written = chan_offset + chunk_len as u64;
                node.update_transfer_progress(&tid, written);
                node.shared.telemetry.record_file_rx(chunk_len as u64);
                Ok(())
            },
        );

        let res = tokio::select! {
            _ = &mut cancel_rx => {
                self.shared.transfer_cancels.lock().expect("cancels").remove(&transfer_id);
                self.note_resume(&source, &transfer_id, total, written);
                self.set_transfer_phase(
                    &transfer_id,
                    TransferPhase::Failed,
                    Some("cancelled by user".into()),
                );
                return Err(tetherly_net::NetError::File("cancelled by user".into()));
            }
            r = recv_fut => {
                self.shared.transfer_cancels.lock().expect("cancels").remove(&transfer_id);
                r
            }
        };

        if let Err(e) = res {
            // Persist a chunk-aligned resume marker (in memory + sidecar) so the
            // next offer resumes instead of restarting.
            self.note_resume(&source, &transfer_id, total, written);
            self.set_transfer_phase(&transfer_id, TransferPhase::Failed, Some(e.to_string()));
            return Err(e);
        }

        self.set_transfer_phase(&transfer_id, TransferPhase::Verifying, None);

        let computed_digest = tetherly_core::hex_lower(&res.unwrap());
        if computed_digest != expected {
            let _ = std::fs::remove_file(&part);
            self.clear_resume(&transfer_id);
            self.set_transfer_phase(
                &transfer_id,
                TransferPhase::Failed,
                Some("sha256 mismatch".into()),
            );
            return Err(tetherly_net::NetError::File("sha256".into()));
        }

        // Drop part_file so the file handle is released before rename.
        drop(part_file);

        let path = dest_dir.join(&name);
        let _ = std::fs::remove_file(&path);
        std::fs::rename(&part, &path)?;
        self.clear_resume(&transfer_id);
        let done = FileDone {
            transfer_id: transfer_id.clone(),
            sha256: computed_digest,
        };
        {
            let mut hubs = self.shared.hubs.lock().expect("hubs");
            let _ = hubs.files.mark_done(&done);
        }
        let _ = self.send_to(&source, done.to_frame()?).await;
        self.set_transfer_phase(&transfer_id, TransferPhase::Completed, None);
        self.emit(UiEvent::FileDone {
            transfer_id: transfer_id.clone(),
        });
        // Settle any drag session that originated this inbound transfer:
        // applies MOTW and reveals the file in the native file manager.
        self.settle_drag_for_transfer(&transfer_id, &path);
        let banner_title = "已接收文件";
        let banner_body = format!("文件 {name} 接收完成");
        let _ = self.dispatch_notification_banner(
            banner_title,
            &banner_body,
            Some("Tetherly File"),
            false,
        );
        Ok(())
    }

    /// Accepts an incoming file transfer, records the active transfer, dispatches
    /// the accept frame to the peer, and returns a structured decision report.
    pub async fn user_accept_file_reported(
        &self,
        transfer_id: &str,
    ) -> Result<FileDecisionReport, CoreError> {
        let (decision, source, name, total, resume_from) = {
            let mut hubs = self.shared.hubs.lock().expect("hubs");
            let decision = hubs
                .files
                .accept(transfer_id, self.shared.clock.unix_ms())?;
            let t = hubs
                .files
                .get(transfer_id)
                .ok_or(CoreError::UnknownTransfer)?;
            (
                decision,
                t.source.clone(),
                t.files
                    .first()
                    .map(|f| f.name.clone())
                    .unwrap_or_else(|| "file.bin".into()),
                t.files.iter().map(|f| f.size).sum::<u64>(),
                t.resume_from.unwrap_or(0),
            )
        };
        let now = self.shared.clock.unix_ms();
        {
            let mut transfers = self.shared.active_transfers.lock().expect("transfers");
            transfers.insert(
                transfer_id.to_string(),
                ActiveTransfer {
                    transfer_id: transfer_id.to_string(),
                    peer: source.clone(),
                    direction: TransferDirection::Inbound,
                    filename: name.clone(),
                    total,
                    transferred: resume_from,
                    speed_bps: 0,
                    phase: TransferPhase::Transferring,
                    error: None,
                    started_ms: now,
                    updated_ms: now,
                    resume_offset: if resume_from > 0 {
                        Some(resume_from)
                    } else {
                        None
                    },
                    is_resumed: resume_from > 0,
                },
            );
        }
        // Token is derived from the live session handshake hash.
        self.send_to(&source, decision.accept_frame()?).await?;
        Ok(FileDecisionReport {
            ok: true,
            accepted: true,
            action: "accepted".to_string(),
            transfer_id: transfer_id.to_string(),
            peer: source.to_string(),
            filename: name,
            total_bytes: total,
            resume_from,
            timestamp_ms: now,
        })
    }

    pub async fn user_accept_file(&self, transfer_id: &str) -> Result<(), CoreError> {
        self.user_accept_file_reported(transfer_id)
            .await
            .map(|_| ())
    }

    pub fn file_token_for(&self, peer: &DeviceId, transfer_id: &str) -> Option<[u8; 32]> {
        let live = self.shared.live.lock().expect("live");
        live.get(peer)
            .map(|p| tetherly_crypto::file_token(&p.handshake_hash, transfer_id.as_bytes()))
    }

    /// Rejects an incoming file transfer, emits a rejection event, dispatches
    /// the reject frame to the peer, and returns a structured decision report.
    pub async fn user_reject_file_reported(
        &self,
        transfer_id: &str,
    ) -> Result<FileDecisionReport, CoreError> {
        let (decision, source, name, total, resume_from) = {
            let mut hubs = self.shared.hubs.lock().expect("hubs");
            let t = hubs
                .files
                .get(transfer_id)
                .ok_or(CoreError::UnknownTransfer)?;
            let source = t.source.clone();
            let name = t
                .files
                .first()
                .map(|f| f.name.clone())
                .unwrap_or_else(|| "file.bin".into());
            let total = t.files.iter().map(|f| f.size).sum::<u64>();
            let resume_from = t.resume_from.unwrap_or(0);
            let decision = hubs.files.reject(transfer_id)?;
            (decision, source, name, total, resume_from)
        };
        self.set_transfer_phase(
            transfer_id,
            TransferPhase::Failed,
            Some("rejected locally".into()),
        );
        self.emit(UiEvent::FileRejected {
            transfer_id: transfer_id.to_string(),
            source: source.to_string(),
            reason: Some("rejected locally".into()),
        });
        self.send_to(&source, decision.reject_frame()?).await?;
        Ok(FileDecisionReport {
            ok: true,
            accepted: false,
            action: "rejected".to_string(),
            transfer_id: transfer_id.to_string(),
            peer: source.to_string(),
            filename: name,
            total_bytes: total,
            resume_from,
            timestamp_ms: self.shared.clock.unix_ms(),
        })
    }

    pub async fn user_reject_file(&self, transfer_id: &str) -> Result<(), CoreError> {
        self.user_reject_file_reported(transfer_id)
            .await
            .map(|_| ())
    }

    /// Batch accepts pending incoming file offers, optionally filtered by peer.
    pub async fn batch_accept_files(
        &self,
        peer_filter: Option<&str>,
    ) -> Result<FileBatchDecisionReport, CoreError> {
        let target_peer = if let Some(p) = peer_filter {
            match self.resolve_peer(p) {
                Ok(id) => Some(id),
                Err(_) => {
                    return Ok(FileBatchDecisionReport {
                        ok: true,
                        action: "accepted".to_string(),
                        accepted_count: 0,
                        rejected_count: 0,
                        total_evaluated: 0,
                        decisions: Vec::new(),
                    });
                }
            }
        } else {
            None
        };
        let candidates: Vec<String> = {
            let hubs = self.shared.hubs.lock().expect("hubs");
            hubs.files
                .offered()
                .into_iter()
                .filter(|t| match &target_peer {
                    Some(expected) => &t.source == expected,
                    None => true,
                })
                .map(|t| t.transfer_id)
                .collect()
        };
        let mut reports = Vec::with_capacity(candidates.len());
        for tid in &candidates {
            if let Ok(rep) = self.user_accept_file_reported(tid).await {
                reports.push(rep);
            }
        }
        let accepted_count = reports.len();
        Ok(FileBatchDecisionReport {
            ok: true,
            action: "accepted".to_string(),
            accepted_count,
            rejected_count: 0,
            total_evaluated: candidates.len(),
            decisions: reports,
        })
    }

    /// Batch rejects pending incoming file offers, optionally filtered by peer.
    pub async fn batch_reject_files(
        &self,
        peer_filter: Option<&str>,
    ) -> Result<FileBatchDecisionReport, CoreError> {
        let target_peer = if let Some(p) = peer_filter {
            match self.resolve_peer(p) {
                Ok(id) => Some(id),
                Err(_) => {
                    return Ok(FileBatchDecisionReport {
                        ok: true,
                        action: "rejected".to_string(),
                        accepted_count: 0,
                        rejected_count: 0,
                        total_evaluated: 0,
                        decisions: Vec::new(),
                    });
                }
            }
        } else {
            None
        };
        let candidates: Vec<String> = {
            let hubs = self.shared.hubs.lock().expect("hubs");
            hubs.files
                .offered()
                .into_iter()
                .filter(|t| match &target_peer {
                    Some(expected) => &t.source == expected,
                    None => true,
                })
                .map(|t| t.transfer_id)
                .collect()
        };
        let mut reports = Vec::with_capacity(candidates.len());
        for tid in &candidates {
            if let Ok(rep) = self.user_reject_file_reported(tid).await {
                reports.push(rep);
            }
        }
        let rejected_count = reports.len();
        Ok(FileBatchDecisionReport {
            ok: true,
            action: "rejected".to_string(),
            accepted_count: 0,
            rejected_count,
            total_evaluated: candidates.len(),
            decisions: reports,
        })
    }

    /// Queries pending incoming file offers with optional transfer ID or peer filtering.
    pub fn file_offers(
        &self,
        id_filter: Option<&str>,
        peer_filter: Option<&str>,
    ) -> Result<FileOffersReport, CoreError> {
        let target_peer = if let Some(p) = peer_filter {
            match self.resolve_peer(p) {
                Ok(id) => Some(id),
                Err(_) => {
                    return Ok(FileOffersReport {
                        ok: true,
                        count: 0,
                        auto_accept_trusted: self
                            .shared
                            .auto_accept_trusted
                            .load(Ordering::Relaxed),
                        offers: Vec::new(),
                    });
                }
            }
        } else {
            None
        };
        let raw_offers = self.offered_files();
        let trusted_map: HashMap<DeviceId, Option<String>> = {
            let trust = self.shared.trust.lock().expect("trust");
            trust
                .all()
                .into_iter()
                .map(|p| {
                    let alias = if p.alias.is_empty() {
                        None
                    } else {
                        Some(p.alias)
                    };
                    (p.device_id, alias)
                })
                .collect()
        };
        let mut filtered = Vec::new();
        for t in raw_offers {
            if let Some(expected_id) = id_filter {
                if t.transfer_id != expected_id {
                    continue;
                }
            }
            if let Some(ref expected_peer) = target_peer {
                if &t.source != expected_peer {
                    continue;
                }
            }
            let peer_alias = trusted_map.get(&t.source).cloned().flatten();
            let total_bytes: u64 = t.files.iter().map(|f| f.size).sum();
            let (name, sha256) = t
                .files
                .first()
                .map(|f| (f.name.clone(), f.sha256.clone()))
                .unwrap_or_default();
            filtered.push(FileOfferDto {
                transfer_id: t.transfer_id,
                filename: name,
                total_bytes,
                resume_from: t.resume_from.unwrap_or(0),
                source: t.source.to_string(),
                peer: t.source.to_string(),
                peer_alias,
                peer_name: None,
                size: total_bytes,
                sha256,
                timestamp_ms: t.offered_ms,
                offered_ms: t.offered_ms,
            });
        }
        if id_filter.is_some() && filtered.is_empty() {
            return Err(CoreError::UnknownTransfer);
        }
        Ok(FileOffersReport {
            ok: true,
            count: filtered.len(),
            auto_accept_trusted: self.shared.auto_accept_trusted.load(Ordering::Relaxed),
            offers: filtered,
        })
    }

    /// Status and configuration of auto-accept policy for trusted peers.
    pub fn auto_accept_status(&self) -> AutoAcceptReport {
        AutoAcceptReport {
            ok: true,
            enabled: self.shared.auto_accept_trusted.load(Ordering::Relaxed),
            auto_accepted_count: self.shared.auto_accepted_count.load(Ordering::Relaxed),
        }
    }

    /// Read the current granular auto-accept policy.
    pub fn auto_accept_policy(&self) -> AutoAcceptPolicy {
        self.shared
            .auto_accept_policy
            .lock()
            .expect("auto_accept_policy")
            .clone()
    }

    /// Comprehensive status and configuration report for granular file auto-accept policy.
    pub fn auto_accept_policy_report(&self) -> AutoAcceptPolicyReport {
        AutoAcceptPolicyReport {
            ok: true,
            policy: self.auto_accept_policy(),
            auto_accepted_count: self.shared.auto_accepted_count.load(Ordering::Relaxed),
            message: "auto-accept policy report".to_string(),
        }
    }

    /// Update the granular auto-accept policy and persist to disk atomically.
    pub fn set_auto_accept_policy(
        &self,
        policy: AutoAcceptPolicy,
    ) -> Result<AutoAcceptPolicyReport, CoreError> {
        let now = self.shared.clock.unix_ms();
        crate::store::save_auto_accept_policy(&self.shared.cfg.data_dir, &policy)
            .map_err(|e| CoreError::File(format!("failed to save auto_accept.json: {e}")))?;
        self.shared
            .auto_accept_trusted
            .store(policy.enabled, Ordering::Relaxed);
        *self
            .shared
            .auto_accept_policy
            .lock()
            .expect("auto_accept_policy") = policy.clone();

        self.emit(UiEvent::auto_accept_policy_changed(
            policy.enabled,
            policy.allowed_groups.clone(),
            policy.allowed_tags.clone(),
            policy
                .allowed_types
                .iter()
                .map(|t| t.as_str().to_string())
                .collect(),
            policy.max_file_size,
            now,
        ));

        self.record_audit(
            AuditCategory::File,
            AuditSeverity::Info,
            "auto_accept_policy_changed",
            None::<String>,
            format!(
                "Updated file auto-accept policy: enabled={}, groups={:?}, tags={:?}, types={:?}, max_size={:?}",
                policy.enabled,
                policy.allowed_groups,
                policy.allowed_tags,
                policy.allowed_types,
                policy.max_file_size
            ),
            true,
        );

        Ok(AutoAcceptPolicyReport {
            ok: true,
            policy,
            auto_accepted_count: self.shared.auto_accepted_count.load(Ordering::Relaxed),
            message: "auto-accept policy updated successfully".to_string(),
        })
    }

    /// Configures the auto-accept policy for trusted peers and emits `UiEvent::FileAutoAcceptChanged`.
    pub fn set_auto_accept(&self, enabled: bool) -> AutoAcceptReport {
        let mut policy = self.auto_accept_policy();
        policy.enabled = enabled;
        let _ = self.set_auto_accept_policy(policy);
        self.emit(UiEvent::FileAutoAcceptChanged { enabled });
        self.auto_accept_status()
    }

    pub async fn offer_and_send_bytes(
        &self,
        peer: &DeviceId,
        name: &str,
        data: &[u8],
        _wait_accept: bool,
    ) -> Result<String, CoreError> {
        // Symmetric with the receiver's cap: never offer bytes the peer is
        // obliged to refuse, and never clone a body we cannot hold.
        if data.len() as u64 > tetherly_core::FILE_MAX {
            return Err(CoreError::FileTooLarge(tetherly_core::FILE_MAX));
        }
        // Content-addressed: the same bytes re-offered after an interruption
        // carry the same id, so the receiver's persisted partial can be matched
        // and resumed. Different content -> different id -> fresh transfer.
        let digest = tetherly_core::hex_sha256(data);
        let transfer_id = format!("tr_{}", &digest[..16]);
        let offer = FileOffer {
            transfer_id: transfer_id.clone(),
            files: vec![tetherly_core::FileMeta {
                name: name.into(),
                size: data.len() as u64,
                sha256: digest.clone(),
            }],
        };
        self.shared.pending_out.lock().expect("pending").insert(
            transfer_id.clone(),
            OutboundFile {
                peer: peer.clone(),
                name: name.into(),
                source: OutboundSource::Bytes(data.to_vec()),
                size: data.len() as u64,
            },
        );
        let now = self.shared.clock.unix_ms();
        self.shared
            .active_transfers
            .lock()
            .expect("transfers")
            .insert(
                transfer_id.clone(),
                ActiveTransfer {
                    transfer_id: transfer_id.clone(),
                    peer: peer.clone(),
                    direction: TransferDirection::Outbound,
                    filename: name.into(),
                    total: data.len() as u64,
                    transferred: 0,
                    speed_bps: 0,
                    phase: TransferPhase::Transferring,
                    error: None,
                    started_ms: now,
                    updated_ms: now,
                    resume_offset: None,
                    is_resumed: false,
                },
            );
        let outbox_id = format!("out_{transfer_id}_{now}");
        let alias = self.peer_alias(peer);
        self.shared
            .outbox_engine
            .lock()
            .expect("outbox_engine")
            .push(OutboxEntry {
                id: outbox_id.clone(),
                transfer_id: transfer_id.clone(),
                peer: peer.to_string(),
                peer_alias: alias,
                filename: name.to_string(),
                size: data.len() as u64,
                sha256: digest,
                source_path: None,
                status: OutboxStatus::InFlight,
                error: None,
                started_ms: now,
                completed_ms: None,
                speed_bps: 0,
            });
        self.emit(UiEvent::outbox_updated(
            outbox_id,
            &transfer_id,
            peer.to_string(),
            name,
            data.len() as u64,
            OutboxStatus::InFlight.as_str(),
            now,
        ));
        self.send_to(peer, offer.to_frame()?).await?;
        let _ = _wait_accept;
        Ok(transfer_id)
    }

    /// Offer and stream a file directly from a filesystem path. Reads the file
    /// in 64 KiB chunks for incremental SHA-256 calculation and outbound streaming,
    /// avoiding full in-memory buffering (O(1) memory bound).
    pub async fn offer_and_send_file(
        &self,
        peer: &DeviceId,
        path: &std::path::Path,
        name: Option<&str>,
    ) -> Result<String, CoreError> {
        if !path.is_file() {
            return Err(CoreError::File("not a regular file".into()));
        }
        let meta = std::fs::metadata(path)?;
        let size = meta.len();
        if size > tetherly_core::FILE_MAX {
            return Err(CoreError::FileTooLarge(tetherly_core::FILE_MAX));
        }
        let filename = name
            .map(|s| s.to_string())
            .or_else(|| {
                path.file_name()
                    .and_then(|n| n.to_str())
                    .map(|s| s.to_string())
            })
            .unwrap_or_else(|| "file.bin".to_string());
        let safe_name = tetherly_core::sanitize_file_name(&filename)?;

        // Stream file in 64 KiB chunks to compute SHA-256 with O(1) memory.
        // Off-thread: a whole-file read can stall a tokio worker for seconds
        // on large payloads (up to FILE_MAX), starving every other task on it.
        let hash_path = path.to_path_buf();
        let digest = tokio::task::spawn_blocking(move || -> Result<String, std::io::Error> {
            let mut hasher = tetherly_core::IncrementalSha256::new();
            use std::io::Read;
            let mut f = std::fs::File::open(&hash_path)?;
            let mut buf = [0u8; tetherly_core::FILE_CHUNK];
            loop {
                let n = f.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                hasher.update(&buf[..n]);
            }
            Ok(hasher.finalize_hex())
        })
        .await
        .map_err(|e| CoreError::File(format!("hash task panicked: {e}")))??;
        let transfer_id = format!("tr_{}", &digest[..16]);
        let offer = FileOffer {
            transfer_id: transfer_id.clone(),
            files: vec![tetherly_core::FileMeta {
                name: safe_name.clone(),
                size,
                sha256: digest.clone(),
            }],
        };
        self.shared.pending_out.lock().expect("pending").insert(
            transfer_id.clone(),
            OutboundFile {
                peer: peer.clone(),
                name: safe_name.clone(),
                source: OutboundSource::Path(path.to_path_buf()),
                size,
            },
        );
        let now = self.shared.clock.unix_ms();
        self.shared
            .active_transfers
            .lock()
            .expect("transfers")
            .insert(
                transfer_id.clone(),
                ActiveTransfer {
                    transfer_id: transfer_id.clone(),
                    peer: peer.clone(),
                    direction: TransferDirection::Outbound,
                    filename: safe_name.clone(),
                    total: size,
                    transferred: 0,
                    speed_bps: 0,
                    phase: TransferPhase::Transferring,
                    error: None,
                    started_ms: now,
                    updated_ms: now,
                    resume_offset: None,
                    is_resumed: false,
                },
            );
        let outbox_id = format!("out_{transfer_id}_{now}");
        let alias = self.peer_alias(peer);
        self.shared
            .outbox_engine
            .lock()
            .expect("outbox_engine")
            .push(OutboxEntry {
                id: outbox_id.clone(),
                transfer_id: transfer_id.clone(),
                peer: peer.to_string(),
                peer_alias: alias,
                filename: safe_name.clone(),
                size,
                sha256: digest,
                source_path: Some(path.to_string_lossy().to_string()),
                status: OutboxStatus::InFlight,
                error: None,
                started_ms: now,
                completed_ms: None,
                speed_bps: 0,
            });
        self.emit(UiEvent::outbox_updated(
            outbox_id,
            &transfer_id,
            peer.to_string(),
            &safe_name,
            size,
            OutboxStatus::InFlight.as_str(),
            now,
        ));
        self.send_to(peer, offer.to_frame()?).await?;
        Ok(transfer_id)
    }

    /// Offer and stream a file to multiple targets matched by a `TargetSelector`.
    /// Performs single-pass file reading and SHA-256 calculation (O(1) memory bound),
    /// generates unique transfer IDs and Outbox entries for each online peer,
    /// sends FileOffer frames concurrently, and produces an aggregated MultiFileSendReport.
    pub async fn offer_and_send_file_multi(
        &self,
        selector: &TargetSelector,
        path: &std::path::Path,
        name: Option<&str>,
    ) -> Result<MultiFileSendReport, CoreError> {
        if !path.is_file() {
            return Err(CoreError::File("not a regular file".into()));
        }
        let meta = std::fs::metadata(path)?;
        let size = meta.len();
        if size > tetherly_core::FILE_MAX {
            return Err(CoreError::FileTooLarge(tetherly_core::FILE_MAX));
        }
        let filename = name
            .map(|s| s.to_string())
            .or_else(|| {
                path.file_name()
                    .and_then(|n| n.to_str())
                    .map(|s| s.to_string())
            })
            .unwrap_or_else(|| "file.bin".to_string());
        let safe_name = tetherly_core::sanitize_file_name(&filename)?;

        // Single-pass stream in 64 KiB chunks to compute SHA-256 with O(1) memory
        // bound. Off-thread so a large read cannot stall the async runtime.
        let hash_path = path.to_path_buf();
        let digest = tokio::task::spawn_blocking(move || -> Result<String, std::io::Error> {
            let mut hasher = tetherly_core::IncrementalSha256::new();
            use std::io::Read;
            let mut f = std::fs::File::open(&hash_path)?;
            let mut buf = [0u8; tetherly_core::FILE_CHUNK];
            loop {
                let n = f.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                hasher.update(&buf[..n]);
            }
            Ok(hasher.finalize_hex())
        })
        .await
        .map_err(|e| CoreError::File(format!("hash task panicked: {e}")))??;

        let targets = self.resolve_dispatch_targets(selector)?;
        let live = self.live_peers();
        let live_map: std::collections::HashSet<DeviceId> =
            live.into_iter().map(|p| p.device_id).collect();

        let now = self.shared.clock.unix_ms();
        static MULTI_TRANSFER_COUNTER: AtomicU64 = AtomicU64::new(1);

        let mut dispatches = Vec::with_capacity(targets.len());
        let mut success_count = 0;
        let mut skipped_offline_count = 0;
        let mut failed_count = 0;

        for peer_id in &targets {
            let peer_str = peer_id.to_string();
            let alias = self.peer_alias(peer_id);

            if !live_map.contains(peer_id) {
                skipped_offline_count += 1;
                dispatches.push(PeerFileSendItem {
                    peer: peer_str,
                    peer_alias: alias,
                    transfer_id: None,
                    outbox_id: None,
                    success: false,
                    error: Some("peer is offline".to_string()),
                });
                continue;
            }

            let seq = MULTI_TRANSFER_COUNTER.fetch_add(1, Ordering::Relaxed);
            let peer_short = &peer_id.as_str()[..8.min(peer_id.as_str().len())];
            let transfer_id = format!("tr_{}_{}_{seq}", &digest[..10], peer_short);
            let offer = FileOffer {
                transfer_id: transfer_id.clone(),
                files: vec![tetherly_core::FileMeta {
                    name: safe_name.clone(),
                    size,
                    sha256: digest.clone(),
                }],
            };

            self.shared.pending_out.lock().expect("pending").insert(
                transfer_id.clone(),
                OutboundFile {
                    peer: peer_id.clone(),
                    name: safe_name.clone(),
                    source: OutboundSource::Path(path.to_path_buf()),
                    size,
                },
            );

            self.shared
                .active_transfers
                .lock()
                .expect("transfers")
                .insert(
                    transfer_id.clone(),
                    ActiveTransfer {
                        transfer_id: transfer_id.clone(),
                        peer: peer_id.clone(),
                        direction: TransferDirection::Outbound,
                        filename: safe_name.clone(),
                        total: size,
                        transferred: 0,
                        speed_bps: 0,
                        phase: TransferPhase::Transferring,
                        error: None,
                        started_ms: now,
                        updated_ms: now,
                        resume_offset: None,
                        is_resumed: false,
                    },
                );

            let outbox_id = format!("out_{transfer_id}_{now}");
            self.shared
                .outbox_engine
                .lock()
                .expect("outbox_engine")
                .push(OutboxEntry {
                    id: outbox_id.clone(),
                    transfer_id: transfer_id.clone(),
                    peer: peer_str.clone(),
                    peer_alias: alias.clone(),
                    filename: safe_name.clone(),
                    size,
                    sha256: digest.clone(),
                    source_path: Some(path.to_string_lossy().to_string()),
                    status: OutboxStatus::InFlight,
                    error: None,
                    started_ms: now,
                    completed_ms: None,
                    speed_bps: 0,
                });

            self.emit(UiEvent::outbox_updated(
                outbox_id.clone(),
                &transfer_id,
                peer_id.to_string(),
                &safe_name,
                size,
                OutboxStatus::InFlight.as_str(),
                now,
            ));

            let frame_res = offer.to_frame();
            let send_res = match frame_res {
                Ok(frame) => self.send_to(peer_id, frame).await,
                Err(e) => Err(e),
            };

            match send_res {
                Ok(_) => {
                    success_count += 1;
                    dispatches.push(PeerFileSendItem {
                        peer: peer_str,
                        peer_alias: alias,
                        transfer_id: Some(transfer_id),
                        outbox_id: Some(outbox_id),
                        success: true,
                        error: None,
                    });
                }
                Err(e) => {
                    failed_count += 1;
                    let err_str = e.to_string();
                    self.shared
                        .pending_out
                        .lock()
                        .expect("pending")
                        .remove(&transfer_id);
                    if let Ok(mut engine) = self.shared.outbox_engine.lock() {
                        engine.update_status(
                            &transfer_id,
                            OutboxStatus::Failed,
                            Some(err_str.clone()),
                            Some(now),
                            Some(0),
                        );
                    }
                    if let Ok(mut transfers) = self.shared.active_transfers.lock() {
                        if let Some(t) = transfers.get_mut(&transfer_id) {
                            t.phase = TransferPhase::Failed;
                            t.error = Some(err_str.clone());
                            t.updated_ms = now;
                        }
                    }
                    dispatches.push(PeerFileSendItem {
                        peer: peer_str,
                        peer_alias: alias,
                        transfer_id: Some(transfer_id),
                        outbox_id: Some(outbox_id),
                        success: false,
                        error: Some(err_str),
                    });
                }
            }
        }

        self.record_audit(
            AuditCategory::File,
            AuditSeverity::Info,
            "file_multi_sent",
            None::<String>,
            format!(
                "Multicast file '{}' ({size} bytes) via {} (targets: {}, success: {}, skipped: {}, failed: {})",
                safe_name,
                selector.as_label(),
                targets.len(),
                success_count,
                skipped_offline_count,
                failed_count
            ),
            failed_count == 0,
        );

        self.emit(UiEvent::multi_dispatch_completed(
            "file",
            selector.as_label(),
            targets.len(),
            success_count,
            skipped_offline_count,
            failed_count,
            now,
        ));

        Ok(MultiFileSendReport {
            ok: success_count > 0 || (targets.is_empty() && skipped_offline_count == 0),
            selector: selector.clone(),
            filename: safe_name,
            size,
            sha256: digest,
            target_count: targets.len(),
            success_count,
            skipped_offline_count,
            failed_count,
            dispatches,
        })
    }

    pub async fn offer_and_send_bytes_multi(
        &self,
        selector: &TargetSelector,
        name: &str,
        data: &[u8],
    ) -> Result<MultiFileSendReport, CoreError> {
        let size = data.len() as u64;
        if size > tetherly_core::FILE_MAX {
            return Err(CoreError::FileTooLarge(tetherly_core::FILE_MAX));
        }
        let safe_name = tetherly_core::sanitize_file_name(name)?;
        let digest = tetherly_core::hex_sha256(data);

        let targets = self.resolve_dispatch_targets(selector)?;
        let live = self.live_peers();
        let live_map: std::collections::HashSet<DeviceId> =
            live.into_iter().map(|p| p.device_id).collect();

        let now = self.shared.clock.unix_ms();
        static MULTI_BYTES_COUNTER: AtomicU64 = AtomicU64::new(1);

        let mut dispatches = Vec::with_capacity(targets.len());
        let mut success_count = 0;
        let mut skipped_offline_count = 0;
        let mut failed_count = 0;

        for peer_id in &targets {
            let peer_str = peer_id.to_string();
            let alias = self.peer_alias(peer_id);

            if !live_map.contains(peer_id) {
                skipped_offline_count += 1;
                dispatches.push(PeerFileSendItem {
                    peer: peer_str,
                    peer_alias: alias,
                    transfer_id: None,
                    outbox_id: None,
                    success: false,
                    error: Some("peer is offline".to_string()),
                });
                continue;
            }

            let seq = MULTI_BYTES_COUNTER.fetch_add(1, Ordering::Relaxed);
            let peer_short = &peer_id.as_str()[..8.min(peer_id.as_str().len())];
            let transfer_id = format!("tr_{}_{}_{seq}", &digest[..10], peer_short);
            let offer = FileOffer {
                transfer_id: transfer_id.clone(),
                files: vec![tetherly_core::FileMeta {
                    name: safe_name.clone(),
                    size,
                    sha256: digest.clone(),
                }],
            };

            self.shared.pending_out.lock().expect("pending").insert(
                transfer_id.clone(),
                OutboundFile {
                    peer: peer_id.clone(),
                    name: safe_name.clone(),
                    source: OutboundSource::Bytes(data.to_vec()),
                    size,
                },
            );

            self.shared
                .active_transfers
                .lock()
                .expect("transfers")
                .insert(
                    transfer_id.clone(),
                    ActiveTransfer {
                        transfer_id: transfer_id.clone(),
                        peer: peer_id.clone(),
                        direction: TransferDirection::Outbound,
                        filename: safe_name.clone(),
                        total: size,
                        transferred: 0,
                        speed_bps: 0,
                        phase: TransferPhase::Transferring,
                        error: None,
                        started_ms: now,
                        updated_ms: now,
                        resume_offset: None,
                        is_resumed: false,
                    },
                );

            let outbox_id = format!("out_{transfer_id}_{now}");
            self.shared
                .outbox_engine
                .lock()
                .expect("outbox_engine")
                .push(OutboxEntry {
                    id: outbox_id.clone(),
                    transfer_id: transfer_id.clone(),
                    peer: peer_str.clone(),
                    peer_alias: alias.clone(),
                    filename: safe_name.clone(),
                    size,
                    sha256: digest.clone(),
                    source_path: None,
                    status: OutboxStatus::InFlight,
                    error: None,
                    started_ms: now,
                    completed_ms: None,
                    speed_bps: 0,
                });

            self.emit(UiEvent::outbox_updated(
                outbox_id.clone(),
                &transfer_id,
                peer_id.to_string(),
                &safe_name,
                size,
                OutboxStatus::InFlight.as_str(),
                now,
            ));

            let frame_res = offer.to_frame();
            let send_res = match frame_res {
                Ok(frame) => self.send_to(peer_id, frame).await,
                Err(e) => Err(e),
            };

            match send_res {
                Ok(_) => {
                    success_count += 1;
                    dispatches.push(PeerFileSendItem {
                        peer: peer_str,
                        peer_alias: alias,
                        transfer_id: Some(transfer_id),
                        outbox_id: Some(outbox_id),
                        success: true,
                        error: None,
                    });
                }
                Err(e) => {
                    failed_count += 1;
                    let err_str = e.to_string();
                    self.shared
                        .pending_out
                        .lock()
                        .expect("pending")
                        .remove(&transfer_id);
                    if let Ok(mut engine) = self.shared.outbox_engine.lock() {
                        engine.update_status(
                            &transfer_id,
                            OutboxStatus::Failed,
                            Some(err_str.clone()),
                            Some(now),
                            Some(0),
                        );
                    }
                    if let Ok(mut transfers) = self.shared.active_transfers.lock() {
                        if let Some(t) = transfers.get_mut(&transfer_id) {
                            t.phase = TransferPhase::Failed;
                            t.error = Some(err_str.clone());
                            t.updated_ms = now;
                        }
                    }
                    dispatches.push(PeerFileSendItem {
                        peer: peer_str,
                        peer_alias: alias,
                        transfer_id: Some(transfer_id),
                        outbox_id: Some(outbox_id),
                        success: false,
                        error: Some(err_str),
                    });
                }
            }
        }

        self.record_audit(
            AuditCategory::File,
            AuditSeverity::Info,
            "file_multi_sent",
            None::<String>,
            format!(
                "Multicast in-memory file '{}' ({size} bytes) via {} (targets: {}, success: {}, skipped: {}, failed: {})",
                safe_name,
                selector.as_label(),
                targets.len(),
                success_count,
                skipped_offline_count,
                failed_count
            ),
            failed_count == 0,
        );

        self.emit(UiEvent::multi_dispatch_completed(
            "file",
            selector.as_label(),
            targets.len(),
            success_count,
            skipped_offline_count,
            failed_count,
            now,
        ));

        Ok(MultiFileSendReport {
            ok: success_count > 0 || (targets.is_empty() && skipped_offline_count == 0),
            selector: selector.clone(),
            filename: safe_name,
            size,
            sha256: digest,
            target_count: targets.len(),
            success_count,
            skipped_offline_count,
            failed_count,
            dispatches,
        })
    }

    /// Offer and stream multiple files as a batch operation to a connected peer.
    pub async fn send_files_batch(
        &self,
        peer: &DeviceId,
        paths: &[std::path::PathBuf],
    ) -> Result<BatchSendReport, CoreError> {
        let items: Vec<BatchSendItem> = paths
            .iter()
            .map(|p| BatchSendItem {
                path: Some(p.to_string_lossy().to_string()),
                name: None,
                data_base64: None,
            })
            .collect();
        self.send_batch_items(peer, items).await
    }

    /// Submit a heterogeneous batch of file paths or in-memory base64 buffers to a peer.
    pub async fn send_batch_items(
        &self,
        peer: &DeviceId,
        items: Vec<BatchSendItem>,
    ) -> Result<BatchSendReport, CoreError> {
        let total_files = items.len();
        if total_files == 0 {
            return Err(CoreError::File("batch items list is empty".into()));
        }
        let batch_hex = tetherly_core::hex_sha256(
            format!("{}:{}:{}", peer, self.shared.clock.unix_ms(), total_files).as_bytes(),
        );
        let batch_id = format!("tbt_{}", &batch_hex[..12]);
        let mut reports = Vec::with_capacity(total_files);
        let mut total_bytes: u64 = 0;
        let mut successful_files = 0;
        let mut failed_files = 0;

        for item in items {
            if let Some(ref path_str) = item.path {
                let p = std::path::Path::new(path_str);
                let fname = item.name.clone().unwrap_or_else(|| {
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("file.bin")
                        .to_string()
                });
                let safe_name = match tetherly_core::sanitize_file_name(&fname) {
                    Ok(n) => n,
                    Err(e) => {
                        reports.push(BatchTransferItemReport {
                            transfer_id: None,
                            name: fname,
                            size: 0,
                            sha256: None,
                            status: "failed".to_string(),
                            error: Some(format!("invalid file name: {e}")),
                        });
                        failed_files += 1;
                        continue;
                    }
                };
                if !p.is_file() {
                    reports.push(BatchTransferItemReport {
                        transfer_id: None,
                        name: safe_name,
                        size: 0,
                        sha256: None,
                        status: "failed".to_string(),
                        error: Some("not a regular file or file not found".into()),
                    });
                    failed_files += 1;
                    continue;
                }
                let meta = match std::fs::metadata(p) {
                    Ok(m) => m,
                    Err(e) => {
                        reports.push(BatchTransferItemReport {
                            transfer_id: None,
                            name: safe_name,
                            size: 0,
                            sha256: None,
                            status: "failed".to_string(),
                            error: Some(format!("cannot read file metadata: {e}")),
                        });
                        failed_files += 1;
                        continue;
                    }
                };
                let size = meta.len();
                if size > tetherly_core::FILE_MAX {
                    reports.push(BatchTransferItemReport {
                        transfer_id: None,
                        name: safe_name,
                        size,
                        sha256: None,
                        status: "failed".to_string(),
                        error: Some("file exceeds FILE_MAX (64 GiB)".into()),
                    });
                    failed_files += 1;
                    continue;
                }
                match self.offer_and_send_file(peer, p, Some(&safe_name)).await {
                    Ok(transfer_id) => {
                        let sha = transfer_id.strip_prefix("tr_").map(|s| s.to_string());
                        reports.push(BatchTransferItemReport {
                            transfer_id: Some(transfer_id),
                            name: safe_name,
                            size,
                            sha256: sha,
                            status: "sent".to_string(),
                            error: None,
                        });
                        successful_files += 1;
                        total_bytes += size;
                    }
                    Err(e) => {
                        reports.push(BatchTransferItemReport {
                            transfer_id: None,
                            name: safe_name,
                            size,
                            sha256: None,
                            status: "failed".to_string(),
                            error: Some(e.to_string()),
                        });
                        failed_files += 1;
                    }
                }
            } else if let Some(ref b64) = item.data_base64 {
                use base64::Engine;
                let decoded = match base64::engine::general_purpose::STANDARD.decode(b64.as_bytes())
                {
                    Ok(b) => b,
                    Err(e) => {
                        reports.push(BatchTransferItemReport {
                            transfer_id: None,
                            name: item.name.clone().unwrap_or_else(|| "payload.bin".into()),
                            size: 0,
                            sha256: None,
                            status: "failed".to_string(),
                            error: Some(format!("invalid base64: {e}")),
                        });
                        failed_files += 1;
                        continue;
                    }
                };
                let fname = item
                    .name
                    .clone()
                    .unwrap_or_else(|| "upload.bin".to_string());
                let safe_name = match tetherly_core::sanitize_file_name(&fname) {
                    Ok(n) => n,
                    Err(e) => {
                        reports.push(BatchTransferItemReport {
                            transfer_id: None,
                            name: fname,
                            size: 0,
                            sha256: None,
                            status: "failed".to_string(),
                            error: Some(format!("invalid file name: {e}")),
                        });
                        failed_files += 1;
                        continue;
                    }
                };
                let size = decoded.len() as u64;
                if size > tetherly_core::FILE_MAX {
                    reports.push(BatchTransferItemReport {
                        transfer_id: None,
                        name: safe_name,
                        size,
                        sha256: None,
                        status: "failed".to_string(),
                        error: Some("file exceeds FILE_MAX (64 GiB)".into()),
                    });
                    failed_files += 1;
                    continue;
                }
                let digest = tetherly_core::hex_sha256(&decoded);
                match self
                    .offer_and_send_bytes(peer, &safe_name, &decoded, false)
                    .await
                {
                    Ok(transfer_id) => {
                        reports.push(BatchTransferItemReport {
                            transfer_id: Some(transfer_id),
                            name: safe_name,
                            size,
                            sha256: Some(digest),
                            status: "sent".to_string(),
                            error: None,
                        });
                        successful_files += 1;
                        total_bytes += size;
                    }
                    Err(e) => {
                        reports.push(BatchTransferItemReport {
                            transfer_id: None,
                            name: safe_name,
                            size,
                            sha256: Some(digest),
                            status: "failed".to_string(),
                            error: Some(e.to_string()),
                        });
                        failed_files += 1;
                    }
                }
            } else {
                reports.push(BatchTransferItemReport {
                    transfer_id: None,
                    name: item.name.clone().unwrap_or_else(|| "unknown".into()),
                    size: 0,
                    sha256: None,
                    status: "failed".to_string(),
                    error: Some("either path or data_base64 must be provided".into()),
                });
                failed_files += 1;
            }
        }

        let peer_alias = self.peer_alias(peer);
        Ok(BatchSendReport {
            ok: successful_files > 0 || total_files == 0,
            batch_id,
            peer: peer.to_string(),
            peer_alias,
            total_files,
            successful_files,
            failed_files,
            total_bytes,
            items: reports,
        })
    }

    pub(super) async fn on_peer_accepted_file(&self, decision: &tetherly_core::FileDecision) {
        let transfer_id = decision.transfer_id.as_str();
        let (outbound, cancel_rx) = {
            let mut cancels = self.shared.transfer_cancels.lock().expect("cancels");
            let mut pending = self.shared.pending_out.lock().expect("pending");
            let mut transfers = self.shared.active_transfers.lock().expect("transfers");

            let Some(t) = transfers.get_mut(transfer_id) else {
                return;
            };
            if t.phase == TransferPhase::Failed {
                // User cancelled before acceptance frame arrived
                pending.remove(transfer_id);
                return;
            }
            let Some(outbound) = pending.remove(transfer_id) else {
                return;
            };

            let start = decision.resume_from.unwrap_or(0);
            if start > outbound.size {
                warn!(transfer = %transfer_id, "resume offset beyond local data; aborting");
                t.phase = TransferPhase::Failed;
                t.error = Some("invalid resume offset".into());
                t.speed_bps = 0;
                t.updated_ms = self.shared.clock.unix_ms();
                return;
            }

            let now = self.shared.clock.unix_ms();
            t.transferred = start;
            if start > 0 {
                t.resume_offset = Some(start);
                t.is_resumed = true;
            }
            t.phase = TransferPhase::Transferring;
            t.updated_ms = now;

            let (cancel_tx, cancel_rx) = oneshot::channel::<()>();
            cancels.insert(transfer_id.to_string(), cancel_tx);
            (outbound, cancel_rx)
        };
        let (addr, file_port, token) = {
            let live = self.shared.live.lock().expect("live");
            let Some(peer) = live.get(&outbound.peer) else {
                warn!(transfer = %transfer_id, "accepted file but peer gone");
                self.set_transfer_phase(
                    transfer_id,
                    TransferPhase::Failed,
                    Some("peer gone".into()),
                );
                return;
            };
            let token = tetherly_crypto::file_token(&peer.handshake_hash, transfer_id.as_bytes());
            (peer.addr, peer.file_port.unwrap_or(self.file_port()), token)
        };
        let start = decision.resume_from.unwrap_or(0);
        let mut cancel_rx = cancel_rx;
        let node = self.clone_handle();
        let tid = transfer_id.to_string();
        tokio::spawn(async move {
            let send_fut = async {
                match outbound.source {
                    OutboundSource::Path(p) => match tokio::fs::File::open(&p).await {
                        Ok(mut f) => {
                            if start > 0 {
                                use tokio::io::AsyncSeekExt;
                                if let Err(e) = f.seek(std::io::SeekFrom::Start(start)).await {
                                    warn!(%e, transfer = %tid, "failed to seek in source file");
                                    return Err(tetherly_net::NetError::Io(e));
                                }
                            }
                            let dest = SocketAddr::new(addr.ip(), file_port);
                            match TcpStream::connect(dest).await {
                                Ok(mut stream) => {
                                    let node_prog = node.clone_handle();
                                    let tid_prog = tid.clone();
                                    tetherly_net::filechan::send_stream_from_with_progress(
                                        &mut f,
                                        &mut stream,
                                        &token,
                                        &tid,
                                        outbound.size,
                                        start,
                                        move |offset, len| {
                                            node_prog.update_transfer_progress(
                                                &tid_prog,
                                                offset + len as u64,
                                            );
                                            node_prog.shared.telemetry.record_file_tx(len as u64);
                                            Ok(())
                                        },
                                    )
                                    .await
                                }
                                Err(e) => Err(tetherly_net::NetError::Io(e)),
                            }
                        }
                        Err(e) => Err(tetherly_net::NetError::Io(e)),
                    },
                    OutboundSource::Bytes(ref b) => {
                        node.send_file_data(addr, file_port, &token, &tid, outbound.size, start, b)
                            .await
                    }
                }
            };

            let res = tokio::select! {
                _ = &mut cancel_rx => {
                    info!(transfer = %tid, "outbound transfer cancelled by user");
                    node.shared.transfer_cancels.lock().expect("cancels").remove(&tid);
                    node.set_transfer_phase(
                        &tid,
                        TransferPhase::Failed,
                        Some("cancelled by user".into()),
                    );
                    return;
                }
                r = send_fut => {
                    node.shared.transfer_cancels.lock().expect("cancels").remove(&tid);
                    r
                }
            };

            match res {
                Ok(()) => {
                    node.set_transfer_phase(&tid, TransferPhase::Completed, None);
                }
                Err(ref e) => {
                    warn!(%e, transfer = %tid, "file data send failed");
                    node.set_transfer_phase(&tid, TransferPhase::Failed, Some(e.to_string()));
                }
            }
        });
    }

    /// Send file bytes from `start` (the resume offset) to the end of `data`,
    /// where `data` holds the whole file and `total` is its length on the wire.
    #[allow(clippy::too_many_arguments)]
    pub async fn send_file_data(
        &self,
        peer_addr: SocketAddr,
        file_port: u16,
        token: &[u8; 32],
        transfer_id: &str,
        total: u64,
        start: u64,
        data: &[u8],
    ) -> Result<(), tetherly_net::NetError> {
        let dest = SocketAddr::new(peer_addr.ip(), file_port);
        let mut stream = TcpStream::connect(dest).await?;
        let res = tetherly_net::filechan::send_chunks_from(
            &mut stream,
            token,
            transfer_id,
            total,
            start,
            &data[start as usize..],
        )
        .await;
        if res.is_ok() {
            self.update_transfer_progress(transfer_id, total);
            self.shared
                .telemetry
                .record_file_tx(total.saturating_sub(start));
        }
        res
    }

    pub fn offered_files(&self) -> Vec<tetherly_core::IncomingTransfer> {
        self.shared.hubs.lock().expect("hubs").files.offered()
    }

    pub fn update_transfer_progress(&self, transfer_id: &str, transferred: u64) {
        let now = self.shared.clock.unix_ms();
        let mut transfers = self.shared.active_transfers.lock().expect("transfers");
        if let Some(t) = transfers.get_mut(transfer_id) {
            if t.phase == TransferPhase::Failed {
                return;
            }
            let delta_bytes = transferred.saturating_sub(t.transferred);
            let delta_ms = now.saturating_sub(t.updated_ms);
            if delta_ms > 0 && delta_bytes > 0 {
                let inst_speed = (delta_bytes * 1000) / delta_ms;
                t.speed_bps = if t.speed_bps == 0 {
                    inst_speed
                } else {
                    (t.speed_bps * 7 + inst_speed * 3) / 10
                };
            }
            t.transferred = transferred;
            t.updated_ms = now;
        }
    }

    pub(super) fn prune_terminal_transfers_locked(transfers: &mut HashMap<String, ActiveTransfer>) {
        let mut terminal: Vec<(String, u64)> = transfers
            .iter()
            .filter(|(_, t)| {
                t.phase == TransferPhase::Completed || t.phase == TransferPhase::Failed
            })
            .map(|(id, t)| (id.clone(), t.updated_ms))
            .collect();
        if terminal.len() > MAX_TERMINAL_TRANSFERS {
            terminal.sort_by_key(|(_, ts)| *ts);
            let excess = terminal.len() - MAX_TERMINAL_TRANSFERS;
            for (id, _) in terminal.into_iter().take(excess) {
                transfers.remove(&id);
            }
        }
    }

    pub fn set_transfer_phase(
        &self,
        transfer_id: &str,
        phase: TransferPhase,
        error: Option<String>,
    ) {
        let now = self.shared.clock.unix_ms();
        let outbox_info = {
            let mut transfers = self.shared.active_transfers.lock().expect("transfers");
            if let Some(t) = transfers.get_mut(transfer_id) {
                if t.phase == TransferPhase::Failed && phase != TransferPhase::Failed {
                    return;
                }
                let was_in_flight =
                    t.phase == TransferPhase::Transferring || t.phase == TransferPhase::Verifying;
                if was_in_flight {
                    let mut stats = self.shared.transfer_stats.lock().expect("transfer_stats");
                    match phase {
                        TransferPhase::Completed => {
                            match t.direction {
                                TransferDirection::Outbound => {
                                    stats.bytes_sent = stats.bytes_sent.saturating_add(t.total);
                                }
                                TransferDirection::Inbound => {
                                    stats.bytes_received =
                                        stats.bytes_received.saturating_add(t.total);
                                }
                            }
                            stats.completed_count = stats.completed_count.saturating_add(1);
                        }
                        TransferPhase::Failed => {
                            stats.failed_count = stats.failed_count.saturating_add(1);
                        }
                        _ => {}
                    }
                }
                let speed = if phase == TransferPhase::Completed || phase == TransferPhase::Failed {
                    let dur = now.saturating_sub(t.started_ms);
                    if dur > 0 && t.transferred > 0 {
                        (t.transferred * 1000) / dur
                    } else {
                        0
                    }
                } else {
                    t.speed_bps
                };
                if phase == TransferPhase::Completed || phase == TransferPhase::Failed {
                    t.speed_bps = 0;
                }
                t.phase = phase;
                t.error = error.clone();
                t.updated_ms = now;

                let is_outbound = t.direction == TransferDirection::Outbound;
                let peer_str = t.peer.to_string();
                let filename = t.filename.clone();
                let size = t.total;

                if t.phase == TransferPhase::Completed || t.phase == TransferPhase::Failed {
                    Self::prune_terminal_transfers_locked(&mut transfers);
                }

                if is_outbound {
                    Some((peer_str, filename, size, speed))
                } else {
                    None
                }
            } else {
                None
            }
        };

        if let Some((peer_str, filename, size, speed)) = outbox_info {
            let outbox_status = match phase {
                TransferPhase::Completed => Some(OutboxStatus::Completed),
                TransferPhase::Failed => {
                    if error.as_deref() == Some("rejected by peer") {
                        Some(OutboxStatus::Rejected)
                    } else if error.as_deref() == Some("cancelled by user") {
                        Some(OutboxStatus::Cancelled)
                    } else {
                        Some(OutboxStatus::Failed)
                    }
                }
                _ => None,
            };
            if let Some(status) = outbox_status {
                let outbox_id = format!("out_{}", transfer_id);
                {
                    let mut outbox = self.shared.outbox_engine.lock().expect("outbox_engine");
                    outbox.update_status(
                        transfer_id,
                        status,
                        error.clone(),
                        Some(now),
                        Some(speed),
                    );
                }
                self.emit(UiEvent::outbox_updated(
                    outbox_id,
                    transfer_id,
                    peer_str,
                    filename,
                    size,
                    status.as_str(),
                    now,
                ));
            }
        }
    }

    pub fn active_transfers(&self) -> Vec<ActiveTransfer> {
        let transfers = self.shared.active_transfers.lock().expect("transfers");
        let mut list: Vec<ActiveTransfer> = transfers.values().cloned().collect();
        list.sort_by_key(|a| std::cmp::Reverse(a.started_ms));
        list
    }

    pub fn transfer_by_id(&self, id: &str) -> Option<ActiveTransfer> {
        let transfers = self.shared.active_transfers.lock().expect("transfers");
        transfers.get(id).cloned()
    }

    pub fn transfers_report(&self) -> TransfersReport {
        let transfers = self.active_transfers();
        let stats = self.transfer_stats();
        let active_count = transfers
            .iter()
            .filter(|t| {
                t.phase == TransferPhase::Transferring || t.phase == TransferPhase::Verifying
            })
            .count();
        let terminal_count = transfers.len().saturating_sub(active_count);
        TransfersReport {
            ok: true,
            active_count,
            terminal_count,
            total_count: transfers.len(),
            transfers,
            stats,
        }
    }

    pub fn transfer_stats(&self) -> TransferStats {
        self.shared
            .transfer_stats
            .lock()
            .expect("transfer_stats")
            .clone()
    }

    pub fn clear_terminal_transfers(&self) -> usize {
        let mut transfers = self.shared.active_transfers.lock().expect("transfers");
        let to_remove: Vec<String> = transfers
            .iter()
            .filter(|(_, t)| {
                t.phase == TransferPhase::Completed || t.phase == TransferPhase::Failed
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in &to_remove {
            transfers.remove(id);
        }
        to_remove.len()
    }

    /// Cancel an active in-flight transfer (inbound or outbound).
    pub async fn cancel_transfer(&self, transfer_id: &str) -> Result<(), CoreError> {
        let (cancel_tx, peer_to_notify, is_outbound, filename, size) = {
            let mut cancels = self.shared.transfer_cancels.lock().expect("cancels");
            let mut pending = self.shared.pending_out.lock().expect("pending");
            let mut transfers = self.shared.active_transfers.lock().expect("transfers");

            let t = transfers
                .get_mut(transfer_id)
                .ok_or(CoreError::UnknownTransfer)?;
            if t.phase != TransferPhase::Transferring && t.phase != TransferPhase::Verifying {
                return Err(CoreError::File("transfer is not active".into()));
            }
            {
                let mut stats = self.shared.transfer_stats.lock().expect("transfer_stats");
                stats.failed_count = stats.failed_count.saturating_add(1);
            }
            t.phase = TransferPhase::Failed;
            t.error = Some("cancelled by user".into());
            t.speed_bps = 0;
            t.updated_ms = self.shared.clock.unix_ms();

            let is_outbound = t.direction == TransferDirection::Outbound;
            let filename = t.filename.clone();
            let size = t.total;
            let peer = t.peer.clone();
            pending.remove(transfer_id);
            Self::prune_terminal_transfers_locked(&mut transfers);
            (
                cancels.remove(transfer_id),
                peer,
                is_outbound,
                filename,
                size,
            )
        };

        if let Some(tx) = cancel_tx {
            let _ = tx.send(());
        }

        let now = self.shared.clock.unix_ms();
        if is_outbound {
            let outbox_id = format!("out_{}", transfer_id);
            {
                let mut outbox = self.shared.outbox_engine.lock().expect("outbox_engine");
                outbox.update_status(
                    transfer_id,
                    OutboxStatus::Cancelled,
                    Some("cancelled by user".into()),
                    Some(now),
                    Some(0),
                );
            }
            self.emit(UiEvent::outbox_updated(
                outbox_id,
                transfer_id,
                peer_to_notify.to_string(),
                filename,
                size,
                OutboxStatus::Cancelled.as_str(),
                now,
            ));
        }

        let reject_decision = tetherly_core::FileDecision {
            transfer_id: transfer_id.to_string(),
            resume_from: None,
        };
        if let Ok(frame) = reject_decision.reject_frame() {
            let _ = self.send_to(&peer_to_notify, frame).await;
        }

        info!(transfer = %transfer_id, "transfer cancelled by user");
        Ok(())
    }

    // =========================================================================
    // Roaming File Drag & Drop (Engine II - Deskflow+ Semantics)
    // =========================================================================

    pub fn drag_controller(&self) -> &Mutex<DragController> {
        &self.shared.drag_controller
    }

    pub fn drag_target(&self) -> &Mutex<DragTarget> {
        &self.shared.drag_target
    }

    pub fn initiate_drag(
        &self,
        target_peer: &str,
        file_paths: Vec<std::path::PathBuf>,
        norm_x: f32,
        norm_y: f32,
    ) -> Result<
        (
            tetherly_core::drag::DragControlMsg,
            Option<tetherly_core::drag::DragControlMsg>,
        ),
        CoreError,
    > {
        let now = self.shared.clock.unix_ms();
        let mut ctl = self.shared.drag_controller.lock().expect("drag_controller");
        ctl.initiate_cross_border_drag(target_peer, file_paths, norm_x, norm_y, now)
    }

    pub fn update_drag_move(
        &self,
        norm_x: f32,
        norm_y: f32,
    ) -> Option<tetherly_core::drag::DragControlMsg> {
        let mut ctl = self.shared.drag_controller.lock().expect("drag_controller");
        ctl.sync_remote_move(norm_x, norm_y)
    }

    pub fn cancel_drag(
        &self,
        reason: tetherly_core::drag::DragCancelReason,
    ) -> Option<tetherly_core::drag::DragControlMsg> {
        let mut ctl = self.shared.drag_controller.lock().expect("drag_controller");
        ctl.cancel(reason)
    }

    /// Peek at the transfer a drop would commit, without consuming the session.
    pub fn peek_drag_transfer(&self, drag_id: u64) -> Option<OutgoingTransferRequest> {
        let ctl = self.shared.drag_controller.lock().expect("drag_controller");
        ctl.peek_transfer(drag_id)
    }

    pub fn confirm_drop(
        &self,
        drag_id: u64,
        norm_x: f32,
        norm_y: f32,
    ) -> Option<OutgoingTransferRequest> {
        let mut ctl = self.shared.drag_controller.lock().expect("drag_controller");
        ctl.on_drop_confirm(drag_id, norm_x, norm_y)
    }

    /// Roll a committed-but-failed transfer back into an active session.
    pub fn abort_drag_transfer(&self, req: OutgoingTransferRequest) {
        let mut ctl = self.shared.drag_controller.lock().expect("drag_controller");
        ctl.abort_transfer(req);
    }

    pub fn now_ms(&self) -> u64 {
        self.shared.clock.unix_ms()
    }
}
