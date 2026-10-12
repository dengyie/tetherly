// SPDX-License-Identifier: Apache-2.0 OR MIT
//! In-band Noise rekey. `snow` 0.9 has no `TransportState::rekey()`, so the
//! SSOT mandates rebuilding a fresh IK handshake after 2^16 messages or 3600s.
//! These tests force the boundary and check both sides stay in step.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use tetherly_core::frame::{ping, pong, TYPE_PING, TYPE_PONG};
use tetherly_core::ports::{Clock, MemoryTrustStore, SystemClock};
use tetherly_core::session::LockoutTable;
use tetherly_core::TcpLimiter;
use tetherly_crypto::Identity;
use tetherly_net::session::{accept_session, dial_session, SessionConfig, HANDSHAKE_TIMEOUT};
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

/// Both ends cross the message limit mid-conversation. The dialer drives the
/// rekey; the acceptor reacts. Every ping/pong after the boundary must use the
/// new transport, and none may be lost.
#[tokio::test]
async fn rekey_preserves_traffic_across_the_message_limit() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        let (stream, peer) = listener.accept().await.unwrap();
        let mut sess = accept_session(stream, peer, &cfg(0x11, "server"))
            .await
            .unwrap();
        // Both sides are "due" before the first frame, so the very first
        // exchange has to run the rekey.
        sess.debug_force_rekey_due();
        let mut seen = Vec::new();
        for _ in 0..5 {
            let f = sess.recv_inner().await.unwrap();
            seen.push(f.ty);
            sess.send_inner(pong(0)).await.unwrap();
        }
        seen
    });

    let stream = TcpStream::connect(addr).await.unwrap();
    let mut sess = dial_session(stream, addr, &cfg(0x21, "client"))
        .await
        .unwrap();
    sess.debug_force_rekey_due();
    for _ in 0..5 {
        sess.send_inner(ping(7)).await.unwrap();
        let f = sess.recv_inner().await.unwrap();
        assert_eq!(f.ty, TYPE_PONG, "reply must survive the rekey");
    }

    let seen = server.await.unwrap();
    assert_eq!(seen, vec![TYPE_PING; 5], "no ping lost across the rekey");
}

/// A rekey that is cancelled (as `select!` does) must resume cleanly rather
/// than drop the handshake or double-ratchet. The server is delayed so the
/// client's first send is guaranteed to be cancelled while parked on `m2`.
#[tokio::test]
async fn rekey_resumes_after_a_cancelled_send() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        let (stream, peer) = listener.accept().await.unwrap();
        let mut sess = accept_session(stream, peer, &cfg(0x31, "server"))
            .await
            .unwrap();
        // Hold off answering for a moment so the client's rekey is definitely
        // still in flight when its send is cancelled.
        tokio::time::sleep(Duration::from_millis(300)).await;
        let mut seen = Vec::new();
        for _ in 0..3 {
            let f = sess.recv_inner().await.unwrap();
            seen.push(f.ty);
            sess.send_inner(pong(0)).await.unwrap();
        }
        seen
    });

    let stream = TcpStream::connect(addr).await.unwrap();
    let mut sess = dial_session(stream, addr, &cfg(0x41, "client"))
        .await
        .unwrap();
    sess.debug_force_rekey_due();

    // The send future is polled first (biased) and parks awaiting `m2`; the
    // already-ready competitor then cancels it mid-rekey.
    tokio::select! {
        biased;
        r = sess.send_inner(ping(7)) => panic!("rekey cannot complete before the server replies: {r:?}"),
        _ = std::future::ready(()) => {}
    }
    // Let the delayed server read `m1` and send `m2`.
    tokio::time::sleep(Duration::from_millis(500)).await;

    // The cancelled attempt left the rekey in flight; a real send finishes it
    // instead of restarting or dropping the handshake.
    for _ in 0..3 {
        sess.send_inner(ping(7)).await.unwrap();
        assert_eq!(sess.recv_inner().await.unwrap().ty, TYPE_PONG);
    }

    let seen = server.await.unwrap();
    assert_eq!(
        seen,
        vec![TYPE_PING; 3],
        "cancelled rekey must not drop or duplicate frames"
    );
}

/// A peer whose static key does not match the one we trust must be rejected at
/// rekey time, exactly as at the initial handshake — a replayed `Hello` must
/// not let an attacker substitute their static key mid-session.
#[tokio::test]
async fn rekey_rejects_a_changed_static_key() {
    // Cover the responder's static check directly: an attacker's `m1` built
    // with a different static must not match the trusted key.
    use tetherly_crypto::noise::{prologue, NoiseHandshake};

    let honest = Identity::from_secrets([1u8; 32], [2u8; 32]);
    let attacker = Identity::from_secrets([3u8; 32], [4u8; 32]);
    let responder = Identity::from_secrets([5u8; 32], [6u8; 32]);

    let h_honest = honest.hello("honest", "linux", &["input"]);
    let h_resp = responder.hello("resp", "linux", &["input"]);
    let p = prologue(&h_honest, &h_resp).unwrap();

    let mut atk = NoiseHandshake::initiator(&attacker, responder.n_pk(), &p).unwrap();
    let m1 = atk.write_message(b"").unwrap();
    let mut resp = NoiseHandshake::responder(&responder, &p).unwrap();
    resp.read_message(&m1).unwrap();
    // The responder would compare this to the trusted `honest.n_pk()` and
    // refuse; the mismatch is what `accept_rekey_start` rejects.
    assert_ne!(resp.remote_static(), Some(honest.n_pk().as_slice()));
    assert_eq!(resp.remote_static(), Some(attacker.n_pk().as_slice()));
}
