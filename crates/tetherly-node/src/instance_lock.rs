// SPDX-License-Identifier: Apache-2.0 OR MIT
//! One daemon per data dir (v0.2.99).
//!
//! Production 2026-11-26: `service install` (launchd KeepAlive) and the
//! packaging script's `open` each started a daemon 0.4 s apart on the same
//! data dir. Both minted `api.token`/`client.token` before binding the UI
//! port, so the loser rotated the winner's tokens and then died on
//! `AddrInUse` — every shell/CLI call afterwards got 401. On Windows the
//! per-user `TetherlyDaemon` task can race the SYSTEM launcher the same way.
//!
//! `run` takes this lock first, before any token or socket exists.
//!
//! INVARIANTS:
//! - Held for the whole process lifetime; the OS drops it on exit or crash,
//!   so a stale lock file never blocks a restart.
//! - Only a genuine "another process holds it" is [`LockError::Held`]. Any
//!   other failure (ACL between SYSTEM and the user, read-only dir) is
//!   [`LockError::Io`] and the caller starts anyway: refusing to start on a
//!   lock we cannot even evaluate would be worse than the race it prevents.
//! - Waits up to `wait` for the holder to go away: a self-restart spawns
//!   the successor before the predecessor exits (`lifecycle.rs`, ≤ ~1 s).

use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Lock file name inside the data dir.
pub const LOCK_FILE: &str = "daemon.lock";

/// The held lock; dropping it (or exiting) releases it.
#[derive(Debug)]
pub struct InstanceLock {
    _file: File,
    path: PathBuf,
}

impl InstanceLock {
    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[derive(Debug)]
pub enum LockError {
    /// Another live process holds the lock for this data dir.
    Held(PathBuf),
    /// The lock could not be evaluated; callers proceed without it.
    Io(PathBuf, std::io::Error),
}

impl std::fmt::Display for LockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LockError::Held(p) => write!(f, "another tetherly daemon holds {}", p.display()),
            LockError::Io(p, e) => write!(f, "instance lock {} unavailable: {e}", p.display()),
        }
    }
}

impl std::error::Error for LockError {}

pub fn lock_path(data_dir: &Path) -> PathBuf {
    data_dir.join(LOCK_FILE)
}

/// Acquire the per-data-dir daemon lock, polling until `wait` elapses.
pub fn acquire(data_dir: &Path, wait: Duration) -> Result<InstanceLock, LockError> {
    let path = lock_path(data_dir);
    let deadline = Instant::now() + wait;
    loop {
        match try_acquire_once(&path) {
            Ok(file) => {
                write_pid(&file);
                return Ok(InstanceLock { _file: file, path });
            }
            Err(Attempt::Held) => {
                if Instant::now() >= deadline {
                    return Err(LockError::Held(path));
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(Attempt::Io(e)) => return Err(LockError::Io(path, e)),
        }
    }
}

enum Attempt {
    Held,
    Io(std::io::Error),
}

#[cfg(unix)]
fn try_acquire_once(path: &Path) -> Result<File, Attempt> {
    use std::os::unix::fs::OpenOptionsExt;
    use std::os::unix::io::AsRawFd;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(path)
        .map_err(Attempt::Io)?;
    // flock locks belong to the open file description, so a second open in
    // this same process conflicts too (the unit tests rely on that).
    let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if rc == 0 {
        return Ok(file);
    }
    let err = std::io::Error::last_os_error();
    if err.raw_os_error() == Some(libc::EWOULDBLOCK) {
        Err(Attempt::Held)
    } else {
        Err(Attempt::Io(err))
    }
}

#[cfg(windows)]
fn try_acquire_once(path: &Path) -> Result<File, Attempt> {
    use std::os::windows::fs::OpenOptionsExt;
    const ERROR_SHARING_VIOLATION: i32 = 32;
    const ERROR_LOCK_VIOLATION: i32 = 33;
    // Share mode 0: while this handle is open no other open succeeds; the
    // kernel closes it when the process dies.
    match std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .share_mode(0)
        .open(path)
    {
        Ok(f) => Ok(f),
        Err(e)
            if matches!(
                e.raw_os_error(),
                Some(ERROR_SHARING_VIOLATION) | Some(ERROR_LOCK_VIOLATION)
            ) =>
        {
            Err(Attempt::Held)
        }
        Err(e) => Err(Attempt::Io(e)),
    }
}

#[cfg(not(any(unix, windows)))]
fn try_acquire_once(path: &Path) -> Result<File, Attempt> {
    std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(Attempt::Io)
}

/// Best-effort diagnostics: the holder's pid, readable on unix.
fn write_pid(file: &File) {
    use std::io::{Seek, SeekFrom, Write};
    let mut f = file;
    let _ = f.set_len(0);
    let _ = f.seek(SeekFrom::Start(0));
    let _ = writeln!(f, "{}", std::process::id());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> PathBuf {
        // A counter, not a timestamp: parallel tests on macOS can read the
        // same microsecond-resolution clock value.
        static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let d = std::env::temp_dir().join(format!("tetherly-lock-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn second_acquire_is_held_until_first_drops() {
        let dir = tmp();
        let first = acquire(&dir, Duration::ZERO).expect("first lock");
        match acquire(&dir, Duration::from_millis(250)) {
            Err(LockError::Held(p)) => assert_eq!(p, lock_path(&dir)),
            other => panic!("expected Held, got {other:?}"),
        }
        drop(first);
        let again = acquire(&dir, Duration::ZERO).expect("released on drop");
        assert_eq!(again.path(), lock_path(&dir).as_path());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn waiter_gets_lock_when_holder_exits_within_wait() {
        let dir = tmp();
        let first = acquire(&dir, Duration::ZERO).expect("first lock");
        let releaser = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            drop(first);
        });
        acquire(&dir, Duration::from_secs(5)).expect("self-restart successor waits");
        releaser.join().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn stale_lock_file_does_not_block() {
        let dir = tmp();
        std::fs::write(lock_path(&dir), "99999\n").unwrap();
        acquire(&dir, Duration::ZERO).expect("leftover file is not a lock");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_dir_is_io_not_held() {
        let dir = tmp().join("does-not-exist");
        match acquire(&dir, Duration::ZERO) {
            Err(LockError::Io(..)) => {}
            other => panic!("expected Io, got {other:?}"),
        }
    }
}
