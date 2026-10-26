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
use tetherly_net::session::{dial_session, SessionConfig, HANDSHAKE_TIMEOUT};
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
        memory_notifier: true,
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
        screen_codec: None,
        screen_codec_h264: false,
        screen_bitrate_kbps: 4000,
        screen_bitrate_auto: true,
        screen_fps: 30,
        screen_fps_auto: true,
        screen_scale_pct: 100,
        screen_scale_auto: true,
        screen_profile: tetherly_core::ScreenContentProfile::Auto,
        screen_profile_auto: true,
        screen_qp_min: tetherly_core::SCREEN_QP_MIN_DEFAULT,
        screen_qp_max: tetherly_core::SCREEN_QP_MAX_DEFAULT,
        screen_matrix_mode: tetherly_core::ScreenMatrixMode::Auto,
        screen_playout_mode: tetherly_core::ScreenPlayoutMode::Adaptive,
        screen_cursor_mode: tetherly_core::ScreenCursorMode::Decoupled,
        screen_squelch_mode: tetherly_core::ScreenSquelchMode::Auto,
        screen_fec_mode: tetherly_core::ScreenFecMode::Auto,
        screen_pacer_mode: tetherly_core::ScreenPacerMode::Auto,
        screen_trendline_mode: tetherly_core::ScreenTrendlineMode::Auto,
        screen_harq_mode: tetherly_core::ScreenHarqMode::Auto,
        screen_twcc_mode: tetherly_core::ScreenTwccMode::Auto,
        screen_probe_mode: tetherly_core::ScreenProbeMode::Auto,
        screen_sync_mode: tetherly_core::ScreenAudioSyncMode::Auto,
        screen_latency_mode: tetherly_core::ScreenProfilerMode::Auto,
        screen_display_id: 0,
        screen_sink: None,
        input_sink: None,
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

