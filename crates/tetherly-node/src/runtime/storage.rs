// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Storage/inbox maintenance and resume markers.

use super::*;
use crate::events::UiEvent;
use tetherly_core::{CoreError, DeviceId};

/// Detailed inspection of an in-progress or interrupted transfer capable of being resumed.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ResumeMarkerDto {
    pub transfer_id: String,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_alias: Option<String>,
    pub total_bytes: u64,
    pub offset_bytes: u64,
    pub progress_percent: f64,
    pub chunk_aligned: bool,
    pub part_file_exists: bool,
    pub part_file_bytes: u64,
}

/// Comprehensive lightweight report of all stored resume markers in memory and disk sidecars.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ResumeMarkersReport {
    pub ok: bool,
    pub count: usize,
    pub total_partial_bytes: u64,
    pub markers: Vec<ResumeMarkerDto>,
}

/// Lightweight status report for node data directory and storage consumption.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct StorageReport {
    pub ok: bool,
    pub data_dir: String,
    pub inbox_dir: String,
    pub inbox_total_bytes: u64,
    pub inbox_file_count: usize,
    pub partial_bytes: u64,
    pub partial_count: usize,
    pub orphaned_partial_count: usize,
    pub orphaned_partial_bytes: u64,
    pub meta_bytes: u64,
    #[serde(default)]
    pub resume_markers_count: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resume_markers: Vec<ResumeMarkerDto>,
}

/// Report returned by a storage cleanup operation.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct StorageCleanReport {
    pub ok: bool,
    pub freed_bytes: u64,
    pub deleted_files_count: usize,
    pub cleared_resume_entries: usize,
}

/// Metadata entry for a completed received file in the incoming inbox.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct InboxFileDto {
    pub name: String,
    pub size: u64,
    pub modified_at_secs: u64,
    pub is_safe: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<String>,
}

/// Comprehensive report of files residing in the incoming inbox.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct InboxReport {
    pub ok: bool,
    pub count: usize,
    pub total_bytes: u64,
    pub files: Vec<InboxFileDto>,
}

/// Result of deleting a specific file from the incoming inbox.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct InboxDeleteReport {
    pub ok: bool,
    pub name: String,
    pub freed_bytes: u64,
}

/// Result of clearing multiple or all files from the incoming inbox.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct InboxClearReport {
    pub ok: bool,
    pub deleted_count: usize,
    pub freed_bytes: u64,
}

/// Result of opening a specific file or the inbox directory itself in the OS.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct InboxOpenReport {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub path: String,
    pub message: String,
    /// The named file is not a known passive type, so the inbox folder was revealed
    /// instead of launching it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub demoted: bool,
}

/// Types "open" may hand to the OS default handler: passive documents and
/// media whose usual handlers do not execute embedded code. Anything else —
/// executables, scripts, installers, disk images, macro-enabled Office files,
/// profiles, shortcuts, and every type nobody thought of — only reveals the
/// inbox folder. Inbox files reach this machine from a peer, and on Windows
/// the daemon runs as SYSTEM, so a deny-list that misses one extension would
/// be code execution; an allow-list fails safe.
const PASSIVE_EXTENSIONS: &[&str] = &[
    // text
    "txt", "md", "log", "csv", "tsv", "json",
    // documents (macro-free OOXML / OpenDocument)
    "pdf", "docx", "xlsx", "pptx", "odt", "ods", "odp", "epub", // images
    "png", "jpg", "jpeg", "gif", "webp", "bmp", "heic", "heif", "tif", "tiff", "avif",
    // audio
    "mp3", "m4a", "aac", "wav", "flac", "ogg", "opus", // video
    "mp4", "m4v", "mov", "mkv", "webm", "avi",
    // archives (opening browses / extracts; nothing runs)
    "zip", "7z", "rar", "tar", "gz",
];

/// True when "open" on this inbox file must be demoted to revealing the
/// folder: anything that is not a known passive type.
fn inbox_file_needs_demotion(name: &str) -> bool {
    // NTFS stream syntax ("x.exe::$DATA", "a.txt:payload") resolves to a
    // different object than the extension suggests.
    if name.contains(':') {
        return true;
    }
    // Windows ignores trailing dots/spaces, so "x.exe. " launches as x.exe.
    let trimmed = name.trim_end_matches(['.', ' ']);
    match trimmed.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => {
            !PASSIVE_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str())
        }
        _ => true,
    }
}

