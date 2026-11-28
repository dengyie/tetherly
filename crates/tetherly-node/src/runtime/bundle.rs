//! v0.3.3 folder transfer: a dragged folder travels as one tar transfer
//! (`FileMeta::bundle == BUNDLE_DIR_TAR`, name `<folder>.tar`) and the
//! receiver unpacks it into a real folder.
//!
//! One transfer instead of one per file keeps a folder of many small files
//! fast (one offer, one channel, one digest), and an older receiver still
//! gets a usable `.tar`.
//!
//! INVARIANTS:
//! - Packing never follows symlinks (they are skipped), so a folder cannot
//!   smuggle files from outside itself.
//! - Unpacking only creates regular files and directories, every path
//!   component must be a plain sanitized name, and nothing lands outside the
//!   destination directory.

use std::path::{Component, Path, PathBuf};

/// Suffix of a bundled folder's transfer name.
pub(crate) const TAR_SUFFIX: &str = ".tar";
/// Entries per bundle (files + directories), both when packing and unpacking.
pub(crate) const MAX_ENTRIES: usize = 100_000;

/// Finder / Explorer litter not worth carrying.
fn is_litter(name: &str) -> bool {
    name == ".DS_Store" || name.starts_with("._") || name.eq_ignore_ascii_case("Thumbs.db")
}

/// Bytes `tar::Builder::into_inner` appends: two zero blocks.
const TAR_TRAILER: u64 = 1024;
/// Tar data is padded to this block.
const BLOCK: u64 = 512;

fn pad_block(n: u64) -> u64 {
    n.div_ceil(BLOCK) * BLOCK
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EntryKind {
    Dir,
    File,
}

/// One entry of a scanned folder, with the metadata its tar header is
/// built from (so the header bytes depend on the scan, not on when the
/// entry is written).
#[derive(Debug)]
struct ScanEntry {
    path: PathBuf,
    rel: PathBuf,
    kind: EntryKind,
    len: u64,
    mtime: (u64, u32),
    mode: u32,
}

impl ScanEntry {
    fn header(&self, size: u64) -> tar::Header {
        let mut h = tar::Header::new_gnu();
        h.set_entry_type(match self.kind {
            EntryKind::Dir => tar::EntryType::Directory,
            EntryKind::File => tar::EntryType::Regular,
        });
        h.set_size(size);
        h.set_mode(self.mode);
        h.set_mtime(self.mtime.0);
        h.set_uid(0);
        h.set_gid(0);
        h
    }
}

fn mtime_of(meta: &std::fs::Metadata) -> (u64, u32) {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| (d.as_secs(), d.subsec_nanos()))
        .unwrap_or((0, 0))
}

fn mode_of(meta: &std::fs::Metadata, kind: EntryKind) -> u32 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = kind;
        meta.permissions().mode() & 0o777
    }
    #[cfg(not(unix))]
    {
        let _ = meta;
        match kind {
            EntryKind::Dir => 0o755,
            EntryKind::File => 0o644,
        }
    }
}

/// Counts what a tar builder writes.
struct Counter(u64);

impl std::io::Write for Counter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0 += buf.len() as u64;
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// A folder walked once: the entries in archive order, the exact size of
/// the tar [`write_tar`] produces from them, and a fingerprint of the tree.
#[derive(Debug)]
pub(crate) struct DirScan {
    entries: Vec<ScanEntry>,
    /// Regular files in the bundle.
    pub(crate) files: usize,
    /// Exact tar size in bytes (headers, padding and trailer included).
    pub(crate) total: u64,
    /// SHA-256 (lower hex) over the folder name and every entry's relative
    /// path, kind, length and mtime: equal for an unchanged folder, so a
    /// re-offer reuses the transfer id and resumes.
    pub(crate) fingerprint: String,
}

/// Walk `src` (sorted, litter and symlinks skipped) within the transfer cap.
pub(crate) fn scan_dir(src: &Path) -> Result<DirScan, String> {
    scan_dir_limited(src, tetherly_core::FILE_MAX)
}

