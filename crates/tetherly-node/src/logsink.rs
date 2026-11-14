// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Daemon log sink: a size-capped rotating log file plus an optional stdout
//! mirror.
//!
//! The daemon used to append every tracing event to `tetherly.log` forever
//! (Mac 17.8 MB after twelve days) and, on Windows, to write the same bytes a
//! second time through stdout into the launcher's `>> tetherly_system.log`
//! redirect. `RotatingFile` bounds the file at `cap` bytes, keeping one
//! previous generation as `<name>.1`; [`mirror_stdout`] keeps the console
//! copy only where somebody can actually see it.

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Size at which `tetherly.log` rolls over to `tetherly.log.1`.
pub const LOG_FILE_CAP: u64 = 16 * 1024 * 1024;

/// Append-only log file that rolls over to `<path>.1` once it would grow
/// past `cap` bytes. At most `2 * cap` bytes of logs live on disk.
pub struct RotatingFile {
    path: PathBuf,
    file: File,
    len: u64,
    cap: u64,
}

impl RotatingFile {
    pub fn open(path: &Path, cap: u64) -> io::Result<Self> {
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        let len = file.metadata()?.len();
        Ok(Self {
            path: path.to_path_buf(),
            file,
            len,
            cap,
        })
    }

    fn rotated_path(&self) -> PathBuf {
        let mut name = self.path.as_os_str().to_owned();
        name.push(".1");
        PathBuf::from(name)
    }

    fn rotate(&mut self) -> io::Result<()> {
        // Renaming a file this process holds open is fine on unix and on
        // Windows (std opens with FILE_SHARE_DELETE); `fs::rename` replaces
        // an existing `.1`. If the move is blocked, truncate in place: that
        // still bounds the size, losing only history. The truncation goes
        // through a fresh write handle because an append-only handle lacks
        // FILE_WRITE_DATA on Windows, so `set_len` on it fails there.
        if std::fs::rename(&self.path, self.rotated_path()).is_err() {
            OpenOptions::new()
                .write(true)
                .truncate(true)
                .open(&self.path)?;
        }
        self.file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        self.len = 0;
        Ok(())
    }

    pub fn write_record(&mut self, buf: &[u8]) -> io::Result<()> {
        if self.len > 0 && self.len + buf.len() as u64 > self.cap && self.rotate().is_err() {
            // A transiently failed rotation keeps the record, but a file that
            // can neither move nor shrink must not grow without bound.
            if self.len + buf.len() as u64 > self.cap.saturating_mul(2) {
                return Ok(());
            }
        }
        self.file.write_all(buf)?;
        self.len += buf.len() as u64;
        Ok(())
    }

