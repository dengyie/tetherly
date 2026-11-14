// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Daemon logging: one tracing pipeline into the rotating `tetherly.log`,
//! a runtime-reloadable filter, a panic hook, and log tailing for the API.
//!
//! Before v0.2.49 the `[takeover]` / `[cursor]` diagnostics went through
//! `eprintln!`, which a daemon never sees (Windows release builds call
//! `FreeConsole`; launchd points stderr at /dev/null), panics vanished
//! without a trace, the filter was fixed at start-up, and reading the log on
//! home-win took an ssh + PowerShell round trip. Everything now lands in the
//! one file, `POST /api/log/level` raises verbosity live (optionally for a
//! bounded time), and `GET /api/log/tail` reads it back over the API.

use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use serde::Serialize;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{reload, EnvFilter, Registry};

use crate::logsink::{LogSink, RotatingFile, LOG_FILE_CAP};

/// Filter used when `RUST_LOG` is unset. Dependencies at `warn` so their
/// real problems surface; mdns-sd is silenced because it logs ERROR
/// "Cannot find valid addrs for TYPE_SRV" continuously on IPv6-only and
/// TUN interfaces (interface-enumeration noise, not a discovery failure).
pub const DEFAULT_FILTER: &str =
    "warn,tetherly=info,tetherly_node=info,tetherly_core=info,tetherly_net=info,mdns_sd=off";

/// Most lines one tail request returns.
pub const TAIL_MAX_LINES: usize = 2000;
/// Bytes scanned backwards per log generation by one tail request.
pub const TAIL_SCAN_BYTES: u64 = 4 * 1024 * 1024;
/// Longest temporary filter override (`revert_after_secs`).
pub const MAX_REVERT_SECS: u64 = 24 * 3600;

type FilterHandle = reload::Handle<EnvFilter, Registry>;

/// Runtime control over the active filter. Separate from the global so it
/// can be exercised in tests against a locally scoped subscriber.
pub struct LogControl {
    handle: FilterHandle,
    baseline: String,
    current: Mutex<String>,
    /// Bumped on every change; a pending revert only fires if nothing
    /// changed the filter after it was scheduled.
    generation: AtomicU64,
    revert_deadline_ms: AtomicU64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LogStatus {
    pub path: Option<String>,
    pub size_bytes: u64,
    pub rotated_size_bytes: u64,
    pub cap_bytes: u64,
    pub filter: String,
    pub baseline_filter: String,
    /// Unix ms at which a temporary override reverts to the baseline.
    pub revert_at_ms: Option<u64>,
}

impl LogControl {
    pub fn new(handle: FilterHandle, baseline: String) -> Self {
        Self {
            handle,
            current: Mutex::new(baseline.clone()),
            baseline,
            generation: AtomicU64::new(0),
            revert_deadline_ms: AtomicU64::new(0),
        }
    }

    pub fn current(&self) -> String {
        self.current.lock().map(|c| c.clone()).unwrap_or_default()
    }

    pub fn baseline(&self) -> &str {
        &self.baseline
    }

    pub fn revert_at_ms(&self) -> Option<u64> {
        match self.revert_deadline_ms.load(Ordering::SeqCst) {
            0 => None,
            ms => Some(ms),
        }
    }

    /// Install `spec` as the active filter. Returns the generation of this
    /// change, used to tie a scheduled revert to it.
    pub fn set(&self, spec: &str) -> Result<u64, String> {
        let filter = parse_filter(spec)?;
        self.handle
            .reload(filter)
            .map_err(|e| format!("filter reload failed: {e}"))?;
        if let Ok(mut c) = self.current.lock() {
            *c = spec.trim().to_string();
        }
        self.revert_deadline_ms.store(0, Ordering::SeqCst);
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        tracing::warn!(filter = spec.trim(), generation, "log filter changed");
        Ok(generation)
    }

    pub fn reset(&self) -> Result<u64, String> {
        let baseline = self.baseline.clone();
        self.set(&baseline)
    }

    /// Record that `generation` reverts at `deadline_ms` (status only).
    pub fn note_revert(&self, generation: u64, deadline_ms: u64) {
        if self.generation.load(Ordering::SeqCst) == generation {
            self.revert_deadline_ms.store(deadline_ms, Ordering::SeqCst);
        }
    }