/// [`scan_dir`] with the cap as a parameter. The cap applies to the tar
/// itself, since that is what is offered. Header sizes come from a dry run
/// of the same builder [`write_tar`] uses (GNU long names included), so
/// `total` is exact rather than estimated.
fn scan_dir_limited(src: &Path, limit: u64) -> Result<DirScan, String> {
    let mut headers = tar::Builder::new(Counter(0));
    let mut fp = tetherly_core::IncrementalSha256::new();
    fp.update(b"tetherly-dir-v1\0");
    if let Some(name) = src.file_name() {
        fp.update(name.to_string_lossy().as_bytes());
    }
    let mut entries = Vec::new();
    let mut files = 0usize;
    let mut data = 0u64;
    let mut stack: Vec<(PathBuf, PathBuf)> = vec![(src.to_path_buf(), PathBuf::new())];
    while let Some((dir, rel)) = stack.pop() {
        let mut children: Vec<_> = std::fs::read_dir(&dir)
            .map_err(|e| format!("无法读取 {}：{e}", dir.display()))?
            .flatten()
            .collect();
        children.sort_by_key(|e| e.file_name());
        // Pushed in reverse so the walk pops them in sorted order.
        let mut subdirs = Vec::new();
        for child in children {
            let name = child.file_name();
            let Some(name_str) = name.to_str() else {
                continue; // not representable on the peer
            };
            if is_litter(name_str) {
                continue;
            }
            let Ok(ft) = child.file_type() else { continue };
            let kind = if ft.is_dir() {
                EntryKind::Dir
            } else if ft.is_file() {
                EntryKind::File
            } else {
                continue; // symlinks and special files are skipped
            };
            let Ok(meta) = child.metadata() else { continue };
            let entry = ScanEntry {
                path: child.path(),
                rel: rel.join(&name),
                kind,
                len: if kind == EntryKind::File {
                    meta.len()
                } else {
                    0
                },
                mtime: mtime_of(&meta),
                mode: mode_of(&meta, kind),
            };
            if entries.len() >= MAX_ENTRIES {
                return Err(format!("{} 文件过多（上限 {MAX_ENTRIES}）", src.display()));
            }
            headers
                .append_data(&mut entry.header(0), &entry.rel, std::io::empty())
                .map_err(|e| format!("打包 {} 失败：{e}", entry.path.display()))?;
            data = data.saturating_add(pad_block(entry.len));
            let total = headers.get_ref().0 + data + TAR_TRAILER;
            if total > limit {
                return Err(format!("{} 超过单次传输上限", src.display()));
            }
            fp.update(entry.rel.to_string_lossy().as_bytes());
            fp.update(&[0, kind as u8]);
            fp.update(&entry.len.to_le_bytes());
            fp.update(&entry.mtime.0.to_le_bytes());
            fp.update(&entry.mtime.1.to_le_bytes());
            if kind == EntryKind::Dir {
                subdirs.push((entry.path.clone(), entry.rel.clone()));
            } else {
                files += 1;
            }
            entries.push(entry);
        }
        stack.extend(subdirs.into_iter().rev());
    }
    let total = headers.get_ref().0 + data + TAR_TRAILER;
    Ok(DirScan {
        entries,
        files,
        total,
        fingerprint: fp.finalize_hex(),
    })
}

/// Reads exactly `left` bytes, failing on a short source (a file truncated
/// after the scan) instead of letting the tar writer pad a short entry.
struct ExactReader<R> {
    inner: R,
    left: u64,
}

impl<R: std::io::Read> std::io::Read for ExactReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.left == 0 {
            return Ok(0);
        }
        let want = (buf.len() as u64).min(self.left) as usize;
        let n = self.inner.read(&mut buf[..want])?;
        if n == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "file shrank while being sent",
            ));
        }
        self.left -= n as u64;
        Ok(n)
    }
}

fn changed(path: &Path) -> std::io::Error {
    std::io::Error::other(format!("{} 在发送过程中被修改", path.display()))
}