async fn wait_input_port(node: &Node, peer: &tetherly_core::DeviceId, timeout: Duration) {
    let start = std::time::Instant::now();
    loop {
        if node
            .live_peers()
            .iter()
            .any(|p| &p.device_id == peer && p.input_port.is_some())
        {
            return;
        }
        if start.elapsed() > timeout {
            panic!("inputport= never advertised");
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
        caps: None,
    };
    let err = match dial_session(stream, addr, &cfg).await {
        Err(e) => e,
        Ok(_) => panic!("unpaired screen dial must fail"),
    };
    assert!(
        matches!(err, NetError::ScreenRequiresTrust),
        "unpaired 45720 must refuse pairing, got {err}"
    );
    // The listener bound an ephemeral port (config asked for 0); a zero here
    // would mean it never came up. `screen_addr` dials this same port.
    assert_ne!(host.screen_port(), 0, "screen listener did not bind");
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
        tokio::time::sleep(Duration::from_millis(35)).await;
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
// M5.1-h264 — negotiated H.264 frames cross the wire and decode (ADR-079)
// --------------------------------------------------------------------------
#[tokio::test(flavor = "multi_thread")]
async fn m5_1_h264_stream_roundtrips_through_the_wire() {
    // The host's source is the memory screen behind a codec that prefers
    // H.264 with peer support pre-armed; the runtime caps-merge hook keeps it
    // armed because both peers advertise screencodecs=zstd,h264.
    let src: Arc<dyn tetherly_core::ScreenSource> = Arc::new(tetherly_node::EncodedSource::new(
        MemoryScreenSource::new(16, 16).expect("16x16 screen"),
        Arc::new(tetherly_node::NegotiatedCodec::new(true, 1_000_000, true)),
    ));
    let (da, ga) = temp_dir("host-h264");
    let (db, gb) = temp_dir("visitor-h264");
    let host = Node::start(cfg(da, "host", Some(src))).unwrap();
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
            panic!("host never started streaming (h264)");
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    for _ in 0..5 {
        tokio::time::sleep(Duration::from_millis(35)).await;
        assert!(host.screen_send_frame().await.unwrap());
    }
    let start = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() >= 5 {
            break;
        }
        if start.elapsed() > Duration::from_secs(5) {
            panic!(
                "h264 frames did not arrive, count={}",
                visitor.screen_sink_snapshot().count()
            );
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let sink = visitor.screen_sink_snapshot();
    assert_eq!(sink.presented(), vec![1, 2, 3, 4, 5]);
    // The visitor presents *decoded* frames: a decoded 16x16 BGRA body means
    // the tag-3 dispatch went through the real H.264 decoder, not zstd.
    let last = sink.last().expect("presented frame");
    assert!(!last.compressed());
    assert_eq!(last.bytes.len(), 16 * 16 * 4);

    host.shutdown();
    visitor.shutdown();
    drop(ga);
    drop(gb);
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

// --------------------------------------------------------------------------
// Grab-failure recovery: one failed grab must never poison the next tick.
// The pump keeps calling `screen_send_frame`, so source-level self-healing
// (AdbSource respawn, DXGI retry) can actually engage.
// --------------------------------------------------------------------------
#[tokio::test(flavor = "multi_thread")]
async fn grab_failure_does_not_poison_the_next_frame() {
    let (da, _ga) = temp_dir("host-fail");
    let (db, _gb) = temp_dir("visitor-fail");
    let src = Arc::new(MemoryScreenSource::new(16, 16).unwrap());
    let host = Node::start(cfg(da, "host", Some(src.clone()))).unwrap();
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

    // One grab fails (the device-blip analog).
    src.fail_on_grab(1);
    assert!(
        host.screen_send_frame().await.is_err(),
        "the failed grab surfaces as an error"
    );

    // The very next tick recovers and the frame reaches the visitor.
    assert!(
        host.screen_send_frame().await.unwrap(),
        "recovered grab queues a frame"
    );
    let start = std::time::Instant::now();
    loop {
        let sink = visitor.screen_sink_snapshot();
        if sink.count() >= 1 {
            break;
        }
        if start.elapsed() > Duration::from_secs(3) {
            panic!("recovered frame did not arrive");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    host.shutdown();
    visitor.shutdown();
}

// --------------------------------------------------------------------------
// v1.34 / ADR-027 — Screen & input loopback API, revoke, and peer targeting.
// --------------------------------------------------------------------------
#[tokio::test(flavor = "multi_thread")]
async fn screen_and_input_lifecycle_and_peer_targeting_via_loopback_api() {
    let (host, visitor, _ga, _gb) = pair_view().await;
    let host_ui = host.spawn_ui().await.unwrap();
    let visitor_ui = visitor.spawn_ui().await.unwrap();
    let host_id = host.identity().device_id().clone();
    let visitor_id = visitor.identity().device_id().clone();

    wait_input_port(&host, &visitor_id, Duration::from_secs(3)).await;
    wait_input_port(&visitor, &host_id, Duration::from_secs(3)).await;

    // 1. Initial GET /api/screen on host
    let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let v: serde_json::Value = serde_json::from_slice(&b).unwrap();
    assert_eq!(v["ok"], true);
    assert_eq!(v["state"], "idle");
    assert_eq!(v["channel_open"], false);
    assert_eq!(v["sent"], 0);

    // 2. Initial GET /api/input on visitor
    let (c, b) = tetherly_node::loopback_get(visitor_ui, "/api/input")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let v: serde_json::Value = serde_json::from_slice(&b).unwrap();
    assert_eq!(v["ok"], true);
    assert_eq!(v["channel_open"], false);
    assert_eq!(v["seat"], "local");

    // Open screen channel from visitor to host so host enters requested state.
    visitor.open_screen(&host_id).await.unwrap();
    tokio::time::sleep(Duration::from_millis(80)).await;

    let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let v: serde_json::Value = serde_json::from_slice(&b).unwrap();
    assert_eq!(v["state"], "requested");

    let (c, b) = tetherly_node::loopback_get(visitor_ui, "/api/screen")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let v: serde_json::Value = serde_json::from_slice(&b).unwrap();
    assert_eq!(v["channel_open"], true);

    // 3. POST /api/screen/allow and POST /api/screen/revoke on host
    let (c, b) = tetherly_node::loopback_post(host_ui, "/api/screen/allow", b"{}")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let v: serde_json::Value = serde_json::from_slice(&b).unwrap();
    assert_eq!(v["state"], "allowed");

    let (c, b) = tetherly_node::loopback_post(host_ui, "/api/screen/revoke", b"{}")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let v: serde_json::Value = serde_json::from_slice(&b).unwrap();
    assert_eq!(v["ok"], true);
    assert_eq!(v["state"], "requested");

    // 4. Reject screen start when targeting an unknown peer
    let bad_payload = serde_json::json!({ "peer": "tdev_nonexistent" });
    let (c, _) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/start",
        &serde_json::to_vec(&bad_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 400);

    // 5. Host grants consent again
    let (c, _) = tetherly_node::loopback_post(host_ui, "/api/screen/allow", b"{}")
        .await
        .unwrap();
    assert_eq!(c, 200);

    // 6. Visitor starts screen streaming targeting the host peer
    let start_payload = serde_json::json!({ "peer": host_id.as_str() });
    let (c, b) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/start",
        &serde_json::to_vec(&start_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200, "screen start body: {}", String::from_utf8_lossy(&b));

    // Wait until host enters streaming state
    let start = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let v: serde_json::Value = serde_json::from_slice(&b).unwrap();
        if v["state"] == "streaming" {
            break;
        }
        if start.elapsed() > Duration::from_secs(5) {
            panic!("host screen never reached streaming state via loopback API");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // Host pushes 2 frames
    assert!(host.screen_send_frame().await.unwrap());
    tokio::time::sleep(Duration::from_millis(35)).await;
    assert!(host.screen_send_frame().await.unwrap());

    // Verify GET /api/screen reflects frames sent
    let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let v: serde_json::Value = serde_json::from_slice(&b).unwrap();
    assert!(v["sent"].as_u64().unwrap() >= 2);

    // 7. Visitor stops screen streaming
    let (c, _) = tetherly_node::loopback_post(visitor_ui, "/api/screen/stop", b"{}")
        .await
        .unwrap();
    assert_eq!(c, 200);

    // 8. Test input channel start, query, and stop
    let (c, _) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/input/start",
        &serde_json::to_vec(&bad_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 400);

    let (c, b) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/input/start",
        &serde_json::to_vec(&start_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200, "input start: {}", String::from_utf8_lossy(&b));

    let (c, b) = tetherly_node::loopback_get(visitor_ui, "/api/input")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let v: serde_json::Value = serde_json::from_slice(&b).unwrap();
    assert_eq!(v["ok"], true);
    assert_eq!(v["channel_open"], true);

    let (c, b) = tetherly_node::loopback_post(visitor_ui, "/api/input/stop", b"{}")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let v: serde_json::Value = serde_json::from_slice(&b).unwrap();
    assert_eq!(v["ok"], true);
    assert_eq!(v["seat"], "local");

    let (c, b) = tetherly_node::loopback_get(visitor_ui, "/api/input")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let v: serde_json::Value = serde_json::from_slice(&b).unwrap();
    assert_eq!(v["channel_open"], false);

    host.shutdown();
    visitor.shutdown();
}

// --------------------------------------------------------------------------
// v1.54 / ADR-047 — Screen & input lifecycle governance, typed reports, active
// peer tracking, and event bus integration.
// --------------------------------------------------------------------------
#[tokio::test(flavor = "multi_thread")]
async fn screen_and_input_lifecycle_governance_and_events_via_loopback_api() {
    let (host, visitor, _ga, _gb) = pair_view().await;
    let host_ui = host.spawn_ui().await.unwrap();
    let visitor_ui = visitor.spawn_ui().await.unwrap();
    let host_id = host.identity().device_id().clone();
    let visitor_id = visitor.identity().device_id().clone();

    wait_input_port(&host, &visitor_id, Duration::from_secs(3)).await;
    wait_input_port(&visitor, &host_id, Duration::from_secs(3)).await;

    // Connect to SSE stream on host
    let mut host_sse = tetherly_node::loopback_events_connect(host_ui)
        .await
        .expect("host SSE connects");

    // 1. Initial GET /api/screen on host -> ScreenStatusReport
    let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let s0: tetherly_node::ScreenStatusReport = serde_json::from_slice(&b).unwrap();
    assert!(s0.ok);
    assert_eq!(s0.state, "idle");
    assert!(!s0.allowed);
    assert!(!s0.channel_open);
    assert_eq!(s0.sent, 0);
    assert_eq!(s0.dropped, 0);
    assert_eq!(s0.refused, 0);
    assert_eq!(s0.active_peer, None);

    // 2. Initial GET /api/input on visitor -> InputStatusReport
    let (c, b) = tetherly_node::loopback_get(visitor_ui, "/api/input")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let i0: tetherly_node::InputStatusReport = serde_json::from_slice(&b).unwrap();
    assert!(i0.ok);
    assert!(!i0.channel_open);
    assert_eq!(i0.seat, "local");
    assert_eq!(i0.active_peer, None);

    // Open screen channel from visitor to host so host enters requested state.
    visitor.open_screen(&host_id).await.unwrap();
    tokio::time::sleep(Duration::from_millis(80)).await;

    // Verify host transitioned to requested state and tracks visitor as active_peer
    let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let s_req: tetherly_node::ScreenStatusReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(s_req.state, "requested");
    assert_eq!(s_req.active_peer.as_deref(), Some(visitor_id.as_str()));

    // 3. Host grants consent via POST /api/screen/allow -> ScreenControlReport
    let (c, b) = tetherly_node::loopback_post(host_ui, "/api/screen/allow", b"{}")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let allow_rep: tetherly_node::ScreenControlReport = serde_json::from_slice(&b).unwrap();
    assert!(allow_rep.ok);
    assert_eq!(allow_rep.action, "allow");
    assert_eq!(allow_rep.state, "allowed");
    assert!(allow_rep.allowed);

    // Verify host SSE receives ScreenStateChanged event for allow
    let mut found_allow_event = false;
    let start_ev = std::time::Instant::now();
    while start_ev.elapsed() < Duration::from_secs(3) {
        if let Ok(Ok(Some(raw))) =
            tokio::time::timeout(Duration::from_millis(200), host_sse.next_event()).await
        {
            if let Ok(ev) = serde_json::from_str::<serde_json::Value>(&raw) {
                if ev["kind"] == "screen_state_changed"
                    && ev["state"] == "allowed"
                    && ev["allowed"] == true
                {
                    found_allow_event = true;
                    break;
                }
            }
        }
    }
    assert!(
        found_allow_event,
        "host SSE received ScreenStateChanged for allow"
    );

    // 4. Host revokes consent via POST /api/screen/revoke -> ScreenControlReport
    let (c, b) = tetherly_node::loopback_post(host_ui, "/api/screen/revoke", b"{}")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let revoke_rep: tetherly_node::ScreenControlReport = serde_json::from_slice(&b).unwrap();
    assert!(revoke_rep.ok);
    assert_eq!(revoke_rep.action, "revoke");
    assert_eq!(revoke_rep.state, "requested");
    assert!(!revoke_rep.allowed);

    // Verify host SSE receives ScreenStateChanged event for revoke
    let mut found_revoke_event = false;
    let start_ev = std::time::Instant::now();
    while start_ev.elapsed() < Duration::from_secs(3) {
        if let Ok(Ok(Some(raw))) =
            tokio::time::timeout(Duration::from_millis(200), host_sse.next_event()).await
        {
            if let Ok(ev) = serde_json::from_str::<serde_json::Value>(&raw) {
                if ev["kind"] == "screen_state_changed" && ev["allowed"] == false {
                    found_revoke_event = true;
                    break;
                }
            }
        }
    }
    assert!(
        found_revoke_event,
        "host SSE received ScreenStateChanged for revoke"
    );

    // 5. Host grants consent again
    let (c, _) = tetherly_node::loopback_post(host_ui, "/api/screen/allow", b"{}")
        .await
        .unwrap();
    assert_eq!(c, 200);

    // 6. Visitor starts screen streaming targeting the host peer via POST /api/screen/start
    let start_payload = serde_json::json!({ "peer": host_id.as_str() });
    let (c, b) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/start",
        &serde_json::to_vec(&start_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let start_rep: tetherly_node::ScreenControlReport = serde_json::from_slice(&b).unwrap();
    assert!(start_rep.ok);
    assert_eq!(start_rep.action, "start");
    assert_eq!(start_rep.peer.as_deref(), Some(host_id.as_str()));

    // Wait until host enters streaming state
    let wait_start = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let s: tetherly_node::ScreenStatusReport = serde_json::from_slice(&b).unwrap();
        if s.state == "streaming" {
            assert!(s.allowed);
            assert!(s.channel_open);
            assert_eq!(s.active_peer.as_deref(), Some(visitor_id.as_str()));
            break;
        }
        if wait_start.elapsed() > Duration::from_secs(5) {
            panic!("host screen never reached streaming state");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // Host pushes 2 frames
    assert!(host.screen_send_frame().await.unwrap());
    tokio::time::sleep(Duration::from_millis(35)).await;
    assert!(host.screen_send_frame().await.unwrap());

    // Verify GET /api/screen reflects frames sent and active peer
    let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let s_streaming: tetherly_node::ScreenStatusReport = serde_json::from_slice(&b).unwrap();
    assert!(s_streaming.sent >= 2);
    assert_eq!(
        s_streaming.active_peer.as_deref(),
        Some(visitor_id.as_str())
    );

    // 7. Visitor stops screen streaming via POST /api/screen/stop
    let (c, b) = tetherly_node::loopback_post(visitor_ui, "/api/screen/stop", b"{}")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let stop_rep: tetherly_node::ScreenControlReport = serde_json::from_slice(&b).unwrap();
    assert!(stop_rep.ok);
    assert_eq!(stop_rep.action, "stop");

    // 8. Test input channel lifecycle and events
    let mut visitor_sse = tetherly_node::loopback_events_connect(visitor_ui)
        .await
        .expect("visitor SSE connects");

    let input_start_payload = serde_json::json!({ "peer": host_id.as_str() });
    let (c, b) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/input/start",
        &serde_json::to_vec(&input_start_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let inp_start_rep: tetherly_node::InputControlReport = serde_json::from_slice(&b).unwrap();
    assert!(inp_start_rep.ok);
    assert_eq!(inp_start_rep.action, "start");
    assert!(inp_start_rep.channel_open);
    assert_eq!(inp_start_rep.seat, "local");
    assert_eq!(inp_start_rep.peer.as_deref(), Some(host_id.as_str()));

    // Query visitor input status
    let (c, b) = tetherly_node::loopback_get(visitor_ui, "/api/input")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let inp_status: tetherly_node::InputStatusReport = serde_json::from_slice(&b).unwrap();
    assert!(inp_status.ok);
    assert!(inp_status.channel_open);
    assert_eq!(inp_status.seat, "local");
    assert_eq!(inp_status.active_peer.as_deref(), Some(host_id.as_str()));

    // Disconnect input channel via POST /api/input/stop
    let (c, b) = tetherly_node::loopback_post(visitor_ui, "/api/input/stop", b"{}")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let inp_stop_rep: tetherly_node::InputControlReport = serde_json::from_slice(&b).unwrap();
    assert!(inp_stop_rep.ok);
    assert_eq!(inp_stop_rep.action, "stop");
    assert!(!inp_stop_rep.channel_open);
    assert_eq!(inp_stop_rep.seat, "local");

    // Verify visitor SSE received InputSeatChanged event for teardown
    let mut found_input_event = false;
    let start_inp_ev = std::time::Instant::now();
    while start_inp_ev.elapsed() < Duration::from_secs(3) {
        if let Ok(Ok(Some(raw))) =
            tokio::time::timeout(Duration::from_millis(200), visitor_sse.next_event()).await
        {
            if let Ok(ev) = serde_json::from_str::<serde_json::Value>(&raw) {
                if ev["kind"] == "input_seat_changed" && ev["channel_open"] == false {
                    found_input_event = true;
                    break;
                }
            }
        }
    }
    assert!(
        found_input_event,
        "visitor SSE received InputSeatChanged for stop"
    );

    host.shutdown();
    visitor.shutdown();
}

// --------------------------------------------------------------------------
// M5.1-ack-rtt — Bi-directional frame acknowledgment, RTT telemetry,
// and congestion control backpressure (v1.93 / ADR-080)
// --------------------------------------------------------------------------
#[tokio::test(flavor = "multi_thread")]
async fn m5_1_bidirectional_frame_ack_rtt_and_congestion_backpressure() {
    let (host, visitor, _ga, _gb) = pair_view().await;
    let host_id = host.identity().device_id().clone();

    visitor.open_screen(&host_id).await.unwrap();
    tokio::time::sleep(Duration::from_millis(80)).await;

    // Host consents locally
    host.screen_allow();
    assert_eq!(host.screen_state(), ScreenState::Allowed);

    // Visitor starts streaming
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

    // Push 3 frames
    for _ in 0..3 {
        tokio::time::sleep(Duration::from_millis(35)).await;
        let sent = host.screen_send_frame().await.unwrap();
        assert!(sent, "frame should queue while streaming");
    }

    // Wait for frames to be presented by the visitor and Acknowledged back to the host
    let start = std::time::Instant::now();
    loop {
        let h_stats = host.screen_stats();
        if h_stats.acked >= 3 {
            break;
        }
        if start.elapsed() > Duration::from_secs(5) {
            panic!(
                "host did not receive acks in time: acked={}, sent={}, in_flight={}",
                h_stats.acked, h_stats.sent, h_stats.in_flight
            );
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let h_stats = host.screen_stats();
    assert_eq!(h_stats.sent, 3);
    assert!(h_stats.acked >= 3, "expected at least 3 acked frames");
    assert_eq!(h_stats.last_acked_seq, 3);
    assert_eq!(
        h_stats.in_flight, 0,
        "in_flight should drain to 0 after all acks"
    );
    assert!(
        h_stats.rtt_latest_us.is_some(),
        "rtt_latest_us must be recorded"
    );
    assert!(
        !host.is_screen_congested(),
        "channel should not be congested"
    );

    // Telemetry DTO validation via screen_status_report
    let rep = host.screen_status_report().await;
    assert!(rep.ok);
    assert_eq!(rep.state, "streaming");
    assert_eq!(rep.sent, 3);
    assert_eq!(rep.acked, 3);
    assert_eq!(rep.in_flight, 0);
    assert!(rep.rtt_latest_us.is_some());
    assert!(!rep.is_congested);

    host.shutdown();
    visitor.shutdown();
}

// --------------------------------------------------------------------------
// M5.5 — On-demand keyframe (IDR) request & recovery across the wire (ADR-081)
// --------------------------------------------------------------------------
#[tokio::test(flavor = "multi_thread")]
async fn m5_5_keyframe_request_triggers_idr_recovery() {
    let src: Arc<dyn tetherly_core::ScreenSource> = Arc::new(tetherly_node::EncodedSource::new(
        MemoryScreenSource::new(16, 16).expect("16x16 screen"),
        Arc::new(tetherly_node::NegotiatedCodec::new(true, 1_000_000, true)),
    ));
    let (da, ga) = temp_dir("host-h264-keyframe");
    let (db, gb) = temp_dir("visitor-h264-keyframe");
    let host = Node::start(cfg(da, "host", Some(src))).unwrap();
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
            panic!("host never started streaming (h264)");
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    // Host sends 2 frames (seq 1, seq 2)
    assert!(host.screen_send_frame().await.unwrap());
    tokio::time::sleep(Duration::from_millis(35)).await;
    assert!(host.screen_send_frame().await.unwrap());

    // Wait for visitor to receive and present 2 frames
    let start = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() >= 2 {
            break;
        }
        if start.elapsed() > Duration::from_secs(5) {
            panic!(
                "h264 frames did not arrive, count={}",
                visitor.screen_sink_snapshot().count()
            );
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    assert_eq!(host.screen_stats().keyframes_requested, 0);

    // Visitor issues on-demand keyframe request
    visitor.screen_keyframe_request().await.unwrap();

    // Verify host receives the KeyframeRequest control message
    let start = std::time::Instant::now();
    loop {
        if host.screen_stats().keyframes_requested >= 1 {
            break;
        }
        if start.elapsed() > Duration::from_secs(5) {
            panic!(
                "host did not receive keyframe request, count={}",
                host.screen_stats().keyframes_requested
            );
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // Host sends frame 3, which must encode as an IDR frame due to force_idr latch
    tokio::time::sleep(Duration::from_millis(35)).await;
    assert!(host.screen_send_frame().await.unwrap());

    // Wait for visitor to receive frame 3
    let start = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() >= 3 {
            break;
        }
        if start.elapsed() > Duration::from_secs(5) {
            panic!(
                "frame 3 did not arrive, count={}",
                visitor.screen_sink_snapshot().count()
            );
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let sink = visitor.screen_sink_snapshot();
    assert_eq!(sink.presented(), vec![1, 2, 3]);

    // Verify status report reports keyframes_requested
    let rep = host.screen_status_report().await;
    assert!(rep.keyframes_requested >= 1);

    host.shutdown();
    visitor.shutdown();
    drop(ga);
    drop(gb);
}

// --------------------------------------------------------------------------
// M5.6 — Dynamic AIMD rate adaptation and runtime bitrate control (v1.96 / ADR-082)
// --------------------------------------------------------------------------
#[tokio::test(flavor = "multi_thread")]
async fn m5_6_screen_dynamic_rate_adaptation() {
    let codec = Arc::new(tetherly_node::NegotiatedCodec::new(true, 2_000_000, true));
    let src: Arc<dyn tetherly_core::ScreenSource> = Arc::new(tetherly_node::EncodedSource::new(
        MemoryScreenSource::new(16, 16).expect("16x16 screen"),
        codec.clone(),
    ));
    let (da, ga) = temp_dir("host-h264-bitrate");
    let (db, gb) = temp_dir("visitor-h264-bitrate");
    let host = Node::start(cfg(da, "host", Some(src))).unwrap();
    let visitor = Node::start(cfg(db, "visitor", None)).unwrap();
    host.spawn_listeners().await.unwrap();
    visitor.spawn_listeners().await.unwrap();
    let host_ui = host.spawn_ui().await.unwrap();
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
            panic!("host never started streaming (h264-bitrate)");
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    // 1. Initial GET /api/screen/bitrate
    let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/bitrate")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let rep: tetherly_node::ScreenBitrateReport = serde_json::from_slice(&b).unwrap();
    assert!(rep.ok);
    assert_eq!(rep.min_bps, tetherly_core::SCREEN_BITRATE_MIN_BPS);
    assert_eq!(rep.max_bps, tetherly_core::SCREEN_BITRATE_MAX_BPS);
    assert_eq!(host.screen_bitrate_bps(), rep.bitrate_bps);

    // 2. Override bitrate to 3500 kbps (3.5 Mbps) via POST /api/screen/bitrate
    let req = serde_json::json!({ "kbps": 3500 });
    let (c, b) = tetherly_node::loopback_post(
        host_ui,
        "/api/screen/bitrate",
        &serde_json::to_vec(&req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let rep: tetherly_node::ScreenBitrateReport = serde_json::from_slice(&b).unwrap();
    assert!(rep.ok);
    assert_eq!(rep.bitrate_kbps, 3500);
    assert_eq!(rep.bitrate_bps, 3_500_000);
    assert_eq!(host.screen_bitrate_bps(), 3_500_000);
    assert_eq!(codec.bitrate_bps(), 3_500_000);

    // 3. Verify ScreenStatusReport reflects updated bitrate
    let s_rep = host.screen_status_report().await;
    assert_eq!(s_rep.bitrate_bps, 3_500_000);

    // 4. Test clamping bounds: request 50_000 kbps -> clamps to max (20_000 kbps)
    let req_max = serde_json::json!({ "kbps": 50000 });
    let (c, b) = tetherly_node::loopback_post(
        host_ui,
        "/api/screen/bitrate",
        &serde_json::to_vec(&req_max).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let rep_max: tetherly_node::ScreenBitrateReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(rep_max.bitrate_bps, tetherly_core::SCREEN_BITRATE_MAX_BPS);
    assert_eq!(
        host.screen_bitrate_bps(),
        tetherly_core::SCREEN_BITRATE_MAX_BPS
    );

    // Request 10 kbps -> clamps to min (200 kbps)
    let req_min = serde_json::json!({ "kbps": 10 });
    let (c, b) = tetherly_node::loopback_post(
        host_ui,
        "/api/screen/bitrate",
        &serde_json::to_vec(&req_min).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let rep_min: tetherly_node::ScreenBitrateReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(rep_min.bitrate_bps, tetherly_core::SCREEN_BITRATE_MIN_BPS);
    assert_eq!(
        host.screen_bitrate_bps(),
        tetherly_core::SCREEN_BITRATE_MIN_BPS
    );

    // Restore to normal rate for transmission
    host.screen_set_bitrate_bps(2_500_000);
    assert_eq!(host.screen_bitrate_bps(), 2_500_000);

    // 5. Send frames through reconfigured encoder and verify delivery & presentation
    for _ in 0..3 {
        tokio::time::sleep(Duration::from_millis(35)).await;
        assert!(host.screen_send_frame().await.unwrap());
    }

    let start = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() >= 3 {
            break;
        }
        if start.elapsed() > Duration::from_secs(5) {
            panic!(
                "frames did not arrive under dynamic rate, count={}",
                visitor.screen_sink_snapshot().count()
            );
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let sink = visitor.screen_sink_snapshot();
    assert_eq!(sink.presented(), vec![1, 2, 3]);

    host.shutdown();
    visitor.shutdown();
    drop(ga);
    drop(gb);
}

// --------------------------------------------------------------------------
// M5.7 — Dynamic FPS & Adaptive Frame Pacing Controller (v1.97 / ADR-083)
// --------------------------------------------------------------------------
#[tokio::test(flavor = "multi_thread")]
async fn m5_7_screen_dynamic_fps_pacing_and_telemetry() {
    let codec = Arc::new(tetherly_node::NegotiatedCodec::new(true, 2_000_000, true));
    let src: Arc<dyn tetherly_core::ScreenSource> = Arc::new(tetherly_node::EncodedSource::new(
        MemoryScreenSource::new(16, 16).expect("16x16 screen"),
        codec.clone(),
    ));
    let (da, ga) = temp_dir("host-h264-fps");
    let (db, gb) = temp_dir("visitor-h264-fps");
    let host = Node::start(cfg(da, "host", Some(src))).unwrap();
    let visitor = Node::start(cfg(db, "visitor", None)).unwrap();
    host.spawn_listeners().await.unwrap();
    visitor.spawn_listeners().await.unwrap();
    let host_ui = host.spawn_ui().await.unwrap();
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
            panic!("host never started streaming (h264-fps)");
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    // 1. Initial GET /api/screen/fps
    let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/fps")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let rep: tetherly_node::ScreenFpsReport = serde_json::from_slice(&b).unwrap();
    assert!(rep.ok);
    assert_eq!(rep.min_fps, tetherly_core::SCREEN_FPS_MIN);
    assert_eq!(rep.max_fps, tetherly_core::SCREEN_FPS_MAX);
    assert_eq!(rep.fps, 30);
    assert_eq!(rep.interval_us, 33_333);
    assert_eq!(host.screen_fps(), 30);

    // 2. Override target frame rate to 60 FPS via POST /api/screen/fps
    let req_60 = serde_json::json!({ "fps": 60 });
    let (c, b) = tetherly_node::loopback_post(
        host_ui,
        "/api/screen/fps",
        &serde_json::to_vec(&req_60).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let rep_60: tetherly_node::ScreenFpsReport = serde_json::from_slice(&b).unwrap();
    assert!(rep_60.ok);
    assert_eq!(rep_60.fps, 60);
    assert_eq!(rep_60.interval_us, 16_666);
    assert_eq!(host.screen_fps(), 60);
    assert_eq!(codec.fps(), 60);

    // 3. Verify ScreenStatusReport reflects updated fps_target
    let s_rep = host.screen_status_report().await;
    assert_eq!(s_rep.fps_target, 60);

    // 4. Test clamping bounds: request 240 FPS -> clamps to max (120 FPS)
    let req_max = serde_json::json!({ "fps": 240 });
    let (c, b) = tetherly_node::loopback_post(
        host_ui,
        "/api/screen/fps",
        &serde_json::to_vec(&req_max).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let rep_max: tetherly_node::ScreenFpsReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(rep_max.fps, tetherly_core::SCREEN_FPS_MAX);
    assert_eq!(rep_max.interval_us, 8_333);
    assert_eq!(host.screen_fps(), tetherly_core::SCREEN_FPS_MAX);

    // Request 1 FPS -> clamps to min (5 FPS, 200ms interval)
    let req_min = serde_json::json!({ "fps": 1 });
    let (c, b) = tetherly_node::loopback_post(
        host_ui,
        "/api/screen/fps",
        &serde_json::to_vec(&req_min).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let rep_min: tetherly_node::ScreenFpsReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(rep_min.fps, tetherly_core::SCREEN_FPS_MIN);
    assert_eq!(rep_min.interval_us, 200_000);
    assert_eq!(host.screen_fps(), tetherly_core::SCREEN_FPS_MIN);

    // 5. Test frame pacing enforcement under 5 FPS (200ms pacing interval)
    // First frame succeeds and consumes pacing slot
    assert!(host.screen_send_frame().await.unwrap());

    // Immediate second call should be throttled by pacing check (returns Ok(false))
    let throttled = host.screen_send_frame().await.unwrap();
    assert!(!throttled, "immediate frame should be throttled by pacer");

    // Reconfigure to 60 FPS (16.6ms interval) for smooth streaming
    host.screen_set_fps(60);
    assert_eq!(host.screen_fps(), 60);

    // Wait slightly more than 1 frame interval (25ms) and send next frames
    tokio::time::sleep(Duration::from_millis(25)).await;
    assert!(host.screen_send_frame().await.unwrap());
    tokio::time::sleep(Duration::from_millis(25)).await;
    assert!(host.screen_send_frame().await.unwrap());

    // 6. Verify frames arrive at visitor sink
    let start = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() >= 3 {
            break;
        }
        if start.elapsed() > Duration::from_secs(5) {
            panic!(
                "frames did not arrive under dynamic fps pacing, count={}",
                visitor.screen_sink_snapshot().count()
            );
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let sink = visitor.screen_sink_snapshot();
    assert_eq!(sink.presented(), vec![1, 2, 3]);

    host.shutdown();
    visitor.shutdown();
    drop(ga);
    drop(gb);
}

#[tokio::test]
async fn m5_8_screen_dynamic_resolution_scaling() {
    let codec = Arc::new(tetherly_node::NegotiatedCodec::new(true, 2_000_000, true));
    let src: Arc<dyn tetherly_core::ScreenSource> = Arc::new(tetherly_node::EncodedSource::new(
        MemoryScreenSource::new(32, 32).expect("32x32 screen"),
        codec.clone(),
    ));
    let (da, ga) = temp_dir("host-h264-scale");
    let (db, gb) = temp_dir("visitor-h264-scale");
    let host = Node::start(cfg(da, "host", Some(src))).unwrap();
    let visitor = Node::start(cfg(db, "visitor", None)).unwrap();
    host.spawn_listeners().await.unwrap();
    visitor.spawn_listeners().await.unwrap();
    let host_ui = host.spawn_ui().await.unwrap();
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
            panic!("host never started streaming (h264-scale)");
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    // 1. Initial GET /api/screen/scale
    let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/scale")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let rep: tetherly_node::ScreenScaleReport = serde_json::from_slice(&b).unwrap();
    assert!(rep.ok);
    assert_eq!(rep.min_scale_pct, tetherly_core::SCREEN_SCALE_MIN_PCT);
    assert_eq!(rep.max_scale_pct, tetherly_core::SCREEN_SCALE_MAX_PCT);
    assert_eq!(rep.scale_pct, 100);
    assert_eq!(host.screen_scale_pct(), 100);

    // Initial ScreenStatusReport check
    let s_rep = host.screen_status_report().await;
    assert_eq!(s_rep.scale_pct, 100);

    // 2. Override resolution scale to 50% via POST /api/screen/scale
    let req_50 = serde_json::json!({ "scale": 50 });
    let (c, b) = tetherly_node::loopback_post(
        host_ui,
        "/api/screen/scale",
        &serde_json::to_vec(&req_50).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let rep_50: tetherly_node::ScreenScaleReport = serde_json::from_slice(&b).unwrap();
    assert!(rep_50.ok);
    assert_eq!(rep_50.scale_pct, 50);
    assert_eq!(host.screen_scale_pct(), 50);
    assert_eq!(codec.scale_pct(), 50);

    // Verify ScreenStatusReport reflects scale_pct
    let s_rep_50 = host.screen_status_report().await;
    assert_eq!(s_rep_50.scale_pct, 50);

    // 3. Test clamping bounds: request 200% -> clamps to max (100%)
    let req_max = serde_json::json!({ "scale": 200 });
    let (c, b) = tetherly_node::loopback_post(
        host_ui,
        "/api/screen/scale",
        &serde_json::to_vec(&req_max).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let rep_max: tetherly_node::ScreenScaleReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(rep_max.scale_pct, tetherly_core::SCREEN_SCALE_MAX_PCT);
    assert_eq!(host.screen_scale_pct(), tetherly_core::SCREEN_SCALE_MAX_PCT);

    // Request 5% -> clamps to min (25%)
    let req_min = serde_json::json!({ "scale": 5 });
    let (c, b) = tetherly_node::loopback_post(
        host_ui,
        "/api/screen/scale",
        &serde_json::to_vec(&req_min).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let rep_min: tetherly_node::ScreenScaleReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(rep_min.scale_pct, tetherly_core::SCREEN_SCALE_MIN_PCT);
    assert_eq!(host.screen_scale_pct(), tetherly_core::SCREEN_SCALE_MIN_PCT);

    // Query parameter POST test: /api/screen/scale?scale=75
    let (c, b) = tetherly_node::loopback_post(host_ui, "/api/screen/scale?scale=75", b"")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let rep_75: tetherly_node::ScreenScaleReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(rep_75.scale_pct, 75);
    assert_eq!(host.screen_scale_pct(), 75);

    // 4. Send frames under 50% scale
    host.screen_set_scale_pct(50);
    assert_eq!(host.screen_scale_pct(), 50);

    // Send 3 frames
    for _ in 0..3 {
        tokio::time::sleep(Duration::from_millis(35)).await;
        assert!(host.screen_send_frame().await.unwrap());
    }

    // 5. Verify frames arrive at visitor sink
    let start = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() >= 3 {
            break;
        }
        if start.elapsed() > Duration::from_secs(5) {
            panic!(
                "frames did not arrive under dynamic resolution scaling, count={}",
                visitor.screen_sink_snapshot().count()
            );
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let sink = visitor.screen_sink_snapshot();
    assert_eq!(sink.presented(), vec![1, 2, 3]);

    // 6. Bandwidth auto-tiering helper validation
    assert_eq!(tetherly_node::screen_scale_from_profile(None), 100);
    assert_eq!(
        tetherly_node::screen_scale_from_profile(Some(1_500_000.0)),
        50
    );
    assert_eq!(
        tetherly_node::screen_scale_from_profile(Some(6_000_000.0)),
        75
    );
    assert_eq!(
        tetherly_node::screen_scale_from_profile(Some(25_000_000.0)),
        100
    );

    host.shutdown();
    visitor.shutdown();
    drop(ga);
    drop(gb);
}

// --------------------------------------------------------------------------
// v1.99 / ADR-085 — Screen dynamic content profiles & QP quality controller.
// --------------------------------------------------------------------------
#[tokio::test(flavor = "multi_thread")]
async fn m5_9_screen_dynamic_content_profile() {
    let codec = Arc::new(tetherly_node::NegotiatedCodec::new(true, 2_000_000, true));
    let src: Arc<dyn tetherly_core::ScreenSource> = Arc::new(tetherly_node::EncodedSource::new(
        MemoryScreenSource::new(32, 32).expect("32x32 screen"),
        codec.clone(),
    ));
    let (da, ga) = temp_dir("host-h264-profile");
    let (db, gb) = temp_dir("visitor-h264-profile");
    let host = Node::start(cfg(da, "host", Some(src))).unwrap();
    let visitor = Node::start(cfg(db, "visitor", None)).unwrap();
    host.spawn_listeners().await.unwrap();
    visitor.spawn_listeners().await.unwrap();
    let host_ui = host.spawn_ui().await.unwrap();
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
            panic!("host never started streaming (h264-profile)");
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    // 1. Initial GET /api/screen/profile
    let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/profile")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let rep: tetherly_node::ScreenProfileReport = serde_json::from_slice(&b).unwrap();
    assert!(rep.ok);
    assert_eq!(rep.profile, tetherly_core::ScreenContentProfile::Auto);
    assert_eq!(rep.min_qp, tetherly_core::SCREEN_QP_TEXT_DEFAULT.0);
    assert_eq!(rep.max_qp, tetherly_core::SCREEN_QP_TEXT_DEFAULT.1);
    assert!(!rep.is_custom_qp);
    assert!(rep.auto_profile);

    // Initial ScreenStatusReport check
    let s_rep = host.screen_status_report().await;
    assert_eq!(s_rep.profile, "auto");
    assert_eq!(s_rep.qp_min, tetherly_core::SCREEN_QP_TEXT_DEFAULT.0);
    assert_eq!(s_rep.qp_max, tetherly_core::SCREEN_QP_TEXT_DEFAULT.1);

    // 2. Override profile to Motion via POST /api/screen/profile
    let req_motion = serde_json::json!({ "profile": "motion" });
    let (c, b) = tetherly_node::loopback_post(
        host_ui,
        "/api/screen/profile",
        &serde_json::to_vec(&req_motion).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let rep_motion: tetherly_node::ScreenProfileReport = serde_json::from_slice(&b).unwrap();
    assert!(rep_motion.ok);
    assert_eq!(
        rep_motion.profile,
        tetherly_core::ScreenContentProfile::Motion
    );
    assert_eq!(rep_motion.min_qp, tetherly_core::SCREEN_QP_MOTION_DEFAULT.0);
    assert_eq!(rep_motion.max_qp, tetherly_core::SCREEN_QP_MOTION_DEFAULT.1);
    assert!(!rep_motion.is_custom_qp);
    assert_eq!(
        host.screen_profile(),
        tetherly_core::ScreenContentProfile::Motion
    );
    assert_eq!(
        codec.content_profile(),
        tetherly_core::ScreenContentProfile::Motion
    );

    // 3. Override QP range to custom bounds
    let req_custom_qp = serde_json::json!({ "min_qp": 10, "max_qp": 26 });
    let (c, b) = tetherly_node::loopback_post(
        host_ui,
        "/api/screen/profile",
        &serde_json::to_vec(&req_custom_qp).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let rep_qp: tetherly_node::ScreenProfileReport = serde_json::from_slice(&b).unwrap();
    assert!(rep_qp.ok);
    assert_eq!(rep_qp.min_qp, 10);
    assert_eq!(rep_qp.max_qp, 26);
    assert!(rep_qp.is_custom_qp);
    assert_eq!(host.screen_qp_range(), (10, 26));
    assert_eq!(codec.qp_range(), (10, 26));

    // Verify ScreenStatusReport reflects updated profile and QP
    let s_rep_custom = host.screen_status_report().await;
    assert_eq!(s_rep_custom.profile, "motion");
    assert_eq!(s_rep_custom.qp_min, 10);
    assert_eq!(s_rep_custom.qp_max, 26);

    // 4. Send 3 frames under Motion profile with custom QP
    for _ in 0..3 {
        tokio::time::sleep(Duration::from_millis(35)).await;
        assert!(host.screen_send_frame().await.unwrap());
    }

    // 5. Verify frames arrive at visitor sink
    let start = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() >= 3 {
            break;
        }
        if start.elapsed() > Duration::from_secs(5) {
            panic!(
                "frames did not arrive under dynamic content profile, count={}",
                visitor.screen_sink_snapshot().count()
            );
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let sink = visitor.screen_sink_snapshot();
    assert_eq!(sink.presented(), vec![1, 2, 3]);

    // 6. Switch profile to Text via query parameter
    let (c, b) = tetherly_node::loopback_post(host_ui, "/api/screen/profile?profile=text", b"")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let rep_text: tetherly_node::ScreenProfileReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(rep_text.profile, tetherly_core::ScreenContentProfile::Text);
    assert_eq!(
        host.screen_profile(),
        tetherly_core::ScreenContentProfile::Text
    );
    assert_eq!(
        codec.content_profile(),
        tetherly_core::ScreenContentProfile::Text
    );

    // Send 2 more frames under Text profile
    for _ in 0..2 {
        tokio::time::sleep(Duration::from_millis(35)).await;
        assert!(host.screen_send_frame().await.unwrap());
    }

    let start = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() >= 5 {
            break;
        }
        if start.elapsed() > Duration::from_secs(5) {
            panic!(
                "frames did not arrive under text profile, count={}",
                visitor.screen_sink_snapshot().count()
            );
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let sink2 = visitor.screen_sink_snapshot();
    assert_eq!(sink2.presented(), vec![1, 2, 3, 4, 5]);

    // 7. Auto profile helper verification
    assert_eq!(
        tetherly_node::screen_profile_from_profile(None),
        tetherly_core::ScreenContentProfile::Text
    );
    assert_eq!(
        tetherly_node::screen_profile_from_profile(Some(50_000_000.0)),
        tetherly_core::ScreenContentProfile::Text
    );

    host.shutdown();
    visitor.shutdown();
    drop(ga);
    drop(gb);
}

// --------------------------------------------------------------------------
// v2.00 / ADR-086 — Screen streaming 4D autonomous adaptation matrix coordinator.
// --------------------------------------------------------------------------
#[tokio::test(flavor = "multi_thread")]
async fn m5_10_screen_4d_matrix_coordinator() {
    let codec = Arc::new(tetherly_node::NegotiatedCodec::new(true, 2_000_000, true));
    let src: Arc<dyn tetherly_core::ScreenSource> = Arc::new(tetherly_node::EncodedSource::new(
        MemoryScreenSource::new(32, 32).expect("32x32 screen"),
        codec.clone(),
    ));
    let (da, ga) = temp_dir("host-matrix-4d");
    let (db, gb) = temp_dir("visitor-matrix-4d");
    let host = Node::start(cfg(da, "host", Some(src))).unwrap();
    let visitor = Node::start(cfg(db, "visitor", None)).unwrap();
    host.spawn_listeners().await.unwrap();
    visitor.spawn_listeners().await.unwrap();
    let host_ui = host.spawn_ui().await.unwrap();
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
            panic!("host never started streaming (matrix-4d)");
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    // 1. Initial GET /api/screen/matrix
    let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/matrix")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let rep: tetherly_node::ScreenMatrixReport = serde_json::from_slice(&b).unwrap();
    assert!(rep.ok);
    assert_eq!(rep.mode, "auto");
    assert_eq!(rep.health, "healthy");
    assert_eq!(rep.scale_pct, 100);
    assert!(rep.auto_fps);
    assert!(rep.auto_scale);

    // Initial ScreenStatusReport check
    let s_rep = host.screen_status_report().await;
    assert_eq!(s_rep.matrix_mode, "auto");
    assert_eq!(s_rep.network_health, "healthy");

    // 2. Set coordinator mode to Conservative via POST /api/screen/matrix
    let req_conservative = serde_json::json!({ "mode": "conservative" });
    let (c, b) = tetherly_node::loopback_post(
        host_ui,
        "/api/screen/matrix",
        &serde_json::to_vec(&req_conservative).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let rep_cons: tetherly_node::ScreenMatrixReport = serde_json::from_slice(&b).unwrap();
    assert!(rep_cons.ok);
    assert_eq!(rep_cons.mode, "conservative");
    assert_eq!(
        host.screen_matrix_mode(),
        tetherly_core::ScreenMatrixMode::Conservative
    );

    // 3. Set coordinator mode to Manual via query string
    let (c, b) = tetherly_node::loopback_post(host_ui, "/api/screen/matrix?mode=manual", b"")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let rep_manual: tetherly_node::ScreenMatrixReport = serde_json::from_slice(&b).unwrap();
    assert!(rep_manual.ok);
    assert_eq!(rep_manual.mode, "manual");
    assert_eq!(
        host.screen_matrix_mode(),
        tetherly_core::ScreenMatrixMode::Manual
    );

    // 4. Set coordinator mode back to Auto
    let req_auto = serde_json::json!({ "mode": "auto" });
    let (c, b) = tetherly_node::loopback_post(
        host_ui,
        "/api/screen/matrix",
        &serde_json::to_vec(&req_auto).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let rep_auto: tetherly_node::ScreenMatrixReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(rep_auto.mode, "auto");
    assert_eq!(
        host.screen_matrix_mode(),
        tetherly_core::ScreenMatrixMode::Auto
    );

    // 5. Send frames and verify visitor sink presentation and telemetry updates
    for _ in 0..4 {
        tokio::time::sleep(Duration::from_millis(35)).await;
        assert!(host.screen_send_frame().await.unwrap());
    }

    let start = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() >= 4 {
            break;
        }
        if start.elapsed() > Duration::from_secs(5) {
            panic!(
                "frames did not arrive under 4D coordinator, count={}",
                visitor.screen_sink_snapshot().count()
            );
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let sink = visitor.screen_sink_snapshot();
    assert_eq!(sink.presented(), vec![1, 2, 3, 4]);

    // 6. Verify matrix report telemetry reflects active session
    let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/matrix")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let final_rep: tetherly_node::ScreenMatrixReport = serde_json::from_slice(&b).unwrap();
    assert!(final_rep.ok);
    assert_eq!(final_rep.mode, "auto");
    assert_eq!(final_rep.health, "healthy");

    host.shutdown();
    visitor.shutdown();
    drop(ga);
    drop(gb);
}

// --------------------------------------------------------------------------
// M5.11-playout-dejitter — Receiver Playout Jitter Buffer & Dejittering
// Controller (v2.01 / ADR-087)
// --------------------------------------------------------------------------
#[tokio::test(flavor = "multi_thread")]
async fn m5_11_screen_playout_dejitter_buffer() {
    let (host, visitor, ga, gb) = pair_view().await;
    let host_ui = host.spawn_ui().await.unwrap();
    let visitor_ui = visitor.spawn_ui().await.unwrap();
    let host_id = host.identity().device_id().clone();
    let visitor_id = visitor.identity().device_id().clone();

    wait_screen_port(&host, &visitor_id, Duration::from_secs(3)).await;
    wait_screen_port(&visitor, &host_id, Duration::from_secs(3)).await;

    // 1. Initial playout status on visitor: defaults to Adaptive mode
    let (c, b) = tetherly_node::loopback_get(visitor_ui, "/api/screen/playout")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let init_rep: tetherly_node::ScreenPlayoutReport = serde_json::from_slice(&b).unwrap();
    assert!(init_rep.ok);
    assert_eq!(init_rep.mode, "adaptive");
    assert_eq!(init_rep.target_delay_ms, 15.0);
    assert_eq!(init_rep.buffered_frames, 0);

    // 2. Dynamic mode configuration via POST /api/screen/playout
    let zero_payload = serde_json::json!({ "mode": "zerolatency" });
    let (c, b) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/playout",
        &serde_json::to_vec(&zero_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let zero_rep: tetherly_node::ScreenPlayoutReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(zero_rep.mode, "zerolatency");
    assert_eq!(zero_rep.target_delay_ms, 5.0);

    let smooth_payload = serde_json::json!({ "mode": "smooth" });
    let (c, b) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/playout",
        &serde_json::to_vec(&smooth_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let smooth_rep: tetherly_node::ScreenPlayoutReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(smooth_rep.mode, "smooth");
    assert_eq!(smooth_rep.target_delay_ms, 35.0);

    // Restore to adaptive
    let auto_payload = serde_json::json!({ "mode": "adaptive" });
    let (c, b) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/playout",
        &serde_json::to_vec(&auto_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let adapt_rep: tetherly_node::ScreenPlayoutReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(adapt_rep.mode, "adaptive");
    assert_eq!(adapt_rep.target_delay_ms, 15.0);

    // 3. Visitor opens screen channel, host allows, and visitor starts streaming
    visitor.open_screen(&host_id).await.unwrap();
    tokio::time::sleep(Duration::from_millis(80)).await;

    let (c, _) = tetherly_node::loopback_post(host_ui, "/api/screen/allow", b"{}")
        .await
        .unwrap();
    assert_eq!(c, 200);

    let start_payload = serde_json::json!({ "peer": host_id.as_str() });
    let (c, _) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/start",
        &serde_json::to_vec(&start_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);

    let wait_start = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let s: tetherly_node::ScreenStatusReport = serde_json::from_slice(&b).unwrap();
        if s.state == "streaming" {
            break;
        }
        if wait_start.elapsed() > Duration::from_secs(5) {
            panic!("host screen never reached streaming state");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 4. Send 4 frames from host
    for _ in 0..4 {
        tokio::time::sleep(Duration::from_millis(35)).await;
        assert!(host.screen_send_frame().await.unwrap());
    }

    // 5. Wait for frames to pass through playout buffer and be presented to visitor sink
    let wait_recv = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() >= 4 {
            break;
        }
        if wait_recv.elapsed() > Duration::from_secs(6) {
            panic!(
                "frames did not play out through jitter buffer in time, presented count={}",
                visitor.screen_sink_snapshot().count()
            );
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let sink = visitor.screen_sink_snapshot();
    assert_eq!(sink.presented(), vec![1, 2, 3, 4]);

    // 6. Verify visitor GET /api/screen/playout reflects played frames
    let (c, b) = tetherly_node::loopback_get(visitor_ui, "/api/screen/playout")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let final_rep: tetherly_node::ScreenPlayoutReport = serde_json::from_slice(&b).unwrap();
    assert!(final_rep.ok);
    assert_eq!(final_rep.mode, "adaptive");
    assert!(final_rep.played_frames >= 4);

    // 7. Verify visitor GET /api/screen includes playout metrics
    let (c, b) = tetherly_node::loopback_get(visitor_ui, "/api/screen")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let status_rep: tetherly_node::ScreenStatusReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(status_rep.playout_mode, "adaptive");
    assert!(status_rep.playout_delay_ms > 0.0);

    host.shutdown();
    visitor.shutdown();
    drop(ga);
    drop(gb);
}

#[tokio::test]
async fn m5_12_screen_hardware_cursor_decoupling() {
    let (host, visitor, ga, gb) = pair_view().await;
    let host_ui = host.spawn_ui().await.unwrap();
    let visitor_ui = visitor.spawn_ui().await.unwrap();
    let host_id = host.identity().device_id().clone();
    let visitor_id = visitor.identity().device_id().clone();

    wait_screen_port(&host, &visitor_id, Duration::from_secs(3)).await;
    wait_screen_port(&visitor, &host_id, Duration::from_secs(3)).await;

    // 1. Initial cursor status on visitor: defaults to Decoupled mode (ADR-088)
    let (c, b) = tetherly_node::loopback_get(visitor_ui, "/api/screen/cursor")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let init_rep: tetherly_node::ScreenCursorReport = serde_json::from_slice(&b).unwrap();
    assert!(init_rep.ok);
    assert_eq!(init_rep.mode, "decoupled");
    assert_eq!(init_rep.updates_sent, 0);
    assert_eq!(init_rep.updates_received, 0);

    // 2. Dynamic mode configuration via POST /api/screen/cursor
    let embed_payload = serde_json::json!({ "mode": "embedded" });
    let (c, b) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/cursor",
        &serde_json::to_vec(&embed_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let embed_rep: tetherly_node::ScreenCursorReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(embed_rep.mode, "embedded");

    let hidden_payload = serde_json::json!({ "mode": "hidden" });
    let (c, b) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/cursor",
        &serde_json::to_vec(&hidden_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let hidden_rep: tetherly_node::ScreenCursorReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(hidden_rep.mode, "hidden");

    // Restore to decoupled
    let dec_payload = serde_json::json!({ "mode": "decoupled" });
    let (c, b) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/cursor",
        &serde_json::to_vec(&dec_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let dec_rep: tetherly_node::ScreenCursorReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(dec_rep.mode, "decoupled");

    // 3. Visitor opens screen channel, host allows, and visitor starts streaming
    visitor.open_screen(&host_id).await.unwrap();
    tokio::time::sleep(Duration::from_millis(80)).await;

    let (c, _) = tetherly_node::loopback_post(host_ui, "/api/screen/allow", b"{}")
        .await
        .unwrap();
    assert_eq!(c, 200);

    let start_payload = serde_json::json!({ "peer": host_id.as_str() });
    let (c, _) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/start",
        &serde_json::to_vec(&start_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);

    let wait_start = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let s: tetherly_node::ScreenStatusReport = serde_json::from_slice(&b).unwrap();
        if s.state == "streaming" {
            break;
        }
        if wait_start.elapsed() > Duration::from_secs(5) {
            panic!("host screen never reached streaming state");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 4. Send video frame from host
    assert!(host.screen_send_frame().await.unwrap());

    // 5. Send decoupled cursor updates from host
    // Update 1: (120, 240, true, Hand)
    let up1 = tetherly_core::ScreenCursorUpdate {
        seq: 0,
        x: 120,
        y: 240,
        visible: true,
        shape: tetherly_core::CursorShape::Hand,
        hotspot_x: 0,
        hotspot_y: 0,
        width: 0,
        height: 0,
        custom_rgba: None,
    };
    let sent1 = host.screen_send_cursor(up1).await.unwrap();
    assert!(
        sent1,
        "first cursor update should be queued and transmitted"
    );

    // Update 2: redundant update at same position/shape -> should be dropped by deduplication
    let up2 = tetherly_core::ScreenCursorUpdate {
        seq: 0,
        x: 120,
        y: 240,
        visible: true,
        shape: tetherly_core::CursorShape::Hand,
        hotspot_x: 0,
        hotspot_y: 0,
        width: 0,
        height: 0,
        custom_rgba: None,
    };
    let sent2 = host.screen_send_cursor(up2).await.unwrap();
    assert!(!sent2, "redundant cursor update should be deduplicated");

    // Advance clock/sleep to exceed SCREEN_CURSOR_MIN_INTERVAL_US (8ms)
    tokio::time::sleep(Duration::from_millis(15)).await;

    // Update 3: moved position (350, 480, true, IBeam)
    let up3 = tetherly_core::ScreenCursorUpdate {
        seq: 0,
        x: 350,
        y: 480,
        visible: true,
        shape: tetherly_core::CursorShape::IBeam,
        hotspot_x: 0,
        hotspot_y: 0,
        width: 0,
        height: 0,
        custom_rgba: None,
    };
    let sent3 = host.screen_send_cursor(up3).await.unwrap();
    assert!(sent3, "new cursor position should be transmitted");

    // 6. Wait for visitor to receive cursor updates
    let wait_cursor = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(visitor_ui, "/api/screen/cursor")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let cur_rep: tetherly_node::ScreenCursorReport = serde_json::from_slice(&b).unwrap();
        if cur_rep.updates_received >= 2 {
            assert_eq!(cur_rep.x, 350);
            assert_eq!(cur_rep.y, 480);
            assert!(cur_rep.visible);
            assert_eq!(cur_rep.shape, "ibeam");
            break;
        }
        if wait_cursor.elapsed() > Duration::from_secs(5) {
            panic!(
                "visitor did not receive cursor updates in time: {:?}",
                cur_rep
            );
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 7. Verify host and visitor screen status reports include cursor telemetry
    let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let host_status: tetherly_node::ScreenStatusReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(host_status.cursor_mode, "decoupled");
    assert_eq!(host_status.cursor_updates_sent, 2);

    let (c, b) = tetherly_node::loopback_get(visitor_ui, "/api/screen")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let visitor_status: tetherly_node::ScreenStatusReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(visitor_status.cursor_mode, "decoupled");
    assert_eq!(visitor_status.cursor_x, 350);
    assert_eq!(visitor_status.cursor_y, 480);
    assert!(visitor_status.cursor_visible);
    assert_eq!(visitor_status.cursor_shape, "ibeam");
    assert_eq!(visitor_status.cursor_updates_received, 2);

    host.shutdown();
    visitor.shutdown();
    drop(ga);
    drop(gb);
}

#[tokio::test]
async fn m5_13_screen_dirty_rect_squelch() {
    let (host, visitor, ga, gb) = pair_view().await;
    let host_ui = host.spawn_ui().await.unwrap();
    let visitor_ui = visitor.spawn_ui().await.unwrap();
    let host_id = host.identity().device_id().clone();
    let visitor_id = visitor.identity().device_id().clone();

    wait_screen_port(&host, &visitor_id, Duration::from_secs(3)).await;
    wait_screen_port(&visitor, &host_id, Duration::from_secs(3)).await;

    // 1. Initial squelch status on host: defaults to Auto mode (ADR-089)
    let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/squelch")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let init_rep: tetherly_node::ScreenSquelchReport = serde_json::from_slice(&b).unwrap();
    assert!(init_rep.ok);
    assert_eq!(init_rep.mode, "auto");
    assert!(!init_rep.is_squelched);
    assert_eq!(init_rep.frames_evaluated, 0);
    assert_eq!(init_rep.frames_passed, 0);
    assert_eq!(init_rep.frames_squelched, 0);

    // 2. Dynamic mode configuration via POST /api/screen/squelch
    let agg_payload = serde_json::json!({ "mode": "aggressive" });
    let (c, b) = tetherly_node::loopback_post(
        host_ui,
        "/api/screen/squelch",
        &serde_json::to_vec(&agg_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let agg_rep: tetherly_node::ScreenSquelchReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(agg_rep.mode, "aggressive");

    let dis_payload = serde_json::json!({ "mode": "disabled" });
    let (c, b) = tetherly_node::loopback_post(
        host_ui,
        "/api/screen/squelch",
        &serde_json::to_vec(&dis_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let dis_rep: tetherly_node::ScreenSquelchReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(dis_rep.mode, "disabled");

    // Restore to auto via query string (?mode=auto)
    let (c, b) = tetherly_node::loopback_post(host_ui, "/api/screen/squelch?mode=auto", b"")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let auto_rep: tetherly_node::ScreenSquelchReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(auto_rep.mode, "auto");

    // 3. Visitor opens screen channel, host allows, and visitor starts streaming
    visitor.open_screen(&host_id).await.unwrap();
    tokio::time::sleep(Duration::from_millis(80)).await;

    let (c, _) = tetherly_node::loopback_post(host_ui, "/api/screen/allow", b"{}")
        .await
        .unwrap();
    assert_eq!(c, 200);

    let start_payload = serde_json::json!({ "peer": host_id.as_str() });
    let (c, _) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/start",
        &serde_json::to_vec(&start_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);

    let wait_start = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let s: tetherly_node::ScreenStatusReport = serde_json::from_slice(&b).unwrap();
        if s.state == "streaming" {
            break;
        }
        if wait_start.elapsed() > Duration::from_secs(5) {
            panic!("host screen never reached streaming state");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 4. Send video frame from host
    assert!(host.screen_send_frame().await.unwrap());

    // Wait for frame to reach visitor
    let wait_frame = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() >= 1 {
            break;
        }
        if wait_frame.elapsed() > Duration::from_secs(5) {
            panic!("visitor never received first frame");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 5. Test Keyframe Request waking squelch
    let (c, _) = tetherly_node::loopback_post(visitor_ui, "/api/screen/keyframe", b"{}")
        .await
        .unwrap();
    assert_eq!(c, 200);

    // 6. Test decoupled hardware cursor continues to stream at high frequency even if screen is static
    let cur_update = tetherly_core::ScreenCursorUpdate {
        seq: 0,
        x: 450,
        y: 600,
        visible: true,
        shape: tetherly_core::CursorShape::Crosshair,
        hotspot_x: 0,
        hotspot_y: 0,
        width: 0,
        height: 0,
        custom_rgba: None,
    };
    let sent_cur = host.screen_send_cursor(cur_update).await.unwrap();
    assert!(
        sent_cur,
        "decoupled cursor update sent during static screen"
    );

    // Wait for cursor update on visitor
    let wait_cursor = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(visitor_ui, "/api/screen/cursor")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let cur_rep: tetherly_node::ScreenCursorReport = serde_json::from_slice(&b).unwrap();
        if cur_rep.updates_received >= 1 && cur_rep.shape == "crosshair" {
            assert_eq!(cur_rep.x, 450);
            assert_eq!(cur_rep.y, 600);
            break;
        }
        if wait_cursor.elapsed() > Duration::from_secs(5) {
            panic!("visitor never received decoupled cursor update during squelch test");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 7. Verify status report contains squelch telemetry fields
    let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let host_status: tetherly_node::ScreenStatusReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(host_status.squelch_mode, "auto");

    host.shutdown();
    visitor.shutdown();
    drop(ga);
    drop(gb);
}

#[tokio::test]
async fn m5_14_screen_adaptive_fec_protection() {
    let (host, visitor, ga, gb) = pair_view().await;
    let host_ui = host.spawn_ui().await.unwrap();
    let visitor_ui = visitor.spawn_ui().await.unwrap();
    let host_id = host.identity().device_id().clone();
    let visitor_id = visitor.identity().device_id().clone();

    wait_screen_port(&host, &visitor_id, Duration::from_secs(3)).await;
    wait_screen_port(&visitor, &host_id, Duration::from_secs(3)).await;

    // 1. Initial FEC query on host: defaults to Auto mode (ADR-090)
    let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/fec")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let init_rep: tetherly_core::ScreenFecReport = serde_json::from_slice(&b).unwrap();
    assert!(init_rep.ok);
    assert_eq!(init_rep.mode, "auto");
    assert_eq!(init_rep.redundancy_pct, 0);
    assert_eq!(init_rep.blocks_encoded, 0);
    assert_eq!(init_rep.recovered_shards, 0);
    assert_eq!(init_rep.unrecoverable_blocks, 0);

    // 2. Dynamic FEC mode configuration via POST /api/screen/fec
    let cons_payload = serde_json::json!({ "mode": "conservative" });
    let (c, b) = tetherly_node::loopback_post(
        host_ui,
        "/api/screen/fec",
        &serde_json::to_vec(&cons_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let cons_rep: tetherly_core::ScreenFecReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(cons_rep.mode, "conservative");
    assert_eq!(cons_rep.redundancy_pct, 10);

    let dis_payload = serde_json::json!({ "mode": "disabled" });
    let (c, b) = tetherly_node::loopback_post(
        host_ui,
        "/api/screen/fec",
        &serde_json::to_vec(&dis_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let dis_rep: tetherly_core::ScreenFecReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(dis_rep.mode, "disabled");
    assert_eq!(dis_rep.redundancy_pct, 0);

    // Set to aggressive (25% redundancy) for active Cauchy RS shard generation
    let agg_payload = serde_json::json!({ "mode": "aggressive" });
    let (c, b) = tetherly_node::loopback_post(
        host_ui,
        "/api/screen/fec",
        &serde_json::to_vec(&agg_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let agg_rep: tetherly_core::ScreenFecReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(agg_rep.mode, "aggressive");
    assert_eq!(agg_rep.redundancy_pct, 25);

    // 3. Visitor opens screen channel, host allows, and visitor starts streaming
    visitor.open_screen(&host_id).await.unwrap();
    tokio::time::sleep(Duration::from_millis(80)).await;

    let (c, _) = tetherly_node::loopback_post(host_ui, "/api/screen/allow", b"{}")
        .await
        .unwrap();
    assert_eq!(c, 200);

    let start_payload = serde_json::json!({ "peer": host_id.as_str() });
    let (c, _) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/start",
        &serde_json::to_vec(&start_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);

    let wait_start = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let s: tetherly_node::ScreenStatusReport = serde_json::from_slice(&b).unwrap();
        if s.state == "streaming" {
            break;
        }
        if wait_start.elapsed() > Duration::from_secs(5) {
            panic!("host screen never reached streaming state");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 4. Send video frame from host under Aggressive FEC mode
    assert!(host.screen_send_frame().await.unwrap());

    // Wait for frame to reach visitor and assemble via FEC
    let wait_frame = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() >= 1 {
            break;
        }
        if wait_frame.elapsed() > Duration::from_secs(5) {
            panic!("visitor never received frame under aggressive FEC protection");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 5. Verify FEC telemetry on host
    let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/fec")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let host_fec: tetherly_core::ScreenFecReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(host_fec.mode, "aggressive");
    assert_eq!(host_fec.redundancy_pct, 25);
    assert!(host_fec.blocks_encoded >= 1, "expected blocks_encoded >= 1");
    assert!(
        host_fec.data_shards_sent >= 1,
        "expected data_shards_sent >= 1"
    );
    assert!(
        host_fec.parity_shards_sent >= 1,
        "expected parity_shards_sent >= 1"
    );

    // 6. Verify status report contains FEC telemetry fields
    let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let host_status: tetherly_node::ScreenStatusReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(host_status.fec_mode, "aggressive");
    assert_eq!(host_status.fec_redundancy_pct, 25);

    let (c, b) = tetherly_node::loopback_get(visitor_ui, "/api/screen")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let visitor_status: tetherly_node::ScreenStatusReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(visitor_status.fec_unrecoverable_blocks, 0);

    host.shutdown();
    visitor.shutdown();
    drop(ga);
    drop(gb);
}

#[tokio::test]
async fn m5_15_screen_packet_pacing_and_burst_shaping() {
    let (host, visitor, ga, gb) = pair_view().await;
    let host_ui = host.spawn_ui().await.unwrap();
    let visitor_ui = visitor.spawn_ui().await.unwrap();
    let host_id = host.identity().device_id().clone();
    let visitor_id = visitor.identity().device_id().clone();

    wait_screen_port(&host, &visitor_id, Duration::from_secs(3)).await;
    wait_screen_port(&visitor, &host_id, Duration::from_secs(3)).await;

    // 1. Initial Pacer query on host: defaults to Auto mode (ADR-091)
    let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/pacer")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let init_rep: tetherly_core::ScreenPacerReport = serde_json::from_slice(&b).unwrap();
    assert!(init_rep.ok);
    assert_eq!(init_rep.mode, "auto");
    assert_eq!(init_rep.packets_paced, 0);
    assert_eq!(init_rep.bursts_smoothed, 0);

    // 2. Dynamic Pacer mode configuration via POST /api/screen/pacer
    let agg_payload = serde_json::json!({ "mode": "aggressive" });
    let (c, b) = tetherly_node::loopback_post(
        host_ui,
        "/api/screen/pacer",
        &serde_json::to_vec(&agg_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let agg_rep: tetherly_core::ScreenPacerReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(agg_rep.mode, "aggressive");

    let dis_payload = serde_json::json!({ "mode": "disabled" });
    let (c, b) = tetherly_node::loopback_post(
        host_ui,
        "/api/screen/pacer",
        &serde_json::to_vec(&dis_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let dis_rep: tetherly_core::ScreenPacerReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(dis_rep.mode, "disabled");

    // Set back to Auto mode
    let auto_payload = serde_json::json!({ "mode": "auto" });
    let (c, b) = tetherly_node::loopback_post(
        host_ui,
        "/api/screen/pacer",
        &serde_json::to_vec(&auto_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let auto_rep: tetherly_core::ScreenPacerReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(auto_rep.mode, "auto");

    // 3. Visitor opens screen channel, host allows, and visitor starts streaming
    visitor.open_screen(&host_id).await.unwrap();
    tokio::time::sleep(Duration::from_millis(80)).await;

    let (c, _) = tetherly_node::loopback_post(host_ui, "/api/screen/allow", b"{}")
        .await
        .unwrap();
    assert_eq!(c, 200);

    let start_payload = serde_json::json!({ "peer": host_id.as_str() });
    let (c, _) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/start",
        &serde_json::to_vec(&start_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);

    let wait_start = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let s: tetherly_node::ScreenStatusReport = serde_json::from_slice(&b).unwrap();
        if s.state == "streaming" {
            break;
        }
        if wait_start.elapsed() > Duration::from_secs(5) {
            panic!("host screen never reached streaming state");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 4. Send video frame from host under Auto Pacing mode
    assert!(host.screen_send_frame().await.unwrap());

    // Wait for frame to reach visitor
    let wait_frame = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() >= 1 {
            break;
        }
        if wait_frame.elapsed() > Duration::from_secs(5) {
            panic!("visitor never received paced frame");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 5. Verify Pacer telemetry on host
    let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/pacer")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let host_pacer: tetherly_core::ScreenPacerReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(host_pacer.mode, "auto");
    assert!(
        host_pacer.packets_paced >= 1,
        "expected packets_paced >= 1, got {}",
        host_pacer.packets_paced
    );

    // 6. Verify status report contains Pacer telemetry fields
    let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let host_status: tetherly_node::ScreenStatusReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(host_status.pacer_mode, "auto");
    assert!(host_status.pacer_packets_paced >= 1);

    host.shutdown();
    visitor.shutdown();
    drop(ga);
    drop(gb);
}

#[tokio::test]
async fn m5_16_screen_delay_based_trendline_filter() {
    let (host, visitor, ga, gb) = pair_view().await;
    let host_ui = host.spawn_ui().await.unwrap();
    let visitor_ui = visitor.spawn_ui().await.unwrap();
    let host_id = host.identity().device_id().clone();
    let visitor_id = visitor.identity().device_id().clone();

    wait_screen_port(&host, &visitor_id, Duration::from_secs(3)).await;
    wait_screen_port(&visitor, &host_id, Duration::from_secs(3)).await;

    // 1. Initial Trendline query on host: defaults to Auto mode (ADR-092)
    let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/trendline")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let init_rep: tetherly_core::ScreenTrendlineReport = serde_json::from_slice(&b).unwrap();
    assert!(init_rep.ok);
    assert_eq!(init_rep.mode, tetherly_core::ScreenTrendlineMode::Auto);
    assert_eq!(
        init_rep.state,
        tetherly_core::ScreenBandwidthUsageState::Normal
    );
    assert_eq!(init_rep.overuse_count, 0);
    assert_eq!(init_rep.underuse_count, 0);

    // 2. Dynamic Trendline mode configuration via POST /api/screen/trendline
    let agg_payload = serde_json::json!({ "mode": "aggressive" });
    let (c, b) = tetherly_node::loopback_post(
        host_ui,
        "/api/screen/trendline",
        &serde_json::to_vec(&agg_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let agg_rep: tetherly_core::ScreenTrendlineReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(agg_rep.mode, tetherly_core::ScreenTrendlineMode::Aggressive);

    let cons_payload = serde_json::json!({ "mode": "conservative" });
    let (c, b) = tetherly_node::loopback_post(
        host_ui,
        "/api/screen/trendline",
        &serde_json::to_vec(&cons_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let cons_rep: tetherly_core::ScreenTrendlineReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(
        cons_rep.mode,
        tetherly_core::ScreenTrendlineMode::Conservative
    );

    let dis_payload = serde_json::json!({ "mode": "disabled" });
    let (c, b) = tetherly_node::loopback_post(
        host_ui,
        "/api/screen/trendline",
        &serde_json::to_vec(&dis_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let dis_rep: tetherly_core::ScreenTrendlineReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(dis_rep.mode, tetherly_core::ScreenTrendlineMode::Disabled);

    // Set back to Auto mode
    let auto_payload = serde_json::json!({ "mode": "auto" });
    let (c, b) = tetherly_node::loopback_post(
        host_ui,
        "/api/screen/trendline",
        &serde_json::to_vec(&auto_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let auto_rep: tetherly_core::ScreenTrendlineReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(auto_rep.mode, tetherly_core::ScreenTrendlineMode::Auto);

    // 3. Visitor opens screen channel, host allows, and visitor starts streaming
    visitor.open_screen(&host_id).await.unwrap();
    tokio::time::sleep(Duration::from_millis(80)).await;

    let (c, _) = tetherly_node::loopback_post(host_ui, "/api/screen/allow", b"{}")
        .await
        .unwrap();
    assert_eq!(c, 200);

    let start_payload = serde_json::json!({ "peer": host_id.as_str() });
    let (c, _) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/start",
        &serde_json::to_vec(&start_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);

    let wait_start = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let s: tetherly_node::ScreenStatusReport = serde_json::from_slice(&b).unwrap();
        if s.state == "streaming" {
            break;
        }
        if wait_start.elapsed() > Duration::from_secs(5) {
            panic!("host screen never reached streaming state");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 4. Send video frame from host
    assert!(host.screen_send_frame().await.unwrap());

    // Wait for frame to reach visitor
    let wait_frame = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() >= 1 {
            break;
        }
        if wait_frame.elapsed() > Duration::from_secs(5) {
            panic!("visitor never received frame");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 5. Verify Trendline report on host
    let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/trendline")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let host_trendline: tetherly_core::ScreenTrendlineReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(
        host_trendline.mode,
        tetherly_core::ScreenTrendlineMode::Auto
    );
    assert_eq!(
        host_trendline.state,
        tetherly_core::ScreenBandwidthUsageState::Normal
    );

    // 6. Verify status report contains Trendline fields
    let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let host_status: tetherly_node::ScreenStatusReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(host_status.trendline_mode, "auto");
    assert_eq!(host_status.trendline_state, "normal");

    host.shutdown();
    visitor.shutdown();
    drop(ga);
    drop(gb);
}

#[tokio::test]
async fn m5_17_screen_selective_nack_and_hybrid_arq() {
    let (host, visitor, ga, gb) = pair_view().await;
    let host_ui = host.spawn_ui().await.unwrap();
    let visitor_ui = visitor.spawn_ui().await.unwrap();
    let host_id = host.identity().device_id().clone();
    let visitor_id = visitor.identity().device_id().clone();

    wait_screen_port(&host, &visitor_id, Duration::from_secs(3)).await;
    wait_screen_port(&visitor, &host_id, Duration::from_secs(3)).await;

    // 1. Initial HARQ query on host: defaults to Auto mode (ADR-093)
    let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/harq")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let init_rep: tetherly_core::ScreenHarqReport = serde_json::from_slice(&b).unwrap();
    assert!(init_rep.ok);
    assert_eq!(init_rep.mode, tetherly_core::ScreenHarqMode::Auto);
    assert_eq!(init_rep.shards_retransmitted, 0);
    assert_eq!(init_rep.recovered_frames, 0);
    assert_eq!(init_rep.nacks_sent, 0);
    assert_eq!(init_rep.nacks_received, 0);

    // 2. Dynamic HARQ mode configuration via POST /api/screen/harq
    let agg_payload = serde_json::json!({ "mode": "aggressive" });
    let (c, b) = tetherly_node::loopback_post(
        host_ui,
        "/api/screen/harq",
        &serde_json::to_vec(&agg_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let agg_rep: tetherly_core::ScreenHarqReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(agg_rep.mode, tetherly_core::ScreenHarqMode::Aggressive);

    let cons_payload = serde_json::json!({ "mode": "conservative" });
    let (c, b) = tetherly_node::loopback_post(
        host_ui,
        "/api/screen/harq",
        &serde_json::to_vec(&cons_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let cons_rep: tetherly_core::ScreenHarqReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(cons_rep.mode, tetherly_core::ScreenHarqMode::Conservative);

    let dis_payload = serde_json::json!({ "mode": "disabled" });
    let (c, b) = tetherly_node::loopback_post(
        host_ui,
        "/api/screen/harq",
        &serde_json::to_vec(&dis_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let dis_rep: tetherly_core::ScreenHarqReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(dis_rep.mode, tetherly_core::ScreenHarqMode::Disabled);

    // Set back to Auto mode
    let auto_payload = serde_json::json!({ "mode": "auto" });
    let (c, b) = tetherly_node::loopback_post(
        host_ui,
        "/api/screen/harq",
        &serde_json::to_vec(&auto_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let auto_rep: tetherly_core::ScreenHarqReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(auto_rep.mode, tetherly_core::ScreenHarqMode::Auto);

    // 3. Visitor opens screen channel, host allows, and visitor starts streaming
    visitor.open_screen(&host_id).await.unwrap();
    tokio::time::sleep(Duration::from_millis(80)).await;

    let (c, _) = tetherly_node::loopback_post(host_ui, "/api/screen/allow", b"{}")
        .await
        .unwrap();
    assert_eq!(c, 200);

    let start_payload = serde_json::json!({ "peer": host_id.as_str() });
    let (c, _) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/start",
        &serde_json::to_vec(&start_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);

    let wait_start = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let s: tetherly_node::ScreenStatusReport = serde_json::from_slice(&b).unwrap();
        if s.state == "streaming" {
            break;
        }
        if wait_start.elapsed() > Duration::from_secs(5) {
            panic!("host screen never reached streaming state");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 4. Send video frame from host
    assert!(host.screen_send_frame().await.unwrap());

    // Wait for frame to reach visitor
    let wait_frame = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() >= 1 {
            break;
        }
        if wait_frame.elapsed() > Duration::from_secs(5) {
            panic!("visitor never received frame");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 5. Verify HARQ report on host
    let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/harq")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let host_harq: tetherly_core::ScreenHarqReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(host_harq.mode, tetherly_core::ScreenHarqMode::Auto);
    assert!(host_harq.ok);

    // 6. Verify status report contains HARQ fields
    let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let host_status: tetherly_node::ScreenStatusReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(host_status.harq_mode, "auto");
    assert_eq!(host_status.harq_shards_retransmitted, 0);
    assert_eq!(host_status.harq_recovered_frames, 0);

    // 7. Verify node direct accessor methods
    assert_eq!(host.screen_harq_mode(), tetherly_core::ScreenHarqMode::Auto);
    let set_res = host.screen_set_harq_mode(tetherly_core::ScreenHarqMode::Conservative);
    assert_eq!(set_res.mode, tetherly_core::ScreenHarqMode::Conservative);
    assert_eq!(
        host.screen_harq_mode(),
        tetherly_core::ScreenHarqMode::Conservative
    );

    host.shutdown();
    visitor.shutdown();
    drop(ga);
    drop(gb);
}

#[tokio::test]
async fn m5_18_screen_receiver_driven_twcc_and_congestion_feedback() {
    let (host, visitor, ga, gb) = pair_view().await;
    let host_ui = host.spawn_ui().await.unwrap();
    let visitor_ui = visitor.spawn_ui().await.unwrap();
    let host_id = host.identity().device_id().clone();
    let visitor_id = visitor.identity().device_id().clone();

    wait_screen_port(&host, &visitor_id, Duration::from_secs(3)).await;
    wait_screen_port(&visitor, &host_id, Duration::from_secs(3)).await;

    // 1. Initial TWCC query on host: defaults to Auto mode (ADR-094)
    let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/twcc")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let init_rep: tetherly_core::ScreenTwccReport = serde_json::from_slice(&b).unwrap();
    assert!(init_rep.ok);
    assert_eq!(init_rep.mode, tetherly_core::ScreenTwccMode::Auto);
    assert_eq!(init_rep.feedbacks_sent, 0);
    assert_eq!(init_rep.feedbacks_received, 0);
    assert_eq!(init_rep.packets_lost, 0);

    // 2. Dynamic TWCC mode configuration via POST /api/screen/twcc
    let agg_payload = serde_json::json!({ "mode": "aggressive" });
    let (c, b) = tetherly_node::loopback_post(
        host_ui,
        "/api/screen/twcc",
        &serde_json::to_vec(&agg_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let agg_rep: tetherly_core::ScreenTwccReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(agg_rep.mode, tetherly_core::ScreenTwccMode::Aggressive);

    let cons_payload = serde_json::json!({ "mode": "conservative" });
    let (c, b) = tetherly_node::loopback_post(
        host_ui,
        "/api/screen/twcc",
        &serde_json::to_vec(&cons_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let cons_rep: tetherly_core::ScreenTwccReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(cons_rep.mode, tetherly_core::ScreenTwccMode::Conservative);

    let dis_payload = serde_json::json!({ "mode": "disabled" });
    let (c, b) = tetherly_node::loopback_post(
        host_ui,
        "/api/screen/twcc",
        &serde_json::to_vec(&dis_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let dis_rep: tetherly_core::ScreenTwccReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(dis_rep.mode, tetherly_core::ScreenTwccMode::Disabled);

    // Set back to Auto mode
    let auto_payload = serde_json::json!({ "mode": "auto" });
    let (c, b) = tetherly_node::loopback_post(
        host_ui,
        "/api/screen/twcc",
        &serde_json::to_vec(&auto_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let auto_rep: tetherly_core::ScreenTwccReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(auto_rep.mode, tetherly_core::ScreenTwccMode::Auto);

    // 3. Visitor opens screen channel, host allows, and visitor starts streaming
    visitor.open_screen(&host_id).await.unwrap();
    tokio::time::sleep(Duration::from_millis(80)).await;

    let (c, _) = tetherly_node::loopback_post(host_ui, "/api/screen/allow", b"{}")
        .await
        .unwrap();
    assert_eq!(c, 200);

    let start_payload = serde_json::json!({ "peer": host_id.as_str() });
    let (c, _) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/start",
        &serde_json::to_vec(&start_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);

    let wait_start = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let s: tetherly_node::ScreenStatusReport = serde_json::from_slice(&b).unwrap();
        if s.state == "streaming" {
            break;
        }
        if wait_start.elapsed() > Duration::from_secs(5) {
            panic!("host screen never reached streaming state");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 4. Send video frame from host
    assert!(host.screen_send_frame().await.unwrap());

    // Wait for frame to reach visitor
    let wait_frame = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() >= 1 {
            break;
        }
        if wait_frame.elapsed() > Duration::from_secs(5) {
            panic!("visitor never received frame");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 5. Verify TWCC report on host
    let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/twcc")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let host_twcc: tetherly_core::ScreenTwccReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(host_twcc.mode, tetherly_core::ScreenTwccMode::Auto);
    assert!(host_twcc.ok);

    // 6. Verify status report contains TWCC fields
    let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let host_status: tetherly_node::ScreenStatusReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(host_status.twcc_mode, "auto");

    // 7. Verify node direct accessor methods
    assert_eq!(host.screen_twcc_mode(), tetherly_core::ScreenTwccMode::Auto);
    let set_res = host.screen_set_twcc_mode(tetherly_core::ScreenTwccMode::Conservative);
    assert_eq!(set_res.mode, tetherly_core::ScreenTwccMode::Conservative);
    assert_eq!(
        host.screen_twcc_mode(),
        tetherly_core::ScreenTwccMode::Conservative
    );

    host.shutdown();
    visitor.shutdown();
    drop(ga);
    drop(gb);
}

#[tokio::test]
async fn m5_19_screen_active_bandwidth_probing_and_fast_rampup() {
    let (host, visitor, ga, gb) = pair_view().await;
    let host_ui = host.spawn_ui().await.unwrap();
    let visitor_ui = visitor.spawn_ui().await.unwrap();
    let host_id = host.identity().device_id().clone();
    let visitor_id = visitor.identity().device_id().clone();

    wait_screen_port(&host, &visitor_id, Duration::from_secs(3)).await;
    wait_screen_port(&visitor, &host_id, Duration::from_secs(3)).await;

    // 1. Initial Probe query on host: defaults to Auto mode (ADR-095)
    let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/probe")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let init_rep: tetherly_core::ScreenProbeReport = serde_json::from_slice(&b).unwrap();
    assert!(init_rep.ok);
    assert_eq!(init_rep.mode, tetherly_core::ScreenProbeMode::Auto);
    assert_eq!(init_rep.state, tetherly_core::ScreenProbeState::Idle);
    assert_eq!(init_rep.probes_triggered, 0);
    assert_eq!(init_rep.probes_completed, 0);
    assert_eq!(init_rep.probes_failed, 0);

    // 2. Dynamic Probe mode configuration via POST /api/screen/probe
    let agg_payload = serde_json::json!({ "mode": "aggressive" });
    let (c, b) = tetherly_node::loopback_post(
        host_ui,
        "/api/screen/probe",
        &serde_json::to_vec(&agg_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let agg_rep: tetherly_core::ScreenProbeReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(agg_rep.mode, tetherly_core::ScreenProbeMode::Aggressive);

    let cons_payload = serde_json::json!({ "mode": "conservative" });
    let (c, b) = tetherly_node::loopback_post(
        host_ui,
        "/api/screen/probe",
        &serde_json::to_vec(&cons_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let cons_rep: tetherly_core::ScreenProbeReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(cons_rep.mode, tetherly_core::ScreenProbeMode::Conservative);

    let dis_payload = serde_json::json!({ "mode": "disabled" });
    let (c, b) = tetherly_node::loopback_post(
        host_ui,
        "/api/screen/probe",
        &serde_json::to_vec(&dis_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let dis_rep: tetherly_core::ScreenProbeReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(dis_rep.mode, tetherly_core::ScreenProbeMode::Disabled);

    // Set back to Auto mode
    let auto_payload = serde_json::json!({ "mode": "auto" });
    let (c, b) = tetherly_node::loopback_post(
        host_ui,
        "/api/screen/probe",
        &serde_json::to_vec(&auto_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let auto_rep: tetherly_core::ScreenProbeReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(auto_rep.mode, tetherly_core::ScreenProbeMode::Auto);

    // 3. Visitor opens screen channel, host allows, and visitor starts streaming
    visitor.open_screen(&host_id).await.unwrap();
    tokio::time::sleep(Duration::from_millis(80)).await;

    let (c, _) = tetherly_node::loopback_post(host_ui, "/api/screen/allow", b"{}")
        .await
        .unwrap();
    assert_eq!(c, 200);

    let start_payload = serde_json::json!({ "peer": host_id.as_str() });
    let (c, _) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/start",
        &serde_json::to_vec(&start_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);

    let wait_start = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let s: tetherly_node::ScreenStatusReport = serde_json::from_slice(&b).unwrap();
        if s.state == "streaming" {
            break;
        }
        if wait_start.elapsed() > Duration::from_secs(5) {
            panic!("host screen never reached streaming state");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 4. Trigger active bandwidth probe burst via /api/screen/probe/trigger
    let (c, b) = tetherly_node::loopback_post(host_ui, "/api/screen/probe/trigger", b"{}")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let trig_rep: serde_json::Value = serde_json::from_slice(&b).unwrap();
    assert_eq!(trig_rep["ok"], true);

    // 5. Send video frame from host
    assert!(host.screen_send_frame().await.unwrap());

    // Wait for frame to reach visitor
    let wait_frame = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() >= 1 {
            break;
        }
        if wait_frame.elapsed() > Duration::from_secs(5) {
            panic!("visitor never received frame");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 6. Verify probe report on host
    let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/probe")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let host_probe: tetherly_core::ScreenProbeReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(host_probe.mode, tetherly_core::ScreenProbeMode::Auto);
    assert!(host_probe.ok);
    assert!(host_probe.probes_triggered >= 1);

    // 7. Verify status report contains Probe fields
    let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let host_status: tetherly_node::ScreenStatusReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(host_status.probe_mode, "auto");
    assert!(host_status.probes_triggered >= 1);

    // 8. Verify node direct accessor methods
    assert_eq!(
        host.screen_probe_mode(),
        tetherly_core::ScreenProbeMode::Auto
    );
    let set_res = host.screen_set_probe_mode(tetherly_core::ScreenProbeMode::Aggressive);
    assert_eq!(set_res.mode, tetherly_core::ScreenProbeMode::Aggressive);
    assert_eq!(
        host.screen_probe_mode(),
        tetherly_core::ScreenProbeMode::Aggressive
    );

    host.shutdown();
    visitor.shutdown();
    drop(ga);
    drop(gb);
}

#[tokio::test]
async fn m5_20_screen_audio_sync_and_clock_drift_compensation() {
    let (host, visitor, ga, gb) = pair_view().await;
    let host_ui = host.spawn_ui().await.unwrap();
    let visitor_ui = visitor.spawn_ui().await.unwrap();
    let host_id = host.identity().device_id().clone();
    let visitor_id = visitor.identity().device_id().clone();

    wait_screen_port(&host, &visitor_id, Duration::from_secs(3)).await;
    wait_screen_port(&visitor, &host_id, Duration::from_secs(3)).await;

    // 1. Initial Sync query on host: defaults to Auto mode (ADR-096)
    let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/sync")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let init_rep: tetherly_core::ScreenAudioSyncReport = serde_json::from_slice(&b).unwrap();
    assert!(init_rep.ok);
    assert_eq!(init_rep.mode, tetherly_core::ScreenAudioSyncMode::Auto);
    assert_eq!(init_rep.beacons_sent, 0);
    assert_eq!(init_rep.beacons_received, 0);
    assert_eq!(init_rep.current_skew_us, 0);
    assert_eq!(init_rep.clock_drift_ppm, 0.0);
    assert!(init_rep.in_sync);

    // 2. Dynamic Sync mode configuration via POST /api/screen/sync
    let strict_payload = serde_json::json!({ "mode": "strict" });
    let (c, b) = tetherly_node::loopback_post(
        host_ui,
        "/api/screen/sync",
        &serde_json::to_vec(&strict_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let strict_rep: tetherly_core::ScreenAudioSyncReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(strict_rep.mode, tetherly_core::ScreenAudioSyncMode::Strict);

    let loose_payload = serde_json::json!({ "mode": "loose" });
    let (c, b) = tetherly_node::loopback_post(
        host_ui,
        "/api/screen/sync",
        &serde_json::to_vec(&loose_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let loose_rep: tetherly_core::ScreenAudioSyncReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(loose_rep.mode, tetherly_core::ScreenAudioSyncMode::Loose);

    let dis_payload = serde_json::json!({ "mode": "disabled" });
    let (c, b) = tetherly_node::loopback_post(
        host_ui,
        "/api/screen/sync",
        &serde_json::to_vec(&dis_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let dis_rep: tetherly_core::ScreenAudioSyncReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(dis_rep.mode, tetherly_core::ScreenAudioSyncMode::Disabled);

    // Set back to Auto mode
    let auto_payload = serde_json::json!({ "mode": "auto" });
    let (c, b) = tetherly_node::loopback_post(
        host_ui,
        "/api/screen/sync",
        &serde_json::to_vec(&auto_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let auto_rep: tetherly_core::ScreenAudioSyncReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(auto_rep.mode, tetherly_core::ScreenAudioSyncMode::Auto);

    // 3. Visitor opens screen channel, host allows, and visitor starts streaming
    visitor.open_screen(&host_id).await.unwrap();
    tokio::time::sleep(Duration::from_millis(80)).await;

    let (c, _) = tetherly_node::loopback_post(host_ui, "/api/screen/allow", b"{}")
        .await
        .unwrap();
    assert_eq!(c, 200);

    let start_payload = serde_json::json!({ "peer": host_id.as_str() });
    let (c, _) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/start",
        &serde_json::to_vec(&start_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);

    let wait_start = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let s: tetherly_node::ScreenStatusReport = serde_json::from_slice(&b).unwrap();
        if s.state == "streaming" {
            break;
        }
        if wait_start.elapsed() > Duration::from_secs(5) {
            panic!("host screen never reached streaming state");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 4. Trigger active sync beacon via /api/screen/sync/trigger
    let (c, b) = tetherly_node::loopback_post(host_ui, "/api/screen/sync/trigger", b"{}")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let trig_rep: serde_json::Value = serde_json::from_slice(&b).unwrap();
    assert_eq!(trig_rep["ok"], true);

    // 5. Send video frame from host
    assert!(host.screen_send_frame().await.unwrap());

    // Wait for frame to reach visitor
    let wait_frame = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() >= 1 {
            break;
        }
        if wait_frame.elapsed() > Duration::from_secs(5) {
            panic!("visitor never received frame");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 6. Verify sync report on host
    let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/sync")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let host_sync: tetherly_core::ScreenAudioSyncReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(host_sync.mode, tetherly_core::ScreenAudioSyncMode::Auto);
    assert!(host_sync.ok);
    assert!(host_sync.beacons_sent >= 1);

    // 7. Verify status report contains Sync fields
    let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let host_status: tetherly_node::ScreenStatusReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(host_status.sync_mode, "auto");
    assert!(host_status.sync_beacons_sent >= 1);

    // 8. Verify node direct accessor methods
    assert_eq!(
        host.screen_sync_mode(),
        tetherly_core::ScreenAudioSyncMode::Auto
    );
    let set_res = host.screen_set_sync_mode(tetherly_core::ScreenAudioSyncMode::Strict);
    assert_eq!(set_res.mode, tetherly_core::ScreenAudioSyncMode::Strict);
    assert_eq!(
        host.screen_sync_mode(),
        tetherly_core::ScreenAudioSyncMode::Strict
    );

    let trig_res = host.trigger_screen_sync().await.unwrap();
    assert!(trig_res.ok);
    assert!(trig_res.beacons_sent >= 2);

    host.shutdown();
    visitor.shutdown();
    drop(ga);
    drop(gb);
}

#[tokio::test]
async fn m5_21_screen_multi_display_and_dynamic_switch_pipeline() {
    let (host, visitor, ga, gb) = pair_view().await;
    let host_ui = host.spawn_ui().await.unwrap();
    let visitor_ui = visitor.spawn_ui().await.unwrap();
    let host_id = host.identity().device_id().clone();
    let visitor_id = visitor.identity().device_id().clone();

    wait_screen_port(&host, &visitor_id, Duration::from_secs(3)).await;
    wait_screen_port(&visitor, &host_id, Duration::from_secs(3)).await;

    // 1. Configure multi-display topology on host
    let d0 = tetherly_core::ScreenDisplayInfo::primary_default();
    let d1 = tetherly_core::ScreenDisplayInfo::new(
        1,
        "Display 1 (Secondary)",
        2560,
        1440,
        125,
        false,
        false,
    );
    let d2 = tetherly_core::ScreenDisplayInfo::new(
        2,
        "Virtual Display (ADR-073)",
        1920,
        1200,
        100,
        false,
        true,
    );
    host.screen_set_displays(vec![d0.clone(), d1.clone(), d2.clone()]);

    // 2. Query displays via GET /api/screen/displays on host
    let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/displays")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let init_rep: tetherly_core::ScreenDisplayReport = serde_json::from_slice(&b).unwrap();
    assert!(init_rep.ok);
    assert_eq!(init_rep.active_display_id, 0);
    assert_eq!(init_rep.displays.len(), 3);
    assert_eq!(init_rep.switches_count, 0);

    // 3. Visitor opens screen channel, host allows, and visitor starts streaming
    visitor.open_screen(&host_id).await.unwrap();
    tokio::time::sleep(Duration::from_millis(80)).await;

    let (c, _) = tetherly_node::loopback_post(host_ui, "/api/screen/allow", b"{}")
        .await
        .unwrap();
    assert_eq!(c, 200);

    let start_payload = serde_json::json!({ "peer": host_id.as_str() });
    let (c, _) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/start",
        &serde_json::to_vec(&start_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);

    let wait_start = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let s: tetherly_node::ScreenStatusReport = serde_json::from_slice(&b).unwrap();
        if s.state == "streaming" {
            break;
        }
        if wait_start.elapsed() > Duration::from_secs(5) {
            panic!("host screen never reached streaming state");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 4. Dynamic Switch to display 1 via POST /api/screen/switch
    let switch_payload = serde_json::json!({ "display_id": 1 });
    let (c, b) = tetherly_node::loopback_post(
        host_ui,
        "/api/screen/switch",
        &serde_json::to_vec(&switch_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let switch_rep: tetherly_core::ScreenDisplayReport = serde_json::from_slice(&b).unwrap();
    assert!(switch_rep.ok);
    assert_eq!(switch_rep.active_display_id, 1);
    assert_eq!(switch_rep.switches_count, 1);

    // 5. Send video frame after switch and verify receipt
    assert!(host.screen_send_frame().await.unwrap());
    let wait_frame = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() >= 1 {
            break;
        }
        if wait_frame.elapsed() > Duration::from_secs(5) {
            panic!("visitor never received frame");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 6. Switch to Virtual Display (ID 2) via node direct method
    let switch2_rep = host.screen_switch_display(2).await.unwrap();
    assert!(switch2_rep.ok);
    assert_eq!(switch2_rep.active_display_id, 2);
    assert_eq!(switch2_rep.switches_count, 2);
    assert_eq!(host.screen_active_display_id(), 2);
    assert_eq!(host.screen_display_switches_count(), 2);

    // 7. Verify status report contains active display and switch counters
    let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let host_status: tetherly_node::ScreenStatusReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(host_status.active_display_id, 2);
    assert_eq!(host_status.display_count, 3);
    assert_eq!(host_status.display_switches_count, 2);

    // 8. Verify error rejection on invalid display ID
    let invalid_payload = serde_json::json!({ "display_id": 99 });
    let (c, b) = tetherly_node::loopback_post(
        host_ui,
        "/api/screen/switch",
        &serde_json::to_vec(&invalid_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 404);
    assert!(String::from_utf8_lossy(&b).contains("Display 99 not found"));

    host.shutdown();
    visitor.shutdown();
    drop(ga);
    drop(gb);
}

#[tokio::test]
async fn m5_22_screen_privacy_mode_and_input_lockout_pipeline() {
    let (host, visitor, ga, gb) = pair_view().await;
    let host_ui = host.spawn_ui().await.unwrap();
    let visitor_ui = visitor.spawn_ui().await.unwrap();
    let host_id = host.identity().device_id().clone();
    let visitor_id = visitor.identity().device_id().clone();

    wait_screen_port(&host, &visitor_id, Duration::from_secs(3)).await;
    wait_screen_port(&visitor, &host_id, Duration::from_secs(3)).await;

    // 1. Initial privacy status check on host (defaults: disabled, unlocked)
    let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/privacy")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let init_priv: tetherly_core::ScreenPrivacyReport = serde_json::from_slice(&b).unwrap();
    assert!(init_priv.ok);
    assert!(!init_priv.enabled);
    assert!(!init_priv.lock_input);
    assert_eq!(init_priv.blanking_method, "virtual_overlay");

    // 2. Visitor opens screen channel, host allows, and visitor starts streaming
    visitor.open_screen(&host_id).await.unwrap();
    tokio::time::sleep(Duration::from_millis(80)).await;

    let (c, _) = tetherly_node::loopback_post(host_ui, "/api/screen/allow", b"{}")
        .await
        .unwrap();
    assert_eq!(c, 200);

    let start_payload = serde_json::json!({ "peer": host_id.as_str() });
    let (c, _) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/start",
        &serde_json::to_vec(&start_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);

    let wait_start = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let s: tetherly_node::ScreenStatusReport = serde_json::from_slice(&b).unwrap();
        if s.state == "streaming" {
            break;
        }
        if wait_start.elapsed() > Duration::from_secs(5) {
            panic!("host screen never reached streaming state");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 3. Enable Privacy Mode with input lockout via POST /api/screen/privacy
    let enable_req = serde_json::json!({
        "enabled": true,
        "lock_input": true
    });
    let (c, b) = tetherly_node::loopback_post(
        host_ui,
        "/api/screen/privacy",
        &serde_json::to_vec(&enable_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let enabled_rep: tetherly_core::ScreenPrivacyReport = serde_json::from_slice(&b).unwrap();
    assert!(enabled_rep.ok);
    assert!(enabled_rep.enabled);
    assert!(enabled_rep.lock_input);

    // 4. Verify host ScreenStatusReport includes privacy telemetry
    let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let status_rep: tetherly_node::ScreenStatusReport = serde_json::from_slice(&b).unwrap();
    assert!(status_rep.privacy_enabled);
    assert!(status_rep.privacy_input_locked);
    assert_eq!(status_rep.privacy_blanking_method, "virtual_overlay");

    // 5. Send video frame while privacy mode is active
    assert!(host.screen_send_frame().await.unwrap());
    let wait_frame = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() >= 1 {
            break;
        }
        if wait_frame.elapsed() > Duration::from_secs(5) {
            panic!("visitor never received frame under privacy mode");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 6. Disable Privacy Mode via POST /api/screen/privacy/disable
    let (c, b) = tetherly_node::loopback_post(host_ui, "/api/screen/privacy/disable", &[])
        .await
        .unwrap();
    assert_eq!(c, 200);
    let disabled_rep: tetherly_core::ScreenPrivacyReport = serde_json::from_slice(&b).unwrap();
    assert!(!disabled_rep.enabled);
    assert!(!disabled_rep.lock_input);

    // 7. Re-enable via convenience route /api/screen/privacy/enable?lock_input=true
    let (c, b) =
        tetherly_node::loopback_post(host_ui, "/api/screen/privacy/enable?lock_input=true", &[])
            .await
            .unwrap();
    assert_eq!(c, 200);
    let re_enabled: tetherly_core::ScreenPrivacyReport = serde_json::from_slice(&b).unwrap();
    assert!(re_enabled.enabled);
    assert!(re_enabled.lock_input);

    // 8. Fail-safe Disconnect Recovery: stopping screen session automatically disengages privacy mode
    let stop_payload = serde_json::json!({ "peer": host_id.as_str() });
    let (c, _) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/stop",
        &serde_json::to_vec(&stop_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Direct check on host privacy report after session teardown
    let host_priv = host.screen_privacy_report();
    assert!(
        !host_priv.enabled,
        "Fail-safe: privacy mode must be disengaged after session teardown"
    );
    assert!(
        !host_priv.lock_input,
        "Fail-safe: input lockout must be released after session teardown"
    );

    host.shutdown();
    visitor.shutdown();
    drop(ga);
    drop(gb);
}

#[tokio::test]
async fn m5_23_screen_microsecond_latency_breakdown_and_multi_stage_profiler_pipeline() {
    let (host, visitor, ga, gb) = pair_view().await;
    let host_ui = host.spawn_ui().await.unwrap();
    let visitor_ui = visitor.spawn_ui().await.unwrap();
    let host_id = host.identity().device_id().clone();
    let visitor_id = visitor.identity().device_id().clone();

    wait_screen_port(&host, &visitor_id, Duration::from_secs(3)).await;
    wait_screen_port(&visitor, &host_id, Duration::from_secs(3)).await;

    // 1. Initial latency profiler status check on host (defaults: auto mode, 0 frames)
    let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/latency")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let init_rep: tetherly_core::ScreenLatencyReport = serde_json::from_slice(&b).unwrap();
    assert!(init_rep.ok);
    assert_eq!(init_rep.mode, "auto");
    assert_eq!(init_rep.frames_profiled, 0);

    // 2. Configure latency profiler mode to 'detailed' via POST /api/screen/latency
    let mode_req = serde_json::json!({ "mode": "detailed" });
    let (c, b) = tetherly_node::loopback_post(
        host_ui,
        "/api/screen/latency",
        &serde_json::to_vec(&mode_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let set_rep: tetherly_core::ScreenLatencyReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(set_rep.mode, "detailed");

    // 3. Visitor opens screen channel, host allows, visitor starts streaming
    visitor.open_screen(&host_id).await.unwrap();
    tokio::time::sleep(Duration::from_millis(80)).await;

    let (c, _) = tetherly_node::loopback_post(host_ui, "/api/screen/allow", b"{}")
        .await
        .unwrap();
    assert_eq!(c, 200);

    let start_payload = serde_json::json!({ "peer": host_id.as_str() });
    let (c, _) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/start",
        &serde_json::to_vec(&start_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);

    let wait_start = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let s: tetherly_node::ScreenStatusReport = serde_json::from_slice(&b).unwrap();
        if s.state == "streaming" {
            break;
        }
        if wait_start.elapsed() > Duration::from_secs(5) {
            panic!("host screen never reached streaming state");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 4. Send video frames from host and verify reception on visitor
    for _ in 0..5 {
        assert!(host.screen_send_frame().await.unwrap());
        tokio::time::sleep(Duration::from_millis(30)).await;
    }

    let wait_frame = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() >= 3 {
            break;
        }
        if wait_frame.elapsed() > Duration::from_secs(5) {
            panic!("visitor never received frames");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 5. Query latency report on visitor receiver side
    let (c, b) = tetherly_node::loopback_get(visitor_ui, "/api/screen/latency")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let rx_lat: tetherly_core::ScreenLatencyReport = serde_json::from_slice(&b).unwrap();
    assert!(rx_lat.ok);
    assert_eq!(rx_lat.mode, "auto");
    assert!(rx_lat.frames_profiled >= 1, "frames_profiled must be >= 1");
    assert!(rx_lat.current.frame_seq >= 1);
    assert!(
        rx_lat.current.decode_us > 0
            || rx_lat.current.render_us > 0
            || rx_lat.current.total_e2e_us > 0
    );

    // 6. Verify ScreenStatusReport contains latency telemetry
    let (c, b) = tetherly_node::loopback_get(visitor_ui, "/api/screen")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let rx_status: tetherly_node::ScreenStatusReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(rx_status.latency_mode, "auto");
    assert!(rx_status.latency_frames_profiled >= 1);
    assert!(!rx_status.latency_bottleneck.is_empty());

    // 7. Test POST /api/screen/latency/reset
    let (c, b) = tetherly_node::loopback_post(visitor_ui, "/api/screen/latency/reset", &[])
        .await
        .unwrap();
    assert_eq!(c, 200);
    let reset_rep: tetherly_core::ScreenLatencyReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(reset_rep.frames_profiled, 0);
    assert_eq!(reset_rep.bottleneck, "balanced");

    // 8. Stop streaming session cleanly
    let stop_payload = serde_json::json!({ "peer": host_id.as_str() });
    let (c, _) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/stop",
        &serde_json::to_vec(&stop_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);

    host.shutdown();
    visitor.shutdown();
    drop(ga);
    drop(gb);
}
