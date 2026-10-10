// SPDX-License-Identifier: Apache-2.0 OR MIT
//! M0.2–M0.5: SPAKE2 pairing, lockout, ping, MITM Hello, device_id mismatch.

use std::sync::{Arc, Mutex};
use std::time::Duration;
use tetherly_core::hello::{encode_b64_32, Hello};
use tetherly_core::ports::MemoryTrustStore;
use tetherly_core::session::LockoutTable;
use tetherly_core::{CoreError, DeviceId, ManualClock, TcpLimiter};
use tetherly_crypto::Identity;
use tetherly_net::codec::{read_len_prefixed, write_len_prefixed};
use tetherly_net::session::{
    accept_session, dial_session, serve_pong_once, SessionConfig, HANDSHAKE_TIMEOUT,
};
use tetherly_net::NetError;
use tokio::net::{TcpListener, TcpStream};

fn pin() -> [u8; 8] {
    tetherly_core::pin::parse_pin("25170394").unwrap()
}

fn cfg(identity: Identity, name: &str, pin: Option<[u8; 8]>) -> SessionConfig {
    SessionConfig {
        identity,
        name: name.into(),
        platform: "linux".into(),
        trust: Arc::new(Mutex::new(MemoryTrustStore::default())),
        lockout: Arc::new(Mutex::new(LockoutTable::default())),
        tcp_limiter: Arc::new(Mutex::new(TcpLimiter::default())),
        pin,
        pin_created_ms: 1,
        clock: Arc::new(ManualClock::new(1_000)),
        handshake_timeout: HANDSHAKE_TIMEOUT,
        hello_override: None,
        resume_only: false,
        screen_only: false,
    }
}

async fn bind() -> (TcpListener, u16) {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port();
    (l, port)
}

#[tokio::test]
async fn m0_2_spake2_success_then_ping() {
    let (listener, port) = bind().await;
    let a = Identity::from_secrets([0x11; 32], [0x12; 32]);
    let b = Identity::from_secrets([0x21; 32], [0x22; 32]);
    let host = cfg(a, "host", Some(pin()));
    let peer = cfg(b, "peer", Some(pin()));

    let server = tokio::spawn(async move {
        let (stream, addr) = listener.accept().await.unwrap();
        let mut sess = accept_session(stream, addr, &host).await.unwrap();
        serve_pong_once(&mut sess).await.unwrap();
        sess.peer_id
    });
    let stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let addr = stream.peer_addr().unwrap();
    let mut sess = dial_session(stream, addr, &peer).await.unwrap();
    let echoed = sess.ping_pong(42).await.unwrap();
    assert_eq!(echoed, 42);
    server.await.unwrap();
}

#[tokio::test]
async fn m0_2_wrong_pin_five_times_locks() {
    let a = Identity::from_secrets([0x31; 32], [0x32; 32]);
    let b = Identity::from_secrets([0x41; 32], [0x42; 32]);
    let host_id = a.device_id().clone();
    let lockout = Arc::new(Mutex::new(LockoutTable::default()));
    let trust = Arc::new(Mutex::new(MemoryTrustStore::default()));

    for i in 0..5 {
        let (listener, port) = bind().await;
        let mut host = cfg(
            Identity::from_secrets([0x31; 32], [0x32; 32]),
            "host",
            Some(pin()),
        );
        host.lockout = lockout.clone();
        host.trust = trust.clone();
        let mut peer = cfg(
            Identity::from_secrets([0x41; 32], [0x42; 32]),
            "peer",
            Some(tetherly_core::pin::parse_pin("00000000").unwrap()),
        );
        peer.lockout = lockout.clone();

        let server = tokio::spawn(async move {
            let (stream, addr) = listener.accept().await.unwrap();
            accept_session(stream, addr, &host).await
        });
        let stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        let addr = stream.peer_addr().unwrap();
        let client = dial_session(stream, addr, &peer).await;
        assert!(client.is_err(), "attempt {i} should fail");
        let _ = server.await.unwrap();
    }

    let loopback: std::net::IpAddr = "127.0.0.1".parse().unwrap();
    let table = lockout.lock().unwrap();
    assert!(
        table.check(Some(&host_id), None, 1_000)
            || table.check(Some(b.device_id()), None, 1_000)
            || table.check(None, Some(loopback), 1_000),
        "five wrong PINs must lock the peer or the source IP"
    );
}