/// Write the tar of `scan` into `out`: exactly `scan.total` bytes, or an
/// error if a file changed since the scan (length or mtime).
fn write_tar<W: std::io::Write>(scan: &DirScan, out: W) -> std::io::Result<W> {
    let mut b = tar::Builder::new(out);
    for e in &scan.entries {
        match e.kind {
            EntryKind::Dir => b.append_data(&mut e.header(0), &e.rel, std::io::empty())?,
            EntryKind::File => {
                let f = std::fs::File::open(&e.path)?;
                let meta = f.metadata()?;
                if meta.len() != e.len || mtime_of(&meta) != e.mtime {
                    return Err(changed(&e.path));
                }
                let reader = ExactReader {
                    inner: std::io::Read::take(f, e.len),
                    left: e.len,
                };
                b.append_data(&mut e.header(e.len), &e.rel, reader)?;
            }
        }
    }
    b.into_inner()
}

/// What [`pack_dir`] produced.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Packed {
    /// Regular files packed.
    pub files: usize,
    /// SHA-256 (lower hex) of the tar file, hashed as it was written so the
    /// offer does not read the whole bundle again.
    pub sha256: String,
}

/// Hashes every byte on its way to the inner writer.
struct HashingWriter<W> {
    inner: W,
    hasher: tetherly_core::IncrementalSha256,
}

impl<W: std::io::Write> std::io::Write for HashingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.hasher.update(&buf[..n]);
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

/// Pack the folder `src` into the tar file `dst` (entries rooted at the
/// folder's contents). The staging path for peers without
/// [`tetherly_core::FILE_STREAM_CAP`]; others get [`spawn_dir_stream`].
pub(crate) fn pack_dir(src: &Path, dst: &Path) -> Result<Packed, String> {
    pack_dir_limited(src, dst, tetherly_core::FILE_MAX)
}

fn pack_dir_limited(src: &Path, dst: &Path, limit: u64) -> Result<Packed, String> {
    let scan = scan_dir_limited(src, limit)?;
    let out = std::fs::File::create(dst).map_err(|e| format!("创建打包文件失败：{e}"))?;
    // BufWriter outside the hasher: it sees large chunks, not 512 B blocks.
    let buffered = std::io::BufWriter::new(HashingWriter {
        inner: out,
        hasher: tetherly_core::IncrementalSha256::new(),
    });
    let hashing = write_tar(&scan, buffered)
        .and_then(|w| w.into_inner().map_err(|e| e.into_error()))
        .map_err(|e| format!("打包失败：{e}"))?;
    Ok(Packed {
        files: scan.files,
        sha256: hashing.hasher.finalize_hex(),
    })
}

/// Write side of a streamed bundle: drops the bytes before `start` (the
/// receiver already holds them; they are still hashed upstream) and hands
/// the rest out in channel-chunk pieces.
struct ChunkSink {
    tx: tokio::sync::mpsc::Sender<Vec<u8>>,
    pos: u64,
    start: u64,
    total: u64,
    buf: Vec<u8>,
}

impl ChunkSink {
    fn send(&mut self, chunk: Vec<u8>) -> std::io::Result<()> {
        self.tx.blocking_send(chunk).map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::BrokenPipe, "folder stream reader gone")
        })
    }
}

impl std::io::Write for ChunkSink {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        let end = self.pos + data.len() as u64;
        if end > self.total {
            return Err(std::io::Error::other("folder grew past its scanned size"));
        }
        let skip = self.start.saturating_sub(self.pos).min(data.len() as u64) as usize;
        self.buf.extend_from_slice(&data[skip..]);
        self.pos = end;
        while self.buf.len() >= STREAM_CHUNK {
            let rest = self.buf.split_off(STREAM_CHUNK);
            let chunk = std::mem::replace(&mut self.buf, rest);
            self.send(chunk)?;
        }
        Ok(data.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        if !self.buf.is_empty() {
            let chunk = std::mem::take(&mut self.buf);
            self.send(chunk)?;
        }
        Ok(())
    }
}

/// Channel piece size of a streamed bundle (the file channel's chunk).
const STREAM_CHUNK: usize = tetherly_core::FILE_CHUNK;
/// Pieces buffered between the tar writer and the socket.
const STREAM_DEPTH: usize = 8;

