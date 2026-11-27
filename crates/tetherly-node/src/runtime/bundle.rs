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

/// Pack the folder `src` into the tar file `dst` (entries rooted at the
/// folder's contents). Returns the number of files packed.
pub(crate) fn pack_dir(src: &Path, dst: &Path) -> Result<usize, String> {
    let out = std::fs::File::create(dst).map_err(|e| format!("创建打包文件失败：{e}"))?;
    let mut builder = tar::Builder::new(std::io::BufWriter::new(out));
    builder.follow_symlinks(false);
    let mut files = 0usize;
    let mut entries = 0usize;
    let mut total = 0u64;
    let mut stack: Vec<(PathBuf, PathBuf)> = vec![(src.to_path_buf(), PathBuf::new())];
    while let Some((dir, rel)) = stack.pop() {
        let mut children: Vec<_> = std::fs::read_dir(&dir)
            .map_err(|e| format!("无法读取 {}：{e}", dir.display()))?
            .flatten()
            .collect();
        children.sort_by_key(|e| e.file_name());
        for child in children {
            let name = child.file_name();
            let Some(name_str) = name.to_str() else {
                continue; // not representable on the peer
            };
            if is_litter(name_str) {
                continue;
            }
            let Ok(ft) = child.file_type() else { continue };
            let path = child.path();
            let rel_child = rel.join(&name);
            entries += 1;
            if entries > MAX_ENTRIES {
                return Err(format!("{} 文件过多（上限 {MAX_ENTRIES}）", src.display()));
            }
            if ft.is_dir() {
                builder
                    .append_dir(&rel_child, &path)
                    .map_err(|e| format!("打包 {} 失败：{e}", path.display()))?;
                stack.push((path, rel_child));
            } else if ft.is_file() {
                let len = child.metadata().map(|m| m.len()).unwrap_or(0);
                total = total.saturating_add(len);
                if total > tetherly_core::FILE_MAX {
                    return Err(format!("{} 超过单次传输上限", src.display()));
                }
                let mut f = std::fs::File::open(&path)
                    .map_err(|e| format!("无法读取 {}：{e}", path.display()))?;
                builder
                    .append_file(&rel_child, &mut f)
                    .map_err(|e| format!("打包 {} 失败：{e}", path.display()))?;
                files += 1;
            }
            // Symlinks and special files are skipped.
        }
    }
    builder
        .into_inner()
        .and_then(|mut w| std::io::Write::flush(&mut w))
        .map_err(|e| format!("打包失败：{e}"))?;
    Ok(files)
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
        assert_eq!(pack_dir(&src, &tar).unwrap(), 3);
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
        assert_eq!(pack_dir(&src, &tar).unwrap(), 1);
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
