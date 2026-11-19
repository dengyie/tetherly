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

use serde::{Deserialize, Serialize};
use std::time::Duration;
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

pub const EVENT_CHANNEL: &str = "tetherly://event";
pub const LINK_CHANNEL: &str = "tetherly://link";

/// What a window sees on `tetherly://link`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LinkState {
    Connecting,
    Live,
    Resyncing,
    Offline,
}

/// One forwarded frame: the envelope minus nothing — `{seq, type, data}`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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
    let parsed: serde_json::Value = serde_json::from_str(&data).ok()?;
    // v1 envelopes repeat the SSE id in `seq`; reject mismatches so the
    // reconnect cursor can never advance past an event that was not delivered.
    let is_v1_envelope = parsed.is_object() && parsed.get("seq").is_some();
    let bare = if is_v1_envelope {
        if parsed.get("seq").and_then(serde_json::Value::as_u64) != Some(seq)
            || parsed.get("type").is_none()
            || parsed.get("data").is_none()
        {
            return None;
        }
        parsed["data"].clone()
    } else {
        // Legacy event streams carry bare JSON payloads; retain that shape.
        parsed
    };
    Some(WireEvent {
        seq,
        ty,
        data: bare,
    })
}

const BACKOFF_MS: [u64; 4] = [1000, 2000, 5000, 10000];
const SSE_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const SSE_IDLE_TIMEOUT: Duration = Duration::from_secs(45);

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
        match connect(&app, port, &token, &mut last_seq, &mut attempt).await {
            // Ok is Infallible: connect only returns via StreamEnd::Refused.
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
/// the process exits, killing this task). `frame_sink` receives each parsed
/// frame — tests use it to observe reconnect behavior without an app handle.
/// What the connection loop hands its consumer: the moment the daemon
/// accepted the SSE request (§6.3 tray claim waits for this), or a frame.
#[derive(Debug, PartialEq)]
enum SinkEvent {
    Established,
    Frame(WireEvent),
}

async fn read_sse_headers<R>(reader: &mut BufReader<R>) -> Result<(), StreamEnd>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut status_line = String::new();
    reader
        .read_line(&mut status_line)
        .await
        .map_err(|_| StreamEnd::Refused)?;
    let status = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or(StreamEnd::Refused)?;
    if status != 200 {
        return Err(StreamEnd::Refused);
    }

    let mut content_type_ok = false;
    let mut line = String::new();
    loop {
        line.clear();
        reader
            .read_line(&mut line)
            .await
            .map_err(|_| StreamEnd::Refused)?;
        if line.is_empty() {
            return Err(StreamEnd::Refused);
        }
        let trimmed = line.trim_end_matches(['\n', '\r']);
        if trimmed.is_empty() {
            break;
        }
        if let Some((name, value)) = trimmed.split_once(':') {
            if name.eq_ignore_ascii_case("content-type") {
                content_type_ok = value
                    .trim()
                    .split(';')
                    .next()
                    .is_some_and(|mime| mime.eq_ignore_ascii_case("text/event-stream"));
            }
        }
    }
    if content_type_ok {
        Ok(())
    } else {
        Err(StreamEnd::Refused)
    }
}

async fn read_sse_line<R>(reader: &mut BufReader<R>, idle: Duration) -> Result<String, StreamEnd>
where
    R: tokio::io::AsyncRead + Unpin,
{
    tokio::time::timeout(idle, async {
        let mut line = String::new();
        match reader.read_line(&mut line).await {
            Ok(0) => Err(StreamEnd::Refused),
            Ok(_) => Ok(line),
            Err(_) => Err(StreamEnd::Refused),
        }
    })
    .await
    .unwrap_or(Err(StreamEnd::Refused))
}

async fn connect_with<S>(
    port: u16,
    token: &str,
    last_seq: &mut Option<u64>,
    frame_sink: S,
) -> Result<std::convert::Infallible, StreamEnd>
where
    S: FnMut(SinkEvent),
{
    connect_with_deadlines(
        port,
        token,
        last_seq,
        SSE_HANDSHAKE_TIMEOUT,
        SSE_IDLE_TIMEOUT,
        frame_sink,
    )
    .await
}

