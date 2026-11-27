//! v0.3.1: content-addressed receive dedupe.
//!
//! Every offer names its payload by sha256. When this node already holds a
//! file with that digest (a previous receive in the inbox, or the retained
//! copy of a native drag payload), the offer is accepted at `resume_from =
//! total` and nothing crosses the wire; the existing bytes are reused through
//! a hard link (no data copy on the same volume).
//!
//! Layout under `<data_dir>/inbox`:
//! - `.dedupe.json`: sha256 → `{path, size, mtime_ms}` of delivered files.
//!   An entry is trusted only while the file still exists with the same size
//!   and modification time; a stale entry is dropped on lookup.
//! - `.cache/<sha256>`: hard links that keep native drag payloads (whose own
//!   directory is swept after an hour) reusable, capped by total size.
//!
//! On an index miss the inbox itself is scanned for same-size files, which
//! are hashed (bounded), so files received before the index existed count.
//! Everything here does blocking I/O: call it from `spawn_blocking`.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

const INDEX_FILE: &str = ".dedupe.json";
const CACHE_DIR: &str = ".cache";
/// Index entries kept (oldest delivery dropped first).
const INDEX_MAX: usize = 4096;
/// Retained native-drag payload budget.
const CACHE_MAX_BYTES: u64 = 4 * 1024 * 1024 * 1024;
/// Inbox scan bounds on an index miss.
const SCAN_MAX_FILES: usize = 16;
const SCAN_MAX_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// Serializes index read-modify-write across concurrent receives.
static INDEX_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct Entry {
    path: PathBuf,
    size: u64,
    mtime_ms: u64,
    recorded_ms: u64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Index {
    entries: BTreeMap<String, Entry>,
}

pub(crate) fn inbox_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("inbox")
}

fn cache_dir(data_dir: &Path) -> PathBuf {
    inbox_dir(data_dir).join(CACHE_DIR)
}

fn index_path(data_dir: &Path) -> PathBuf {
    inbox_dir(data_dir).join(INDEX_FILE)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn mtime_ms(meta: &std::fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn is_digest(sha: &str) -> bool {
    sha.len() == 64 && sha.bytes().all(|b| b.is_ascii_hexdigit())
}

fn load(data_dir: &Path) -> Index {
    match std::fs::read(index_path(data_dir)) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|e| {
            warn!(error = %e, "dedupe index unreadable; starting empty");
            Index::default()
        }),
        Err(_) => Index::default(),
    }
}

fn save(data_dir: &Path, index: &Index) {
    let path = index_path(data_dir);
    let tmp = path.with_extension("json.tmp");
    let res = serde_json::to_vec(index)
        .map_err(std::io::Error::other)
        .and_then(|bytes| std::fs::write(&tmp, bytes))
        .and_then(|()| std::fs::rename(&tmp, &path));
    if let Err(e) = res {
        warn!(error = %e, "dedupe index write failed");
        let _ = std::fs::remove_file(&tmp);
    }
}

/// The entry still describes the file on disk.
fn entry_valid(e: &Entry) -> bool {
    std::fs::metadata(&e.path)
        .map(|m| m.is_file() && m.len() == e.size && mtime_ms(&m) == e.mtime_ms)
        .unwrap_or(false)
}

fn insert(index: &mut Index, sha: &str, path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    index.entries.insert(
        sha.to_ascii_lowercase(),
        Entry {
            path: path.to_path_buf(),
            size: meta.len(),
            mtime_ms: mtime_ms(&meta),
            recorded_ms: now_ms(),
        },
    );
    if index.entries.len() > INDEX_MAX {
        index.entries.retain(|_, e| e.path.exists());
        while index.entries.len() > INDEX_MAX {
            let oldest = index
                .entries
                .iter()
                .min_by_key(|(_, e)| e.recorded_ms)
                .map(|(k, _)| k.clone());
            match oldest {
                Some(k) => {
                    index.entries.remove(&k);
                }
                None => break,
            }
        }
    }
    true
}

/// Remember that `path` holds content `sha`.
pub(crate) fn record(data_dir: &Path, sha: &str, path: &Path) {
    if !is_digest(sha) {
        return;
    }
    let _g = INDEX_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let mut index = load(data_dir);
    if insert(&mut index, sha, path) {
        save(data_dir, &index);
    }
}

