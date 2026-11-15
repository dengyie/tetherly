// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Loopback API proxy: the shell's `invoke("api", {method, path, body})`
//! lands here, and this module speaks HTTP/1.1 to the daemon on 127.0.0.1.
//!
//! Hand-rolled like the daemon's own `uihttp` — the workspace carries no
//! HTTP client crate, and the surface is tiny: the daemon answers JSON
//! responses with `Content-Length` + `Connection: close`, so a request is
//! write-head + write-body + read-to-EOF. The token is attached here and
//! never crosses into the WebView's JS context (§4.4).

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// What the WebView gets back: status + parsed JSON body (or null for 204).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProxyReply {
    pub status: u16,
    /// JSON value or null; non-2xx carries the daemon's `{"error": …}` body.
    pub body: serde_json::Value,
}

#[derive(Debug)]
pub enum ProxyError {
    /// Daemon unreachable / connection died mid-request.
    Io(std::io::Error),
    /// Response violated the framing the daemon guarantees.
    BadResponse(&'static str),
}

impl std::fmt::Display for ProxyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProxyError::Io(e) => write!(f, "daemon unreachable: {e}"),
            ProxyError::BadResponse(m) => write!(f, "bad daemon response: {m}"),
        }
    }
}

/// One proxied request. `path` starts with `/api/v1`; body is the already
/// serialized JSON (or absent).
pub async fn request(
    port: u16,
    token: &str,
    method: &str,
    path: &str,
    body: Option<&serde_json::Value>,
) -> Result<ProxyReply, ProxyError> {
    // Defense in depth: the WebView may only reach the versioned API on the
    // loopback UI port, never arbitrary paths (§5.1 capabilities minimalism).
    if !path.starts_with("/api/v1/") && path != "/api/v1" {
        return Err(ProxyError::BadResponse("path outside /api/v1 rejected"));
    }
    let method = method.to_ascii_uppercase();
    if !matches!(method.as_str(), "GET" | "POST" | "PUT" | "PATCH" | "DELETE") {
        return Err(ProxyError::BadResponse("method not allowed"));
    }

    let body_bytes = body
        .map(serde_json::to_vec)
        .transpose()
        .map_err(|_| ProxyError::BadResponse("body not serializable"))?;

    let mut stream = TcpStream::connect(("127.0.0.1", port))
        .await
        .map_err(ProxyError::Io)?;

    let mut head = format!(
        "{method} {path} HTTP/1.1\r\n\
         Host: 127.0.0.1:{port}\r\n\
         Authorization: Bearer {token}\r\n\
         Tetherly-Client: tauri/1.0.0\r\n\
         Connection: close\r\n"
    );
    if let Some(b) = &body_bytes {
        head.push_str(&format!(
            "Content-Type: application/json\r\nContent-Length: {}\r\n",
            b.len()
        ));
    }
    head.push_str("\r\n");
    stream
        .write_all(head.as_bytes())
        .await
        .map_err(ProxyError::Io)?;
    if let Some(b) = &body_bytes {
        stream.write_all(b).await.map_err(ProxyError::Io)?;
    }
    stream.flush().await.map_err(ProxyError::Io)?;

    let mut raw = Vec::with_capacity(16 * 1024);
    stream.read_to_end(&mut raw).await.map_err(ProxyError::Io)?;
    parse_reply(&raw)
}