    /// Revert to the baseline only if `generation` is still the latest
    /// change. Returns whether it reverted.
    pub fn revert_if_current(&self, generation: u64) -> bool {
        if self.generation.load(Ordering::SeqCst) != generation {
            return false;
        }
        self.reset().is_ok()
    }
}

/// Validate a filter spec (`RUST_LOG` syntax). Rejects empty input and any
/// directive EnvFilter cannot parse, instead of silently ignoring it.
pub fn parse_filter(spec: &str) -> Result<EnvFilter, String> {
    let spec = spec.trim();
    if spec.is_empty() {
        return Err("empty filter".into());
    }
    if spec.len() > 1024 {
        return Err("filter too long".into());
    }
    EnvFilter::builder()
        .parse(spec)
        .map_err(|e| format!("invalid filter: {e}"))
}

/// The start-up filter: `RUST_LOG` when set and valid, else [`DEFAULT_FILTER`].
pub fn baseline_filter_spec(rust_log: Option<&str>) -> String {
    match rust_log.map(str::trim) {
        Some(s) if !s.is_empty() && parse_filter(s).is_ok() => s.to_string(),
        _ => DEFAULT_FILTER.to_string(),
    }
}

static CONTROL: OnceLock<LogControl> = OnceLock::new();
static LOG_PATH: OnceLock<PathBuf> = OnceLock::new();

/// Global control, present once [`init`] ran (absent in tests and in
/// library embeddings such as Android).
pub fn control() -> Option<&'static LogControl> {
    CONTROL.get()
}

pub fn log_path() -> Option<&'static Path> {
    LOG_PATH.get().map(PathBuf::as_path)
}

/// Install the global subscriber writing to `<data_dir>/tetherly.log`
/// (plus stdout when `mirror` says so) and the panic hook.
pub fn init(data_dir: &Path, stdout_tty: bool) {
    let path = data_dir.join("tetherly.log");
    let log_file = RotatingFile::open(&path, LOG_FILE_CAP).ok();
    if log_file.is_some() {
        let _ = LOG_PATH.set(path);
    }
    let mirror = crate::logsink::mirror_stdout(log_file.is_some(), stdout_tty);
    let sink = LogSink::new(log_file, mirror);

    let baseline = baseline_filter_spec(std::env::var("RUST_LOG").ok().as_deref());
    let filter = parse_filter(&baseline).unwrap_or_else(|_| EnvFilter::new("info"));
    let (filter_layer, handle) = reload::Layer::new(filter);
    let fmt_layer = tracing_subscriber::fmt::layer()
        // Colour codes only for a human terminal; the file stays greppable.
        .with_ansi(stdout_tty)
        .with_thread_names(true)
        .with_writer(move || sink.clone());
    let _ = tracing_subscriber::registry()
        .with(filter_layer)
        .with(fmt_layer)
        .try_init();
    let _ = CONTROL.set(LogControl::new(handle, baseline));
    install_panic_hook();
}

/// Log every panic (thread, location, payload, backtrace) through tracing
/// before the default hook runs, so daemon panics reach `tetherly.log`.
pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "<unknown>".into());
        let backtrace = std::backtrace::Backtrace::force_capture();
        tracing::error!(
            target: "tetherly::panic",
            thread = thread.name().unwrap_or("<unnamed>"),
            location = %location,
            payload = %panic_payload_text(info.payload()),
            "panic\n{backtrace}"
        );
        previous(info);
    }));
}

/// Human text of a panic payload (`&str` / `String`, else a placeholder).
pub fn panic_payload_text(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "<non-string panic payload>".into()
    }
}

/// Current status for `GET /api/log`.
pub fn status() -> LogStatus {
    let path = log_path();
    let size = |p: Option<PathBuf>| {
        p.and_then(|p| std::fs::metadata(p).ok())
            .map(|m| m.len())
            .unwrap_or(0)
    };
    let ctl = control();
    LogStatus {
        path: path.map(|p| p.display().to_string()),
        size_bytes: size(path.map(Path::to_path_buf)),
        rotated_size_bytes: size(path.map(rotated_path)),
        cap_bytes: LOG_FILE_CAP,
        filter: ctl.map(LogControl::current).unwrap_or_default(),
        baseline_filter: ctl.map(|c| c.baseline().to_string()).unwrap_or_default(),
        revert_at_ms: ctl.and_then(LogControl::revert_at_ms),
    }
}

pub fn rotated_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".1");
    PathBuf::from(name)
}

/// Query for [`tail`].
#[derive(Debug, Clone, Default)]
pub struct TailQuery {
    pub lines: usize,
    /// Case-insensitive substring every returned line must contain.
    pub grep: Option<String>,
    /// Minimum level (`error|warn|info|debug|trace`).
    pub min_level: Option<tracing::Level>,
}

