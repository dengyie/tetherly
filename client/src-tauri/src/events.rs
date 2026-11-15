// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Shell-side event stream: ONE SSE connection to the daemon's
//! `/api/v1/events`, owned by this module (never the WebView), forwarded to
//! every window as `tetherly://event` payloads and `tetherly://link` state
//! (§5.2).
//!
//! Reconnect mirrors the SPA's HttpTransport policy: 1s → 2s → 5s → 10s
//! capped, resuming with `last_event_id` so the daemon replays or
//! re-snapshots. Frames arrive in the uniform envelope shape
//! `{seq, type, data}` and are re-emitted verbatim as JSON.

use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

pub const EVENT_CHANNEL: &str = "tetherly://event";
pub const LINK_CHANNEL: &str = "tetherly://link";

/// What a window sees on `tetherly://link`.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LinkState {
    Connecting,
    Live,
    Resyncing,
    Offline,
}

/// One forwarded frame: the envelope minus nothing — `{seq, type, data}`.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct WireEvent {
    pub seq: u64,
    #[serde(rename = "type")]
    pub ty: String,
    pub data: serde_json::Value,
}

/// Parse one SSE frame (header block already split into lines). The wire
/// data is the EventEnvelope `{seq, type, data}` (§4.3); the bare `data`
/// payload is forwarded, matching what the SPA's HttpTransport unwraps to.
/// Comment/keepalive lines yield `None`.
pub fn parse_frame(lines: &[&str]) -> Option<WireEvent> {
    let mut seq: Option<u64> = None;
    let mut ty: Option<String> = None;
    let mut data = String::new();
    for line in lines {
        if let Some(rest) = line.strip_prefix(":") {
            tracing::debug!("sse keepalive: {rest}");
        } else if let Some(rest) = line.strip_prefix("id:") {
            seq = rest.trim().parse().ok();
        } else if let Some(rest) = line.strip_prefix("event:") {
            ty = Some(rest.trim().to_string());
        } else if let Some(rest) = line.strip_prefix("data:") {
            data.push_str(rest.strip_prefix(' ').unwrap_or(rest));
        }
        // `retry:` is advisory; the reconnect policy lives below.
    }
    let seq = seq?;
    let ty = ty.unwrap_or_else(|| "message".into());
    if data.is_empty() {
        return None;
    }
    let parsed: serde_json::Value = serde_json::from_str(&data).unwrap_or(serde_json::Value::Null);
    // Envelope unwrap: `{seq, type, data: …}` → bare payload.
    let bare = if parsed.is_object()
        && parsed.get("seq").is_some()
        && parsed.get("type").is_some()
        && parsed.get("data").is_some()
    {
        parsed["data"].clone()
    } else {
        parsed
    };
    Some(WireEvent {
        seq,
        ty,
        data: bare,
    })
}

const BACKOFF_MS: [u64; 4] = [1000, 2000, 5000, 10000];

/// Run the single connection for the life of the app. `mutate` events drive
/// `tetherly://event`; connection transitions drive `tetherly://link`.
pub async fn run(app: AppHandle, port: u16, token: String) {
    let mut last_seq: Option<u64> = None;
    let mut attempt: usize = 0;
    loop {
        let resyncing = last_seq.is_some();
        let state = if resyncing {
            LinkState::Resyncing
        } else {
            LinkState::Connecting
        };
        let _ = app.emit(LINK_CHANNEL, state);
        match connect(&app, port, &token, &mut last_seq).await {
            // Ok is Infallible: connect only returns via StreamEnd.
            Err(StreamEnd::Refused) => {
                let _ = app.emit(LINK_CHANNEL, LinkState::Offline);
                let delay = BACKOFF_MS[attempt.min(BACKOFF_MS.len() - 1)];
                attempt += 1;
                tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
            }
        }
    }
}

enum StreamEnd {
    /// Connection could not be established (or the app is exiting).
    Refused,
}

/// One connection attempt: stream frames until EOF. Returns Ok only when the
/// shutdown signal fires (listening on the app handle is out of scope here —
/// the process exits, killing this task).
async fn connect(
    app: &AppHandle,
    port: u16,
    token: &str,
    last_seq: &mut Option<u64>,
) -> Result<std::convert::Infallible, StreamEnd> {
    let url_path = match last_seq {
        None => "/api/v1/events".to_string(),
        Some(s) => format!("/api/v1/events?last_event_id={s}"),
    };
    let req = format!(
        "GET {url_path} HTTP/1.1\r\n\
         Host: 127.0.0.1:{port}\r\n\
         Authorization: Bearer {token}\r\n\
         Tetherly-Client: tauri/1.0.0\r\n\
         Accept: text/event-stream\r\n\
         Connection: close\r\n\
         \r\n"
    );
    let Ok(mut stream) = TcpStream::connect(("127.0.0.1", port)).await else {
        return Err(StreamEnd::Refused);
    };
    if stream.write_all(req.as_bytes()).await.is_err() {
        return Err(StreamEnd::Refused);
    }
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    let mut frame_lines: Vec<String> = Vec::new();
    loop {
        line.clear();
        match reader.read_line(&mut line).await {
            Ok(0) | Err(_) => return Err(StreamEnd::Refused),
            Ok(_) => {
                let trimmed = line.trim_end_matches(['\n', '\r']).to_string();
                if trimmed.is_empty() {
                    if frame_lines.is_empty() {
                        continue;
                    }
                    let refs: Vec<&str> = frame_lines.iter().map(String::as_str).collect();
                    if let Some(ev) = parse_frame(&refs) {
                        *last_seq = Some(ev.seq);
                        let _ = app.emit(EVENT_CHANNEL, &ev);
                    }
                    frame_lines.clear();
                } else {
                    frame_lines.push(trimmed);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_frame_reads_envelope_and_ignores_comments() {
        let lines = [
            "id: 7",
            "event: snapshot",
            "data: {\"seq\":7,\"type\":\"snapshot\",\"data\":{\"a\":1}}",
        ];
        let ev = parse_frame(&lines).unwrap();
        assert_eq!(ev.seq, 7);
        assert_eq!(ev.ty, "snapshot");
        assert_eq!(ev.data["a"], 1, "envelope unwrapped to the bare payload");
    }

    #[test]
    fn parse_frame_skips_keepalive_and_retry_lines() {
        assert!(parse_frame(&[": ping"]).is_none());
        assert!(parse_frame(&["retry: 2000"]).is_none());
        assert!(parse_frame(&[
            "id: 3",
            "data: {\"seq\":3,\"type\":\"x.changed\",\"data\":null}"
        ])
        .is_some());
    }

    #[test]
    fn parse_frame_multi_line_data_concatenates() {
        let lines = [
            "id: 2",
            "event: e",
            "data: {\"seq\":2,",
            "data:  \"type\":\"m\",\"data\":1}",
        ];
        let ev = parse_frame(&lines).unwrap();
        assert_eq!(ev.data, serde_json::json!(1));
    }

    #[test]
    fn wire_event_serializes_with_type_field() {
        let ev = WireEvent {
            seq: 1,
            ty: "activity.changed".into(),
            data: serde_json::json!({"k": 1}),
        };
        let v = serde_json::to_value(&ev).unwrap();
        assert_eq!(v["type"], "activity.changed");
        assert_eq!(v["seq"], 1);
        assert_eq!(v["data"]["k"], 1);
    }
}