    pub fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

/// Whether log lines should also go to stdout: only when the file could not
/// be opened (stdout is then the only sink) or a human is watching a
/// terminal. Daemons launched by launchd / the Windows scheduled task have
/// stdout pointed at /dev/null or the launcher's redirect file, where the
/// copy is pure duplicate I/O on the input hot path.
pub fn mirror_stdout(file_ok: bool, stdout_is_terminal: bool) -> bool {
    !file_ok || stdout_is_terminal
}

/// `MakeWriter` target for tracing-subscriber: cheap to clone, one shared
/// rotating file behind a mutex.
#[derive(Clone)]
pub struct LogSink {
    file: Option<Arc<Mutex<RotatingFile>>>,
    stdout: bool,
}

impl LogSink {
    pub fn new(file: Option<RotatingFile>, stdout: bool) -> Self {
        Self {
            file: file.map(|f| Arc::new(Mutex::new(f))),
            stdout,
        }
    }
}

impl Write for LogSink {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.stdout {
            let _ = io::stdout().write_all(buf);
        }
        if let Some(file) = &self.file {
            if let Ok(mut f) = file.lock() {
                let _ = f.write_record(buf);
            }
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.stdout {
            let _ = io::stdout().flush();
        }
        if let Some(file) = &self.file {
            if let Ok(mut f) = file.lock() {
                let _ = f.flush();
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "tetherly-logsink-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn size_stays_bounded_when_rotation_cannot_rename() {
        // A directory squatting on `<log>.1` makes the rename fail on every
        // platform; the file must still be truncated rather than grow forever.
        // (On Windows the append-only handle cannot `set_len`, which is why
        // truncation goes through a separate write handle.)
        let d = tmpdir("blocked-rename");
        let path = d.join("t.log");
        std::fs::create_dir_all(d.join("t.log.1").join("x")).unwrap();
        let mut f = RotatingFile::open(&path, 64).unwrap();
        for _ in 0..50 {
            f.write_record(&[b'x'; 40]).unwrap();
        }
        f.flush().unwrap();
        let len = std::fs::metadata(&path).unwrap().len();
        assert!(len <= 64, "log grew to {len} bytes despite a 64 byte cap");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn rotating_file_appends_below_cap() {
        let d = tmpdir("append");
        let p = d.join("t.log");
        std::fs::write(&p, b"old\n").unwrap();
        let mut f = RotatingFile::open(&p, 1024).unwrap();
        f.write_record(b"new\n").unwrap();
        f.flush().unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"old\nnew\n");
        assert!(!d.join("t.log.1").exists());
    }

    #[test]
    fn rotating_file_rolls_over_to_dot_one_when_cap_exceeded() {
        let d = tmpdir("roll");
        let p = d.join("t.log");
        let mut f = RotatingFile::open(&p, 10).unwrap();
        f.write_record(b"aaaaaaaa\n").unwrap(); // 9 bytes
        f.write_record(b"bbbb\n").unwrap(); // would reach 14 > 10 -> rotate
        f.flush().unwrap();
        assert_eq!(std::fs::read(d.join("t.log.1")).unwrap(), b"aaaaaaaa\n");
        assert_eq!(std::fs::read(&p).unwrap(), b"bbbb\n");
    }

    #[test]
    fn rotating_file_keeps_only_one_previous_generation() {
        let d = tmpdir("gen");
        let p = d.join("t.log");
        let mut f = RotatingFile::open(&p, 4).unwrap();
        for rec in [b"111\n", b"222\n", b"333\n"] {
            f.write_record(rec).unwrap();
        }
        f.flush().unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"333\n");
        assert_eq!(std::fs::read(d.join("t.log.1")).unwrap(), b"222\n");
        assert!(!d.join("t.log.2").exists());
    }

    #[test]
    fn rotating_file_counts_preexisting_size_toward_cap() {
        let d = tmpdir("pre");
        let p = d.join("t.log");
        std::fs::write(&p, vec![b'x'; 20]).unwrap();
        let mut f = RotatingFile::open(&p, 16).unwrap();
        f.write_record(b"y\n").unwrap();
        f.flush().unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"y\n");
        assert_eq!(std::fs::read(d.join("t.log.1")).unwrap().len(), 20);
    }

    #[test]
    fn oversized_single_record_is_still_written() {
        let d = tmpdir("big");
        let p = d.join("t.log");
        let mut f = RotatingFile::open(&p, 4).unwrap();
        f.write_record(b"0123456789\n").unwrap();
        f.flush().unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"0123456789\n");
    }

    #[test]
    fn stdout_mirror_only_for_terminals_or_missing_file() {
        assert!(
            !mirror_stdout(true, false),
            "daemon with file: no duplicate"
        );
        assert!(
            mirror_stdout(true, true),
            "interactive terminal keeps console"
        );
        assert!(
            mirror_stdout(false, false),
            "no file: stdout is the only sink"
        );
    }

    #[test]
    fn log_sink_writes_records_to_file() {
        let d = tmpdir("sink");
        let p = d.join("t.log");
        let mut sink = LogSink::new(Some(RotatingFile::open(&p, 1024).unwrap()), false);
        let mut clone = sink.clone();
        sink.write_all(b"one\n").unwrap();
        clone.write_all(b"two\n").unwrap();
        sink.flush().unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"one\ntwo\n");
    }
}
