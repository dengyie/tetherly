// SPDX-License-Identifier: Apache-2.0 OR MIT
//! HTTP/SSE client for a node's **own** loopback UI API.
//!
//! Both binaries (`tetherly` and `tetherly-cli`) drive a running daemon through
//! `127.0.0.1:<ui_port>`, and the node's router tests use the same client. The
//! implementation used to exist twice — once here and once in the CLI — so a
//! fix to one copy silently left the other behind. It lives here now, with the
//! bounded-timeout policy that was missing from both copies: every network wait
//! has an explicit deadline, so a wedged or half-open daemon surfaces as a
//! `TimedOut` error instead of hanging the caller forever.
//!
//! Loopback only: the address is always `127.0.0.1` and the port is passed in.

use std::io;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::net::TcpStream;

/// Deadline for establishing the loopback TCP connection itself.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// Deadline for a complete request/response exchange on a connected socket.
///
/// Every CLI route answers from in-memory state; the slowest (`/api/screen/*`
/// snapshots) still finishes in well under a second. A minute of silence means
/// the daemon is wedged, not busy.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Deadline for the SSE handshake (status line plus response headers).
pub const SSE_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// Deadline for the next line of an established SSE stream.
///
/// The node pings every 15 s, so four missed pings means the stream is dead.
pub const SSE_IDLE_TIMEOUT: Duration = Duration::from_secs(60);

fn timed_out(what: &str, budget: Duration) -> io::Error {
    io::Error::new(
        io::ErrorKind::TimedOut,
        format!("loopback {what} did not complete within {budget:?}"),
    )
}

/// Split a raw HTTP response into `(status_code, body)`.
///
/// Binary safe: the body is located by the `\r\n\r\n` separator, never by
/// `String` conversion. A response without headers yields code `0` and an empty
/// body rather than an error, which is what the callers' status checks expect.
pub fn parse_http_response(buf: &[u8]) -> (u16, Vec<u8>) {
    let header_end = buf.windows(4).position(|w| w == b"\r\n\r\n");
    let (head_bytes, body_bytes) = match header_end {
        Some(pos) => (&buf[..pos], &buf[pos + 4..]),
        None => (buf, &[][..]),
    };
    let head_text = String::from_utf8_lossy(head_bytes);
    let code = head_text
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .unwrap_or(0);
    (code, body_bytes.to_vec())
}

async fn connect(port: u16, connect_timeout: Duration) -> io::Result<TcpStream> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    match tokio::time::timeout(connect_timeout, TcpStream::connect(addr)).await {
        Ok(res) => res,
        Err(_) => Err(timed_out(
            &format!("connect to 127.0.0.1:{port}"),
            connect_timeout,
        )),
    }
}

/// One request/response exchange with explicit deadlines.
///
/// `body` is only sent for methods that carry one (`POST`, `PUT`). The server
/// always answers `Connection: close`, so `read_to_end` terminates on EOF.
async fn exchange(
    port: u16,
    head: String,
    body: Option<&[u8]>,
    connect_timeout: Duration,
    request_timeout: Duration,
) -> io::Result<(u16, Vec<u8>)> {
    let fut = async {
        let mut stream = connect(port, connect_timeout).await?;
        stream.write_all(head.as_bytes()).await?;
        if let Some(body) = body {
            stream.write_all(body).await?;
        }
        stream.flush().await?;
        let mut buf = Vec::new();
        stream.read_to_end(&mut buf).await?;
        Ok::<_, io::Error>(parse_http_response(&buf))
    };
    match tokio::time::timeout(request_timeout, fut).await {
        Ok(res) => res,
        Err(_) => Err(timed_out(
            &format!("request to 127.0.0.1:{port}"),
            request_timeout,
        )),
    }
}

fn body_head(method: &str, path: &str, body: &[u8]) -> String {
    format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
}

fn headless_head(method: &str, path: &str) -> String {
    format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
}

/// `GET` a path and return `(status, body)`.
pub async fn loopback_get(port: u16, path: &str) -> io::Result<(u16, Vec<u8>)> {
    exchange(
        port,
        headless_head("GET", path),
        None,
        CONNECT_TIMEOUT,
        REQUEST_TIMEOUT,
    )
    .await
}