pub(super) enum OutboundSource {
    Bytes(Vec<u8>),
    Path(std::path::PathBuf),
}

pub(super) struct OutboundFile {
    pub(super) peer: DeviceId,
    #[allow(dead_code)]
    pub(super) name: String,
    pub(super) source: OutboundSource,
    pub(super) size: u64,
}

/// Sidecar persisted next to a `.part` file: lets an interrupted incoming
/// transfer resume after a process restart. No file content, only progress.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct ResumeSidecar {
    #[serde(alias = "source")]
    pub(super) device_id: String,
    #[serde(alias = "total_bytes")]
    pub(super) total: u64,
    pub(super) offset: u64,
}

impl Node {
    /// Inspect local storage, inbox files, in-flight/orphaned partial transfers and metadata.
    pub fn storage_report(&self) -> StorageReport {
        let data_dir = self.shared.cfg.data_dir.clone();
        let inbox_dir = data_dir.join("inbox");
        let mut inbox_total_bytes: u64 = 0;
        let mut inbox_file_count: usize = 0;
        let mut partial_bytes: u64 = 0;
        let mut partial_count: usize = 0;
        let mut orphaned_partial_count: usize = 0;
        let mut orphaned_partial_bytes: u64 = 0;
        let mut meta_bytes: u64 = 0;

        {
            let active = self.shared.active_transfers.lock().expect("transfers");
            let hubs = self.shared.hubs.lock().expect("hubs");

            if let Ok(entries) = std::fs::read_dir(&inbox_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if !path.is_file() {
                        continue;
                    }
                    let file_len = entry.metadata().map(|m| m.len()).unwrap_or(0);
                    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    if name.starts_with('.') && name.ends_with(".part") {
                        let transfer_id = name
                            .strip_prefix('.')
                            .and_then(|s| s.strip_suffix(".part"))
                            .unwrap_or("");
                        partial_count += 1;
                        partial_bytes += file_len;

                        let is_active = active.get(transfer_id).is_some_and(|t| {
                            t.phase == TransferPhase::Transferring
                                || t.phase == TransferPhase::Verifying
                        });
                        let is_resumable = hubs.resume.get(transfer_id).is_some();
                        if !is_active && !is_resumable {
                            orphaned_partial_count += 1;
                            orphaned_partial_bytes += file_len;
                        }
                    } else if name.starts_with('.') && name.ends_with(".resume.json") {
                        meta_bytes += file_len;
                    } else if !name.starts_with('.') {
                        inbox_file_count += 1;
                        inbox_total_bytes += file_len;
                    }
                }
            }
        }

        for meta_file in [
            "identity.bin",
            "identity.ref",
            "trusted.json",
            "routes.json",
        ] {
            let p = data_dir.join(meta_file);
            if let Ok(m) = std::fs::metadata(&p) {
                meta_bytes += m.len();
            }
        }

        let markers_rep = self.resume_markers_report();