async fn connect_with_deadlines<S>(
    port: u16,
    token: &str,
    last_seq: &mut Option<u64>,
    handshake_timeout: Duration,
    idle_timeout: Duration,
    mut frame_sink: S,
) -> Result<std::convert::Infallible, StreamEnd>
where
    S: FnMut(SinkEvent),
{
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
    let handshake = async {
        let mut stream = TcpStream::connect(("127.0.0.1", port))
            .await
            .map_err(|_| StreamEnd::Refused)?;
        stream
            .write_all(req.as_bytes())
            .await
            .map_err(|_| StreamEnd::Refused)?;
        let mut reader = BufReader::new(stream);
        read_sse_headers(&mut reader).await?;
        Ok::<_, StreamEnd>(reader)
    };
    let mut reader = match tokio::time::timeout(handshake_timeout, handshake).await {
        Ok(Ok(reader)) => reader,
        Ok(Err(_)) | Err(_) => return Err(StreamEnd::Refused),
    };
    // The handshake is complete only after status and Content-Type have been
    // validated. A legal stream may be idle before its first application frame.
    frame_sink(SinkEvent::Established);
    let mut frame_lines: Vec<String> = Vec::new();
    loop {
        let line = read_sse_line(&mut reader, idle_timeout).await?;
        let trimmed = line.trim_end_matches(['\n', '\r']).to_string();
        if trimmed.is_empty() {
            if frame_lines.is_empty() {
                continue;
            }
            let refs: Vec<&str> = frame_lines.iter().map(String::as_str).collect();
            if let Some(ev) = parse_frame(&refs) {
                if last_seq.is_some_and(|previous| ev.seq <= previous) {
                    frame_lines.clear();
                    continue;
                }
                *last_seq = Some(ev.seq);
                frame_sink(SinkEvent::Frame(ev));
            }
            frame_lines.clear();
        } else {
            frame_lines.push(trimmed);
        }
    }
}

#[cfg(test)]
fn sse_response(body: &str) -> String {
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream; charset=utf-8\r\nCache-Control: no-cache\r\nConnection: keep-alive\r\n\r\n{body}"
    )
}

async fn connect(
    app: &AppHandle,
    port: u16,
    token: &str,
    last_seq: &mut Option<u64>,
    attempt: &mut usize,
) -> Result<std::convert::Infallible, StreamEnd> {
    connect_with(port, token, last_seq, |ev| match ev {
        SinkEvent::Established => {
            *attempt = 0;
            let _ = app.emit(LINK_CHANNEL, LinkState::Live);
        }
        SinkEvent::Frame(ev) => {
            let _ = app.emit(EVENT_CHANNEL, &ev);
        }
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

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
    fn parse_frame_rejects_malformed_or_mismatched_v1_envelopes() {
        assert!(parse_frame(&["id: 3", "data: {not-json}"]).is_none());
        assert!(parse_frame(&[
            "id: 3",
            "event: x.changed",
            "data: {\"seq\":4,\"type\":\"x.changed\",\"data\":null}"
        ])
        .is_none());
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

    /// SSE frame bytes as the daemon writes them (§4.3): id + event + data.
    fn sse_frame(seq: u64, ty: &str, data: &serde_json::Value) -> String {
        format!(
            "id: {seq}\nevent: {ty}\ndata: {}\r\n\r\n",
            serde_json::json!({"seq": seq, "type": ty, "data": data})
        )
    }

    /// M3-r3 acceptance: kill the daemon, restart it on the same port, and
    /// the shell reconnects — resuming with last_event_id and folding the
    /// new snapshot. Drives [`connect_with`] directly over a real loopback
    /// listener, no app handle needed.
    #[tokio::test]
    async fn reconnect_after_daemon_restart_resumes_with_last_event_id() {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();

        // "Daemon v1": one snapshot frame, then EOF (process killed).
        let v1 = tokio::spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 2048];
            let _n = s.read(&mut buf).await.unwrap();
            let body = sse_response(&sse_frame(7, "snapshot", &serde_json::json!({"gen": 1})));
            let _ = s.write_all(body.as_bytes()).await;
            // Drop = daemon dies mid-stream.
        });
        // "Daemon v2" binds the SAME port after a restart gap. Bind until it
        // succeeds (v1's socket may linger briefly in TIME_WAIT).
        let v2 = tokio::spawn(async move {
            let listener = loop {
                match tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
                    Ok(l) => break l,
                    Err(_) => tokio::time::sleep(std::time::Duration::from_millis(20)).await,
                }
            };
            let (mut s, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 2048];
            let n = s.read(&mut buf).await.unwrap();
            let req = String::from_utf8_lossy(&buf[..n]).to_string();
            // The resume contract: the shell tells v2 where v1 left off.
            assert!(
                req.starts_with("GET /api/v1/events?last_event_id=7 HTTP/1.1\r\n"),
                "v2 must receive the resume request, got: {req}"
            );
            assert!(req.contains("Accept: text/event-stream\r\n"), "{req}");
            let body = sse_response(&sse_frame(8, "snapshot", &serde_json::json!({"gen": 2})));
            let _ = s.write_all(body.as_bytes()).await;
            // Hold v2's connection open until the test observed the frame.
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        });

        let mut last_seq: Option<u64> = None;
        let mut frames: Vec<WireEvent> = Vec::new();
        let mut established = 0usize;
        // Attempt loop mirrors `run`: backoff between StreamEnd::Refused.
        for attempt in 0..10 {
            match connect_with(port, "tok", &mut last_seq, |ev| match ev {
                SinkEvent::Established => established += 1,
                SinkEvent::Frame(f) => frames.push(f),
            })
            .await
            {
                Err(StreamEnd::Refused) => {
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                }
                Ok(inf) => match inf {},
            }
            if frames.len() >= 2 {
                break;
            }
            let _ = attempt;
        }

        assert_eq!(established, 2, "each daemon generation announces once");
        assert_eq!(frames.len(), 2, "one frame per daemon generation");
        assert_eq!(frames[0].seq, 7);
        assert_eq!(frames[0].data["gen"], 1);
        assert_eq!(frames[1].seq, 8);
        assert_eq!(frames[1].data["gen"], 2);
        v1.await.unwrap();
        v2.await.unwrap();
    }
}

