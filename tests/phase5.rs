// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Phase 5 remote-screen port 45720. Resume-only Noise, binary TMV1,
//! explicit local consent. Frames only flow after both `allow()` (host) and
//! `Start` (visitor). Real OS capture / encode / present stay Manual-required.
//! RustDesk is not linked (AGPL-3.0).

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tetherly_core::{MemoryScreenSource, ScreenState};
use tetherly_net::session::{dial_session, SessionConfig, HANDSHAKE_TIMEOUT, SCREEN_PORT};
use tetherly_net::NetError;
use tetherly_node::{Node, NodeConfig, OverlayConfig};
use tokio::net::TcpStream;

struct DirGuard(PathBuf);

impl Drop for DirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn temp_dir(tag: &str) -> (PathBuf, DirGuard) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "tetherly-p5-{tag}-{}-{}-{n}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    (dir.clone(), DirGuard(dir))
}

fn cfg(
    dir: PathBuf,
    name: &str,
    screen_source: Option<Arc<dyn tetherly_core::ScreenSource>>,
) -> NodeConfig {
    NodeConfig {
        data_dir: dir,
        name: name.into(),
        platform: "windows".into(),
        control_port: 0,
        file_port: 0,
        input_port: 0,
        screen_port: 0,
        advertise: false,
        memory_clip: true,
        loopback_only: true,
        reconnect: false,
        ui_port: 0,
        overlay: OverlayConfig {
            enabled: false,
            ..OverlayConfig::default()
        },
        ancs: None,
        ancs_peripheral: "test-iphone".into(),
        screen_source,
    }
}

