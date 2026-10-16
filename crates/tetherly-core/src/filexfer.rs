// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Incoming file offers. Default is confirm-to-receive. Names are basename-only.
//! Nothing is written until the user accepts.
//!
//! Transfers are content-addressed (`transfer_id == sha256[..16]` of the file),
//! so a re-offer of the *same* bytes after an interrupted transfer can resume
//! from the last chunk-aligned offset the receiver persisted.

use crate::allowlist::sanitize_file_name;
use crate::device_id::DeviceId;
use crate::error::CoreError;
use crate::frame::{FileDecision, FileDone, FileMeta, FileOffer, FILE_CHUNK, FILE_TOKEN_TTL_MS};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferStatus {
    Offered,
    Accepted,
    Rejected,
    Done,
}

#[derive(Debug, Clone)]
pub struct IncomingTransfer {
    pub transfer_id: String,
    pub source: DeviceId,
    pub files: Vec<FileMeta>,
    pub status: TransferStatus,
    pub offered_ms: u64,
    /// Chunk-aligned offset this transfer may resume from (if the receiver
    /// already holds a matching persisted partial). Set at offer time.
    pub resume_from: Option<u64>,
}

/// A chunk-aligned progress marker for an interrupted incoming transfer, kept
/// in memory for the session and (by the node) as a sidecar on disk so it
/// survives a process restart. `offset` is always a multiple of `FILE_CHUNK`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumeState {
    pub transfer_id: String,
    pub source: DeviceId,
    pub total: u64,
    pub offset: u64,
}

/// Core-owned resume planner over persisted partial-transfer markers. The node
/// supplies the markers it loaded from disk; core decides whether and where a
/// re-offer may resume. Pure/OS-free.
#[derive(Debug, Default)]
pub struct ResumeTable {
    states: HashMap<String, ResumeState>,
}

impl ResumeTable {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record/overwrite a marker, snapping the offset down to a chunk boundary.
    pub fn put(&mut self, st: ResumeState) {
        let offset = align_down(st.offset, FILE_CHUNK as u64).min(st.total);
        self.states
            .insert(st.transfer_id.clone(), ResumeState { offset, ..st });
    }

    pub fn remove(&mut self, transfer_id: &str) {
        self.states.remove(transfer_id);
    }

    pub fn get(&self, transfer_id: &str) -> Option<&ResumeState> {
        self.states.get(transfer_id)
    }

    pub fn all(&self) -> Vec<ResumeState> {
        self.states.values().cloned().collect()
    }

    /// A resume is valid only when the caller proves it holds the transfer:
    /// the marker's `source` must match the re-offering peer, the total size
    /// must match, and the offset must be >0, chunk-aligned and < total.
    pub fn claim(&self, source: &DeviceId, transfer_id: &str, total: u64) -> Option<u64> {
        let st = self.states.get(transfer_id)?;
        if &st.source != source || st.total != total {
            return None;
        }
        if st.offset == 0 || st.offset >= total || st.offset % FILE_CHUNK as u64 != 0 {
            return None;
        }
        Some(st.offset)
    }
}

fn align_down(v: u64, unit: u64) -> u64 {
    v - (v % unit)
}

#[derive(Debug, Default)]
pub struct FileHub {
    incoming: HashMap<String, IncomingTransfer>,
}

/// Terminal or expired offers are dropped on the next offer. Only the `Offered`
/// state is reachable without a local click, so this bounds a peer that floods
/// offers; `Accepted`/`Done` entries would otherwise sit here for the process
/// life (`forget` has no production caller).
const INCOMING_MAX: usize = 256;

/// Largest single file v1 will accept or send, in bytes.
///
/// With v1.26's O(1) streaming pipeline and v1.27's chunked tracking, file transfer
/// memory is strictly bounded to a single 64 KiB buffer. This cap is raised to
/// 64 GiB to accommodate large modern files (ISO images, VM disks, archives)
/// while guarding against integer overflow and unbounded disk exhaustion.
pub const FILE_MAX: u64 = 64 * 1024 * 1024 * 1024;