#[cfg(test)]
mod established_tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn sse_frame(seq: u64, ty: &str, data: &serde_json::Value) -> String {
        format!(
            "id: {seq}\nevent: {ty}\ndata: {}\r\n\r\n",
            serde_json::json!({"seq": seq, "type": ty, "data": data})
        )
    }

    /// §147: the connection loop must report `Established` — once, before
    /// any frame — when the daemon accepts the SSE request. The §6.3 tray
    /// claim waits for exactly this transition; without it the shell never
    /// claims the tray surface (production 2026-11-17: tray_owner stayed
    /// `daemon` while the shell's SSE sat connected).
    #[tokio::test]
    async fn established_is_announced_before_the_first_frame() {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 2048];
            let _n = s.read(&mut buf).await.unwrap();
            let body = sse_response(&format!(
                "id: 1\nevent: snapshot\ndata: {}\r\n\r\n",
                serde_json::json!({"seq": 1, "type": "snapshot", "data": {"gen": 1}})
            ));
            let _ = s.write_all(body.as_bytes()).await;
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        });

        let mut last_seq: Option<u64> = None;
        let mut events: Vec<SinkEvent> = Vec::new();
        let _ = connect_with(port, "tok", &mut last_seq, |ev| events.push(ev)).await;
        server.await.unwrap();

        assert!(
            matches!(events.first(), Some(SinkEvent::Established)),
            "first sink event must be Established, got {:?}",
            events.first()
        );
        assert_eq!(events.len(), 2, "Established + one frame");
        assert!(matches!(events[1], SinkEvent::Frame(_)));
    }

    #[tokio::test]
    async fn rejects_non_success_or_non_sse_handshakes() {
        let cases = [
            ("401 Unauthorized", "text/event-stream"),
            ("500 Internal Server Error", "text/event-stream"),
            ("200 OK", "application/json"),
        ];
        for (status, content_type) in cases {
            let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
                .await
                .unwrap();
            let port = listener.local_addr().unwrap().port();
            let status = status.to_string();
            let content_type = content_type.to_string();
            let server = tokio::spawn(async move {
                let (mut s, _) = listener.accept().await.unwrap();
                let mut buf = [0u8; 2048];
                let _ = s.read(&mut buf).await.unwrap();
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: 0\r\n\r\n"
                );
                s.write_all(response.as_bytes()).await.unwrap();
            });
            let mut last_seq = None;
            let mut events = Vec::new();
            let result = connect_with(port, "tok", &mut last_seq, |ev| events.push(ev)).await;
            assert!(matches!(result, Err(StreamEnd::Refused)));
            assert!(events.is_empty(), "invalid handshake emitted: {events:?}");
            server.await.unwrap();
        }
    }

    #[tokio::test]
    async fn half_open_stream_times_out_after_a_valid_handshake() {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 2048];
            let _ = s.read(&mut buf).await.unwrap();
            s.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\r\n")
                .await
                .unwrap();
            tokio::time::sleep(Duration::from_secs(5)).await;
        });

        let mut last_seq = None;
        let mut events = Vec::new();
        let result = connect_with_deadlines(
            port,
            "tok",
            &mut last_seq,
            Duration::from_secs(1),
            Duration::from_millis(25),
            |ev| events.push(ev),
        )
        .await;
        assert!(matches!(result, Err(StreamEnd::Refused)));
        assert_eq!(events, vec![SinkEvent::Established]);
        assert_eq!(last_seq, None);
        server.abort();
    }

    #[tokio::test]
    async fn duplicate_and_rewound_sequences_do_not_reach_the_sink() {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 2048];
            let _ = s.read(&mut buf).await.unwrap();
            let body = format!(
                "{}{}{}{}",
                sse_response(&sse_frame(7, "snapshot", &serde_json::json!({"n": 7}))),
                sse_frame(7, "snapshot", &serde_json::json!({"n": 7})),
                sse_frame(6, "snapshot", &serde_json::json!({"n": 6})),
                sse_frame(8, "snapshot", &serde_json::json!({"n": 8})),
            );
            s.write_all(body.as_bytes()).await.unwrap();
        });

        let mut last_seq = None;
        let mut frames = Vec::new();
        let _ = connect_with(port, "tok", &mut last_seq, |ev| {
            if let SinkEvent::Frame(frame) = ev {
                frames.push(frame);
            }
        })
        .await;
        assert_eq!(
            frames.iter().map(|frame| frame.seq).collect::<Vec<_>>(),
            [7, 8]
        );
        assert_eq!(last_seq, Some(8));
        server.await.unwrap();
    }
}