#[tokio::test]
async fn m0_3_paired_ping_and_reconnect_without_re_pair() {
    let (listener, port) = bind().await;
    let a = Identity::from_secrets([0x51; 32], [0x52; 32]);
    let b = Identity::from_secrets([0x61; 32], [0x62; 32]);
    let host_trust = Arc::new(Mutex::new(MemoryTrustStore::default()));
    let peer_trust = Arc::new(Mutex::new(MemoryTrustStore::default()));

    let mut host = cfg(a, "host", Some(pin()));
    host.trust = host_trust.clone();
    let mut peer = cfg(b, "peer", Some(pin()));
    peer.trust = peer_trust.clone();

    let host_for_server = SessionConfig {
        identity: Identity::from_secrets([0x51; 32], [0x52; 32]),
        name: host.name.clone(),
        platform: host.platform.clone(),
        trust: host.trust.clone(),
        lockout: host.lockout.clone(),
        tcp_limiter: host.tcp_limiter.clone(),
        pin: host.pin,
        pin_created_ms: host.pin_created_ms,
        clock: host.clock.clone(),
        handshake_timeout: host.handshake_timeout,
        hello_override: None,
        resume_only: false,
        screen_only: false,
    };
    let server = tokio::spawn(async move {
        let (stream, addr) = listener.accept().await.unwrap();
        let mut sess = accept_session(stream, addr, &host_for_server)
            .await
            .unwrap();
        serve_pong_once(&mut sess).await.unwrap();
    });
    let stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let addr = stream.peer_addr().unwrap();
    let mut sess = dial_session(stream, addr, &peer).await.unwrap();
    assert_eq!(sess.ping_pong(7).await.unwrap(), 7);
    server.await.unwrap();

    assert_eq!(host_trust.lock().unwrap().peers.len(), 1);
    assert_eq!(peer_trust.lock().unwrap().peers.len(), 1);

    let (listener, port) = bind().await;
    let host2 = SessionConfig {
        identity: Identity::from_secrets([0x51; 32], [0x52; 32]),
        name: "host".into(),
        platform: "linux".into(),
        trust: host_trust,
        lockout: Arc::new(Mutex::new(LockoutTable::default())),
        tcp_limiter: Arc::new(Mutex::new(TcpLimiter::default())),
        pin: None,
        pin_created_ms: 1,
        clock: Arc::new(ManualClock::new(2_000)),
        handshake_timeout: HANDSHAKE_TIMEOUT,
        hello_override: None,
        resume_only: false,
        screen_only: false,
    };
    let peer2 = SessionConfig {
        identity: Identity::from_secrets([0x61; 32], [0x62; 32]),
        name: "peer".into(),
        platform: "linux".into(),
        trust: peer_trust,
        lockout: Arc::new(Mutex::new(LockoutTable::default())),
        tcp_limiter: Arc::new(Mutex::new(TcpLimiter::default())),
        pin: None,
        pin_created_ms: 1,
        clock: Arc::new(ManualClock::new(2_000)),
        handshake_timeout: HANDSHAKE_TIMEOUT,
        hello_override: None,
        resume_only: false,
        screen_only: false,
    };
    let server = tokio::spawn(async move {
        let (stream, addr) = listener.accept().await.unwrap();
        let mut sess = accept_session(stream, addr, &host2).await.unwrap();
        serve_pong_once(&mut sess).await.unwrap();
    });
    let stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let addr = stream.peer_addr().unwrap();
    let mut sess = dial_session(stream, addr, &peer2).await.unwrap();
    assert_eq!(sess.ping_pong(9).await.unwrap(), 9);
    server.await.unwrap();
}