/// Split head/body, find Content-Length and the status code, and parse the
/// JSON body. The daemon never chunks JSON responses (uihttp
/// `write_http_response`), so read-to-EOF + Content-Length slice is exact.
fn parse_reply(raw: &[u8]) -> Result<ProxyReply, ProxyError> {
    let sep = find_header_end(raw).ok_or(ProxyError::BadResponse("no header terminator"))?;
    let head =
        std::str::from_utf8(&raw[..sep]).map_err(|_| ProxyError::BadResponse("head not utf8"))?;
    let mut lines = head.split("\r\n");
    let status_line = lines
        .next()
        .ok_or(ProxyError::BadResponse("empty response"))?;
    let status: u16 = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .ok_or(ProxyError::BadResponse("no status code"))?;

    let mut content_length: Option<usize> = None;
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            if k.eq_ignore_ascii_case("content-length") {
                content_length = v.trim().parse().ok();
            }
        }
    }
    let body_start = sep + 4;
    let body_raw = match content_length {
        Some(n) => {
            let end = (body_start + n).min(raw.len());
            &raw[body_start..end]
        }
        None => &raw[body_start..],
    };
    let body = if body_raw.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(body_raw).map_err(|_| ProxyError::BadResponse("body not json"))?
    };
    Ok(ProxyReply { status, body })
}

/// Index of the `\r\n\r\n` boundary.
fn find_header_end(raw: &[u8]) -> Option<usize> {
    raw.windows(4).position(|w| w == b"\r\n\r\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_reply_reads_status_and_json_body() {
        let raw = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 12\r\nConnection: close\r\n\r\n{\"ok\":true}";
        let r = parse_reply(raw).unwrap();
        assert_eq!(r.status, 200);
        assert_eq!(r.body["ok"], true);
    }

    #[test]
    fn parse_reply_error_body_and_204() {
        let raw = b"HTTP/1.1 404 Not Found\r\nContent-Length: 44\r\n\r\n{\"error\":{\"code\":\"not_found\",\"message\":\"x\"}}";
        let r = parse_reply(raw).unwrap();
        assert_eq!(r.status, 404);
        assert_eq!(r.body["error"]["code"], "not_found");

        let r = parse_reply(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n").unwrap();
        assert_eq!(r.status, 204);
        assert_eq!(r.body, serde_json::Value::Null);
    }

    #[test]
    fn request_rejects_paths_outside_v1_and_bad_methods() {
        // These run the early-return guards: no daemon needed.
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let err = request(1, "t", "GET", "/api/file/inbox", None).await;
            assert!(matches!(err, Err(ProxyError::BadResponse(_))), "{err:?}");
            let err = request(1, "t", "TRACE", "/api/v1/health", None).await;
            assert!(matches!(err, Err(ProxyError::BadResponse(_))), "{err:?}");
        });
    }

    /// Full round trip against an in-process mock daemon: assert the exact
    /// request line/headers the shell sends, answer with JSON, see the reply.
    #[tokio::test]
    async fn round_trip_against_mock_daemon() {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 4096];
            let n = s.read(&mut buf).await.unwrap();
            let req = String::from_utf8_lossy(&buf[..n]).to_string();
            assert!(
                req.starts_with("PATCH /api/v1/settings HTTP/1.1\r\n"),
                "{req}"
            );
            assert!(req.contains("Authorization: Bearer sekret\r\n"), "{req}");
            assert!(req.contains("Host: 127.0.0.1:"), "{req}");
            assert!(req.contains("Content-Type: application/json\r\n"), "{req}");
            assert!(req.contains("\"name\":\"Desk\""), "{req}");
            let body = br#"{"general":{"name":"Desk"}}"#;
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            s.write_all(resp.as_bytes()).await.unwrap();
            s.write_all(body).await.unwrap();
        });
        let reply = request(
            port,
            "sekret",
            "PATCH",
            "/api/v1/settings",
            Some(&serde_json::json!({"name": "Desk"})),
        )
        .await
        .unwrap();
        assert_eq!(reply.status, 200);
        assert_eq!(reply.body["general"]["name"], "Desk");
    }

    #[tokio::test]
    async fn unreachable_daemon_maps_to_io_error() {
        // Port 1 on loopback: nothing listens there.
        let err = request(1, "t", "GET", "/api/v1/health", None).await;
        assert!(matches!(err, Err(ProxyError::Io(_))), "{err:?}");
    }
}
