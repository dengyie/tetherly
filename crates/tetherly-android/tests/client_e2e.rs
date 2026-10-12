// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Host-side end-to-end: the Android client dials a real `accept_session`
//! server (the desktop's control path), pairs with the PIN, and delivers a
//! `notify.push`. Runs on every CI platform — no Android toolchain needed.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tetherly_android::AndroidClient;
use tetherly_core::ports::{Clock, MemoryTrustStore, SystemClock};
use tetherly_core::session::LockoutTable;
use tetherly_core::{NotifyPush, TcpLimiter};
use tetherly_crypto::Identity;
use tetherly_net::session::{accept_session, SessionConfig, HANDSHAKE_TIMEOUT};
use tetherly_net::SessionEvent;
use tokio::net::TcpListener;

fn pin() -> [u8; 8] {
    tetherly_core::pin::parse_pin("25170394").unwrap()
}

fn server_cfg() -> SessionConfig {
    SessionConfig {
        identity: Identity::from_secrets([0x11; 32], [0x12; 32]),
        name: "desktop".into(),
        platform: "windows".into(),
        trust: Arc::new(Mutex::new(MemoryTrustStore::default())),
        lockout: Arc::new(Mutex::new(LockoutTable::default())),
        tcp_limiter: Arc::new(Mutex::new(TcpLimiter::default())),
        pin: Some(pin()),
        pin_created_ms: SystemClock.unix_ms(),
        clock: Arc::new(SystemClock),
        handshake_timeout: HANDSHAKE_TIMEOUT,
        hello_override: None,
        resume_only: false,
        screen_only: false,
        caps: None,
    }
}

#[tokio::test]
async fn android_client_pairs_and_delivers_notify() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        let (stream, addr) = listener.accept().await.unwrap();
        let mut sess = accept_session(stream, addr, &server_cfg()).await.unwrap();
        match sess.recv_event().await.unwrap() {
            SessionEvent::Notify(n) => (n.app_id, n.body, n.app_name),
            other => panic!("expected notify, got {other:?}"),
        }
    });

    let dir = std::env::temp_dir().join(format!(
        "tetherly-android-e2e-{}-{port}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let client = AndroidClient::load_or_create(dir.clone(), "android-test").unwrap();
    let handle = client.start(
        format!("127.0.0.1:{port}").parse().unwrap(),
        Some(pin()),
        false,
    );

    let deadline = Instant::now() + Duration::from_secs(10);
    while handle.connected_peer().is_none() && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        handle.connected_peer().is_some(),
        "client should connect: {:?}",
        handle.last_error()
    );

    let push = NotifyPush {
        uid: "e2e-1".into(),
        app_id: "com.example.sms".into(),
        app_name: "Messages".into(),
        title: "网易".into(),
        body: "【网易】验证码：868740".into(),
        ts: 42,
        actions: vec!["copy".into()],
    };
    assert!(handle.notify(push), "queued while connected");

    let (app_id, body, app_name) = server.await.unwrap();
    assert_eq!(app_id, "com.example.sms");
    assert_eq!(app_name, "Messages");
    assert_eq!(body, "【网易】验证码：868740");

    // Pairing committed trust, so a relaunch resumes without the PIN.
    assert!(
        dir.join("trusted.json").exists(),
        "trust must be persisted after pairing"
    );

    handle.stop();
    let _ = std::fs::remove_dir_all(&dir);
}

/// M1.1 harness must work against a real session: a server that answers pings
/// with pongs, so `rtt_bench` collects the requested samples and `percentile`
/// orders them. Verifies the harness itself (the number is transport-dependent).
#[tokio::test]
async fn rtt_bench_collects_samples_against_pong_server() {
    use tetherly_android::percentile;
    use tetherly_core::frame::pong;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        let (stream, addr) = listener.accept().await.unwrap();
        let mut sess = accept_session(stream, addr, &server_cfg()).await.unwrap();
        // Answer a bounded number of pings; the client sends `count`.
        for _ in 0..16 {
            match sess.recv_event().await.unwrap() {
                SessionEvent::Ping(ts) => {
                    sess.send_inner(pong(ts)).await.unwrap();
                }
                other => panic!("expected ping, got {other:?}"),
            }
        }
    });

    let dir = std::env::temp_dir().join(format!(
        "tetherly-android-bench-{}-{port}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let samples = tetherly_android::rtt_bench(
        format!("127.0.0.1:{port}").parse().unwrap(),
        dir.clone(),
        "android-test",
        Some(pin()),
        16,
        Duration::from_millis(1),
    )
    .await
    .expect("bench runs against a live pong server");

    assert_eq!(samples.len(), 16, "one sample per answered ping");
    let p50 = percentile(&samples, 0.50).unwrap();
    let p95 = percentile(&samples, 0.95).unwrap();
    assert!(p50 <= p95, "p50 must not exceed p95");
    assert!(p95 < 1_000_000, "localhost RTT stays well under 1s");

    server.await.unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

/// The desktop can answer a phone notification in place: it sends `notify.reply`
/// and the client's inbox surfaces it exactly once, addressed to the notification.
#[tokio::test]
async fn android_client_receives_a_quick_reply_once() {
    use tetherly_core::frame::NotifyReply;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        let (stream, addr) = listener.accept().await.unwrap();
        let mut sess = accept_session(stream, addr, &server_cfg()).await.unwrap();
        let reply = NotifyReply {
            uid: "e2e-reply".into(),
            app_id: "com.example.sms".into(),
            text: "收到,马上回电".into(),
        };
        sess.send_inner(reply.to_frame().unwrap()).await.unwrap();
        // Keep the session alive until the client drains and stops.
        tokio::time::sleep(Duration::from_millis(500)).await;
    });

    let dir = std::env::temp_dir().join(format!(
        "tetherly-android-reply-{}-{port}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let client = AndroidClient::load_or_create(dir.clone(), "android-test").unwrap();
    let handle = client.start(
        format!("127.0.0.1:{port}").parse().unwrap(),
        Some(pin()),
        false,
    );

    let deadline = Instant::now() + Duration::from_secs(10);
    let replies = loop {
        let r = handle.take_replies();
        if !r.is_empty() {
            break r;
        }
        if Instant::now() > deadline {
            panic!("reply did not arrive: {:?}", handle.last_error());
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    };
    assert_eq!(replies.len(), 1);
    assert_eq!(replies[0].uid, "e2e-reply");
    assert_eq!(replies[0].app_id, "com.example.sms");
    assert_eq!(replies[0].text, "收到,马上回电");
    // Drained exactly once: the Kotlin layer fires the action a single time.
    assert!(handle.take_replies().is_empty());

    handle.stop();
    let _ = server.await;
    let _ = std::fs::remove_dir_all(&dir);
}