/// `DELETE` a path and return `(status, body)`.
pub async fn loopback_delete(port: u16, path: &str) -> io::Result<(u16, Vec<u8>)> {
    exchange(
        port,
        headless_head("DELETE", path),
        None,
        CONNECT_TIMEOUT,
        REQUEST_TIMEOUT,
    )
    .await
}

/// `POST` a JSON body to a path and return `(status, body)`.
///
/// The response body is the only place a freshly minted PIN ever appears: it is
/// returned to the caller and never logged.
pub async fn loopback_post(port: u16, path: &str, body: &[u8]) -> io::Result<(u16, Vec<u8>)> {
    exchange(
        port,
        body_head("POST", path, body),
        Some(body),
        CONNECT_TIMEOUT,
        REQUEST_TIMEOUT,
    )
    .await
}

/// `PUT` a JSON body to a path and return `(status, body)`.
pub async fn loopback_put(port: u16, path: &str, body: &[u8]) -> io::Result<(u16, Vec<u8>)> {
    exchange(
        port,
        body_head("PUT", path, body),
        Some(body),
        CONNECT_TIMEOUT,
        REQUEST_TIMEOUT,
    )
    .await
}

/// An active Server-Sent Events stream from a loopback UI endpoint.
pub struct SseStream {
    lines: Lines<BufReader<TcpStream>>,
    idle_timeout: Duration,
}

impl SseStream {
    /// Read the next event payload.
    ///
    /// Blank separator lines and comments (the node's `: ping` keep-alive) are
    /// skipped; each of them still refreshes the idle deadline, so a quiet
    /// stream stays open while a silent one fails. Returns `Ok(None)` on a
    /// clean close and `TimedOut` if nothing at all arrives within the idle
    /// deadline.
    pub async fn next_event(&mut self) -> io::Result<Option<String>> {
        loop {
            let line = match tokio::time::timeout(self.idle_timeout, self.lines.next_line()).await {
                Ok(res) => res?,
                Err(_) => return Err(timed_out("SSE event", self.idle_timeout)),
            };
            let Some(line) = line else {
                return Ok(None);
            };
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with(':') {
                continue;
            }
            if let Some(payload) = trimmed.strip_prefix("data:") {
                return Ok(Some(payload.trim().to_string()));
            }
        }
    }
}

/// Open the `/api/events` SSE stream with explicit deadlines.
///
/// Prefer [`loopback_events_connect`], which applies the crate's policy; this
/// form exists so callers with a different tolerance (and the tests below) can
/// set their own.
pub async fn loopback_events_connect_with_deadlines(
    port: u16,
    handshake_timeout: Duration,
    idle_timeout: Duration,
) -> io::Result<SseStream> {
    let fut = async {
        let mut stream = connect(port, CONNECT_TIMEOUT).await?;
        let req =
            "GET /api/events HTTP/1.1\r\nHost: 127.0.0.1\r\nAccept: text/event-stream\r\n\r\n";
        stream.write_all(req.as_bytes()).await?;
        stream.flush().await?;

        let mut lines = BufReader::new(stream).lines();
        let status_line = lines.next_line().await?.unwrap_or_default();
        if !status_line.contains("200") {
            return Err(io::Error::other(format!(
                "SSE HTTP handshake failed: {status_line}"
            )));
        }
        while let Some(header) = lines.next_line().await? {
            if header.trim().is_empty() {
                break;
            }
        }
        Ok(SseStream {
            lines,
            idle_timeout,
        })
    };
    match tokio::time::timeout(handshake_timeout, fut).await {
        Ok(res) => res,
        Err(_) => Err(timed_out(
            &format!("SSE handshake with 127.0.0.1:{port}"),
            handshake_timeout,
        )),
    }
}