/// Generate the tar of `scan` into `tx` from byte `start` on; returns the
/// SHA-256 of the whole tar (prefix included). Blocking.
fn stream_dir(
    scan: &DirScan,
    start: u64,
    tx: tokio::sync::mpsc::Sender<Vec<u8>>,
) -> std::io::Result<[u8; 32]> {
    let sink = ChunkSink {
        tx,
        pos: 0,
        start,
        total: scan.total,
        buf: Vec::with_capacity(STREAM_CHUNK),
    };
    let buffered = std::io::BufWriter::with_capacity(
        STREAM_CHUNK,
        HashingWriter {
            inner: sink,
            hasher: tetherly_core::IncrementalSha256::new(),
        },
    );
    let mut hashing = write_tar(scan, buffered)?
        .into_inner()
        .map_err(|e| e.into_error())?;
    std::io::Write::flush(&mut hashing.inner)?;
    if hashing.inner.pos != scan.total {
        return Err(std::io::Error::other(format!(
            "folder stream produced {} of {} bytes",
            hashing.inner.pos, scan.total
        )));
    }
    Ok(hashing.hasher.finalize_bytes())
}

/// Async read side of a streamed bundle. EOF early means the generator
/// stopped; its join handle says why.
pub(crate) struct ChunkReader {
    rx: tokio::sync::mpsc::Receiver<Vec<u8>>,
    cur: Vec<u8>,
    pos: usize,
}

impl ChunkReader {
    /// Wait for the first piece. On a resume the generator re-reads and
    /// hashes the whole prefix before it yields anything; waiting here,
    /// before the data channel opens, keeps that time off the channel's
    /// per-chunk deadline.
    pub(crate) async fn prime(&mut self) {
        if self.pos >= self.cur.len() {
            if let Some(c) = self.rx.recv().await {
                self.cur = c;
                self.pos = 0;
            }
        }
    }
}

impl tokio::io::AsyncRead for ChunkReader {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        use std::task::Poll;
        let this = self.get_mut();
        while this.pos >= this.cur.len() {
            match this.rx.poll_recv(cx) {
                Poll::Ready(Some(c)) => {
                    this.cur = c;
                    this.pos = 0;
                }
                Poll::Ready(None) => return Poll::Ready(Ok(())),
                Poll::Pending => return Poll::Pending,
            }
        }
        let n = buf.remaining().min(this.cur.len() - this.pos);
        buf.put_slice(&this.cur[this.pos..this.pos + n]);
        this.pos += n;
        Poll::Ready(Ok(()))
    }
}

/// Start generating the tar of `scan` from `start` on a blocking thread.
/// Dropping the reader stops the generator (its next send fails with
/// `BrokenPipe`); the handle yields the whole-tar digest.
pub(crate) fn spawn_dir_stream(
    scan: std::sync::Arc<DirScan>,
    start: u64,
) -> (
    ChunkReader,
    tokio::task::JoinHandle<std::io::Result<[u8; 32]>>,
) {
    let (tx, rx) = tokio::sync::mpsc::channel(STREAM_DEPTH);
    let handle = tokio::task::spawn_blocking(move || stream_dir(&scan, start, tx));
    (
        ChunkReader {
            rx,
            cur: Vec::new(),
            pos: 0,
        },
        handle,
    )
}

/// The folder name a bundle named `name` unpacks to.
pub(crate) fn folder_name(name: &str) -> String {
    match name.strip_suffix(TAR_SUFFIX) {
        Some(stem) if !stem.is_empty() => stem.to_string(),
        _ => name.to_string(),
    }
}

/// A tar entry path made only of plain, sanitized names.
fn safe_rel(path: &Path) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::Normal(part) => {
                let part = part.to_str()?;
                if tetherly_core::sanitize_file_name(part).ok()?.as_str() != part {
                    return None;
                }
                out.push(part);
            }
            Component::CurDir => {}
            _ => return None,
        }
    }
    (!out.as_os_str().is_empty()).then_some(out)
}

/// What [`unpack_into`] produced.
#[derive(Debug, Default)]
pub(crate) struct Unpacked {
    /// Regular files written.
    pub(crate) files: Vec<PathBuf>,
    /// Entries not reproduced (unsafe, not a file or folder, or not
    /// creatable here, e.g. a Windows path-length or name limit): the folder
    /// is incomplete and the user must be told.
    pub(crate) skipped: usize,
}

