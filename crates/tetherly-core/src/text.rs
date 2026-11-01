// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Human-readable formatting and percent-encoding shared by every UI surface.
//!
//! `tetherly` and `tetherly-cli` both print the same byte counts, durations and
//! relative timestamps, and both build loopback query strings. Those helpers
//! used to be copy-pasted into each binary (and once more into the node's HTTP
//! router), so a fix in one copy silently left the others behind. They are pure
//! functions of their inputs — no clocks, no I/O — which keeps them trivially
//! testable from here.

/// Format a duration in seconds as `1d 2h 3m 4s`, dropping leading zero units
/// but always keeping the seconds field.
pub fn format_uptime(secs: u64) -> String {
    let days = secs / 86400;
    let hours = (secs % 86400) / 3600;
    let minutes = (secs % 3600) / 60;
    let s = secs % 60;
    if days > 0 {
        format!("{days}d {hours}h {minutes}m {s}s")
    } else if hours > 0 {
        format!("{hours}h {minutes}m {s}s")
    } else if minutes > 0 {
        format!("{minutes}m {s}s")
    } else {
        format!("{s}s")
    }
}

/// Format a byte count with binary units: `512 B`, `1.50 KiB`, `2.00 MiB`.
pub fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;
    if bytes >= GB {
        format!("{:.2} GiB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2} MiB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.2} KiB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

/// Format a throughput in bytes per second.
pub fn format_speed(bps: u64) -> String {
    format!("{}/s", format_bytes(bps))
}

/// Render a wall-clock second count relative to `now_secs`.
///
/// `ts_secs == 0` means "never recorded" and prints `-`. The current time is a
/// parameter rather than read from the system clock so callers (and tests) can
/// pin it.
pub fn format_timestamp_ago(ts_secs: u64, now_secs: u64) -> String {
    if ts_secs == 0 {
        return "-".to_string();
    }
    if now_secs >= ts_secs {
        let diff = now_secs - ts_secs;
        if diff < 60 {
            format!("{diff}s ago")
        } else if diff < 3600 {
            format!("{}m ago", diff / 60)
        } else if diff < 86400 {
            format!("{}h ago", diff / 3600)
        } else {
            format!("{}d ago", diff / 86400)
        }
    } else {
        format!("in {}s", ts_secs - now_secs)
    }
}

/// Percent-encode a URL **query value**: everything outside the unreserved set
/// (`ALPHA / DIGIT / "-" / "_" / "." / "~"`) is escaped, including `/`.
pub fn encode_query_value(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for b in input.bytes() {
        if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.' || b == b'~' {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Percent-encode a URL **path**: like [`encode_query_value`] but `/` survives,
/// so a multi-segment path stays a path.
pub fn encode_path(p: &str) -> String {
    let mut out = String::with_capacity(p.len());
    for b in p.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'/' | b'~' => {
                out.push(b as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uptime_drops_zero_leading_units() {
        assert_eq!(format_uptime(0), "0s");
        assert_eq!(format_uptime(59), "59s");
        assert_eq!(format_uptime(60), "1m 0s");
        assert_eq!(format_uptime(3661), "1h 1m 1s");
        assert_eq!(format_uptime(90061), "1d 1h 1m 1s");
    }

    #[test]
    fn bytes_switch_units_at_binary_boundaries() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(1023), "1023 B");
        assert_eq!(format_bytes(1024), "1.00 KiB");
        assert_eq!(format_bytes(1536), "1.50 KiB");
        assert_eq!(format_bytes(1024 * 1024), "1.00 MiB");
        assert_eq!(format_bytes(3 * 1024 * 1024 * 1024), "3.00 GiB");
    }

    #[test]
    fn speed_appends_per_second() {
        assert_eq!(format_speed(2048), "2.00 KiB/s");
    }

    #[test]
    fn timestamp_ago_is_relative_to_the_supplied_now() {
        assert_eq!(format_timestamp_ago(0, 1_000_000), "-");
        assert_eq!(format_timestamp_ago(1_000_000, 1_000_000), "0s ago");
        assert_eq!(format_timestamp_ago(999_930, 1_000_000), "1m ago");
        assert_eq!(format_timestamp_ago(996_400, 1_000_000), "1h ago");
        assert_eq!(format_timestamp_ago(913_600, 1_000_000), "1d ago");
        assert_eq!(format_timestamp_ago(1_000_030, 1_000_000), "in 30s");
    }

    #[test]
    fn query_values_escape_slashes_and_reserved_bytes() {
        assert_eq!(encode_query_value("plain-1_2.3~"), "plain-1_2.3~");
        assert_eq!(encode_query_value("a/b c&d=e"), "a%2Fb%20c%26d%3De");
        // Multi-byte UTF-8 is escaped byte by byte, not char by char.
        assert_eq!(encode_query_value("é"), "%C3%A9");
    }

    #[test]
    fn paths_keep_separators() {
        assert_eq!(encode_path("/a/b c/d.png"), "/a/b%20c/d.png");
        assert_eq!(encode_path("/inbox/é.txt"), "/inbox/%C3%A9.txt");
    }
}