/// Connect to a node's loopback UI API and open the `/api/events` SSE stream.
pub async fn loopback_events_connect(port: u16) -> io::Result<SseStream> {
    loopback_events_connect_with_deadlines(port, SSE_HANDSHAKE_TIMEOUT, SSE_IDLE_TIMEOUT).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    #[test]
    fn parses_status_and_body() {
        let raw = b"HTTP/1.1 201 Created\r\nContent-Length: 2\r\n\r\nhi";
        assert_eq!(parse_http_response(raw), (201, b"hi".to_vec()));
    }

    #[test]
    fn handles_empty_body_and_garbage() {
        assert_eq!(
            parse_http_response(b"HTTP/1.1 204 No Content\r\n\r\n"),
            (204, Vec::new())
        );
        // No header terminator at all: code 0, nothing that could be a body.
        assert_eq!(parse_http_response(b"nonsense"), (0, Vec::new()));
    }

    #[test]
    fn keeps_binary_bodies_intact() {
        let mut raw = b"HTTP/1.1 200 OK\r\n\r\n".to_vec();
        raw.extend_from_slice(&[0x00, 0xff, 0x80]);
        assert_eq!(parse_http_response(&raw), (200, vec![0x00, 0xff, 0x80]));
    }

    /// A peer that accepts the connection and then says nothing must surface as
    /// a timeout, not a hang. This is the regression the shared client exists
    /// for: both pre-extraction copies waited on `read_to_end` forever.
    #[tokio::test]
    async fn request_times_out_when_the_daemon_never_answers() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let accepted = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            // Hold the socket open without writing a response.
            tokio::time::sleep(Duration::from_secs(5)).await;
            drop(stream);
        });

        let err = exchange(
            port,
            headless_head("GET", "/api/health"),
            None,
            Duration::from_secs(1),
            Duration::from_millis(100),
        )
        .await
        .expect_err("silent peer must not block the caller");
        assert_eq!(err.kind(), io::ErrorKind::TimedOut, "{err}");

        accepted.abort();
    }

    /// The SSE handshake is bounded too: a server that accepts and stays silent
    /// fails instead of wedging the watch loop.
    #[tokio::test]
    async fn sse_handshake_times_out_on_silent_server() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let accepted = tokio::spawn(async move {
            let (_stream, _) = listener.accept().await.unwrap();
            tokio::time::sleep(Duration::from_secs(5)).await;
        });

        let err = match loopback_events_connect_with_deadlines(
            port,
            Duration::from_millis(100),
            Duration::from_millis(100),
        )
        .await
        {
            Ok(_) => panic!("silent SSE handshake must time out"),
            Err(err) => err,
        };
        assert_eq!(err.kind(), io::ErrorKind::TimedOut, "{err}");

        accepted.abort();
    }

    /// A handshaken stream that then goes quiet fails on the idle deadline
    /// rather than blocking a watch loop forever.
    #[tokio::test]
    async fn sse_idle_stream_times_out() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            // Complete the handshake, then send nothing at all.
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\r\n")
                .await
                .unwrap();
            tokio::time::sleep(Duration::from_secs(5)).await;
        });

        let mut sse = loopback_events_connect_with_deadlines(
            port,
            Duration::from_secs(1),
            Duration::from_millis(100),
        )
        .await
        .unwrap();
        let err = sse
            .next_event()
            .await
            .expect_err("silent stream must time out");
        assert_eq!(err.kind(), io::ErrorKind::TimedOut, "{err}");

        server.abort();
    }

    /// Keep-alive comments and blank separators do not end the stream, and the
    /// payload after them is delivered stripped of the `data:` prefix.
    #[tokio::test]
    async fn sse_skips_comments_and_returns_payload() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\r\n\
                      : connected\n\n: ping\n\ndata: {\"kind\":\"x\"}\n\n",
                )
                .await
                .unwrap();
            tokio::time::sleep(Duration::from_secs(1)).await;
        });

        let mut sse = loopback_events_connect_with_deadlines(
            port,
            Duration::from_secs(1),
            Duration::from_secs(1),
        )
        .await
        .unwrap();
        assert_eq!(
            sse.next_event().await.unwrap().as_deref(),
            Some("{\"kind\":\"x\"}")
        );

        server.abort();
    }
}
