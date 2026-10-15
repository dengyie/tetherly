// SPDX-License-Identifier: Apache-2.0 OR MIT
//! A session whose peer stops reading must fail the write loudly, not wedge.
//!
//! This pins the v1.21 "silent black hole": the control session used to `await`
//! a write inside its `select!` body, and `write_tagged`'s trailing `flush()`
//! parks forever once the kernel send buffer fills — so the task went deaf
//! while the **unbounded** queue kept accepting, and every later `notify`
//! reported success while the peer received nothing. `send_bounded` bounds the
//! write, so the same condition now surfaces as `NetError::WriteStalled` and
//! the caller can tear the session down.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tetherly_core::frame::{InnerFrame, INNER_PAYLOAD_MAX, TYPE_PING};
use tetherly_core::ports::{Clock, MemoryTrustStore, SystemClock};
use tetherly_core::session::LockoutTable;
use tetherly_core::TcpLimiter;
use tetherly_crypto::Identity;
use tetherly_net::session::{
    accept_session, dial_session, SessionConfig, HANDSHAKE_TIMEOUT, SESSION_WRITE_TIMEOUT,
};
use tetherly_net::NetError;
use tokio::net::{TcpListener, TcpStream};

fn cfg(seed: u8, name: &str) -> SessionConfig {
    SessionConfig {
        identity: Identity::from_secrets([seed; 32], [seed.wrapping_add(1); 32]),
        name: name.into(),
        platform: "linux".into(),
        trust: Arc::new(Mutex::new(MemoryTrustStore::default())),
        lockout: Arc::new(Mutex::new(LockoutTable::default())),
        tcp_limiter: Arc::new(Mutex::new(TcpLimiter::default())),
        pin: Some(tetherly_core::pin::parse_pin("25170394").unwrap()),
        pin_created_ms: SystemClock.unix_ms(),
        clock: Arc::new(SystemClock),
        handshake_timeout: HANDSHAKE_TIMEOUT,
        hello_override: None,
        resume_only: false,
        screen_only: false,
        caps: None,
    }
}

/// A large frame, so a handful of them exhaust the loopback buffers. Kept just
/// under `INNER_PAYLOAD_MAX`: the Noise transport caps a message at 65535 bytes,
/// and the 12-byte inner header plus the 16-byte tag must fit inside it.
fn filler() -> InnerFrame {
    InnerFrame::new(TYPE_PING, vec![0u8; INNER_PAYLOAD_MAX - 64]).unwrap()
}

/// The peer finishes the handshake then never reads again. The writer must get
/// `WriteStalled` **from the write timeout** — not block forever, and not be
/// rescued by some other error.
#[tokio::test]
async fn a_peer_that_stops_reading_yields_write_stalled_not_a_hang() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        let (stream, peer) = listener.accept().await.unwrap();
        // Complete the handshake, then hold the socket open and read nothing:
        // the client's send window closes and its buffer fills.
        let _sess = accept_session(stream, peer, &cfg(0x31, "server"))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_secs(120)).await;
    });

    let stream = TcpStream::connect(addr).await.unwrap();
    let mut sess = dial_session(stream, addr, &cfg(0x41, "client"))
        .await
        .unwrap();

    let mut stalled_after = None;
    // 4096 * 64 KiB = 256 MiB, far beyond any loopback buffer; if the buffer
    // somehow absorbed it all we would loop out and fail honestly below.
    for _ in 0..4096 {
        let t0 = Instant::now();
        match sess.send_bounded(filler()).await {
            Ok(()) => continue,
            Err(NetError::WriteStalled) => {
                stalled_after = Some(t0.elapsed());
                break;
            }
            Err(other) => panic!("expected WriteStalled, got {other:?}"),
        }
    }
    server.abort();

    let stalled_after =
        stalled_after.expect("a peer that never reads must eventually stall the writer");
    assert!(
        stalled_after >= SESSION_WRITE_TIMEOUT,
        "the stall must come from the write timeout ({SESSION_WRITE_TIMEOUT:?}), \
         but the call returned after {stalled_after:?}"
    );
}

/// The other half of the contract: a peer that keeps reading must never be
/// turned into a false failure by the new bound.
#[tokio::test]
async fn a_reading_peer_never_reports_write_stalled() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        let (stream, peer) = listener.accept().await.unwrap();
        let mut sess = accept_session(stream, peer, &cfg(0x51, "server"))
            .await
            .unwrap();
        for _ in 0..64 {
            sess.recv_inner().await.unwrap();
        }
    });

    let stream = TcpStream::connect(addr).await.unwrap();
    let mut sess = dial_session(stream, addr, &cfg(0x61, "client"))
        .await
        .unwrap();
    for _ in 0..64 {
        sess.send_bounded(filler())
            .await
            .expect("a draining peer must not stall the writer");
    }
    server.await.unwrap();
}