#[tokio::test]
async fn m0_4_mitm_hello_id_pk_spake_fails_no_trust() {
    let (listener, port) = bind().await;
    let a = Identity::from_secrets([0x71; 32], [0x72; 32]);
    let b = Identity::from_secrets([0x81; 32], [0x82; 32]);
    let host = cfg(a, "host", Some(pin()));
    let mut peer = cfg(b, "peer", Some(pin()));
    let fake_pk = [0x99u8; 32];
    let mut hello = peer.identity.hello("peer", "linux", &["notify"]);
    hello.id_pk = encode_b64_32(&fake_pk);
    hello.device_id = DeviceId::from_id_pk(&fake_pk);
    peer.hello_override = Some(hello);
    let trust = host.trust.clone();

    let server = tokio::spawn(async move {
        let (stream, addr) = listener.accept().await.unwrap();
        accept_session(stream, addr, &host).await
    });
    let stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let addr = stream.peer_addr().unwrap();
    let client = dial_session(stream, addr, &peer).await;
    assert!(client.is_err());
    let server_res = server.await.unwrap();
    assert!(server_res.is_err());
    assert!(trust.lock().unwrap().peers.is_empty());
}

#[tokio::test]
async fn m0_4_mitm_hello_name_noise_fails_no_trust() {
    // hello_override would change both sides of the local prologue. The
    // actual threat is an on-wire rewrite: only the responder sees the
    // forged name, so Noise prologue hashes diverge after SPAKE2 succeeds.
    let (listener, port) = bind().await;
    let a = Identity::from_secrets([0x73; 32], [0x74; 32]);
    let b = Identity::from_secrets([0x83; 32], [0x84; 32]);
    let host = cfg(a, "host", Some(pin()));
    let peer = cfg(b, "peer", Some(pin()));
    let trust = host.trust.clone();

    let server = tokio::spawn(async move {
        let (stream, addr) = listener.accept().await.unwrap();
        accept_session(stream, addr, &host).await
    });

    let (proxy, proxy_port) = bind().await;
    let mitm = tokio::spawn(async move {
        let (mut client, _) = proxy.accept().await.unwrap();
        let mut server = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        let hello = read_len_prefixed(&mut client).await.unwrap();
        let mut parsed: Hello = serde_json::from_slice(&hello).unwrap();
        parsed.name = "evil-alias".into();
        write_len_prefixed(&mut server, &parsed.to_canonical_bytes().unwrap())
            .await
            .unwrap();
        let (mut cr, mut cw) = client.into_split();
        let (mut sr, mut sw) = server.into_split();
        let up = tokio::spawn(async move { tokio::io::copy(&mut cr, &mut sw).await });
        let down = tokio::spawn(async move { tokio::io::copy(&mut sr, &mut cw).await });
        let _ = tokio::join!(up, down);
    });

    let stream = TcpStream::connect(("127.0.0.1", proxy_port)).await.unwrap();
    let addr = stream.peer_addr().unwrap();
    let client = dial_session(stream, addr, &peer).await;
    assert!(
        client.is_err(),
        "on-wire Hello name MITM must fail Noise prologue"
    );
    let server_res = server.await.unwrap();
    assert!(server_res.is_err());
    assert!(
        trust.lock().unwrap().peers.is_empty(),
        "failed Noise must not leave a trusted peer"
    );
    let _ = mitm.await;
}