/// Last `q.lines` matching lines of the log at `path`, oldest first,
/// continuing into the rotated `<path>.1` generation when the current file
/// has too few. Each generation is scanned at most [`TAIL_SCAN_BYTES`] back.
pub fn tail(path: &Path, q: &TailQuery) -> io::Result<Vec<String>> {
    let want = q.lines.clamp(1, TAIL_MAX_LINES);
    let needle = q.grep.as_deref().map(str::to_lowercase);
    let keep = |line: &str| {
        needle
            .as_deref()
            .is_none_or(|n| line.to_lowercase().contains(n))
            && q.min_level
                .is_none_or(|min| line_level(line).is_none_or(|l| l <= min))
    };
    let mut out: Vec<String> = Vec::new();
    for generation in [path.to_path_buf(), rotated_path(path)] {
        let text = match read_tail_bytes(&generation, TAIL_SCAN_BYTES) {
            Ok(t) => t,
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e),
        };
        let mut matched: Vec<String> = text
            .lines()
            .rev()
            .filter(|l| !l.is_empty() && keep(l))
            .take(want - out.len())
            .map(str::to_string)
            .collect();
        matched.reverse();
        matched.extend(out);
        out = matched;
        if out.len() >= want {
            break;
        }
    }
    Ok(out)
}

/// Level of a formatted tracing line (`<ts>  INFO target: msg`); `None` for
/// continuation lines (backtraces, multi-line messages), which always pass
/// the level filter.
pub fn line_level(line: &str) -> Option<tracing::Level> {
    line.split_whitespace().nth(1).and_then(|tok| match tok {
        "ERROR" => Some(tracing::Level::ERROR),
        "WARN" => Some(tracing::Level::WARN),
        "INFO" => Some(tracing::Level::INFO),
        "DEBUG" => Some(tracing::Level::DEBUG),
        "TRACE" => Some(tracing::Level::TRACE),
        _ => None,
    })
}

pub fn parse_level(s: &str) -> Option<tracing::Level> {
    s.trim().parse().ok()
}

/// Up to `max` trailing bytes of `path` as text, starting at a line
/// boundary when the read began mid-file.
fn read_tail_bytes(path: &Path, max: u64) -> io::Result<String> {
    let mut f = std::fs::File::open(path)?;
    let len = f.metadata()?.len();
    let start = len.saturating_sub(max);
    f.seek(SeekFrom::Start(start))?;
    let mut buf = Vec::with_capacity((len - start) as usize);
    f.read_to_end(&mut buf)?;
    let mut text = String::from_utf8_lossy(&buf).into_owned();
    if start > 0 {
        match text.find('\n') {
            Some(i) => {
                text.drain(..=i);
            }
            None => text.clear(),
        }
    }
    Ok(text)
}