/// Keep a reusable link to `path` (a native drag payload, whose directory is
/// swept) under `.cache/<sha>` and index it there.
pub(crate) fn retain(data_dir: &Path, sha: &str, path: &Path) {
    if !is_digest(sha) {
        return;
    }
    let sha = sha.to_ascii_lowercase();
    let dir = cache_dir(data_dir);
    if let Err(e) = std::fs::create_dir_all(&dir) {
        warn!(error = %e, "dedupe cache dir");
        return;
    }
    let cached = dir.join(&sha);
    if !cached.is_file() {
        if let Err(e) = std::fs::hard_link(path, &cached) {
            // Another volume or a filesystem without links: index the
            // payload itself, which stays reusable until it is swept.
            debug!(error = %e, "dedupe cache link failed");
            record(data_dir, &sha, path);
            return;
        }
    }
    record(data_dir, &sha, &cached);
    evict_cache(&dir, &cached);
}

/// Drop the oldest cached payloads beyond `CACHE_MAX_BYTES` (never `keep`).
fn evict_cache(dir: &Path, keep: &Path) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<(u64, u64, PathBuf)> = rd
        .flatten()
        .filter_map(|e| {
            let m = e.metadata().ok()?;
            m.is_file().then(|| (mtime_ms(&m), m.len(), e.path()))
        })
        .collect();
    let mut total: u64 = files.iter().map(|f| f.1).sum();
    files.sort();
    for (_, len, p) in files {
        if total <= CACHE_MAX_BYTES {
            break;
        }
        if p == keep {
            continue;
        }
        if std::fs::remove_file(&p).is_ok() {
            total = total.saturating_sub(len);
        }
    }
}

fn file_sha256(path: &Path) -> std::io::Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut h = tetherly_core::IncrementalSha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize_hex())
}

/// A local file whose content is `sha` (`size` bytes), if this node has one.
pub(crate) fn lookup(data_dir: &Path, sha: &str, size: u64) -> Option<PathBuf> {
    if size == 0 || !is_digest(sha) {
        return None;
    }
    let sha = sha.to_ascii_lowercase();
    let _g = INDEX_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let mut index = load(data_dir);
    if let Some(e) = index.entries.get(&sha) {
        if e.size == size && entry_valid(e) {
            return Some(e.path.clone());
        }
        index.entries.remove(&sha);
        save(data_dir, &index);
    }
    let cached = cache_dir(data_dir).join(&sha);
    if std::fs::metadata(&cached).is_ok_and(|m| m.is_file() && m.len() == size) {
        // The name is a digest verified on receive, but the cache shares its
        // inode with the delivered payload (hard link): an app that saved
        // that payload in place changed these bytes too. Without a valid
        // index entry (size + mtime), trust it only after re-hashing.
        let verified = size <= SCAN_MAX_BYTES && file_sha256(&cached).is_ok_and(|h| h == sha);
        if verified {
            if insert(&mut index, &sha, &cached) {
                save(data_dir, &index);
            }
            return Some(cached);
        }
        debug!(path = %cached.display(), "dedupe cache entry stale or too large to verify; dropped");
        let _ = std::fs::remove_file(&cached);
    }
    let hit = scan_inbox(&inbox_dir(data_dir), &sha, size, &index)?;
    if insert(&mut index, &sha, &hit) {
        save(data_dir, &index);
    }
    Some(hit)
}

/// Hash same-size inbox files not already indexed (bounded).
fn scan_inbox(inbox: &Path, sha: &str, size: u64, index: &Index) -> Option<PathBuf> {
    let rd = std::fs::read_dir(inbox).ok()?;
    let mut budget_files = SCAN_MAX_FILES;
    let mut budget_bytes = SCAN_MAX_BYTES;
    for e in rd.flatten() {
        if budget_files == 0 || budget_bytes < size {
            break;
        }
        let name = e.file_name();
        if name.to_string_lossy().starts_with('.') {
            continue;
        }
        let Ok(meta) = e.metadata() else {
            continue;
        };
        if !meta.is_file() || meta.len() != size {
            continue;
        }
        let path = e.path();
        if index.entries.values().any(|x| x.path == path) {
            // Indexed under another digest and still valid: not this content.
            continue;
        }
        budget_files -= 1;
        budget_bytes -= size;
        match file_sha256(&path) {
            Ok(h) if h == sha => return Some(path),
            Ok(_) => {}
            Err(e) => debug!(error = %e, path = %path.display(), "dedupe scan read"),
        }
    }
    None
}