#[tokio::test]
async fn m0_5_device_id_mismatch_disconnects() {
    let (listener, port) = bind().await;
    let a = Identity::from_secrets([0x91; 32], [0x92; 32]);
    let b = Identity::from_secrets([0xA1; 32], [0xA2; 32]);
    let host = cfg(a, "host", Some(pin()));
    let mut peer = cfg(b, "peer", Some(pin()));
    let mut hello = peer.identity.hello("peer", "linux", &["notify"]);
    hello.device_id = DeviceId::from_id_pk(&[0x00; 32]);
    peer.hello_override = Some(hello);

    let server = tokio::spawn(async move {
        let (stream, addr) = listener.accept().await.unwrap();
        accept_session(stream, addr, &host).await
    });
    let stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let addr = stream.peer_addr().unwrap();
    let client = dial_session(stream, addr, &peer).await;
    match client {
        Err(NetError::DeviceIdMismatch) | Err(NetError::Io(_)) | Err(NetError::Timeout) => {}
        Err(e) => panic!("unexpected {e}"),
        Ok(_) => panic!("should disconnect"),
    }
    match server.await.unwrap() {
        Err(NetError::DeviceIdMismatch) => {}
        Err(e) => panic!("server must reject mismatched device_id, got {e}"),
        Ok(_) => panic!("server must reject mismatched device_id, got Ok"),
    }
}

#[test]
fn m0_5_hello_from_bytes_rejects_mismatch() {
    let id_pk = [3u8; 32];
    let hello = Hello {
        proto: 1,
        device_id: DeviceId::from_id_pk(&[4u8; 32]),
        id_pk: encode_b64_32(&id_pk),
        n_pk: encode_b64_32(&[5u8; 32]),
        caps: vec!["notify".into()],
        name: "x".into(),
        platform: "linux".into(),
    };
    let bytes = serde_json::to_vec(&hello).unwrap();
    assert_eq!(
        Hello::from_bytes(&bytes).unwrap_err(),
        CoreError::DeviceIdMismatch
    );
}

#[tokio::test]
async fn handshake_times_out_on_silent_peer() {
    let (listener, port) = bind().await;
    let a = Identity::from_secrets([0xB1; 32], [0xB2; 32]);
    let mut host = cfg(a, "host", Some(pin()));
    host.handshake_timeout = Duration::from_millis(200);

    let server = tokio::spawn(async move {
        let (stream, addr) = listener.accept().await.unwrap();
        accept_session(stream, addr, &host).await
    });
    let _stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    match server.await.unwrap() {
        Err(NetError::Timeout) => {}
        Err(e) => panic!("silent peer must time out, got {e}"),
        Ok(_) => panic!("silent peer must time out, got Ok"),
    }
}

#[tokio::test]
async fn tcp_rate_limit_rejects_before_handshake() {
    let (listener, port) = bind().await;
    let a = Identity::from_secrets([0xC1; 32], [0xC2; 32]);
    let host = cfg(a, "host", Some(pin()));
    {
        let mut lim = host.tcp_limiter.lock().unwrap();
        let ip = "127.0.0.1".parse().unwrap();
        for _ in 0..30 {
            assert!(lim.allow(ip, 1_000));
        }
    }
    let server = tokio::spawn(async move {
        let (stream, addr) = listener.accept().await.unwrap();
        accept_session(stream, addr, &host).await
    });
    let _stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    match server.await.unwrap() {
        Err(NetError::RateLimited) => {}
        Err(e) => panic!("31st TCP from same IP must be rate-limited, got {e}"),
        Ok(_) => panic!("31st TCP from same IP must be rate-limited, got Ok"),
    }
}

#[tokio::test]
async fn expired_pin_refuses_pair() {
    let (listener, port) = bind().await;
    let a = Identity::from_secrets([0xD1; 32], [0xD2; 32]);
    let b = Identity::from_secrets([0xE1; 32], [0xE2; 32]);
    let mut host = cfg(a, "host", Some(pin()));
    host.clock = Arc::new(ManualClock::new(1 + 3 * 60 * 1000 + 1));
    let mut peer = cfg(b, "peer", Some(pin()));
    peer.clock = host.clock.clone();

    let server = tokio::spawn(async move {
        let (stream, addr) = listener.accept().await.unwrap();
        accept_session(stream, addr, &host).await
    });
    let stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let addr = stream.peer_addr().unwrap();
    let client = dial_session(stream, addr, &peer).await;
    assert!(client.is_err());
    let server_res = server.await.unwrap();
    assert!(server_res.is_err(), "expired PIN must not pair");
}