/// Apply `spec` now and, with `revert_after`, schedule the return to the
/// baseline on a detached thread (no runtime needed). A later change
/// cancels the pending revert.
pub fn set_filter_with_revert(
    ctl: &'static LogControl,
    spec: &str,
    revert_after: Option<Duration>,
    now_ms: u64,
) -> Result<(), String> {
    let generation = ctl.set(spec)?;
    if let Some(after) = revert_after {
        let after = after.min(Duration::from_secs(MAX_REVERT_SECS));
        ctl.note_revert(generation, now_ms.saturating_add(after.as_millis() as u64));
        std::thread::Builder::new()
            .name("log-filter-revert".into())
            .spawn(move || {
                std::thread::sleep(after);
                if ctl.revert_if_current(generation) {
                    tracing::warn!(generation, "log filter reverted to baseline");
                }
            })
            .map_err(|e| format!("revert thread: {e}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::Arc;

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "tetherly-logging-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn write_lines(path: &Path, lines: &[&str]) {
        let mut f = std::fs::File::create(path).unwrap();
        for l in lines {
            writeln!(f, "{l}").unwrap();
        }
    }

    #[test]
    fn tail_returns_last_lines_oldest_first() {
        let d = tmpdir("tail");
        let p = d.join("tetherly.log");
        let lines: Vec<String> = (0..50)
            .map(|i| format!("2026-11-14T00:00:{i:02}Z  INFO tetherly_node: line {i}"))
            .collect();
        write_lines(&p, &lines.iter().map(String::as_str).collect::<Vec<_>>());
        let got = tail(
            &p,
            &TailQuery {
                lines: 3,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(got, lines[47..].to_vec());
    }

    #[test]
    fn tail_filters_by_grep_and_level() {
        let d = tmpdir("grep");
        let p = d.join("tetherly.log");
        write_lines(
            &p,
            &[
                "t0  INFO tetherly_node::runtime::input: crossing fired",
                "t1  WARN tetherly_node::runtime::input: Crossing lost",
                "t2 DEBUG tetherly_node::runtime::input: crossing detail",
                "t3 ERROR tetherly_node::lan: dial failed",
            ],
        );
        let grep = tail(
            &p,
            &TailQuery {
                lines: 10,
                grep: Some("CROSSING".into()),
                min_level: None,
            },
        )
        .unwrap();
        assert_eq!(grep.len(), 3, "{grep:?}");
        let warn_up = tail(
            &p,
            &TailQuery {
                lines: 10,
                grep: None,
                min_level: Some(tracing::Level::WARN),
            },
        )
        .unwrap();
        assert_eq!(
            warn_up,
            vec![
                "t1  WARN tetherly_node::runtime::input: Crossing lost".to_string(),
                "t3 ERROR tetherly_node::lan: dial failed".to_string(),
            ]
        );
    }

    #[test]
    fn tail_continues_into_rotated_generation() {
        let d = tmpdir("rotated");
        let p = d.join("tetherly.log");
        write_lines(&rotated_path(&p), &["old 1", "old 2"]);
        write_lines(&p, &["new 1"]);
        let got = tail(
            &p,
            &TailQuery {
                lines: 2,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(got, vec!["old 2".to_string(), "new 1".to_string()]);
    }

    #[test]
    fn tail_of_missing_log_is_empty() {
        let d = tmpdir("missing");
        assert!(tail(
            &d.join("none.log"),
            &TailQuery {
                lines: 5,
                ..Default::default()
            }
        )
        .unwrap()
        .is_empty());
    }

    #[test]
    fn read_tail_bytes_starts_at_a_line_boundary() {
        let d = tmpdir("boundary");
        let p = d.join("tetherly.log");
        write_lines(&p, &["aaaaaaaaaa", "bbbb", "cccc"]);
        // 12 bytes back lands inside "aaaa…"; the partial line is dropped.
        assert_eq!(read_tail_bytes(&p, 12).unwrap(), "bbbb\ncccc\n");
    }

    #[test]
    fn filter_validation_rejects_garbage_and_accepts_rust_log_syntax() {
        assert!(parse_filter("").is_err());
        assert!(parse_filter("tetherly=loud").is_err());
        assert!(parse_filter("debug").is_ok());
        assert!(parse_filter("warn,tetherly_node::runtime::input=trace").is_ok());
        assert!(parse_filter(DEFAULT_FILTER).is_ok());
        assert_eq!(baseline_filter_spec(None), DEFAULT_FILTER);
        assert_eq!(baseline_filter_spec(Some("tetherly=loud")), DEFAULT_FILTER);
        assert_eq!(baseline_filter_spec(Some(" debug ")), "debug");
    }

    #[test]
    fn line_level_reads_the_fmt_level_column() {
        assert_eq!(line_level("2026 ERROR x: y"), Some(tracing::Level::ERROR));
        assert_eq!(line_level("2026  INFO x: y"), Some(tracing::Level::INFO));
        assert_eq!(line_level("   0: std::backtrace"), None);
    }

    #[test]
    fn panic_payload_text_handles_str_and_string() {
        let s: Box<dyn std::any::Any + Send> = Box::new("boom");
        assert_eq!(panic_payload_text(s.as_ref()), "boom");
        let s: Box<dyn std::any::Any + Send> = Box::new(String::from("bang"));
        assert_eq!(panic_payload_text(s.as_ref()), "bang");
        let s: Box<dyn std::any::Any + Send> = Box::new(7u8);
        assert_eq!(panic_payload_text(s.as_ref()), "<non-string panic payload>");
    }

    /// Counts events that pass the filter.
    struct Count(Arc<AtomicU64>);
    impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for Count {
        fn on_event(&self, _e: &tracing::Event<'_>, _c: tracing_subscriber::layer::Context<'_, S>) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn control_reloads_filter_live_and_stale_revert_is_ignored() {
        let (layer, handle) = reload::Layer::new(parse_filter("info").unwrap());
        let seen = Arc::new(AtomicU64::new(0));
        let subscriber = tracing_subscriber::registry()
            .with(layer)
            .with(Count(seen.clone()));
        let ctl = LogControl::new(handle, "info".into());
        tracing::subscriber::with_default(subscriber, || {
            tracing::debug!("hidden at info");
            assert_eq!(seen.load(Ordering::SeqCst), 0);

            let g1 = ctl.set("debug").unwrap();
            let after_set = seen.load(Ordering::SeqCst); // the change itself logs
            tracing::debug!("visible at debug");
            assert_eq!(seen.load(Ordering::SeqCst), after_set + 1);
            assert_eq!(ctl.current(), "debug");

            ctl.note_revert(g1, 123);
            assert_eq!(ctl.revert_at_ms(), Some(123));
            let g2 = ctl.set("trace").unwrap();
            assert_eq!(ctl.revert_at_ms(), None, "a new change clears the revert");
            assert!(!ctl.revert_if_current(g1), "stale revert must not undo g2");
            assert_eq!(ctl.current(), "trace");
            assert!(ctl.revert_if_current(g2));
            assert_eq!(ctl.current(), "info");

            let before = seen.load(Ordering::SeqCst);
            tracing::debug!("hidden again");
            assert_eq!(seen.load(Ordering::SeqCst), before);
            assert!(ctl.set("tetherly=loud").is_err());
            assert_eq!(ctl.current(), "info", "a rejected spec changes nothing");
        });
    }
}