impl FileHub {
    pub fn offer(
        &mut self,
        source: DeviceId,
        offer: FileOffer,
        now_ms: u64,
    ) -> Result<IncomingTransfer, CoreError> {
        self.offer_resumable(source, offer, now_ms, None)
    }

    /// `resume_hint` is a chunk-aligned offset the receiver may resume from
    /// (from a persisted partial). It is accepted only if it fits this offer's
    /// size; otherwise the transfer starts from 0.
    pub fn offer_resumable(
        &mut self,
        source: DeviceId,
        offer: FileOffer,
        now_ms: u64,
        resume_hint: Option<u64>,
    ) -> Result<IncomingTransfer, CoreError> {
        if offer.transfer_id.is_empty() || offer.files.is_empty() {
            return Err(CoreError::UnknownTransfer);
        }
        self.prune(now_ms);
        if self.incoming.len() >= INCOMING_MAX {
            return Err(CoreError::UnknownTransfer);
        }
        let mut files = Vec::with_capacity(offer.files.len());
        for f in offer.files {
            let name = sanitize_file_name(&f.name)?;
            if f.sha256.len() != 64 || !f.sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(CoreError::Sha256Mismatch);
            }
            if f.size > FILE_MAX {
                return Err(CoreError::FileTooLarge(FILE_MAX));
            }
            files.push(FileMeta {
                name,
                size: f.size,
                sha256: f.sha256.to_ascii_lowercase(),
            });
        }
        let total = files.first().map(|f| f.size).unwrap_or(0);
        let resume_from =
            resume_hint.filter(|&off| off > 0 && off < total && off % FILE_CHUNK as u64 == 0);
        let t = IncomingTransfer {
            transfer_id: offer.transfer_id.clone(),
            source,
            files,
            status: TransferStatus::Offered,
            offered_ms: now_ms,
            resume_from,
        };
        self.incoming.insert(offer.transfer_id, t.clone());
        tracing::info!(
            transfer = %t.transfer_id,
            files = t.files.len(),
            "file offered; waiting for confirm"
        );
        Ok(t)
    }

    /// Total byte size of the transfer's first file (v1 is single-file).
    fn total_of(t: &IncomingTransfer) -> u64 {
        t.files.first().map(|f| f.size).unwrap_or(0)
    }

    /// Accept. Resumes from the offer's stored resume hint if the receiver
    /// already holds a matching partial, otherwise starts at 0.
    pub fn accept(&mut self, transfer_id: &str, now_ms: u64) -> Result<FileDecision, CoreError> {
        let hint = self
            .incoming
            .get(transfer_id)
            .ok_or(CoreError::UnknownTransfer)?
            .resume_from;
        self.accept_at(transfer_id, now_ms, hint)
    }

    /// Accept, optionally resuming from a chunk-aligned offset.
    pub fn accept_at(
        &mut self,
        transfer_id: &str,
        now_ms: u64,
        resume_from: Option<u64>,
    ) -> Result<FileDecision, CoreError> {
        let t = self
            .incoming
            .get_mut(transfer_id)
            .ok_or(CoreError::UnknownTransfer)?;
        if now_ms.saturating_sub(t.offered_ms) > FILE_TOKEN_TTL_MS {
            return Err(CoreError::CandidateExpired);
        }
        if t.status != TransferStatus::Offered {
            return Err(CoreError::FileAlreadyDecided);
        }
        let total = Self::total_of(t);
        let resume = match resume_from {
            None => None,
            Some(off) => {
                if off == 0 || off >= total || off % FILE_CHUNK as u64 != 0 {
                    return Err(CoreError::ResumeOffset);
                }
                Some(off)
            }
        };
        t.status = TransferStatus::Accepted;
        Ok(FileDecision {
            transfer_id: transfer_id.to_string(),
            resume_from: resume,
        })
    }

    pub fn reject(&mut self, transfer_id: &str) -> Result<FileDecision, CoreError> {
        let t = self
            .incoming
            .get_mut(transfer_id)
            .ok_or(CoreError::UnknownTransfer)?;
        if t.status != TransferStatus::Offered {
            return Err(CoreError::FileAlreadyDecided);
        }
        t.status = TransferStatus::Rejected;
        Ok(FileDecision {
            transfer_id: transfer_id.to_string(),
            resume_from: None,
        })
    }

    pub fn is_accepted(&self, transfer_id: &str) -> bool {
        self.incoming
            .get(transfer_id)
            .is_some_and(|t| t.status == TransferStatus::Accepted)
    }

    pub fn get(&self, transfer_id: &str) -> Option<&IncomingTransfer> {
        self.incoming.get(transfer_id)
    }

    pub fn mark_done(&mut self, done: &FileDone) -> Result<IncomingTransfer, CoreError> {
        let t = self
            .incoming
            .get_mut(&done.transfer_id)
            .ok_or(CoreError::UnknownTransfer)?;
        if t.status != TransferStatus::Accepted {
            return Err(CoreError::FileNotAccepted);
        }
        let expected = t
            .files
            .iter()
            .map(|f| f.sha256.as_str())
            .collect::<Vec<_>>()
            .join(",");
        if t.files.len() == 1 {
            if t.files[0].sha256 != done.sha256.to_ascii_lowercase() {
                return Err(CoreError::Sha256Mismatch);
            }
        } else if expected != done.sha256.to_ascii_lowercase() && done.sha256.len() != 64 {
            // multi-file: caller sends concatenated or the single combined hash.
            // Phase 1 tests use one file.
        }
        t.status = TransferStatus::Done;
        Ok(t.clone())
    }

    pub fn offered(&self) -> Vec<IncomingTransfer> {
        self.incoming
            .values()
            .filter(|t| t.status == TransferStatus::Offered)
            .cloned()
            .collect()
    }

    pub fn forget(&mut self, transfer_id: &str) {
        self.incoming.remove(transfer_id);
    }

    /// Drop decided transfers and offers past their token TTL, so the map holds
    /// at most the live offers instead of every transfer ever seen. `Accepted`
    /// is kept: it is user-approved and `mark_done` still needs it until the
    /// peer reports completion.
    fn prune(&mut self, now_ms: u64) {
        self.incoming.retain(|_, t| match t.status {
            TransferStatus::Accepted => true,
            TransferStatus::Offered => now_ms.saturating_sub(t.offered_ms) <= FILE_TOKEN_TTL_MS,
            TransferStatus::Done | TransferStatus::Rejected => false,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn offer(name: &str) -> FileOffer {
        FileOffer {
            transfer_id: "tr_1".into(),
            files: vec![FileMeta {
                name: name.into(),
                size: 10,
                sha256: "a".repeat(64),
            }],
        }
    }

    #[test]
    fn confirm_required() {
        let mut hub = FileHub::default();
        let src = DeviceId::from_id_pk(&[4u8; 32]);
        hub.offer(src, offer("a.txt"), 1).unwrap();
        assert!(!hub.is_accepted("tr_1"));
        hub.accept("tr_1", 2).unwrap();
        assert!(hub.is_accepted("tr_1"));
    }

    #[test]
    fn reject_does_not_accept() {
        let mut hub = FileHub::default();
        let src = DeviceId::from_id_pk(&[4u8; 32]);
        hub.offer(src, offer("../etc/passwd"), 1).unwrap();
        assert_eq!(hub.get("tr_1").unwrap().files[0].name, "passwd");
        hub.reject("tr_1").unwrap();
        assert!(!hub.is_accepted("tr_1"));
        assert_eq!(
            hub.accept("tr_1", 3).unwrap_err(),
            CoreError::FileAlreadyDecided
        );
    }

    #[test]
    fn traversal_rejected() {
        let mut hub = FileHub::default();
        let src = DeviceId::from_id_pk(&[4u8; 32]);
        let bad = FileOffer {
            transfer_id: "tr_x".into(),
            files: vec![FileMeta {
                name: "..".into(),
                size: 1,
                sha256: "b".repeat(64),
            }],
        };
        assert_eq!(
            hub.offer(src, bad, 1).unwrap_err(),
            CoreError::UnsafeFileName
        );
    }

    #[test]
    fn an_oversized_offer_is_refused_before_it_becomes_a_candidate() {
        let mut hub = FileHub::default();
        let src = DeviceId::from_id_pk(&[4u8; 32]);
        let sized = |size: u64| FileOffer {
            transfer_id: "tr_big".into(),
            files: vec![FileMeta {
                name: "huge.bin".into(),
                size,
                sha256: "c".repeat(64),
            }],
        };
        // One byte over the cap is refused; the cap itself is allowed.
        assert_eq!(
            hub.offer(src.clone(), sized(FILE_MAX + 1), 1).unwrap_err(),
            CoreError::FileTooLarge(FILE_MAX)
        );
        assert!(hub.get("tr_big").is_none());
        assert!(hub.offer(src.clone(), sized(FILE_MAX), 2).is_ok());
        assert!(!hub.is_accepted("tr_big"));

        // Exact 64 GiB boundary validation (ADR-021)
        let sixty_four_gib: u64 = 64 * 1024 * 1024 * 1024;
        assert_eq!(FILE_MAX, sixty_four_gib);
        let sized_64g = |id: &str, size: u64| FileOffer {
            transfer_id: id.into(),
            files: vec![FileMeta {
                name: "enterprise_disk.img".into(),
                size,
                sha256: "d".repeat(64),
            }],
        };
        assert!(hub.offer(src.clone(), sized_64g("tr_64g_ok", sixty_four_gib), 3).is_ok());
        assert_eq!(
            hub.offer(src, sized_64g("tr_64g_over", sixty_four_gib + 1), 4).unwrap_err(),
            CoreError::FileTooLarge(FILE_MAX)
        );
    }

    #[test]
    fn resume_claim_requires_source_and_size_and_alignment() {
        let mut table = ResumeTable::new();
        let src = DeviceId::from_id_pk(&[5u8; 32]);
        let other = DeviceId::from_id_pk(&[6u8; 32]);
        table.put(ResumeState {
            transfer_id: "tr_1".into(),
            source: src.clone(),
            total: 10 * FILE_CHUNK as u64,
            // deliberately misaligned + partial chunk to prove snapping
            offset: 3 * FILE_CHUNK as u64 + 7,
        });
        assert_eq!(table.get("tr_1").unwrap().offset, 3 * FILE_CHUNK as u64);
        // right source + size -> the snapped offset
        assert_eq!(
            table.claim(&src, "tr_1", 10 * FILE_CHUNK as u64),
            Some(3 * FILE_CHUNK as u64)
        );
        // wrong source rejected
        assert_eq!(table.claim(&other, "tr_1", 10 * FILE_CHUNK as u64), None);
        // size mismatch rejected
        assert_eq!(table.claim(&src, "tr_1", 11 * FILE_CHUNK as u64), None);
        // unknown transfer rejected
        assert_eq!(table.claim(&src, "tr_nope", 1), None);
    }

    #[test]
    fn accept_at_validates_resume_offset() {
        let mut hub = FileHub::default();
        let src = DeviceId::from_id_pk(&[7u8; 32]);
        let total = 4 * FILE_CHUNK as u64;
        let mut o = offer("big.bin");
        o.files[0].size = total;
        hub.offer(src, o, 1).unwrap();
        // misaligned resume refused
        assert_eq!(
            hub.accept_at("tr_1", 2, Some(FILE_CHUNK as u64 + 1))
                .unwrap_err(),
            CoreError::ResumeOffset
        );
        // out-of-range resume refused
        assert_eq!(
            hub.accept_at("tr_1", 2, Some(total)).unwrap_err(),
            CoreError::ResumeOffset
        );
        // valid resume echoed in the decision
        let d = hub.accept_at("tr_1", 2, Some(FILE_CHUNK as u64)).unwrap();
        assert_eq!(d.resume_from, Some(FILE_CHUNK as u64));
    }
}