async fn wait_live(node: &Node, timeout: Duration) {
    let start = std::time::Instant::now();
    while node.live_peers().is_empty() {
        if start.elapsed() > timeout {
            panic!("peer did not come up");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn wait_screen_port(node: &Node, peer: &tetherly_core::DeviceId, timeout: Duration) {
    let start = std::time::Instant::now();
    loop {
        if node
            .live_peers()
            .iter()
            .any(|p| &p.device_id == peer && p.screen_port.is_some())
        {
            return;
        }
        if start.elapsed() > timeout {
            panic!("screenport= never advertised");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn mem_source() -> Arc<dyn tetherly_core::ScreenSource> {
    Arc::new(MemoryScreenSource::new(16, 16).expect("16x16 screen"))
}

/// Pair two nodes on 45717. The host (first) injects a MemoryScreenSource and
/// is the machine being viewed.
async fn pair_view() -> (Node, Node, DirGuard, DirGuard) {
    let (da, ga) = temp_dir("host");
    let (db, gb) = temp_dir("visitor");
    let host = Node::start(cfg(da, "host", Some(mem_source()))).unwrap();
    let visitor = Node::start(cfg(db, "visitor", None)).unwrap();
    host.spawn_listeners().await.unwrap();
    visitor.spawn_listeners().await.unwrap();
    let pin = tetherly_core::pin::parse_pin("25170394").unwrap();
    host.set_pin(pin);
    visitor.set_pin(pin);
    host.dial(visitor.control_addr()).await.unwrap();
    wait_live(&host, Duration::from_secs(25)).await;
    wait_live(&visitor, Duration::from_secs(25)).await;
    wait_screen_port(
        &host,
        visitor.identity().device_id(),
        Duration::from_secs(3),
    )
    .await;
    wait_screen_port(
        &visitor,
        host.identity().device_id(),
        Duration::from_secs(3),
    )
    .await;
    (host, visitor, ga, gb)
}

// --------------------------------------------------------------------------
// M5.4 — unpaired 45720 refuses pairing (ScreenRequiresTrust)
// --------------------------------------------------------------------------
#[tokio::test(flavor = "multi_thread")]
async fn unpaired_45720_refuses_pairing() {
    let (dir, _g) = temp_dir("unpaired");
    let host = Node::start(cfg(dir, "host", None)).unwrap();
    host.spawn_listeners().await.unwrap();
    let stream = TcpStream::connect(host.screen_addr()).await.unwrap();
    let addr = stream.peer_addr().unwrap();
    let stranger = tetherly_crypto::Identity::generate().unwrap();
    let cfg = SessionConfig {
        identity: stranger,
        name: "stranger".into(),
        platform: "windows".into(),
        trust: Arc::new(std::sync::Mutex::new(Default::default())),
        lockout: Arc::new(std::sync::Mutex::new(Default::default())),
        tcp_limiter: Arc::new(std::sync::Mutex::new(Default::default())),
        pin: Some(tetherly_core::pin::parse_pin("25170394").unwrap()),
        pin_created_ms: 1,
        clock: Arc::new(tetherly_core::SystemClock),
        handshake_timeout: HANDSHAKE_TIMEOUT,
        hello_override: None,
        resume_only: false,
        screen_only: true,
    };
    let err = match dial_session(stream, addr, &cfg).await {
        Err(e) => e,
        Ok(_) => panic!("unpaired screen dial must fail"),
    };
    assert!(
        matches!(err, NetError::ScreenRequiresTrust),
        "unpaired 45720 must refuse pairing, got {err}"
    );
    assert_ne!(host.screen_port(), SCREEN_PORT);
    let _ = SocketAddr::from(([127, 0, 0, 1], host.screen_port()));
}

// --------------------------------------------------------------------------
// M5.2 — Start before allow is refused. No frames flow until consented.
// --------------------------------------------------------------------------
#[tokio::test(flavor = "multi_thread")]
async fn m5_2_start_refused_until_allow() {
    let (host, visitor, _ga, _gb) = pair_view().await;
    visitor
        .open_screen(host.identity().device_id())
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(80)).await;
    assert_eq!(host.screen_state(), ScreenState::Requested);

    // Start without allow is counted as refused.
    let _ = visitor.screen_start().await;
    tokio::time::sleep(Duration::from_millis(80)).await;
    assert_eq!(host.screen_state(), ScreenState::Requested);
    assert_eq!(host.screen_stats().refused, 1);

    // Push does nothing while not streaming.
    assert!(!host.screen_send_frame().await.unwrap());

    host.shutdown();
    visitor.shutdown();
}

// --------------------------------------------------------------------------
// M5.1 — Consent-and-start flow: frames arrive monotonic, no jumps.
// --------------------------------------------------------------------------
#[tokio::test(flavor = "multi_thread")]
async fn m5_1_consent_then_start_flows_frames_monotonic() {
    let (host, visitor, _ga, _gb) = pair_view().await;
    visitor
        .open_screen(host.identity().device_id())
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(80)).await;

    // Host consents locally.
    host.screen_allow();
    assert_eq!(host.screen_state(), ScreenState::Allowed);

    // Visitor sends Start.
    visitor.screen_start().await.unwrap();

    // Wait for host to enter Streaming.
    let start = std::time::Instant::now();
    loop {
        if host.screen_state() == ScreenState::Streaming {
            break;
        }
        if start.elapsed() > Duration::from_secs(3) {
            panic!("host never started streaming");
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    // Push 5 frames.
    for _ in 0..5 {
        let sent = host.screen_send_frame().await.unwrap();
        assert!(sent, "frame should queue while streaming");
    }

    // Wait for frames to land on the visitor.
    let start = std::time::Instant::now();
    loop {
        let sink = visitor.screen_sink_snapshot();
        if sink.count() >= 5 {
            break;
        }
        if start.elapsed() > Duration::from_secs(3) {
            panic!("frames did not arrive, count={}", sink.count());
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let sink = visitor.screen_sink_snapshot();
    let presented: Vec<u64> = sink.presented();
    assert_eq!(presented.len(), 5);
    assert_eq!(
        presented,
        vec![1, 2, 3, 4, 5],
        "seqs must be strictly monotonic 1..=5"
    );

    assert_eq!(host.screen_stats().sent, 5);
    assert_eq!(host.screen_stats().refused, 0);

    host.shutdown();
    visitor.shutdown();
}

// --------------------------------------------------------------------------
// M5.3 — Peer disconnection resets state and seq.
// --------------------------------------------------------------------------
#[tokio::test(flavor = "multi_thread")]
async fn m5_3_peer_gone_resets_to_idle() {
    let (host, visitor, _ga, _gb) = pair_view().await;
    visitor
        .open_screen(host.identity().device_id())
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(80)).await;
    host.screen_allow();
    visitor.screen_start().await.unwrap();

    let start = std::time::Instant::now();
    loop {
        if host.screen_state() == ScreenState::Streaming {
            break;
        }
        if start.elapsed() > Duration::from_secs(3) {
            panic!("host never started streaming");
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    host.screen_send_frame().await.unwrap();

    // Shutdown the visitor — host must go back to Idle.
    visitor.shutdown();
    let start = std::time::Instant::now();
    loop {
        if host.screen_state() == ScreenState::Idle {
            break;
        }
        if start.elapsed() > Duration::from_secs(3) {
            panic!("host did not go idle, state={:?}", host.screen_state());
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    // After returning to Idle, a later frame push is dropped.
    assert!(!host.screen_send_frame().await.unwrap());

    host.shutdown();
}