/// Unpack the bundle `tar_path` into `dst` (created if missing). Entries
/// that are unsafe or cannot be created on this platform are skipped,
/// logged and counted.
pub(crate) fn unpack_into(tar_path: &Path, dst: &Path) -> std::io::Result<Unpacked> {
    std::fs::create_dir_all(dst)?;
    let file = std::fs::File::open(tar_path)?;
    let mut archive = tar::Archive::new(std::io::BufReader::new(file));
    archive.set_preserve_permissions(false);
    archive.set_overwrite(true);
    let mut out = Unpacked::default();
    for (n, entry) in archive.entries()?.enumerate() {
        if n >= MAX_ENTRIES {
            tracing::warn!(dst = %dst.display(), "bundle: entry limit reached; rest skipped");
            out.skipped += 1;
            break;
        }
        let mut entry = entry?;
        let kind = entry.header().entry_type();
        if !(kind.is_file() || kind.is_dir()) {
            out.skipped += 1;
            continue;
        }
        let raw = entry.path()?.into_owned();
        let Some(rel) = safe_rel(&raw) else {
            tracing::warn!(entry = %raw.display(), "bundle: unsafe entry skipped");
            out.skipped += 1;
            continue;
        };
        let target = dst.join(&rel);
        if kind.is_dir() {
            std::fs::create_dir_all(&target)?;
            continue;
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        match entry.unpack(&target) {
            Ok(_) => out.files.push(target),
            Err(e) => {
                tracing::warn!(entry = %rel.display(), error = %e, "bundle: entry not unpacked");
                out.skipped += 1;
            }
        }
    }
    Ok(out)
}

/// Reserve a fresh folder `name` (or `name (n)`) inside `dir`.
#[cfg(test)]
pub(crate) fn reserve_dir(dir: &Path, name: &str) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    for n in 0u32.. {
        let candidate = if n == 0 {
            dir.join(name)
        } else {
            dir.join(format!("{name} ({n})"))
        };
        match std::fs::create_dir(&candidate) {
            Ok(()) => return Ok(candidate),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    unreachable!("u32 counter exhausted")
}

/// Move the folder `src` into `dir` under `name` (or `name (n)`).
pub(crate) fn move_dir_into(src: &Path, dir: &Path, name: &str) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    for n in 0u32.. {
        let candidate = if n == 0 {
            dir.join(name)
        } else {
            dir.join(format!("{name} ({n})"))
        };
        if candidate.exists() {
            continue;
        }
        return std::fs::rename(src, &candidate).map(|()| candidate);
    }
    unreachable!("u32 counter exhausted")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> PathBuf {
        let mut b = [0u8; 8];
        getrandom::getrandom(&mut b).unwrap();
        let p =
            std::env::temp_dir().join(format!("tetherly-bundle-{tag}-{:x}", u64::from_le_bytes(b)));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn folder_round_trips_without_litter() {
        let root = temp("rt");
        let src = root.join("项目 A");
        std::fs::create_dir_all(src.join("sub/deeper")).unwrap();
        std::fs::create_dir_all(src.join("empty")).unwrap();
        std::fs::write(src.join("a.txt"), b"a").unwrap();
        std::fs::write(src.join("sub/b.bin"), [0u8, 1, 2]).unwrap();
        std::fs::write(src.join("sub/deeper/c 文件.md"), b"c").unwrap();
        std::fs::write(src.join(".DS_Store"), b"x").unwrap();
        std::fs::write(src.join("._a.txt"), b"x").unwrap();
        let tar = root.join("项目 A.tar");
        assert_eq!(pack_dir(&src, &tar).unwrap().files, 3);
        let out = reserve_dir(&root.join("inbox"), &folder_name("项目 A.tar")).unwrap();
        assert!(out.ends_with("项目 A"));
        let unpacked = unpack_into(&tar, &out).unwrap();
        assert_eq!((unpacked.files.len(), unpacked.skipped), (3, 0));
        assert_eq!(std::fs::read(out.join("a.txt")).unwrap(), b"a");
        assert_eq!(std::fs::read(out.join("sub/b.bin")).unwrap(), [0u8, 1, 2]);
        assert_eq!(
            std::fs::read(out.join("sub/deeper/c 文件.md")).unwrap(),
            b"c"
        );
        assert!(out.join("empty").is_dir());
        assert!(!out.join(".DS_Store").exists() && !out.join("._a.txt").exists());
        let again = reserve_dir(&root.join("inbox"), "项目 A").unwrap();
        assert!(again.ends_with("项目 A (1)"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn unsafe_entries_never_leave_the_destination() {
        let root = temp("unsafe");
        let tar_path = root.join("evil.tar");
        {
            let mut b = tar::Builder::new(std::fs::File::create(&tar_path).unwrap());
            for (path, body) in [("../escape.txt", &b"x"[..]), ("ok.txt", &b"ok"[..])] {
                let mut h = tar::Header::new_gnu();
                h.set_size(body.len() as u64);
                h.set_entry_type(tar::EntryType::Regular);
                h.set_mode(0o644);
                // Bypass the builder's own path check to forge the entry.
                let name = &mut h.as_old_mut().name;
                name[..path.len()].copy_from_slice(path.as_bytes());
                h.set_cksum();
                b.append(&h, body).unwrap();
            }
            let mut h = tar::Header::new_gnu();
            h.set_entry_type(tar::EntryType::Symlink);
            h.set_size(0);
            b.append_link(&mut h, "link", "/etc").unwrap();
            b.finish().unwrap();
        }
        let out = root.join("out");
        let unpacked = unpack_into(&tar_path, &out).unwrap();
        assert_eq!(unpacked.files, vec![out.join("ok.txt")]);
        assert_eq!(
            unpacked.skipped, 2,
            "the escape and the symlink are reported"
        );
        assert!(!root.join("escape.txt").exists());
        assert!(!out.join("link").exists());
        let _ = std::fs::remove_dir_all(root);
    }

    /// The scan's size is exact: it equals the tar pack_dir writes, long
    /// names (both sides of the 100-byte ustar limit) and nested
    /// directories included.
    #[test]
    fn scanned_size_matches_the_written_tar() {
        let src = temp("est-src");
        let long = "n".repeat(150);
        std::fs::create_dir_all(src.join("d").join(&long)).unwrap();
        std::fs::write(src.join("a.txt"), vec![1u8; 700]).unwrap();
        std::fs::write(src.join("d").join(&long).join("b.bin"), vec![2u8; 1]).unwrap();
        std::fs::write(src.join("empty"), b"").unwrap();
        for n in 97..=103 {
            std::fs::write(src.join("f".repeat(n)), b"x").unwrap();
            std::fs::create_dir_all(src.join(format!("{}d", "g".repeat(n - 1)))).unwrap();
        }
        let scan = scan_dir(&src).unwrap();
        let tar = temp("est-out").join("x.tar");
        let packed = pack_dir(&src, &tar).unwrap();
        let bytes = std::fs::read(&tar).unwrap();
        let mut h = tetherly_core::IncrementalSha256::new();
        h.update(&bytes);
        assert_eq!(packed.sha256, h.finalize_hex());
        assert_eq!(bytes.len() as u64, scan.total);
        assert_eq!(scan.files, packed.files);
    }

    /// The cap is on the tar, not the payload: 1 KiB of data fits a 2 KiB
    /// limit by bytes, but its header + trailer push the tar past it.
    #[test]
    fn cap_counts_tar_overhead() {
        let src = temp("cap-src");
        std::fs::write(src.join("a"), vec![0u8; 1024]).unwrap();
        let out = temp("cap-out");
        let err = pack_dir_limited(&src, &out.join("x.tar"), 2048).unwrap_err();
        assert!(err.contains("超过单次传输上限"), "{err}");
        let limit = TAR_TRAILER + BLOCK + 1024;
        assert_eq!(scan_dir_limited(&src, limit).unwrap().total, limit);
        assert_eq!(
            pack_dir_limited(&src, &out.join("y.tar"), limit)
                .unwrap()
                .files,
            1
        );
        assert_eq!(std::fs::metadata(out.join("y.tar")).unwrap().len(), limit);
    }

    fn sample_folder(tag: &str) -> PathBuf {
        let src = temp(tag).join("F");
        std::fs::create_dir_all(src.join("sub")).unwrap();
        // Several channel pieces, not piece-aligned.
        let big: Vec<u8> = (0..(3 * STREAM_CHUNK as u32 + 777))
            .map(|i| (i % 251) as u8)
            .collect();
        std::fs::write(src.join("big.bin"), big).unwrap();
        std::fs::write(src.join("sub/s.txt"), b"small").unwrap();
        src
    }

    async fn collect(mut r: ChunkReader) -> Vec<u8> {
        use tokio::io::AsyncReadExt;
        let mut out = Vec::new();
        r.read_to_end(&mut out).await.unwrap();
        out
    }

    /// The streamed bytes and digest are the staged tar's, and a resume
    /// yields exactly the suffix while still digesting the whole tar.
    #[tokio::test]
    async fn stream_matches_the_packed_tar_and_resumes() {
        let src = sample_folder("stream");
        let tar = src.parent().unwrap().join("F.tar");
        let packed = pack_dir(&src, &tar).unwrap();
        let staged = std::fs::read(&tar).unwrap();
        let scan = std::sync::Arc::new(scan_dir(&src).unwrap());
        assert_eq!(scan.total, staged.len() as u64);

        let (reader, gen) = spawn_dir_stream(scan.clone(), 0);
        assert_eq!(collect(reader).await, staged);
        let digest = gen.await.unwrap().unwrap();
        assert_eq!(tetherly_core::hex_lower(&digest), packed.sha256);

        let start = 2 * STREAM_CHUNK as u64;
        let (mut reader, gen) = spawn_dir_stream(scan.clone(), start);
        reader.prime().await;
        assert_eq!(collect(reader).await, staged[start as usize..]);
        assert_eq!(gen.await.unwrap().unwrap(), digest);

        let again = scan_dir(&src).unwrap();
        assert_eq!(again.fingerprint, scan.fingerprint, "unchanged folder");
        std::fs::write(src.join("sub/s.txt"), b"SMALL!").unwrap();
        assert_ne!(scan_dir(&src).unwrap().fingerprint, scan.fingerprint);
    }

    /// A file edited after the scan fails the stream instead of sending
    /// bytes the scanned size and headers no longer describe.
    #[tokio::test]
    async fn file_changed_after_scan_fails_the_stream() {
        let src = sample_folder("changed");
        let scan = std::sync::Arc::new(scan_dir(&src).unwrap());
        std::fs::write(src.join("sub/s.txt"), b"longer than before").unwrap();
        let (reader, gen) = spawn_dir_stream(scan.clone(), 0);
        let got = collect(reader).await;
        assert!((got.len() as u64) < scan.total);
        let err = gen.await.unwrap().unwrap_err();
        assert!(err.to_string().contains("被修改"), "{err}");
    }

    /// Dropping the reader (cancel, socket error) stops the generator.
    #[tokio::test]
    async fn dropped_reader_stops_the_generator() {
        let src = sample_folder("drop");
        let scan = std::sync::Arc::new(scan_dir(&src).unwrap());
        let (mut reader, gen) = spawn_dir_stream(scan, 0);
        reader.prime().await;
        drop(reader);
        let err = gen.await.unwrap().unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::BrokenPipe);
    }

    #[test]
    #[cfg(unix)]
    fn symlinks_are_not_packed() {
        let root = temp("link");
        let outside = root.join("secret.txt");
        std::fs::write(&outside, b"s").unwrap();
        let src = root.join("f");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("real.txt"), b"r").unwrap();
        std::os::unix::fs::symlink(&outside, src.join("link.txt")).unwrap();
        let tar = root.join("f.tar");
        assert_eq!(pack_dir(&src, &tar).unwrap().files, 1);
        let out = root.join("out");
        unpack_into(&tar, &out).unwrap();
        assert!(out.join("real.txt").exists());
        assert!(!out.join("link.txt").exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn folder_moves_aside_on_collision() {
        let root = temp("mv");
        let src = root.join("staged");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("x"), b"1").unwrap();
        std::fs::create_dir_all(root.join("inbox/F")).unwrap();
        let moved = move_dir_into(&src, &root.join("inbox"), "F").unwrap();
        assert!(moved.ends_with("F (1)"));
        assert!(moved.join("x").exists() && !src.exists());
        assert_eq!(folder_name("a.tar"), "a");
        assert_eq!(folder_name(".tar"), ".tar");
        assert_eq!(folder_name("plain"), "plain");
        let _ = std::fs::remove_dir_all(root);
    }
}