        StorageReport {
            ok: true,
            data_dir: data_dir.to_string_lossy().to_string(),
            inbox_dir: inbox_dir.to_string_lossy().to_string(),
            inbox_total_bytes,
            inbox_file_count,
            partial_bytes,
            partial_count,
            orphaned_partial_count,
            orphaned_partial_bytes,
            meta_bytes,
            resume_markers_count: markers_rep.count,
            resume_markers: markers_rep.markers,
        }
    }

    /// Inspect all active resume markers from memory (ResumeTable) and disk (.resume.json).
    pub fn resume_markers_report(&self) -> ResumeMarkersReport {
        let inbox_dir = self.shared.cfg.data_dir.join("inbox");
        let states = self.shared.hubs.lock().expect("hubs").resume.all();
        let mut markers = Vec::new();
        let mut seen_tids = std::collections::HashSet::new();
        let mut total_partial_bytes: u64 = 0;

        for st in states {
            seen_tids.insert(st.transfer_id.clone());
            let part_path = inbox_dir.join(format!(".{}.part", st.transfer_id));
            let fallback_path = inbox_dir.join(format!("{}.part", st.transfer_id));
            let part_meta = std::fs::metadata(&part_path)
                .or_else(|_| std::fs::metadata(&fallback_path))
                .ok();
            let part_file_exists = part_meta.is_some();
            let part_file_bytes = part_meta.map(|m| m.len()).unwrap_or(0);
            total_partial_bytes += part_file_bytes;

            let progress_percent = if st.total > 0 {
                (st.offset as f64 / st.total as f64) * 100.0
            } else {
                0.0
            };
            let chunk_aligned = st.offset % tetherly_core::FILE_CHUNK as u64 == 0;
            let source_alias = self.peer_alias(&st.source);

            markers.push(ResumeMarkerDto {
                transfer_id: st.transfer_id,
                source: st.source.to_string(),
                source_alias,
                total_bytes: st.total,
                offset_bytes: st.offset,
                progress_percent,
                chunk_aligned,
                part_file_exists,
                part_file_bytes,
            });
        }

        // Also discover any on-disk resume sidecars (.resume.json) not currently cached in memory
        if let Ok(entries) = std::fs::read_dir(&inbox_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                let stem = if name.starts_with('.') && name.ends_with(".resume.json") {
                    name.strip_prefix('.')
                        .and_then(|s| s.strip_suffix(".resume.json"))
                        .map(|s| s.to_string())
                } else if name.ends_with(".resume.json") {
                    name.strip_suffix(".resume.json").map(|s| s.to_string())
                } else {
                    None
                };

                let Some(transfer_id) = stem else { continue };
                if seen_tids.contains(&transfer_id) {
                    continue;
                }

                if let Ok(bytes) = std::fs::read(entry.path()) {
                    if let Ok(dto) = serde_json::from_slice::<ResumeSidecar>(&bytes) {
                        seen_tids.insert(transfer_id.clone());
                        let part_path = inbox_dir.join(format!(".{}.part", transfer_id));
                        let fallback_path = inbox_dir.join(format!("{}.part", transfer_id));
                        let part_meta = std::fs::metadata(&part_path)
                            .or_else(|_| std::fs::metadata(&fallback_path))
                            .ok();
                        let part_file_exists = part_meta.is_some();
                        let part_file_bytes = part_meta.map(|m| m.len()).unwrap_or(0);
                        total_partial_bytes += part_file_bytes;

                        let progress_percent = if dto.total > 0 {
                            (dto.offset as f64 / dto.total as f64) * 100.0
                        } else {
                            0.0
                        };
                        let chunk_aligned = dto.offset % tetherly_core::FILE_CHUNK as u64 == 0;
                        let source_alias = DeviceId::parse(&dto.device_id)
                            .ok()
                            .and_then(|dev| self.peer_alias(&dev));

                        markers.push(ResumeMarkerDto {
                            transfer_id,
                            source: dto.device_id,
                            source_alias,
                            total_bytes: dto.total,
                            offset_bytes: dto.offset,
                            progress_percent,
                            chunk_aligned,
                            part_file_exists,
                            part_file_bytes,
                        });
                    }
                }
            }
        }

        markers.sort_by(|a, b| a.transfer_id.cmp(&b.transfer_id));

        ResumeMarkersReport {
            ok: true,
            count: markers.len(),
            total_partial_bytes,
            markers,
        }
    }

    /// Garbage-collect incomplete or orphaned partial transfers in the inbox.
    pub fn clean_storage(&self, orphaned_only: bool) -> StorageCleanReport {
        let inbox_dir = self.shared.cfg.data_dir.join("inbox");
        let mut freed_bytes: u64 = 0;
        let mut deleted_files_count: usize = 0;
        let mut cleared_resume_entries: usize = 0;

        let active = self.shared.active_transfers.lock().expect("transfers");
        let mut hubs = self.shared.hubs.lock().expect("hubs");

        let active_ids: std::collections::HashSet<String> = active
            .iter()
            .filter(|(_, t)| {
                t.phase == TransferPhase::Transferring || t.phase == TransferPhase::Verifying
            })
            .map(|(id, _)| id.clone())
            .collect();

        if let Ok(entries) = std::fs::read_dir(&inbox_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_file() {
                    continue;
                }
                let file_len = entry.metadata().map(|m| m.len()).unwrap_or(0);
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

                if name.starts_with('.') && name.ends_with(".part") {
                    let transfer_id = name
                        .strip_prefix('.')
                        .and_then(|s| s.strip_suffix(".part"))
                        .unwrap_or("");
                    if active_ids.contains(transfer_id) {
                        continue;
                    }
                    let is_resumable = hubs.resume.get(transfer_id).is_some();
                    if orphaned_only {
                        if !is_resumable && std::fs::remove_file(&path).is_ok() {
                            freed_bytes += file_len;
                            deleted_files_count += 1;
                            let sidecar = inbox_dir.join(format!(".{transfer_id}.resume.json"));
                            if sidecar.exists() {
                                if let Ok(sm) = std::fs::metadata(&sidecar) {
                                    freed_bytes += sm.len();
                                }
                                if std::fs::remove_file(&sidecar).is_ok() {
                                    deleted_files_count += 1;
                                }
                            }
                        }
                    } else {
                        if std::fs::remove_file(&path).is_ok() {
                            freed_bytes += file_len;
                            deleted_files_count += 1;
                        }
                        if hubs.resume.get(transfer_id).is_some() {
                            hubs.resume.remove(transfer_id);
                            cleared_resume_entries += 1;
                        }
                        let sidecar = inbox_dir.join(format!(".{transfer_id}.resume.json"));
                        if sidecar.exists() {
                            if let Ok(sm) = std::fs::metadata(&sidecar) {
                                freed_bytes += sm.len();
                            }
                            if std::fs::remove_file(&sidecar).is_ok() {
                                deleted_files_count += 1;
                            }
                        }
                    }
                } else if name.starts_with('.') && name.ends_with(".resume.json") {
                    let transfer_id = name
                        .strip_prefix('.')
                        .and_then(|s| s.strip_suffix(".resume.json"))
                        .unwrap_or("");
                    if active_ids.contains(transfer_id) {
                        continue;
                    }
                    let matching_part = inbox_dir.join(format!(".{transfer_id}.part"));
                    if !matching_part.exists() {
                        if hubs.resume.get(transfer_id).is_some() {
                            hubs.resume.remove(transfer_id);
                            cleared_resume_entries += 1;
                        }
                        if std::fs::remove_file(&path).is_ok() {
                            freed_bytes += file_len;
                            deleted_files_count += 1;
                        }
                    }
                }
            }
        }

        StorageCleanReport {
            ok: true,
            freed_bytes,
            deleted_files_count,
            cleared_resume_entries,
        }
    }

    /// Inspect files in the incoming inbox, optionally filtering by search substring or limiting output count.
    pub fn inbox_report(&self, search: Option<&str>, limit: Option<usize>) -> InboxReport {
        let inbox_dir = self.shared.cfg.data_dir.join("inbox");
        let mut files = Vec::new();

        if let Ok(entries) = std::fs::read_dir(&inbox_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_file() {
                    continue;
                }
                let name = match path.file_name().and_then(|n| n.to_str()) {
                    Some(n) if !n.starts_with('.') => n.to_string(),
                    _ => continue,
                };

                let (file_len, modified_at_secs) = entry
                    .metadata()
                    .map(|m| {
                        let len = m.len();
                        let mod_time = m
                            .modified()
                            .ok()
                            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                            .map(|d| d.as_secs())
                            .unwrap_or(0);
                        (len, mod_time)
                    })
                    .unwrap_or((0, 0));

                if let Some(q) = search {
                    if !q.is_empty() && !name.to_lowercase().contains(&q.to_lowercase()) {
                        continue;
                    }
                }

                let is_safe = tetherly_core::sanitize_file_name(&name).is_ok();
                let extension = std::path::Path::new(&name)
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|s| s.to_string());

                files.push(InboxFileDto {
                    name,
                    size: file_len,
                    modified_at_secs,
                    is_safe,
                    extension,
                });
            }
        }

        // Sort descending by modified_at_secs, then ascending by name
        files.sort_by(|a, b| {
            b.modified_at_secs
                .cmp(&a.modified_at_secs)
                .then_with(|| a.name.cmp(&b.name))
        });

        let total_bytes: u64 = files.iter().map(|f| f.size).sum();
        let count = files.len();

        if let Some(lim) = limit {
            if files.len() > lim {
                files.truncate(lim);
            }
        }

        InboxReport {
            ok: true,
            count,
            total_bytes,
            files,
        }
    }

    /// Safely delete a file from the incoming inbox, guarding against path traversal attacks.
    pub fn delete_inbox_file(&self, name: &str) -> Result<InboxDeleteReport, CoreError> {
        if name.is_empty()
            || name.contains('/')
            || name.contains('\\')
            || name.contains("..")
            || name.contains('\0')
            || name.starts_with('.')
        {
            return Err(CoreError::UnsafeFileName);
        }

        let clean_name = tetherly_core::sanitize_file_name(name)?;
        if clean_name != name || clean_name.starts_with('.') {
            return Err(CoreError::UnsafeFileName);
        }

        let inbox_dir = self.shared.cfg.data_dir.join("inbox");
        let file_path = inbox_dir.join(&clean_name);

        if !file_path.exists() || !file_path.is_file() {
            return Err(CoreError::File("file not found in inbox".into()));
        }

        let freed_bytes = std::fs::metadata(&file_path).map(|m| m.len()).unwrap_or(0);

        std::fs::remove_file(&file_path)?;

        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        let _ = self.shared.events.send(UiEvent::inbox_file_deleted(
            &clean_name,
            freed_bytes,
            now_ms,
        ));

        self.record_audit(
            AuditCategory::File,
            AuditSeverity::Notice,
            "inbox_file_deleted",
            Some(clean_name.clone()),
            format!("inbox file '{clean_name}' deleted ({freed_bytes} bytes freed)"),
            true,
        );

        Ok(InboxDeleteReport {
            ok: true,
            name: clean_name,
            freed_bytes,
        })
    }

    /// Clear completed received files from the incoming inbox, optionally filtering by modified cutoff time.
    pub fn clear_inbox(&self, before_secs: Option<u64>) -> Result<InboxClearReport, CoreError> {
        let inbox_dir = self.shared.cfg.data_dir.join("inbox");
        let mut deleted_count: usize = 0;
        let mut freed_bytes: u64 = 0;

        if let Ok(entries) = std::fs::read_dir(&inbox_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_file() {
                    continue;
                }
                let name = match path.file_name().and_then(|n| n.to_str()) {
                    Some(n) if !n.starts_with('.') => n,
                    _ => continue,
                };
                let _ = name;

                let (file_len, mod_time) = entry
                    .metadata()
                    .map(|m| {
                        let len = m.len();
                        let t = m
                            .modified()
                            .ok()
                            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                            .map(|d| d.as_secs())
                            .unwrap_or(0);
                        (len, t)
                    })
                    .unwrap_or((0, 0));

                if let Some(cutoff) = before_secs {
                    if mod_time >= cutoff {
                        continue;
                    }
                }

                if std::fs::remove_file(&path).is_ok() {
                    deleted_count += 1;
                    freed_bytes += file_len;
                }
            }
        }

        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        let _ = self
            .shared
            .events
            .send(UiEvent::inbox_cleared(deleted_count, freed_bytes, now_ms));

        self.record_audit(
            AuditCategory::File,
            AuditSeverity::Notice,
            "inbox_cleared",
            None::<String>,
            format!("inbox cleared: {deleted_count} files removed ({freed_bytes} bytes freed)"),
            true,
        );

        Ok(InboxClearReport {
            ok: true,
            deleted_count,
            freed_bytes,
        })
    }

    /// Open a specific file in the inbox with the system default application,
    /// or open the inbox directory itself if no name is specified.
    pub fn open_inbox_file(
        &self,
        name: Option<&str>,
        opener: &dyn tetherly_core::Opener,
    ) -> Result<InboxOpenReport, CoreError> {
        let inbox_dir = self.shared.cfg.data_dir.join("inbox");
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        match name.filter(|s| !s.trim().is_empty()) {
            Some(n) => {
                if n.is_empty()
                    || n.contains('/')
                    || n.contains('\\')
                    || n.contains("..")
                    || n.contains('\0')
                    || n.starts_with('.')
                {
                    return Err(CoreError::UnsafeFileName);
                }

                let clean_name = tetherly_core::sanitize_file_name(n)?;
                if clean_name != n || clean_name.starts_with('.') {
                    return Err(CoreError::UnsafeFileName);
                }

                let file_path = inbox_dir.join(&clean_name);
                if !file_path.exists() || !file_path.is_file() {
                    return Err(CoreError::File("file not found in inbox".into()));
                }

                if inbox_file_needs_demotion(&clean_name) {
                    let dir_str = inbox_dir.to_str().ok_or(CoreError::UnsafeFileName)?;
                    opener.open(dir_str)?;
                    tracing::warn!(
                        name = %clean_name,
                        "inbox open demoted: not a passive file type; folder revealed"
                    );
                    self.record_audit(
                        AuditCategory::File,
                        AuditSeverity::Warning,
                        "inbox_open_demoted",
                        Some(clean_name.clone()),
                        format!(
                            "inbox file '{clean_name}' is not a passive type; revealed the inbox folder instead of launching it"
                        ),
                        true,
                    );
                    return Ok(InboxOpenReport {
                        ok: true,
                        name: Some(clean_name),
                        path: inbox_dir.display().to_string(),
                        message: "file type is not opened directly; inbox folder opened instead"
                            .into(),
                        demoted: true,
                    });
                }

                let path_str = file_path.to_str().ok_or(CoreError::UnsafeFileName)?;
                opener.open(path_str)?;

                let _ = self
                    .shared
                    .events
                    .send(UiEvent::inbox_file_opened(Some(&clean_name), now_ms));

                self.record_audit(
                    AuditCategory::File,
                    AuditSeverity::Info,
                    "inbox_file_opened",
                    Some(clean_name.clone()),
                    format!("inbox file '{clean_name}' opened in system default application"),
                    true,
                );

                Ok(InboxOpenReport {
                    ok: true,
                    name: Some(clean_name),
                    path: file_path.display().to_string(),
                    message: "file opened in system default handler".into(),
                    demoted: false,
                })
            }
            None => {
                std::fs::create_dir_all(&inbox_dir)?;
                let path_str = inbox_dir.to_str().ok_or(CoreError::UnsafeFileName)?;
                opener.open(path_str)?;

                let _ = self
                    .shared
                    .events
                    .send(UiEvent::inbox_file_opened(None::<String>, now_ms));

                self.record_audit(
                    AuditCategory::File,
                    AuditSeverity::Info,
                    "inbox_directory_opened",
                    None::<String>,
                    "inbox directory opened in system file manager",
                    true,
                );

                Ok(InboxOpenReport {
                    ok: true,
                    name: None,
                    path: inbox_dir.display().to_string(),
                    message: "inbox directory opened in system file manager".into(),
                    demoted: false,
                })
            }
        }
    }

    /// Read the bytes and MIME type of a completed file from the inbox,
    /// enforcing strict path traversal guards and FILE_MAX limits.
    pub fn read_inbox_file(
        &self,
        name: &str,
    ) -> Result<(String, &'static str, Vec<u8>), CoreError> {
        if name.is_empty()
            || name.contains('/')
            || name.contains('\\')
            || name.contains("..")
            || name.contains('\0')
            || name.starts_with('.')
        {
            return Err(CoreError::UnsafeFileName);
        }

        let clean_name = tetherly_core::sanitize_file_name(name)?;
        if clean_name != name || clean_name.starts_with('.') {
            return Err(CoreError::UnsafeFileName);
        }

        let inbox_dir = self.shared.cfg.data_dir.join("inbox");
        let file_path = inbox_dir.join(&clean_name);

        if !file_path.exists() || !file_path.is_file() {
            return Err(CoreError::File("file not found in inbox".into()));
        }

        let meta = std::fs::metadata(&file_path)?;
        if meta.len() > tetherly_core::FILE_MAX {
            return Err(CoreError::FileTooLarge(meta.len()));
        }

        let bytes = std::fs::read(&file_path)?;
        let mime = mime_type_from_name(&clean_name);
        Ok((clean_name, mime, bytes))
    }

    /// Record a chunk-aligned resume marker in the hub and on disk (sidecar).
    pub(super) fn note_resume(
        &self,
        source: &DeviceId,
        transfer_id: &str,
        total: u64,
        offset: u64,
    ) {
        let st = tetherly_core::ResumeState {
            transfer_id: transfer_id.to_string(),
            source: source.clone(),
            total,
            offset,
        };
        {
            let mut hubs = self.shared.hubs.lock().expect("hubs");
            hubs.resume.put(st.clone());
        }
        // Persist the chunk-aligned offset the table actually settled on.
        let aligned = self
            .shared
            .hubs
            .lock()
            .expect("hubs")
            .resume
            .get(transfer_id)
            .map(|s| s.offset)
            .unwrap_or(0);
        if aligned == 0 {
            return;
        }
        let side = self.resume_sidecar(transfer_id);
        let dto = ResumeSidecar {
            device_id: source.as_str().to_string(),
            total,
            offset: aligned,
        };
        if let Ok(bytes) = serde_json::to_vec(&dto) {
            let _ = crate::store::atomic_write(&side, &bytes);
        }
    }

    pub(super) fn clear_resume(&self, transfer_id: &str) {
        {
            let mut hubs = self.shared.hubs.lock().expect("hubs");
            hubs.resume.remove(transfer_id);
        }
        let _ = std::fs::remove_file(self.resume_sidecar(transfer_id));
    }

    pub(super) fn resume_sidecar(&self, transfer_id: &str) -> std::path::PathBuf {
        self.shared
            .cfg
            .data_dir
            .join("inbox")
            .join(format!(".{}.resume.json", transfer_id))
    }
}