/// Make `existing`'s content available at `dst` without copying bytes when
/// the filesystem allows (hard link), else by copy.
pub(crate) fn link_or_copy(existing: &Path, dst: &Path) -> std::io::Result<()> {
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent)?;
    }
    match std::fs::hard_link(existing, dst) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Err(e),
        Err(_) => std::fs::copy(existing, dst).map(|_| ()),
    }
}

/// Whether `path` lives in the hidden payload cache (not a user-visible file).
pub(crate) fn is_cached(data_dir: &Path, path: &Path) -> bool {
    path.starts_with(cache_dir(data_dir))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "tetherly-dedupe-{tag}-{}-{}",
            std::process::id(),
            now_ms()
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(inbox_dir(&d)).unwrap();
        d
    }

    fn sha_of(bytes: &[u8]) -> String {
        let mut h = tetherly_core::IncrementalSha256::new();
        h.update(bytes);
        h.finalize_hex()
    }

    #[test]
    fn recorded_file_is_found_until_it_changes() {
        let d = tmp("rec");
        let f = inbox_dir(&d).join("a.txt");
        std::fs::write(&f, b"hello").unwrap();
        let sha = sha_of(b"hello");
        record(&d, &sha, &f);
        assert_eq!(lookup(&d, &sha, 5), Some(f.clone()));
        // Size mismatch with the offer never matches.
        assert_eq!(lookup(&d, &sha, 6), None);
        // Content edited (size changes): stale entry dropped; the scan hashes
        // the edited file, which no longer matches.
        std::fs::write(&f, b"hello world").unwrap();
        assert_eq!(lookup(&d, &sha, 5), None);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn unindexed_inbox_file_is_found_by_scan() {
        let d = tmp("scan");
        std::fs::write(inbox_dir(&d).join("other.bin"), b"xxxxx").unwrap();
        let f = inbox_dir(&d).join("old.bin");
        std::fs::write(&f, b"abcde").unwrap();
        let sha = sha_of(b"abcde");
        assert_eq!(lookup(&d, &sha, 5), Some(f.clone()));
        // Now indexed: found without a scan even if a decoy appears.
        assert_eq!(load(&d).entries.get(&sha).map(|e| e.path.clone()), Some(f));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn retained_payload_outlives_its_source() {
        let d = tmp("ret");
        let drag = d.join("native").join("x.png");
        std::fs::create_dir_all(drag.parent().unwrap()).unwrap();
        std::fs::write(&drag, b"pixels").unwrap();
        let sha = sha_of(b"pixels");
        retain(&d, &sha, &drag);
        std::fs::remove_file(&drag).unwrap();
        let hit = lookup(&d, &sha, 6).expect("cached");
        assert!(is_cached(&d, &hit));
        assert_eq!(std::fs::read(&hit).unwrap(), b"pixels");
        let out = d.join("native2").join("x.png");
        link_or_copy(&hit, &out).unwrap();
        assert_eq!(std::fs::read(&out).unwrap(), b"pixels");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn cache_edited_through_its_link_never_matches() {
        let d = tmp("alias");
        let drag = d.join("native").join("doc.txt");
        std::fs::create_dir_all(drag.parent().unwrap()).unwrap();
        std::fs::write(&drag, b"pixels").unwrap();
        let sha = sha_of(b"pixels");
        retain(&d, &sha, &drag);
        // An app the payload was dropped into saves in place, same size:
        // the cache shares the inode, so its bytes change too.
        let before = std::fs::metadata(&drag).unwrap().modified().unwrap();
        std::fs::write(&drag, b"PIXELS").unwrap();
        // A real save lands seconds later; Linux/NTFS timestamps are coarse
        // enough that an immediate rewrite can keep the same mtime_ms.
        std::fs::File::options()
            .write(true)
            .open(&drag)
            .unwrap()
            .set_modified(before + std::time::Duration::from_secs(2))
            .unwrap();
        assert_eq!(lookup(&d, &sha, 6), None);
        assert!(!cache_dir(&d).join(&sha).exists(), "stale cache dropped");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn empty_or_malformed_digest_never_matches() {
        let d = tmp("bad");
        assert_eq!(lookup(&d, "zz", 5), None);
        assert_eq!(lookup(&d, &sha_of(b""), 0), None);
        let _ = std::fs::remove_dir_all(&d);
    }
}
