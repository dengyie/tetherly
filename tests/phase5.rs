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
        screen_tuning_mode: tetherly_core::ScreenTunerMode::Standard,
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

/// Longer than one frame interval at the test config's `screen_fps` (30 →
/// 33.3 ms). `screen_send_frame` refuses a send that lands inside the pacer
/// interval, so a test that stages a change and then sends must wait past that
/// bound — the old 25 ms sleeps were under it and only passed when the code in
/// between happened to be slow enough.
const PACER_SETTLE: Duration = Duration::from_millis(40);

/// Wait until the *host* has consumed the visitor's `Start` and reports
/// `streaming`.
///
/// `POST /api/screen/start` answers 200 when the visitor has queued the control
/// message; the host enters `Streaming` only when its screen channel task reads
/// it and the local consent gate is already open. Sending a frame before that
/// transition is refused by design (`screen_send_frame` → `Ok(false)`), so a
/// test that asserts on the very next send has to wait for the state it is
/// asserting about rather than sleep a guessed interval.
async fn wait_host_streaming(host_ui: u16) {
    let start = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let report: tetherly_node::ScreenStatusReport = serde_json::from_slice(&b).unwrap();
        if report.state == "streaming" {
            return;
        }
        if start.elapsed() > Duration::from_secs(5) {
            panic!("host never reached streaming state: {}", report.state);
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
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
    let (c, _) = tetherly_node::loopback_post(
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

    // Past one frame interval at the new 60 FPS pacing, so both sends land in
    // a fresh slot.
    tokio::time::sleep(PACER_SETTLE).await;
    assert!(host.screen_send_frame().await.unwrap());
    tokio::time::sleep(PACER_SETTLE).await;
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

    // 6. Verify matrix report telemetry reflects active session.
    //
    // `health` is assessed from the *sender's* view (`in_flight > 0` or a slow
    // RTT reads as "warning"), while the loop above only waits for the
    // *receiver* to present. Presentation does not mean the ack for the last
    // frame has come back yet, so a single-shot `health == "healthy"` here is
    // asserting a value that legitimately lags by one round trip — and on a
    // loaded runner that round trip is long enough to lose the race. The
    // coordinator's contract is that health returns to "healthy" once the
    // burst is acknowledged, so wait for that instead of sampling once.
    let deadline = std::time::Instant::now();
    let final_rep: tetherly_node::ScreenMatrixReport = loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/matrix")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let rep: tetherly_node::ScreenMatrixReport = serde_json::from_slice(&b).unwrap();
        assert!(rep.ok);
        assert_eq!(rep.mode, "auto");
        if rep.health == "healthy" {
            break rep;
        }
        if deadline.elapsed() > Duration::from_secs(3) {
            panic!("matrix health never returned to healthy: {}", rep.health);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
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
        tokio::time::sleep(Duration::from_millis(40)).await;
        let sent = host.screen_send_frame().await.unwrap();
        assert!(sent, "frame should queue while streaming");
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

// ADR-100 Screen Streaming Bottleneck-Driven Autonomous Tuning Closed Loop
#[tokio::test]
async fn m5_24_screen_bottleneck_driven_autonomous_tuning_closed_loop() {
    let (host, visitor, ga, gb) = pair_view().await;
    let host_ui = host.spawn_ui().await.unwrap();
    let visitor_ui = visitor.spawn_ui().await.unwrap();
    let host_id = host.identity().device_id().clone();
    let visitor_id = visitor.identity().device_id().clone();

    wait_screen_port(&host, &visitor_id, Duration::from_secs(3)).await;
    wait_screen_port(&visitor, &host_id, Duration::from_secs(3)).await;

    // 1. Initial tuning report on host: standard mode, no actions applied yet
    let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/tuning")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let init_rep: tetherly_core::ScreenTuningReport = serde_json::from_slice(&b).unwrap();
    assert!(init_rep.ok);
    assert_eq!(init_rep.mode, "standard");
    assert_eq!(init_rep.actions_applied, 0);
    assert_eq!(init_rep.recoveries_applied, 0);
    assert_eq!(init_rep.last_action, "hold");

    // 2. Visitor configures the tuner to aggressive before any session exists
    let mode_req = serde_json::json!({ "mode": "aggressive" });
    let (c, b) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/tuning",
        &serde_json::to_vec(&mode_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let set_rep: tetherly_core::ScreenTuningReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(set_rep.mode, "aggressive");

    // Host is still on the default: without a session there is no control channel
    let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/tuning")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let host_rep: tetherly_core::ScreenTuningReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(host_rep.mode, "standard");

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

    // 4. With the live control channel up, re-set the mode on the visitor; the
    //    ControlMsg::SetTuningMode signal must propagate and flip the host sender's tuner.
    let (c, _) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/tuning",
        &serde_json::to_vec(&serde_json::json!({ "mode": "aggressive" })).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);

    let deadline = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/tuning")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let rep: tetherly_core::ScreenTuningReport = serde_json::from_slice(&b).unwrap();
        if rep.mode == "aggressive" {
            break;
        }
        if deadline.elapsed() > Duration::from_secs(3) {
            panic!("SetTuningMode control signal never reached the host sender");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 5. Stream video frames from host while the visitor receives them
    for _ in 0..5 {
        tokio::time::sleep(Duration::from_millis(40)).await;
        let sent = host.screen_send_frame().await.unwrap();
        assert!(sent, "frame should queue while streaming");
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

    // 6. ScreenStatusReport carries autonomous tuning telemetry on both ends
    let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let host_status: tetherly_node::ScreenStatusReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(host_status.tuning_mode, "aggressive");
    assert!(!host_status.tuning_last_stage.is_empty());
    assert!(!host_status.tuning_last_action.is_empty());

    let (c, b) = tetherly_node::loopback_get(visitor_ui, "/api/screen")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let visitor_status: tetherly_node::ScreenStatusReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(visitor_status.tuning_mode, "aggressive");

    // 7. POST /api/screen/tuning/reset clears counters but preserves the mode
    let (c, b) = tetherly_node::loopback_post(host_ui, "/api/screen/tuning/reset", &[])
        .await
        .unwrap();
    assert_eq!(c, 200);
    let reset_rep: tetherly_core::ScreenTuningReport = serde_json::from_slice(&b).unwrap();
    assert!(reset_rep.ok);
    assert_eq!(reset_rep.mode, "aggressive");
    assert_eq!(reset_rep.actions_applied, 0);
    assert_eq!(reset_rep.last_action, "hold");
    assert!(reset_rep.last_decision.is_none());

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

// ADR-101 Screen Streaming Session Recording Container & Replay Pipeline
#[tokio::test]
async fn m5_25_screen_session_recording_container_and_replay_pipeline() {
    let (host, visitor, ga, gb) = pair_view().await;
    let host_ui = host.spawn_ui().await.unwrap();
    let visitor_ui = visitor.spawn_ui().await.unwrap();
    let host_id = host.identity().device_id().clone();
    let visitor_id = visitor.identity().device_id().clone();

    wait_screen_port(&host, &visitor_id, Duration::from_secs(3)).await;
    wait_screen_port(&visitor, &host_id, Duration::from_secs(3)).await;

    // 1. Initial recording status on both ends: idle, no peer disclosure
    for ui in [host_ui, visitor_ui] {
        let (c, b) = tetherly_node::loopback_get(ui, "/api/screen/recording")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let rep: tetherly_node::ScreenRecordingReport = serde_json::from_slice(&b).unwrap();
        assert!(rep.ok);
        assert!(!rep.active);
        assert!(!rep.peer_recording);
    }

    // 2. Visitor opens screen channel, host allows, visitor starts streaming
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

    // 3. Visitor starts recording with an explicit container path; the
    //    ControlMsg::SetRecordingState disclosure must reach the host sender.
    let out_dir = std::env::temp_dir().join(format!(
        "tetherly_m5_25_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis()
    ));
    std::fs::create_dir_all(&out_dir).unwrap();
    let container = out_dir.join("session.trec");
    let rec_req = serde_json::json!({ "path": container.display().to_string() });
    let (c, b) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/recording/start",
        &serde_json::to_vec(&rec_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let rec_rep: tetherly_node::ScreenRecordingReport = serde_json::from_slice(&b).unwrap();
    assert!(rec_rep.active);

    // Host must learn about the recording via the disclosure signaling
    let deadline = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/recording")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let rep: tetherly_node::ScreenRecordingReport = serde_json::from_slice(&b).unwrap();
        if rep.peer_recording {
            break;
        }
        if deadline.elapsed() > Duration::from_secs(3) {
            panic!("SetRecordingState disclosure never reached the host");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 4. Stream video frames; the visitor records every presented frame
    for _ in 0..6 {
        tokio::time::sleep(Duration::from_millis(40)).await;
        let sent = host.screen_send_frame().await.unwrap();
        assert!(sent, "frame should queue while streaming");
    }

    let wait_frame = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() >= 4 {
            break;
        }
        if wait_frame.elapsed() > Duration::from_secs(5) {
            panic!("visitor never received frames");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    tokio::time::sleep(Duration::from_millis(200)).await;

    // 5. Visitor's recorder must show recorded frames
    let (c, b) = tetherly_node::loopback_get(visitor_ui, "/api/screen/recording")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let rec_rep: tetherly_node::ScreenRecordingReport = serde_json::from_slice(&b).unwrap();
    assert!(rec_rep.active);
    assert!(
        rec_rep.frames_recorded >= 3,
        "recorder must have captured the presented frames, got {}",
        rec_rep.frames_recorded
    );
    assert!(rec_rep.last_frame_seq >= 1);

    // 6. ScreenStatusReport carries recording telemetry
    let (c, b) = tetherly_node::loopback_get(visitor_ui, "/api/screen")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let vs: tetherly_node::ScreenStatusReport = serde_json::from_slice(&b).unwrap();
    assert!(vs.recording_active);
    assert!(vs.recording_frames >= 3);

    // 7. Stop recording and inspect the container via the replay endpoint
    let (c, b) = tetherly_node::loopback_post(visitor_ui, "/api/screen/recording/stop", &[])
        .await
        .unwrap();
    assert_eq!(c, 200);
    let stopped: tetherly_node::ScreenRecordingReport = serde_json::from_slice(&b).unwrap();
    assert!(!stopped.active);
    assert!(container.exists(), "TREC container must exist on disk");

    let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/recording")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let host_rep: tetherly_node::ScreenRecordingReport = serde_json::from_slice(&b).unwrap();
    assert!(
        !host_rep.peer_recording,
        "stop disclosure must clear host flag"
    );

    let replay_q = format!(
        "path={}&limit=32",
        urlencode_query(&container.display().to_string())
    );
    let (c, b) = tetherly_node::loopback_get(
        visitor_ui,
        &format!("/api/screen/recording/replay?{replay_q}"),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let replay: tetherly_node::ScreenRecordingReplayReport = serde_json::from_slice(&b).unwrap();
    assert!(replay.ok);
    assert!(replay.width > 0);
    assert!(replay.height > 0);
    assert!(
        replay.records_returned as u64 >= 3,
        "replay must read back the recorded frames, got {}",
        replay.records_returned
    );
    assert!(!replay.truncated);
    assert_eq!(replay.records[0].seq, 1);

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
    let _ = std::fs::remove_dir_all(&out_dir);
}

#[tokio::test]
async fn m5_26_screen_session_playback_engine_and_virtual_source_feeder() {
    use tetherly_core::ScreenSource;

    let (host, visitor, ga, gb) = pair_view().await;
    let visitor_ui = visitor.spawn_ui().await.unwrap();

    let out_dir = std::env::temp_dir().join(format!(
        "tetherly_m5_26_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis()
    ));
    std::fs::create_dir_all(&out_dir).unwrap();
    let container = out_dir.join("playback_sample.trec");

    // 1. Synthesize a valid 10-frame .trec container
    let mut buf = Vec::new();
    let frame0 = tetherly_core::ScreenFrame {
        seq: 0,
        rect: tetherly_core::ScreenRect {
            x: 0,
            y: 0,
            w: 64,
            h: 48,
        },
        format: tetherly_core::PixelFormat::Bgra8,
        codec_tag: tetherly_core::SCREEN_COMPRESS_NONE,
        stride: 64 * 4,
        bytes: vec![0x42; 64 * 48 * 4],
    };
    let meta = tetherly_core::ScreenRecordingMeta::from_frame(&frame0, 1000);
    let mut writer =
        tetherly_core::ScreenRecordingWriter::new(meta, std::io::Cursor::new(&mut buf)).unwrap();
    for i in 0..10 {
        let frame = tetherly_core::ScreenFrame {
            seq: i,
            rect: tetherly_core::ScreenRect {
                x: 0,
                y: 0,
                w: 64,
                h: 48,
            },
            format: tetherly_core::PixelFormat::Bgra8,
            codec_tag: tetherly_core::SCREEN_COMPRESS_NONE,
            stride: 64 * 4,
            bytes: vec![(i as u8) * 10; 64 * 48 * 4],
        };
        writer.append(&frame, 1000 + i * 16666).unwrap();
    }
    writer.finish().unwrap();
    std::fs::write(&container, &buf).unwrap();

    // 2. REST API: GET /api/screen/playback default report (idle)
    let (c, b) = tetherly_node::loopback_get(visitor_ui, "/api/screen/playback")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let pb_rep: tetherly_core::ScreenPlaybackReport = serde_json::from_slice(&b).unwrap();
    assert!(pb_rep.ok);
    assert_eq!(pb_rep.state, "idle");
    assert_eq!(pb_rep.current_frame, 0);

    // 3. REST API: POST /api/screen/playback/start with looping test mode
    let start_req = serde_json::json!({
        "path": container.display().to_string(),
        "speed": 0.0,
        "loop": true
    });
    let (c, b) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/playback/start",
        &serde_json::to_vec(&start_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let pb_rep: tetherly_core::ScreenPlaybackReport = serde_json::from_slice(&b).unwrap();
    assert!(pb_rep.ok);
    assert_eq!(pb_rep.total_frames, 10);
    assert_eq!(pb_rep.width, 64);
    assert_eq!(pb_rep.height, 48);

    // Give background playback task time to step frames
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Verify frames presented to screen sink
    assert!(
        visitor.screen_sink_snapshot().count() >= 5,
        "playback engine must feed frames to screen sink, got {}",
        visitor.screen_sink_snapshot().count()
    );

    // 4. REST API: GET /api/screen carries playback telemetry
    let (c, b) = tetherly_node::loopback_get(visitor_ui, "/api/screen")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let s_rep: tetherly_node::ScreenStatusReport = serde_json::from_slice(&b).unwrap();
    assert!(s_rep.playback_active);
    assert_eq!(s_rep.playback_state, "playing");

    // 5. REST API: Pause & Resume
    let (c, b) = tetherly_node::loopback_post(visitor_ui, "/api/screen/playback/pause", &[])
        .await
        .unwrap();
    assert_eq!(c, 200);
    let pb_rep: tetherly_core::ScreenPlaybackReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(pb_rep.state, "paused");

    let (c, b) = tetherly_node::loopback_post(visitor_ui, "/api/screen/playback/resume", &[])
        .await
        .unwrap();
    assert_eq!(c, 200);
    let pb_rep: tetherly_core::ScreenPlaybackReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(pb_rep.state, "playing");

    // 6. REST API: Stop playback
    let (c, b) = tetherly_node::loopback_post(visitor_ui, "/api/screen/playback/stop", &[])
        .await
        .unwrap();
    assert_eq!(c, 200);
    let pb_rep: tetherly_core::ScreenPlaybackReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(pb_rep.state, "stopped");

    let (c, b) = tetherly_node::loopback_get(visitor_ui, "/api/screen/playback")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let pb_rep: tetherly_core::ScreenPlaybackReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(pb_rep.state, "stopped");

    // 7. Virtual Screen Source Feeder (ScreenRecordingSource)
    let virtual_source = tetherly_core::ScreenRecordingSource::from_file(&container, true).unwrap();
    let displays = virtual_source.displays();
    assert_eq!(displays.len(), 1);
    assert_eq!(displays[0].title, "Virtual Recorded Display");
    assert_eq!(displays[0].width, 64);
    assert_eq!(displays[0].height, 48);

    assert_eq!(virtual_source.fps(), 30);
    virtual_source.set_fps(60);
    assert_eq!(virtual_source.fps(), 60);

    // Grab 15 frames (surpassing total 10 to verify seamless loop rewinding)
    for expected_seq in 0..15 {
        let frame = virtual_source.grab().unwrap().expect("frame grabbed");
        assert_eq!(frame.rect.w, 64);
        assert_eq!(frame.rect.h, 48);
        assert_eq!(frame.seq, (expected_seq % 10) as u64);
    }
    assert_eq!(virtual_source.frames_grabbed(), 15);

    // Non-looping virtual source reaches EOF and finishes gracefully
    let finite_source = tetherly_core::ScreenRecordingSource::from_file(&container, false).unwrap();
    for _ in 0..10 {
        assert!(finite_source.grab().unwrap().is_some());
    }
    // 11th grab on non-loop source returns None
    assert!(finite_source.grab().unwrap().is_none());

    host.shutdown();
    visitor.shutdown();
    drop(ga);
    drop(gb);
    let _ = std::fs::remove_dir_all(&out_dir);
}

#[tokio::test]
async fn m5_27_screen_live_snapshot_and_recording_frame_extraction() {
    let (host, visitor, ga, gb) = pair_view().await;
    let host_ui = host.spawn_ui().await.unwrap();
    let visitor_ui = visitor.spawn_ui().await.unwrap();
    let host_id = host.identity().device_id().clone();
    let visitor_id = visitor.identity().device_id().clone();

    wait_screen_port(&host, &visitor_id, Duration::from_secs(3)).await;
    wait_screen_port(&visitor, &host_id, Duration::from_secs(3)).await;

    // 1. Establish screen streaming session
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

    // Host sends 1 frame (MemoryScreenSource 16x16)
    assert!(host.screen_send_frame().await.unwrap());

    // Wait for frame to reach visitor screen sink
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

    // 3. Visitor live snapshot in BMP format
    let (c, bmp_bytes) = tetherly_node::loopback_get(visitor_ui, "/api/screen/snapshot?format=bmp")
        .await
        .unwrap();
    assert_eq!(c, 200);
    assert_eq!(&bmp_bytes[0..2], b"BM");
    assert_eq!(bmp_bytes.len(), 54 + (16 * 16 * 4));
    let bi_w = i32::from_le_bytes(bmp_bytes[18..22].try_into().unwrap());
    let bi_h = i32::from_le_bytes(bmp_bytes[22..26].try_into().unwrap());
    assert_eq!(bi_w, 16);
    assert_eq!(bi_h, -16); // Top-Down DIB

    // 4. Visitor live snapshot in JSON format
    let (c, b) = tetherly_node::loopback_get(visitor_ui, "/api/screen/snapshot?format=json")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let snap_rep: tetherly_core::ScreenSnapshotReport = serde_json::from_slice(&b).unwrap();
    assert!(snap_rep.ok);
    assert_eq!(snap_rep.source, "live");
    assert_eq!(snap_rep.width, 16);
    assert_eq!(snap_rep.height, 16);
    assert_eq!(snap_rep.format, "bmp");
    assert_eq!(snap_rep.bytes_len, 54 + (16 * 16 * 4));
    assert!(!snap_rep.privacy_blanked);
    assert!(snap_rep.data_base64.is_some());

    // 5. Visitor live snapshot with disk output
    let temp_snap_dir = std::env::temp_dir().join(format!(
        "tetherly_m5_27_snap_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&temp_snap_dir).unwrap();
    let out_file = temp_snap_dir.join("live_screen.bmp");
    let q_out = format!(
        "/api/screen/snapshot?out={}",
        urlencode_query(out_file.to_str().unwrap())
    );
    let (c, b) = tetherly_node::loopback_get(visitor_ui, &q_out)
        .await
        .unwrap();
    assert_eq!(c, 200);
    let rep: tetherly_core::ScreenSnapshotReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(rep.path.as_deref(), out_file.to_str());
    assert!(out_file.exists());
    let disk_bytes = std::fs::read(&out_file).unwrap();
    assert_eq!(&disk_bytes[0..2], b"BM");
    assert_eq!(disk_bytes.len(), 54 + (16 * 16 * 4));

    // 6. Security guard: directory traversal in `out` is rejected with 400
    let (c, b) = tetherly_node::loopback_get(visitor_ui, "/api/screen/snapshot?out=../escape.bmp")
        .await
        .unwrap();
    assert_eq!(c, 400);
    assert!(String::from_utf8_lossy(&b)
        .to_lowercase()
        .contains("unsafe"));

    // 7. Privacy mode test on host (ADR-098 & ADR-103)
    host.screen_set_privacy_mode(true, false).await.unwrap();
    let (c, host_bmp) = tetherly_node::loopback_get(host_ui, "/api/screen/snapshot?format=bmp")
        .await
        .unwrap();
    assert_eq!(c, 200);
    assert_eq!(&host_bmp[0..2], b"BM");
    // All pixels blanked to 0
    for &b in &host_bmp[54..] {
        assert_eq!(b, 0, "privacy mode must blank all pixels");
    }
    let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/snapshot?format=json")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let host_rep: tetherly_core::ScreenSnapshotReport = serde_json::from_slice(&b).unwrap();
    assert!(host_rep.privacy_blanked);
    host.screen_set_privacy_mode(false, false).await.unwrap();

    // 8. Recording container single-frame extraction (ADR-101 & ADR-103)
    let container_file = temp_snap_dir.join("session_rec.trec");
    {
        let f0 = tetherly_core::ScreenFrame {
            seq: 0,
            rect: tetherly_core::ScreenRect {
                x: 0,
                y: 0,
                w: 32,
                h: 24,
            },
            format: tetherly_core::PixelFormat::Bgra8,
            codec_tag: tetherly_core::SCREEN_COMPRESS_NONE,
            stride: 32 * 4,
            bytes: vec![0x33; 32 * 24 * 4],
        };
        let f1 = tetherly_core::ScreenFrame {
            seq: 1,
            rect: tetherly_core::ScreenRect {
                x: 0,
                y: 0,
                w: 32,
                h: 24,
            },
            format: tetherly_core::PixelFormat::Bgra8,
            codec_tag: tetherly_core::SCREEN_COMPRESS_NONE,
            stride: 32 * 4,
            bytes: vec![0x88; 32 * 24 * 4],
        };
        let meta = tetherly_core::ScreenRecordingMeta::from_frame(&f0, 2);
        let file = std::fs::File::create(&container_file).unwrap();
        let mut writer = tetherly_core::ScreenRecordingWriter::new(meta, file).unwrap();
        writer.append(&f0, 50_000).unwrap();
        writer.append(&f1, 150_000).unwrap();
        writer.finish().unwrap();
    }

    // Extract frame 1 via POST /api/screen/recording/snapshot
    let rec_req = serde_json::json!({
        "path": container_file.to_str().unwrap(),
        "frame": 1,
        "format": "json"
    });
    let (c, b) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/recording/snapshot",
        &serde_json::to_vec(&rec_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let rec_rep: tetherly_core::ScreenSnapshotReport = serde_json::from_slice(&b).unwrap();
    assert!(rec_rep.ok);
    assert_eq!(rec_rep.source, "recording");
    assert_eq!(rec_rep.frame_seq, 1);
    assert_eq!(rec_rep.pts_us, 150_000);
    assert_eq!(rec_rep.width, 32);
    assert_eq!(rec_rep.height, 24);
    assert_eq!(rec_rep.bytes_len, 54 + (32 * 24 * 4));

    // Extract frame 0 as BMP binary stream and verify pixel values
    let rec_bmp_req = serde_json::json!({
        "path": container_file.to_str().unwrap(),
        "pts": 50_000,
        "format": "bmp"
    });
    let (c, b) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/recording/snapshot",
        &serde_json::to_vec(&rec_bmp_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    assert_eq!(&b[0..2], b"BM");
    assert_eq!(b.len(), 54 + (32 * 24 * 4));
    for &pixel_byte in &b[54..] {
        assert_eq!(pixel_byte, 0x33);
    }

    // Extract with traversal rejected
    let evil_rec = serde_json::json!({
        "path": "../not_allowed.trec",
        "frame": 0
    });
    let (c, _) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/recording/snapshot",
        &serde_json::to_vec(&evil_rec).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 400);

    // Verify snapshot telemetry on status endpoint
    let (c, b) = tetherly_node::loopback_get(visitor_ui, "/api/screen")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let st: tetherly_node::ScreenStatusReport = serde_json::from_slice(&b).unwrap();
    assert!(st.snapshot_count >= 3);

    host.shutdown();
    visitor.shutdown();
    drop(ga);
    drop(gb);
    let _ = std::fs::remove_dir_all(&temp_snap_dir);
}

fn urlencode_query(p: &str) -> String {
    let mut out = String::with_capacity(p.len());
    for b in p.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'/' | b'~' => {
                out.push(b as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

#[tokio::test]
async fn m5_28_screen_dynamic_security_watermarking_and_forensic_overlay() {
    let (host, visitor, ga, gb) = pair_view().await;
    let host_ui = host.spawn_ui().await.unwrap();
    let visitor_ui = visitor.spawn_ui().await.unwrap();
    let host_id = host.identity().device_id().clone();
    let visitor_id = visitor.identity().device_id().clone();

    wait_screen_port(&host, &visitor_id, Duration::from_secs(3)).await;
    wait_screen_port(&visitor, &host_id, Duration::from_secs(3)).await;

    // 1. Establish screen streaming session
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

    // Host sends 1 frame (MemoryScreenSource 16x16)
    assert!(host.screen_send_frame().await.unwrap());

    // Wait for frame to reach visitor screen sink
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

    // 2. Initial live snapshot before watermark: watermarked must be false
    let (c, b) = tetherly_node::loopback_get(visitor_ui, "/api/screen/snapshot?format=json")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let initial_snap: tetherly_core::ScreenSnapshotReport = serde_json::from_slice(&b).unwrap();
    assert!(initial_snap.ok);
    assert!(!initial_snap.watermarked);

    // 3. Host enables host-enforced watermark overlay via POST /api/screen/watermark
    let wm_cfg = serde_json::json!({
        "enabled": true,
        "host_enforced": true,
        "text": "CONFIDENTIAL-{peer}",
        "opacity": 200,
        "color": "#00FF00",
        "pattern": "grid",
        "scale": 1,
        "spacing_x": 40,
        "spacing_y": 30
    });
    let (c, b) = tetherly_node::loopback_post(
        host_ui,
        "/api/screen/watermark",
        &serde_json::to_vec(&wm_cfg).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let host_wm_rep: tetherly_core::ScreenWatermarkReport = serde_json::from_slice(&b).unwrap();
    assert!(host_wm_rep.ok);
    assert!(host_wm_rep.enabled);
    assert!(host_wm_rep.host_enforced);
    assert_eq!(host_wm_rep.color_hex, "#00FF00");
    assert_eq!(host_wm_rep.pattern, "grid");

    // 4. Wait for line protocol Tag 10 (ControlMsg::SetWatermarkState) to propagate to visitor
    let wait_sync = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(visitor_ui, "/api/screen/watermark")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let vis_rep: tetherly_core::ScreenWatermarkReport = serde_json::from_slice(&b).unwrap();
        if vis_rep.enabled && vis_rep.host_enforced {
            break;
        }
        if wait_sync.elapsed() > Duration::from_secs(5) {
            panic!("visitor never received host_enforced watermark state via line protocol Tag 10");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 5. Host sovereignty guard (ADR-004 / ADR-104):
    // Visitor tries to disable the watermark or clear host_enforced, expecting HTTP 403 Forbidden!
    let (c, b) = tetherly_node::loopback_post(visitor_ui, "/api/screen/watermark/disable", b"{}")
        .await
        .unwrap();
    assert_eq!(
        c, 403,
        "visitor must not be able to disable host-enforced watermark"
    );
    assert!(String::from_utf8_lossy(&b).contains("enforced by host"));

    let try_disable_cfg = serde_json::json!({
        "enabled": false
    });
    let (c, b) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/watermark",
        &serde_json::to_vec(&try_disable_cfg).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 403);
    assert!(String::from_utf8_lossy(&b).contains("enforced by host"));

    // 6. Visitor takes live snapshot while watermark is enforced: watermarked == true
    let (c, b) = tetherly_node::loopback_get(visitor_ui, "/api/screen/snapshot?format=json")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let wm_snap: tetherly_core::ScreenSnapshotReport = serde_json::from_slice(&b).unwrap();
    assert!(wm_snap.ok);
    assert!(
        wm_snap.watermarked,
        "live snapshot must reflect active watermark overlay"
    );

    // Also verify binary BMP output contains watermark
    let (c, wm_bmp) = tetherly_node::loopback_get(visitor_ui, "/api/screen/snapshot?format=bmp")
        .await
        .unwrap();
    assert_eq!(c, 200);
    assert_eq!(&wm_bmp[0..2], b"BM");

    // 7. Telemetry verification on status endpoint
    let (c, b) = tetherly_node::loopback_get(visitor_ui, "/api/screen")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let st: tetherly_node::ScreenStatusReport = serde_json::from_slice(&b).unwrap();
    assert!(st.watermark_enabled);
    assert!(st.watermark_enforced);

    // 8. Host relaxes lockdown: disables watermark
    let relax_cfg = serde_json::json!({
        "enabled": false,
        "host_enforced": false
    });
    let (c, _) = tetherly_node::loopback_post(
        host_ui,
        "/api/screen/watermark",
        &serde_json::to_vec(&relax_cfg).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);

    // Wait for visitor to receive relaxed state
    let wait_relax = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(visitor_ui, "/api/screen/watermark")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let vis_rep: tetherly_core::ScreenWatermarkReport = serde_json::from_slice(&b).unwrap();
        if !vis_rep.enabled && !vis_rep.host_enforced {
            break;
        }
        if wait_relax.elapsed() > Duration::from_secs(5) {
            panic!("visitor never received relaxed watermark state");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 9. Now visitor is free to enable and disable client-side watermark
    let (c, _) = tetherly_node::loopback_post(visitor_ui, "/api/screen/watermark/enable", b"{}")
        .await
        .unwrap();
    assert_eq!(c, 200);

    let (c, _) = tetherly_node::loopback_post(visitor_ui, "/api/screen/watermark/disable", b"{}")
        .await
        .unwrap();
    assert_eq!(c, 200);

    // 10. Clean shutdown
    host.shutdown();
    visitor.shutdown();
    drop(ga);
    drop(gb);
}

#[tokio::test]
async fn m5_29_screen_collaborative_annotation_and_laser_pointer_pipeline() {
    let (host, visitor, ga, gb) = pair_view().await;
    let host_ui = host.spawn_ui().await.unwrap();
    let visitor_ui = visitor.spawn_ui().await.unwrap();
    let host_id = host.identity().device_id().clone();
    let visitor_id = visitor.identity().device_id().clone();

    wait_screen_port(&host, &visitor_id, Duration::from_secs(3)).await;
    wait_screen_port(&visitor, &host_id, Duration::from_secs(3)).await;

    // 1. Establish screen streaming session
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

    // Host sends 1 frame (MemoryScreenSource 16x16)
    assert!(host.screen_send_frame().await.unwrap());

    // Wait for frame to reach visitor screen sink
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

    // 2. Collaborative Transient Laser Pointer Flow
    // Initial laser status: off
    let (c, b) = tetherly_node::loopback_get(visitor_ui, "/api/screen/laser")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let l_rep: serde_json::Value = serde_json::from_slice(&b).unwrap();
    assert_eq!(l_rep["ok"], true);
    assert_eq!(l_rep["laser_active"], false);

    // Visitor activates laser pointer at (12, 14) with color #00FF00 and radius 8
    let laser_req = serde_json::json!({
        "x": 12,
        "y": 14,
        "active": true,
        "color": "#00FF00",
        "radius": 8
    });
    let (c, b) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/laser",
        &serde_json::to_vec(&laser_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let l_rep: serde_json::Value = serde_json::from_slice(&b).unwrap();
    assert_eq!(l_rep["ok"], true);
    assert_eq!(l_rep["laser_active"], true);
    assert_eq!(l_rep["laser"]["x"], 12);
    assert_eq!(l_rep["laser"]["y"], 14);

    // Host should also receive Tag 11 control message
    let wait_laser = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/laser")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let hl_rep: serde_json::Value = serde_json::from_slice(&b).unwrap();
        if hl_rep["laser_active"].as_bool().unwrap_or(false) {
            assert_eq!(hl_rep["laser"]["x"], 12);
            assert_eq!(hl_rep["laser"]["y"], 14);
            break;
        }
        if wait_laser.elapsed() > Duration::from_secs(5) {
            panic!("host never received Tag 11 laser pointer message");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // Deactivate laser pointer
    let laser_off = serde_json::json!({ "active": false });
    let (c, b) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/laser",
        &serde_json::to_vec(&laser_off).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let l_rep: serde_json::Value = serde_json::from_slice(&b).unwrap();
    assert_eq!(l_rep["laser_active"], false);

    // 3. Collaborative Whiteboard Annotations Flow
    // Add rect annotation
    let rect_ann = serde_json::json!({
        "kind": "rect",
        "x": 2,
        "y": 2,
        "w": 10,
        "h": 8,
        "color": "#FF0000",
        "thickness": 1
    });
    let (c, b) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/annotation",
        &serde_json::to_vec(&rect_ann).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let ann_res: serde_json::Value = serde_json::from_slice(&b).unwrap();
    assert_eq!(ann_res["ok"], true);
    assert!(ann_res["id"].as_u64().unwrap_or(0) > 0);

    // Add stroke annotation
    let stroke_ann = serde_json::json!({
        "kind": "stroke",
        "points_str": "1,1;5,5;10,5",
        "color": "#0000FF",
        "thickness": 2
    });
    let (c, _) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/annotation",
        &serde_json::to_vec(&stroke_ann).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);

    // Add arrow annotation
    let arrow_ann = serde_json::json!({
        "kind": "arrow",
        "from_str": "0,0",
        "to_str": "12,12",
        "color": "#FFFF00"
    });
    let (c, _) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/annotation",
        &serde_json::to_vec(&arrow_ann).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);

    // Add text annotation
    let text_ann = serde_json::json!({
        "kind": "text",
        "text": "HI",
        "x": 4,
        "y": 4,
        "scale": 1,
        "color": "#FFFFFF"
    });
    let (c, _) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/annotation",
        &serde_json::to_vec(&text_ann).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);

    // Add temporary TTL annotation with 100ms TTL
    let temp_ann = serde_json::json!({
        "kind": "text",
        "text": "TMP",
        "x": 0,
        "y": 0,
        "scale": 1,
        "ttl_ms": 100
    });
    let (c, _) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/annotation",
        &serde_json::to_vec(&temp_ann).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);

    // Verify all 5 annotations are active
    let (c, b) = tetherly_node::loopback_get(visitor_ui, "/api/screen/annotation")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let rep: tetherly_core::ScreenAnnotationReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(rep.annotations_count, 5);
    assert_eq!(rep.total_annotations_added, 5);

    // Sleep for 150ms to allow the temporary annotation to expire
    tokio::time::sleep(Duration::from_millis(150)).await;

    // Send a frame to trigger apply_annotations with expired item cleanup
    assert!(host.screen_send_frame().await.unwrap());
    tokio::time::sleep(PACER_SETTLE).await;

    // Verify TTL annotation expired
    let (c, b) = tetherly_node::loopback_get(visitor_ui, "/api/screen/annotation")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let rep: tetherly_core::ScreenAnnotationReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(rep.annotations_count, 4);

    // 4. Whiteboard clear flow: POST /api/screen/annotation/clear
    let (c, b) = tetherly_node::loopback_post(visitor_ui, "/api/screen/annotation/clear", b"{}")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let rep: tetherly_core::ScreenAnnotationReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(rep.annotations_count, 0);

    // 5. Verify telemetry via GET /api/screen
    let (c, b) = tetherly_node::loopback_get(visitor_ui, "/api/screen")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let s: tetherly_node::ScreenStatusReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(s.annotations_count, 0);
    assert!(s.annotations_total >= 5);
    assert!(s.laser_updates_total >= 2);

    // 6. Clean shutdown
    host.shutdown();
    visitor.shutdown();
    drop(ga);
    drop(gb);
}

#[tokio::test]
async fn m5_30_screen_whiteboard_multi_user_sync_and_undo_redo_pipeline() {
    let (host, visitor, ga, gb) = pair_view().await;
    let host_ui = host.spawn_ui().await.unwrap();
    let visitor_ui = visitor.spawn_ui().await.unwrap();
    let host_id = host.identity().device_id().clone();
    let visitor_id = visitor.identity().device_id().clone();

    wait_screen_port(&host, &visitor_id, Duration::from_secs(3)).await;
    wait_screen_port(&visitor, &host_id, Duration::from_secs(3)).await;

    // 1. Establish screen streaming session
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

    // Host sends 1 frame (MemoryScreenSource 16x16)
    assert!(host.screen_send_frame().await.unwrap());

    // Wait for frame to reach visitor screen sink
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

    // 2. Whiteboard Policy Query
    let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/whiteboard/policy")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let pol_rep: serde_json::Value = serde_json::from_slice(&b).unwrap();
    assert_eq!(pol_rep["ok"], true);
    assert_eq!(pol_rep["policy"]["read_only"], false);
    assert_eq!(pol_rep["policy"]["allow_peer_clear"], true);
    assert_eq!(pol_rep["policy"]["max_items"], 1000);

    // 3. Multi-Author Drawing: Alice and Bob draw distinct annotations
    let stroke_alice = serde_json::json!({
        "kind": "stroke",
        "author": "alice",
        "points_str": "1,1;4,4;8,8",
        "color": "#FF0000",
        "thickness": 2
    });
    let (c, b) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/annotation",
        &serde_json::to_vec(&stroke_alice).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let res: serde_json::Value = serde_json::from_slice(&b).unwrap();
    assert_eq!(res["ok"], true);
    let alice_id = res["id"].as_u64().unwrap();

    let rect_bob = serde_json::json!({
        "kind": "rect",
        "author": "bob",
        "x": 2,
        "y": 2,
        "w": 6,
        "h": 6,
        "color": "#00FF00",
        "thickness": 1
    });
    let (c, b) = tetherly_node::loopback_post(
        host_ui,
        "/api/screen/annotation",
        &serde_json::to_vec(&rect_bob).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let res: serde_json::Value = serde_json::from_slice(&b).unwrap();
    assert_eq!(res["ok"], true);
    let bob_id = res["id"].as_u64().unwrap();

    let text_alice2 = serde_json::json!({
        "kind": "text",
        "author": "alice",
        "text": "ALICE2",
        "x": 0,
        "y": 0,
        "scale": 1,
        "color": "#FFFFFF"
    });
    let (c, _) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/annotation",
        &serde_json::to_vec(&text_alice2).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);

    // Verify 3 active items on visitor
    let (c, b) = tetherly_node::loopback_get(visitor_ui, "/api/screen/annotation")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let rep: tetherly_core::ScreenAnnotationReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(rep.annotations_count, 3);
    assert_eq!(rep.redo_count, 0);

    // 4. Per-Author Undo Isolation:
    // Visitor requests undo specifically for "alice".
    // This should undo "ALICE2", leaving Bob's rect and Alice's first stroke active.
    let (c, b) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/whiteboard/undo?author=alice",
        b"{}",
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let undo_rep: serde_json::Value = serde_json::from_slice(&b).unwrap();
    assert_eq!(undo_rep["ok"], true);
    assert_eq!(undo_rep["undone"]["author"], "alice");
    assert_eq!(undo_rep["report"]["annotations_count"], 2);
    assert_eq!(undo_rep["report"]["redo_count"], 1);

    // Verify Bob's item (#bob_id) is still active in the annotations list
    let annotations = undo_rep["report"]["annotations"].as_array().unwrap();
    assert!(annotations.iter().any(|it| it["id"] == bob_id));
    assert!(annotations.iter().any(|it| it["id"] == alice_id));

    // 5. Per-Author Redo:
    // Redo Alice's annotation -> ALICE2 is restored
    let (c, b) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/whiteboard/redo?author=alice",
        b"{}",
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let redo_rep: serde_json::Value = serde_json::from_slice(&b).unwrap();
    assert_eq!(redo_rep["ok"], true);
    assert_eq!(redo_rep["redone"]["author"], "alice");
    assert_eq!(redo_rep["report"]["annotations_count"], 3);
    assert_eq!(redo_rep["report"]["redo_count"], 0);

    // 6. Global Undo:
    // Undo without author parameter undos the globally most recent item (ALICE2)
    let (c, b) = tetherly_node::loopback_post(visitor_ui, "/api/screen/whiteboard/undo", b"{}")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let undo_rep: serde_json::Value = serde_json::from_slice(&b).unwrap();
    assert_eq!(undo_rep["report"]["annotations_count"], 2);

    // 7. Vector Snapshot Export and Import Pipeline
    let (c, b) = tetherly_node::loopback_get(visitor_ui, "/api/screen/whiteboard/export")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let export_rep: serde_json::Value = serde_json::from_slice(&b).unwrap();
    assert_eq!(export_rep["ok"], true);
    let snap = export_rep["export"].clone();
    assert_eq!(snap["active_items"].as_array().unwrap().len(), 2);

    // Clear canvas
    let (c, _) = tetherly_node::loopback_post(visitor_ui, "/api/screen/whiteboard/clear", b"{}")
        .await
        .unwrap();
    assert_eq!(c, 200);

    let (c, b) = tetherly_node::loopback_get(visitor_ui, "/api/screen/annotation")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let rep: tetherly_core::ScreenAnnotationReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(rep.annotations_count, 0);

    // Import snapshot back
    let import_body = serde_json::json!({
        "snapshot": snap,
        "replace": true
    });
    let (c, b) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/whiteboard/import",
        &serde_json::to_vec(&import_body).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let imp_rep: serde_json::Value = serde_json::from_slice(&b).unwrap();
    assert_eq!(imp_rep["imported_count"], 2);
    assert_eq!(imp_rep["report"]["annotations_count"], 2);

    // 8. Whiteboard Moderation Policy & Host Sovereignty
    // Host sets policy: read_only = true
    let pol_body = serde_json::json!({
        "read_only": true,
        "allow_peer_clear": false,
        "max_items": 500
    });
    let (c, _) = tetherly_node::loopback_post(
        host_ui,
        "/api/screen/whiteboard/policy",
        &serde_json::to_vec(&pol_body).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);

    // Host attempts to draw while whiteboard is read-only -> 403 Forbidden!
    let blocked_draw = serde_json::json!({
        "kind": "rect",
        "author": "visitor",
        "x": 0,
        "y": 0,
        "w": 5,
        "h": 5
    });
    let (c, _) = tetherly_node::loopback_post(
        host_ui,
        "/api/screen/annotation",
        &serde_json::to_vec(&blocked_draw).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 403);

    // 9. Path Traversal Guard
    let (c, _) = tetherly_node::loopback_get(
        visitor_ui,
        "/api/screen/whiteboard/export?out=../../evil.json",
    )
    .await
    .unwrap();
    assert_eq!(c, 400);

    // 10. Clean shutdown
    host.shutdown();
    visitor.shutdown();
    drop(ga);
    drop(gb);
}

#[tokio::test]
async fn m5_31_screen_dynamic_roi_crop_and_viewport_zoom_pipeline() {
    let (host, visitor, ga, gb) = pair_view().await;
    let host_ui = host.spawn_ui().await.unwrap();
    let visitor_ui = visitor.spawn_ui().await.unwrap();
    let host_id = host.identity().device_id().clone();
    let visitor_id = visitor.identity().device_id().clone();

    wait_screen_port(&host, &visitor_id, Duration::from_secs(3)).await;
    wait_screen_port(&visitor, &host_id, Duration::from_secs(3)).await;

    // 1. Establish screen streaming session
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

    // 2. Initial state: ROI is inactive, full-frame streaming (16x16)
    let (c, b) = tetherly_node::loopback_get(visitor_ui, "/api/screen/roi")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let roi_rep: tetherly_core::ScreenRoiReport = serde_json::from_slice(&b).unwrap();
    assert!(!roi_rep.active);
    assert_eq!(roi_rep.zoom_percent, 100);

    // Host sends 1 frame (MemoryScreenSource 16x16)
    assert!(host.screen_send_frame().await.unwrap());

    // Wait for frame to reach visitor screen sink
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
    let last_frame = visitor.screen_sink_snapshot().last().expect("frame");
    assert_eq!(last_frame.rect.w, 16);
    assert_eq!(last_frame.rect.h, 16);

    // 3. Visitor requests dynamic ROI crop and zoom (x=4, y=4, w=8, h=8, zoom=200)
    let roi_req = serde_json::json!({
        "x": 4,
        "y": 4,
        "w": 8,
        "h": 8,
        "zoom_percent": 200
    });
    let (c, b) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/roi",
        &serde_json::to_vec(&roi_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let visitor_roi_rep: tetherly_core::ScreenRoiReport = serde_json::from_slice(&b).unwrap();
    assert!(visitor_roi_rep.active);
    assert_eq!(visitor_roi_rep.x, 4);
    assert_eq!(visitor_roi_rep.y, 4);
    assert_eq!(visitor_roi_rep.w, 8);
    assert_eq!(visitor_roi_rep.h, 8);
    assert_eq!(visitor_roi_rep.zoom_percent, 200);
    assert_eq!(visitor_roi_rep.effective_w, 8);
    assert_eq!(visitor_roi_rep.effective_h, 8);

    // Verify host receives the ROI signaling via network loop (Tags 16 & 18)
    let wait_host_roi = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/roi")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let host_roi_rep: tetherly_core::ScreenRoiReport = serde_json::from_slice(&b).unwrap();
        if host_roi_rep.active && host_roi_rep.w == 8 && host_roi_rep.h == 8 {
            assert_eq!(host_roi_rep.x, 4);
            assert_eq!(host_roi_rep.y, 4);
            assert_eq!(host_roi_rep.zoom_percent, 200);
            break;
        }
        if wait_host_roi.elapsed() > Duration::from_secs(5) {
            panic!("host never synchronized ROI from visitor signal");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 4. Host sends frame with active ROI -> cropped to 8x8 in pipeline
    tokio::time::sleep(Duration::from_millis(40)).await;
    let prev_count = visitor.screen_sink_snapshot().count();
    assert!(host.screen_send_frame().await.unwrap());

    let wait_roi_frame = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() > prev_count {
            break;
        }
        if wait_roi_frame.elapsed() > Duration::from_secs(5) {
            panic!("visitor never received ROI-cropped frame");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let cropped_frame = visitor
        .screen_sink_snapshot()
        .last()
        .expect("cropped frame");
    assert_eq!(cropped_frame.rect.w, 8);
    assert_eq!(cropped_frame.rect.h, 8);
    assert_eq!(cropped_frame.rect.x, 4);
    assert_eq!(cropped_frame.rect.y, 4);

    // 5. Dynamic Zoom Adjustment via POST /api/screen/roi/zoom
    let zoom_req = serde_json::json!({ "zoom_percent": 300 });
    let (c, b) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/roi/zoom",
        &serde_json::to_vec(&zoom_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let zoom_rep: tetherly_core::ScreenRoiReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(zoom_rep.zoom_percent, 300);

    // 6. Coordinate Mapping Verification (Bidirectional)
    let roi = tetherly_core::ScreenRoi {
        active: true,
        x: 4,
        y: 4,
        w: 8,
        h: 8,
        zoom_percent: 300,
    };
    // (6, 6) is inside ROI -> relative is (2, 2)
    assert_eq!(roi.map_screen_to_roi(6, 6), Some((2, 2)));
    // (2, 2) is outside ROI -> None
    assert_eq!(roi.map_screen_to_roi(2, 2), None);
    // Relative (2, 2) maps back to screen (6, 6)
    assert_eq!(roi.map_roi_to_screen(2, 2), (6, 6));

    // 7. Reset ROI Viewport via POST /api/screen/roi/reset
    let (c, b) = tetherly_node::loopback_post(visitor_ui, "/api/screen/roi/reset", b"{}")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let reset_rep: tetherly_core::ScreenRoiReport = serde_json::from_slice(&b).unwrap();
    assert!(!reset_rep.active);

    // Verify host receives Tag 17 reset signal
    let wait_host_reset = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/roi")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let host_roi_rep: tetherly_core::ScreenRoiReport = serde_json::from_slice(&b).unwrap();
        if !host_roi_rep.active {
            break;
        }
        if wait_host_reset.elapsed() > Duration::from_secs(5) {
            panic!("host never synchronized ROI reset from visitor signal");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // Host sends frame after reset -> restores full 16x16 frame
    tokio::time::sleep(Duration::from_millis(40)).await;
    let prev_count2 = visitor.screen_sink_snapshot().count();
    assert!(host.screen_send_frame().await.unwrap());

    let wait_full_frame = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() > prev_count2 {
            break;
        }
        if wait_full_frame.elapsed() > Duration::from_secs(5) {
            panic!("visitor never received full frame after reset");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let restored_frame = visitor
        .screen_sink_snapshot()
        .last()
        .expect("restored frame");
    assert_eq!(restored_frame.rect.w, 16);
    assert_eq!(restored_frame.rect.h, 16);

    // 8. Telemetry Status Verification (/api/screen)
    let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let s: tetherly_node::ScreenStatusReport = serde_json::from_slice(&b).unwrap();
    assert!(!s.roi_active);
    assert_eq!(s.roi_zoom_percent, 100);
    assert!(s.roi_updates > 0);

    // 9. Clean shutdown
    host.shutdown();
    visitor.shutdown();
    drop(ga);
    drop(gb);
}

#[tokio::test]
async fn m5_32_screen_adaptive_color_depth_and_grayscale_pipeline() {
    let (host, visitor, ga, gb) = pair_view().await;
    let host_ui = host.spawn_ui().await.unwrap();
    let visitor_ui = visitor.spawn_ui().await.unwrap();
    let host_id = host.identity().device_id().clone();
    let visitor_id = visitor.identity().device_id().clone();

    wait_screen_port(&host, &visitor_id, Duration::from_secs(3)).await;
    wait_screen_port(&visitor, &host_id, Duration::from_secs(3)).await;

    // 1. Establish screen streaming session
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

    // 2. Initial state: FullColor32 (inactive transformation, full color)
    let (c, b) = tetherly_node::loopback_get(visitor_ui, "/api/screen/color")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let color_rep: tetherly_core::ScreenColorReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(color_rep.mode, tetherly_core::ScreenColorMode::FullColor32);
    assert_eq!(color_rep.threshold, 128);
    assert!(!color_rep.is_active);

    // Host sends 1 frame (MemoryScreenSource 16x16 with distinct color channels)
    assert!(host.screen_send_frame().await.unwrap());

    // Wait for frame to reach visitor screen sink
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
    let full_color_frame = visitor.screen_sink_snapshot().last().expect("frame");
    assert_eq!(full_color_frame.rect.w, 16);
    assert_eq!(full_color_frame.rect.h, 16);
    // In full color, color channels are distinct
    let mut has_color_variance = false;
    for px in full_color_frame.bytes.chunks_exact(4) {
        if px[0] != px[1] || px[1] != px[2] {
            has_color_variance = true;
            break;
        }
    }
    assert!(has_color_variance, "native frame must have color variance");

    // 3. Visitor requests Grayscale8 mode via POST /api/screen/color
    let gray_req = serde_json::json!({ "mode": "grayscale" });
    let (c, b) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/color",
        &serde_json::to_vec(&gray_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let gray_rep: tetherly_core::ScreenColorReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(gray_rep.mode, tetherly_core::ScreenColorMode::Grayscale8);
    assert!(gray_rep.is_active);

    // Verify host receives Tag 19 and synchronizes to Grayscale8
    let wait_host_gray = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/color")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let host_color_rep: tetherly_core::ScreenColorReport = serde_json::from_slice(&b).unwrap();
        if host_color_rep.mode == tetherly_core::ScreenColorMode::Grayscale8
            && host_color_rep.is_active
        {
            break;
        }
        if wait_host_gray.elapsed() > Duration::from_secs(5) {
            panic!("host never synchronized Grayscale8 color mode from visitor signal");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // Host sends frame with active Grayscale8 mode -> R == G == B on all pixels
    tokio::time::sleep(Duration::from_millis(40)).await;
    let prev_count = visitor.screen_sink_snapshot().count();
    assert!(host.screen_send_frame().await.unwrap());

    let wait_gray_frame = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() > prev_count {
            break;
        }
        if wait_gray_frame.elapsed() > Duration::from_secs(5) {
            panic!("visitor never received Grayscale8 frame");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let gray_frame = visitor.screen_sink_snapshot().last().expect("gray frame");
    assert_eq!(gray_frame.rect.w, 16);
    assert_eq!(gray_frame.rect.h, 16);
    for px in gray_frame.bytes.chunks_exact(4) {
        assert_eq!(px[0], px[1], "grayscale R and G channels must match");
        assert_eq!(px[1], px[2], "grayscale G and B channels must match");
    }

    // 4. Visitor requests Monochrome1 mode with threshold = 128
    let mono_req = serde_json::json!({
        "mode": "monochrome",
        "threshold": 128
    });
    let (c, b) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/color",
        &serde_json::to_vec(&mono_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let mono_rep: tetherly_core::ScreenColorReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(mono_rep.mode, tetherly_core::ScreenColorMode::Monochrome1);
    assert_eq!(mono_rep.threshold, 128);
    assert!(mono_rep.is_active);

    // Verify host synchronizes Monochrome1
    let wait_host_mono = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/color")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let host_color_rep: tetherly_core::ScreenColorReport = serde_json::from_slice(&b).unwrap();
        if host_color_rep.mode == tetherly_core::ScreenColorMode::Monochrome1
            && host_color_rep.threshold == 128
        {
            break;
        }
        if wait_host_mono.elapsed() > Duration::from_secs(5) {
            panic!("host never synchronized Monochrome1 mode from visitor signal");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // Host sends frame with active Monochrome1 mode -> each pixel is pure 0 or 255
    tokio::time::sleep(Duration::from_millis(40)).await;
    let prev_count2 = visitor.screen_sink_snapshot().count();
    assert!(host.screen_send_frame().await.unwrap());

    let wait_mono_frame = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() > prev_count2 {
            break;
        }
        if wait_mono_frame.elapsed() > Duration::from_secs(5) {
            panic!("visitor never received Monochrome1 frame");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let mono_frame = visitor.screen_sink_snapshot().last().expect("mono frame");
    assert_eq!(mono_frame.rect.w, 16);
    assert_eq!(mono_frame.rect.h, 16);
    for px in mono_frame.bytes.chunks_exact(4) {
        assert_eq!(px[0], px[1]);
        assert_eq!(px[1], px[2]);
        assert!(
            px[0] == 0 || px[0] == 255,
            "monochrome pixel value must be either 0 or 255"
        );
    }

    // 5. Visitor requests ReducedColor16 (RGB565) mode
    let red_req = serde_json::json!({ "mode": "reduced" });
    let (c, b) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/color",
        &serde_json::to_vec(&red_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let red_rep: tetherly_core::ScreenColorReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(red_rep.mode, tetherly_core::ScreenColorMode::ReducedColor16);
    assert!(red_rep.is_active);

    // Verify host synchronizes ReducedColor16
    let wait_host_red = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/color")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let host_color_rep: tetherly_core::ScreenColorReport = serde_json::from_slice(&b).unwrap();
        if host_color_rep.mode == tetherly_core::ScreenColorMode::ReducedColor16 {
            break;
        }
        if wait_host_red.elapsed() > Duration::from_secs(5) {
            panic!("host never synchronized ReducedColor16 mode from visitor signal");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // Host sends frame with ReducedColor16 mode -> verify high-bit replenishment
    tokio::time::sleep(Duration::from_millis(40)).await;
    let prev_count3 = visitor.screen_sink_snapshot().count();
    assert!(host.screen_send_frame().await.unwrap());

    let wait_red_frame = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() > prev_count3 {
            break;
        }
        if wait_red_frame.elapsed() > Duration::from_secs(5) {
            panic!("visitor never received ReducedColor16 frame");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let red_frame = visitor
        .screen_sink_snapshot()
        .last()
        .expect("reduced frame");
    for px in red_frame.bytes.chunks_exact(4) {
        assert_eq!(
            px[0] & 0x07,
            px[0] >> 5,
            "5-bit channel low bits match high bits"
        );
        assert_eq!(
            px[1] & 0x03,
            px[1] >> 6,
            "6-bit channel low bits match high bits"
        );
        assert_eq!(
            px[2] & 0x07,
            px[2] >> 5,
            "5-bit channel low bits match high bits"
        );
    }

    // 6. Reset to FullColor32 via POST /api/screen/color/reset
    let (c, b) = tetherly_node::loopback_post(visitor_ui, "/api/screen/color/reset", b"{}")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let reset_rep: tetherly_core::ScreenColorReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(reset_rep.mode, tetherly_core::ScreenColorMode::FullColor32);
    assert!(!reset_rep.is_active);

    // Verify host receives Tag 20 reset signal
    let wait_host_reset = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/color")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let host_color_rep: tetherly_core::ScreenColorReport = serde_json::from_slice(&b).unwrap();
        if host_color_rep.mode == tetherly_core::ScreenColorMode::FullColor32
            && !host_color_rep.is_active
        {
            break;
        }
        if wait_host_reset.elapsed() > Duration::from_secs(5) {
            panic!("host never synchronized color reset from visitor signal");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // Host sends frame after reset -> restores full color
    tokio::time::sleep(Duration::from_millis(40)).await;
    let prev_count4 = visitor.screen_sink_snapshot().count();
    assert!(host.screen_send_frame().await.unwrap());

    let wait_restored_frame = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() > prev_count4 {
            break;
        }
        if wait_restored_frame.elapsed() > Duration::from_secs(5) {
            panic!("visitor never received full color frame after reset");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let restored_frame = visitor
        .screen_sink_snapshot()
        .last()
        .expect("restored frame");
    let mut restored_variance = false;
    for px in restored_frame.bytes.chunks_exact(4) {
        if px[0] != px[1] || px[1] != px[2] {
            restored_variance = true;
            break;
        }
    }
    assert!(
        restored_variance,
        "restored frame must exhibit color variance"
    );

    // 7. Telemetry Status Verification (/api/screen)
    let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let s: tetherly_node::ScreenStatusReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(s.color_mode, "full_color32");
    assert_eq!(s.color_threshold, 128);
    assert!(s.color_updates >= 4);

    // 8. Clean shutdown
    host.shutdown();
    visitor.shutdown();
    drop(ga);
    drop(gb);
}

#[derive(Debug)]
struct ControllableDeltaScreenSource {
    rect: tetherly_core::ScreenRect,
    seq: std::sync::atomic::AtomicU64,
    pixels: std::sync::Mutex<Vec<u8>>,
}

impl ControllableDeltaScreenSource {
    fn new(w: u16, h: u16) -> Self {
        Self::with_seq(w, h, 1)
    }

    fn with_seq(w: u16, h: u16, seq: u64) -> Self {
        let size = (w as usize) * (h as usize) * 4;
        let mut pixels = vec![0x20u8; size];
        for px in pixels.chunks_exact_mut(4) {
            px[3] = 255;
        }
        Self {
            rect: tetherly_core::ScreenRect::new(0, 0, w, h).unwrap(),
            seq: std::sync::atomic::AtomicU64::new(seq),
            pixels: std::sync::Mutex::new(pixels),
        }
    }

    fn update_rect(&self, x: u16, y: u16, w: u16, h: u16, color: [u8; 4]) {
        let mut lock = self.pixels.lock().unwrap();
        let stride = (self.rect.w as usize) * 4;
        for row in 0..h as usize {
            for col in 0..w as usize {
                let offset = (y as usize + row) * stride + (x as usize + col) * 4;
                if offset + 4 <= lock.len() {
                    lock[offset..offset + 4].copy_from_slice(&color);
                }
            }
        }
    }

    fn update_rect_with<F: FnMut(usize, usize) -> [u8; 4]>(
        &self,
        x: u16,
        y: u16,
        w: u16,
        h: u16,
        mut f: F,
    ) {
        let mut lock = self.pixels.lock().unwrap();
        let stride = (self.rect.w as usize) * 4;
        for row in 0..h as usize {
            for col in 0..w as usize {
                let color = f(col, row);
                let offset = (y as usize + row) * stride + (x as usize + col) * 4;
                if offset + 4 <= lock.len() {
                    lock[offset..offset + 4].copy_from_slice(&color);
                }
            }
        }
    }

    fn set_pattern(&self) {
        let mut lock = self.pixels.lock().unwrap();
        let w = self.rect.w as usize;
        let h = self.rect.h as usize;
        for y in 0..h {
            for x in 0..w {
                let idx = (y * w + x) * 4;
                lock[idx] = ((x * 3) & 0xFF) as u8;
                lock[idx + 1] = ((y * 7) & 0xFF) as u8;
                lock[idx + 2] = ((x ^ y) & 0xFF) as u8;
                lock[idx + 3] = 255;
            }
        }
    }

    fn scroll_vertical(&self, dy: i16, fill_color: [u8; 4]) {
        let mut lock = self.pixels.lock().unwrap();
        let w = self.rect.w as usize;
        let h = self.rect.h as usize;
        let stride = w * 4;
        if dy < 0 {
            let shift = ((-dy) as usize).min(h);
            lock.copy_within(shift * stride..h * stride, 0);
            for row in (h - shift)..h {
                let off = row * stride;
                for col in 0..w {
                    lock[off + col * 4..off + col * 4 + 4].copy_from_slice(&fill_color);
                }
            }
        } else if dy > 0 {
            let shift = (dy as usize).min(h);
            lock.copy_within(0..(h - shift) * stride, shift * stride);
            for row in 0..shift {
                let off = row * stride;
                for col in 0..w {
                    lock[off + col * 4..off + col * 4 + 4].copy_from_slice(&fill_color);
                }
            }
        }
    }

    fn pixels_snapshot(&self) -> Vec<u8> {
        self.pixels.lock().unwrap().clone()
    }
}

impl tetherly_core::ScreenSource for ControllableDeltaScreenSource {
    fn grab(&self) -> Result<Option<tetherly_core::ScreenFrame>, tetherly_core::CoreError> {
        let seq = self.seq.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let bytes = self.pixels.lock().unwrap().clone();
        Ok(Some(tetherly_core::ScreenFrame::raw(
            seq,
            self.rect,
            tetherly_core::PixelFormat::Bgra8,
            bytes,
        )?))
    }
}

#[tokio::test]
async fn m5_33_screen_dynamic_delta_patch_and_temporal_compression_pipeline() {
    let (da, ga) = temp_dir("host-delta");
    let (db, gb) = temp_dir("visitor-delta");
    let source = Arc::new(ControllableDeltaScreenSource::new(64, 64));
    let host = Node::start(cfg(da, "host", Some(source.clone()))).unwrap();
    let visitor = Node::start(cfg(db, "visitor", None)).unwrap();
    host.spawn_listeners().await.unwrap();
    visitor.spawn_listeners().await.unwrap();
    let pin = tetherly_core::pin::parse_pin("25170394").unwrap();
    host.set_pin(pin);
    visitor.set_pin(pin);
    host.dial(visitor.control_addr()).await.unwrap();
    wait_live(&host, Duration::from_secs(25)).await;
    wait_live(&visitor, Duration::from_secs(25)).await;

    let host_ui = host.spawn_ui().await.unwrap();
    let visitor_ui = visitor.spawn_ui().await.unwrap();
    let host_id = host.identity().device_id().clone();
    let visitor_id = visitor.identity().device_id().clone();

    wait_screen_port(&host, &visitor_id, Duration::from_secs(3)).await;
    wait_screen_port(&visitor, &host_id, Duration::from_secs(3)).await;

    // 1. Establish screen streaming session
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

    // 2. Initial state: delta mode is disabled by default
    let (c, b) = tetherly_node::loopback_get(visitor_ui, "/api/screen/delta")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let delta_rep: tetherly_core::ScreenDeltaReport = serde_json::from_slice(&b).unwrap();
    assert!(!delta_rep.enabled);
    assert_eq!(delta_rep.keyframe_interval, 60);
    assert_eq!(delta_rep.saved_bytes, 0);

    // Host sends 1 initial base keyframe (64x64 uniform 0x20 pixels)
    assert!(host.screen_send_frame().await.unwrap());

    // Wait for frame to reach visitor screen sink
    let wait_frame1 = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() >= 1 {
            break;
        }
        if wait_frame1.elapsed() > Duration::from_secs(5) {
            panic!("visitor never received initial keyframe");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let frame1 = visitor.screen_sink_snapshot().last().expect("frame 1");
    assert_eq!(frame1.rect.w, 64);
    assert_eq!(frame1.rect.h, 64);
    assert_eq!(frame1.bytes[0], 0x20);

    // 3. Visitor configures delta mode via POST /api/screen/delta with keyframe_interval = 3
    let delta_req = serde_json::json!({
        "enabled": true,
        "keyframe_interval": 3
    });
    let (c, b) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/delta",
        &serde_json::to_vec(&delta_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let delta_rep: tetherly_core::ScreenDeltaReport = serde_json::from_slice(&b).unwrap();
    assert!(delta_rep.enabled);
    assert_eq!(delta_rep.keyframe_interval, 3);

    // Verify host receives Tag 21 and synchronizes delta mode
    let wait_host_delta = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/delta")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let host_delta_rep: tetherly_core::ScreenDeltaReport = serde_json::from_slice(&b).unwrap();
        if host_delta_rep.enabled && host_delta_rep.keyframe_interval == 3 {
            break;
        }
        if wait_host_delta.elapsed() > Duration::from_secs(5) {
            panic!("host never synchronized delta mode from visitor signal");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 4. Host modifies sub-rectangle (16x16 at 0,0) and sends a delta patch frame
    source.update_rect(0, 0, 16, 16, [0xAA, 0xBB, 0xCC, 255]);
    tokio::time::sleep(Duration::from_millis(40)).await;
    let prev_count = visitor.screen_sink_snapshot().count();
    assert!(host.screen_send_frame().await.unwrap());

    // Wait for frame to reach visitor screen sink
    let wait_frame2 = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() > prev_count {
            break;
        }
        if wait_frame2.elapsed() > Duration::from_secs(5) {
            panic!("visitor never received delta patch frame");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // Verify visitor synthesized full frame with patched region and retained canvas
    let frame2 = visitor.screen_sink_snapshot().last().expect("frame 2");
    assert_eq!(frame2.rect.w, 64);
    assert_eq!(frame2.rect.h, 64);
    // Patched region at (0, 0)
    assert_eq!(frame2.bytes[0], 0xAA);
    assert_eq!(frame2.bytes[1], 0xBB);
    assert_eq!(frame2.bytes[2], 0xCC);
    assert_eq!(frame2.bytes[3], 255);
    // Unmodified region at (48, 48) retains 0x20
    let offset_48 = (48 * 64 + 48) * 4;
    assert_eq!(frame2.bytes[offset_48], 0x20);

    // Verify host delta telemetry
    let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/delta")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let host_rep: tetherly_core::ScreenDeltaReport = serde_json::from_slice(&b).unwrap();
    assert!(host_rep.delta_frames_sent >= 1);
    assert!(host_rep.saved_bytes > 0);

    // 5. Send subsequent frames until keyframe_interval (3) triggers a fresh keyframe
    source.update_rect(0, 0, 16, 16, [0x11, 0x22, 0x33, 255]);
    tokio::time::sleep(PACER_SETTLE).await;
    let prev_count2 = visitor.screen_sink_snapshot().count();
    assert!(host.screen_send_frame().await.unwrap()); // delta 2
    let wait_frame3 = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() > prev_count2 {
            break;
        }
        if wait_frame3.elapsed() > Duration::from_secs(5) {
            panic!("visitor never received delta frame 2");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    source.update_rect(0, 0, 16, 16, [0x44, 0x55, 0x66, 255]);
    tokio::time::sleep(PACER_SETTLE).await;
    let prev_count3 = visitor.screen_sink_snapshot().count();
    assert!(host.screen_send_frame().await.unwrap()); // delta 3
    let wait_frame4 = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() > prev_count3 {
            break;
        }
        if wait_frame4.elapsed() > Duration::from_secs(5) {
            panic!("visitor never received delta frame 3");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // Next frame reaches keyframe_interval threshold -> triggers keyframe
    source.update_rect(0, 0, 16, 16, [0x77, 0x88, 0x99, 255]);
    tokio::time::sleep(PACER_SETTLE).await;
    let prev_count4 = visitor.screen_sink_snapshot().count();
    assert!(host.screen_send_frame().await.unwrap()); // keyframe
    let wait_frame5 = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() > prev_count4 {
            break;
        }
        if wait_frame5.elapsed() > Duration::from_secs(5) {
            panic!("visitor never received periodic keyframe");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // Verify keyframes_sent increased
    let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/delta")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let host_rep: tetherly_core::ScreenDeltaReport = serde_json::from_slice(&b).unwrap();
    assert!(host_rep.keyframes_sent >= 1);
    assert!(host_rep.delta_frames_sent >= 2);

    let (c, b) = tetherly_node::loopback_get(visitor_ui, "/api/screen/delta")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let vis_rep: tetherly_core::ScreenDeltaReport = serde_json::from_slice(&b).unwrap();
    assert!(vis_rep.keyframes_received >= 2);
    assert!(vis_rep.delta_frames_received >= 2);

    // 6. Reset delta mode via POST /api/screen/delta/reset
    let (c, b) = tetherly_node::loopback_post(visitor_ui, "/api/screen/delta/reset", b"{}")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let reset_rep: tetherly_core::ScreenDeltaReport = serde_json::from_slice(&b).unwrap();
    assert!(!reset_rep.enabled);

    // Verify host receives Tag 22 reset signal
    let wait_host_reset = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/delta")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let host_delta_rep: tetherly_core::ScreenDeltaReport = serde_json::from_slice(&b).unwrap();
        if !host_delta_rep.enabled {
            break;
        }
        if wait_host_reset.elapsed() > Duration::from_secs(5) {
            panic!("host never synchronized delta reset from visitor signal");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 7. Telemetry Status Verification (/api/screen)
    let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let s: tetherly_node::ScreenStatusReport = serde_json::from_slice(&b).unwrap();
    assert!(!s.delta_enabled);
    assert!(s.delta_frames_sent >= 2);
    assert!(s.delta_saved_bytes > 0);

    // 8. Clean shutdown
    host.shutdown();
    visitor.shutdown();
    drop(ga);
    drop(gb);
}

#[tokio::test]
async fn m5_34_screen_objective_visual_quality_metric_and_psnr_ssim_pipeline() {
    let (da, ga) = temp_dir("host-metric");
    let (db, gb) = temp_dir("visitor-metric");
    let source = Arc::new(ControllableDeltaScreenSource::new(64, 64));
    let host = Node::start(cfg(da, "host", Some(source.clone()))).unwrap();
    let visitor = Node::start(cfg(db, "visitor", None)).unwrap();
    host.spawn_listeners().await.unwrap();
    visitor.spawn_listeners().await.unwrap();
    let pin = tetherly_core::pin::parse_pin("25170394").unwrap();
    host.set_pin(pin);
    visitor.set_pin(pin);
    host.dial(visitor.control_addr()).await.unwrap();
    wait_live(&host, Duration::from_secs(25)).await;
    wait_live(&visitor, Duration::from_secs(25)).await;

    let host_ui = host.spawn_ui().await.unwrap();
    let visitor_ui = visitor.spawn_ui().await.unwrap();
    let host_id = host.identity().device_id().clone();
    let visitor_id = visitor.identity().device_id().clone();

    wait_screen_port(&host, &visitor_id, Duration::from_secs(3)).await;
    wait_screen_port(&visitor, &host_id, Duration::from_secs(3)).await;

    // 1. Establish screen streaming session
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

    // This test streams IDENTICAL frames to drive MSE=0/PSNR=100 quality
    // evaluations (ADR-110). The ADR-089 adaptive squelch treats an
    // unchanged screen as static and suppresses frames after its 500ms
    // delay — on slow CI runners the grab+queue latency between frames can
    // exceed that, squelching a stream the metric pipeline needs. Squelch
    // is an orthogonal bandwidth feature; disable it for the host here so
    // the metric assertions are deterministic.
    let (c_sq, b_sq) = tetherly_node::loopback_post(
        host_ui,
        "/api/screen/squelch",
        &serde_json::to_vec(&serde_json::json!({ "mode": "disabled" })).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c_sq, 200);
    let sq: tetherly_node::ScreenSquelchReport = serde_json::from_slice(&b_sq).unwrap();
    assert_eq!(sq.mode, "disabled");

    // 2. Initial state: quality metric is disabled by default
    let (c, b) = tetherly_node::loopback_get(visitor_ui, "/api/screen/metric")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let metric_rep: tetherly_core::ScreenQualityMetricReport = serde_json::from_slice(&b).unwrap();
    assert!(!metric_rep.enabled);
    assert_eq!(metric_rep.sample_interval, 10);
    assert_eq!(metric_rep.evaluated_frames, 0);

    // Host sends initial frame
    assert!(host.screen_send_frame().await.unwrap());
    let wait_frame1 = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() >= 1 {
            break;
        }
        if wait_frame1.elapsed() > Duration::from_secs(5) {
            panic!("visitor never received initial frame");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 3. Visitor enables metric mode via POST /api/screen/metric with sample_interval = 2
    let metric_req = serde_json::json!({
        "enabled": true,
        "sample_interval": 2
    });
    let (c, b) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/metric",
        &serde_json::to_vec(&metric_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let metric_rep: tetherly_core::ScreenQualityMetricReport = serde_json::from_slice(&b).unwrap();
    assert!(metric_rep.enabled);
    assert_eq!(metric_rep.sample_interval, 2);

    // Verify host receives Tag 23 and synchronizes metric mode
    let wait_host_metric = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/metric")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let host_metric_rep: tetherly_core::ScreenQualityMetricReport =
            serde_json::from_slice(&b).unwrap();
        if host_metric_rep.enabled && host_metric_rep.sample_interval == 2 {
            break;
        }
        if wait_host_metric.elapsed() > Duration::from_secs(5) {
            panic!("host never synchronized metric mode from visitor signal");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 4. Stream 3 identical frames -> triggers at least 1 evaluation on host and visitor
    for _ in 0..3 {
        tokio::time::sleep(PACER_SETTLE).await;
        let prev = visitor.screen_sink_snapshot().count();
        assert!(host.screen_send_frame().await.unwrap());
        let wait_frame = std::time::Instant::now();
        loop {
            if visitor.screen_sink_snapshot().count() > prev {
                break;
            }
            if wait_frame.elapsed() > Duration::from_secs(5) {
                panic!("visitor frame timeout during identical stream");
            }
            tokio::time::sleep(Duration::from_millis(15)).await;
        }
    }

    // Verify host metric telemetry: identical frames yield MSE = 0.0, PSNR = 100.0, SSIM = 1.0, no warning
    let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/metric")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let host_metric_rep: tetherly_core::ScreenQualityMetricReport =
        serde_json::from_slice(&b).unwrap();
    assert!(host_metric_rep.evaluated_frames >= 1);
    assert_eq!(host_metric_rep.avg_mse, 0.0);
    assert_eq!(host_metric_rep.avg_psnr_db, 100.0);
    assert_eq!(host_metric_rep.avg_ssim, 1.0);
    assert!(!host_metric_rep.quality_warning);

    // 5. Update sub-rectangle and stream perturbed frames
    source.update_rect(0, 0, 32, 32, [0xFF, 0x00, 0x00, 255]);
    for _ in 0..3 {
        tokio::time::sleep(PACER_SETTLE).await;
        let prev = visitor.screen_sink_snapshot().count();
        assert!(host.screen_send_frame().await.unwrap());
        let wait_frame = std::time::Instant::now();
        loop {
            if visitor.screen_sink_snapshot().count() > prev {
                break;
            }
            if wait_frame.elapsed() > Duration::from_secs(5) {
                panic!("visitor frame timeout during perturbed stream");
            }
            tokio::time::sleep(Duration::from_millis(15)).await;
        }
    }

    // Verify evaluated_frames incremented
    let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/metric")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let host_metric_rep: tetherly_core::ScreenQualityMetricReport =
        serde_json::from_slice(&b).unwrap();
    assert!(host_metric_rep.evaluated_frames >= 2);

    // 6. Reset metric mode via POST /api/screen/metric/reset
    let (c, b) = tetherly_node::loopback_post(visitor_ui, "/api/screen/metric/reset", b"{}")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let reset_rep: tetherly_core::ScreenQualityMetricReport = serde_json::from_slice(&b).unwrap();
    assert!(!reset_rep.enabled);
    assert_eq!(reset_rep.sample_interval, 10);
    assert_eq!(reset_rep.evaluated_frames, 0);

    // Verify host receives Tag 24 reset signal
    let wait_host_reset = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/metric")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let host_metric_rep: tetherly_core::ScreenQualityMetricReport =
            serde_json::from_slice(&b).unwrap();
        if !host_metric_rep.enabled && host_metric_rep.evaluated_frames == 0 {
            break;
        }
        if wait_host_reset.elapsed() > Duration::from_secs(5) {
            panic!("host never synchronized metric reset from visitor signal");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 7. Telemetry Status Verification (/api/screen)
    let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let s: tetherly_node::ScreenStatusReport = serde_json::from_slice(&b).unwrap();
    assert!(!s.metric_enabled);
    assert_eq!(s.metric_evaluated_frames, 0);

    // 8. Clean shutdown
    host.shutdown();
    visitor.shutdown();
    drop(ga);
    drop(gb);
}

#[tokio::test]
async fn m5_35_screen_autonomous_qoe_closed_loop_adaptation_pipeline() {
    let (da, ga) = temp_dir("host-qoe");
    let (db, gb) = temp_dir("visitor-qoe");
    let source = Arc::new(ControllableDeltaScreenSource::new(64, 64));
    let host = Node::start(cfg(da, "host", Some(source.clone()))).unwrap();
    let visitor = Node::start(cfg(db, "visitor", None)).unwrap();
    host.spawn_listeners().await.unwrap();
    visitor.spawn_listeners().await.unwrap();
    let pin = tetherly_core::pin::parse_pin("25170395").unwrap();
    host.set_pin(pin);
    visitor.set_pin(pin);
    host.dial(visitor.control_addr()).await.unwrap();
    wait_live(&host, Duration::from_secs(25)).await;
    wait_live(&visitor, Duration::from_secs(25)).await;

    let host_ui = host.spawn_ui().await.unwrap();
    let visitor_ui = visitor.spawn_ui().await.unwrap();
    let host_id = host.identity().device_id().clone();
    let visitor_id = visitor.identity().device_id().clone();

    wait_screen_port(&host, &visitor_id, Duration::from_secs(3)).await;
    wait_screen_port(&visitor, &host_id, Duration::from_secs(3)).await;

    // 1. Establish screen streaming session
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

    // 2. Initial state: QoE closed-loop is disabled by default
    let (c, b) = tetherly_node::loopback_get(visitor_ui, "/api/screen/qoe")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let qoe_rep: tetherly_core::ScreenQoeReport = serde_json::from_slice(&b).unwrap();
    assert!(qoe_rep.ok);
    assert_eq!(qoe_rep.mode, "off");
    assert_eq!(qoe_rep.actions_total, 0);
    assert_eq!(qoe_rep.last_action, "hold");

    // Check status telemetry initially shows qoe_enabled = false
    let (c, b) = tetherly_node::loopback_get(visitor_ui, "/api/screen")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let status_rep: tetherly_node::ScreenStatusReport = serde_json::from_slice(&b).unwrap();
    assert!(!status_rep.qoe_enabled);
    assert_eq!(status_rep.qoe_mode, "off");

    // 3. Visitor configures Balanced QoE mode via POST /api/screen/qoe
    let qoe_req = serde_json::json!({
        "mode": "balanced",
        "target_psnr": 32,
        "target_ssim": 90
    });
    let (c, b) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/qoe",
        &serde_json::to_vec(&qoe_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let qoe_rep: tetherly_core::ScreenQoeReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(qoe_rep.mode, "balanced");
    assert_eq!(qoe_rep.target_psnr_db, 32);
    assert_eq!(qoe_rep.target_ssim_pct, 90);

    // Verify host receives Tag 25 and synchronizes QoE config
    let wait_host_qoe = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/qoe")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let host_qoe_rep: tetherly_core::ScreenQoeReport = serde_json::from_slice(&b).unwrap();
        if host_qoe_rep.mode == "balanced"
            && host_qoe_rep.target_psnr_db == 32
            && host_qoe_rep.target_ssim_pct == 90
        {
            break;
        }
        if wait_host_qoe.elapsed() > Duration::from_secs(5) {
            panic!(
                "host never synchronized QoE mode from visitor signal: {:?}",
                host_qoe_rep
            );
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // Also set sample interval to 1 so evaluations trigger on every frame
    let metric_req = serde_json::json!({
        "enabled": true,
        "sample_interval": 1
    });
    let (c, _) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/metric",
        &serde_json::to_vec(&metric_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);

    // 4. Send initial frame to establish baseline
    assert!(host.screen_send_frame().await.unwrap());
    tokio::time::sleep(Duration::from_millis(50)).await;

    // 5. Inject full color distortion into source to trigger QoE action
    source.update_rect(0, 0, 64, 64, [0x00, 0xFF, 0x00, 255]);
    for i in 0..3 {
        tokio::time::sleep(PACER_SETTLE).await;
        let prev = visitor.screen_sink_snapshot().count();
        assert!(host.screen_send_frame().await.unwrap());
        let wait_frame = std::time::Instant::now();
        loop {
            if visitor.screen_sink_snapshot().count() > prev {
                break;
            }
            if wait_frame.elapsed() > Duration::from_secs(5) {
                panic!(
                    "visitor frame timeout during QoE perturbed stream, iter {}",
                    i
                );
            }
            tokio::time::sleep(Duration::from_millis(15)).await;
        }
    }

    // Verify host QoE controller executed at least one action
    let wait_action = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/qoe")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let host_qoe_rep: tetherly_core::ScreenQoeReport = serde_json::from_slice(&b).unwrap();
        if host_qoe_rep.actions_total >= 1 {
            assert_ne!(host_qoe_rep.last_action, "hold");
            break;
        }
        if wait_action.elapsed() > Duration::from_secs(5) {
            panic!(
                "host QoE controller never triggered closed-loop action: {:?}",
                host_qoe_rep
            );
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // Verify status telemetry shows active QoE with action counts
    let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let status_rep: tetherly_node::ScreenStatusReport = serde_json::from_slice(&b).unwrap();
    assert!(status_rep.qoe_enabled);
    assert_eq!(status_rep.qoe_mode, "balanced");
    assert!(status_rep.qoe_actions_total >= 1);

    // 6. Reset QoE mode via POST /api/screen/qoe/reset (Tag 26)
    let (c, b) = tetherly_node::loopback_post(visitor_ui, "/api/screen/qoe/reset", b"{}")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let reset_rep: tetherly_core::ScreenQoeReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(reset_rep.mode, "off");
    assert_eq!(reset_rep.actions_total, 0);
    assert_eq!(reset_rep.last_action, "hold");

    // Verify host synchronizes Tag 26 reset
    let wait_host_reset = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/qoe")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let host_qoe_rep: tetherly_core::ScreenQoeReport = serde_json::from_slice(&b).unwrap();
        if host_qoe_rep.mode == "off" && host_qoe_rep.actions_total == 0 {
            break;
        }
        if wait_host_reset.elapsed() > Duration::from_secs(5) {
            panic!(
                "host never synchronized QoE reset from visitor signal: {:?}",
                host_qoe_rep
            );
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 7. Verify status telemetry shows QoE disabled
    let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let status_rep: tetherly_node::ScreenStatusReport = serde_json::from_slice(&b).unwrap();
    assert!(!status_rep.qoe_enabled);
    assert_eq!(status_rep.qoe_mode, "off");
    assert_eq!(status_rep.qoe_actions_total, 0);

    // 8. Clean shutdown
    host.shutdown();
    visitor.shutdown();
    drop(ga);
    drop(gb);
}

#[tokio::test]
async fn m5_36_screen_progressive_intra_refresh_pipeline() {
    let (da, ga) = temp_dir("host-pir");
    let (db, gb) = temp_dir("visitor-pir");
    let source = Arc::new(ControllableDeltaScreenSource::new(64, 64));
    let host = Node::start(cfg(da, "host", Some(source.clone()))).unwrap();
    let visitor = Node::start(cfg(db, "visitor", None)).unwrap();
    host.spawn_listeners().await.unwrap();
    visitor.spawn_listeners().await.unwrap();
    let pin = tetherly_core::pin::parse_pin("25170396").unwrap();
    host.set_pin(pin);
    visitor.set_pin(pin);
    host.dial(visitor.control_addr()).await.unwrap();
    wait_live(&host, Duration::from_secs(25)).await;
    wait_live(&visitor, Duration::from_secs(25)).await;

    let host_ui = host.spawn_ui().await.unwrap();
    let visitor_ui = visitor.spawn_ui().await.unwrap();
    let host_id = host.identity().device_id().clone();
    let visitor_id = visitor.identity().device_id().clone();

    wait_screen_port(&host, &visitor_id, Duration::from_secs(3)).await;
    wait_screen_port(&visitor, &host_id, Duration::from_secs(3)).await;

    // 1. Establish screen streaming session
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

    // 2. Initial state: PIR is disabled by default
    let (c, b) = tetherly_node::loopback_get(visitor_ui, "/api/screen/pir")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let pir_rep: tetherly_core::ScreenPirReport = serde_json::from_slice(&b).unwrap();
    assert!(pir_rep.ok);
    assert_eq!(pir_rep.mode, "disabled");
    assert_eq!(pir_rep.bands, tetherly_core::SCREEN_PIR_DEFAULT_BANDS);
    assert_eq!(pir_rep.sweeps_completed, 0);
    assert_eq!(pir_rep.slices_sent, 0);
    assert!(!pir_rep.active_sweep);

    // Check status telemetry initially shows pir_enabled = false
    let (c, b) = tetherly_node::loopback_get(visitor_ui, "/api/screen")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let status_rep: tetherly_node::ScreenStatusReport = serde_json::from_slice(&b).unwrap();
    assert!(!status_rep.pir_enabled);
    assert_eq!(status_rep.pir_mode, "disabled");

    // 3. Send initial base keyframe to establish canvas
    assert!(host.screen_send_frame().await.unwrap());
    let wait_init_frame = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() >= 1 {
            break;
        }
        if wait_init_frame.elapsed() > Duration::from_secs(5) {
            panic!("visitor never received initial keyframe");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 4. Visitor configures Continuous PIR mode via POST /api/screen/pir with bands = 4
    let pir_req = serde_json::json!({
        "mode": "continuous",
        "bands": 4
    });
    let (c, b) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/pir",
        &serde_json::to_vec(&pir_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let pir_rep: tetherly_core::ScreenPirReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(pir_rep.mode, "continuous");
    assert_eq!(pir_rep.bands, 4);
    assert!(pir_rep.active_sweep);

    // Verify host receives Tag 27 and synchronizes PIR config
    let wait_host_pir = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/pir")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let host_pir_rep: tetherly_core::ScreenPirReport = serde_json::from_slice(&b).unwrap();
        if host_pir_rep.mode == "continuous" && host_pir_rep.bands == 4 {
            break;
        }
        if wait_host_pir.elapsed() > Duration::from_secs(5) {
            panic!(
                "host never synchronized PIR mode from visitor signal: {:?}",
                host_pir_rep
            );
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 5. Host pushes frames; with bands = 4, 4 frames complete 1 full sweep
    for i in 0..6 {
        tokio::time::sleep(PACER_SETTLE).await;
        let prev = visitor.screen_sink_snapshot().count();
        assert!(host.screen_send_frame().await.unwrap());
        let wait_frame = std::time::Instant::now();
        loop {
            if visitor.screen_sink_snapshot().count() > prev {
                break;
            }
            if wait_frame.elapsed() > Duration::from_secs(5) {
                panic!("visitor frame timeout during PIR streaming, iter {}", i);
            }
            tokio::time::sleep(Duration::from_millis(15)).await;
        }
    }

    // Verify host PIR telemetry shows completed sweeps and slice progression
    let wait_sweep = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/pir")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let host_pir_rep: tetherly_core::ScreenPirReport = serde_json::from_slice(&b).unwrap();
        if host_pir_rep.sweeps_completed >= 1 && host_pir_rep.slices_sent >= 4 {
            break;
        }
        if wait_sweep.elapsed() > Duration::from_secs(5) {
            panic!("host PIR sweep never completed: {:?}", host_pir_rep);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // Verify status telemetry shows active PIR telemetry
    let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let status_rep: tetherly_node::ScreenStatusReport = serde_json::from_slice(&b).unwrap();
    assert!(status_rep.pir_enabled);
    assert_eq!(status_rep.pir_mode, "continuous");
    assert_eq!(status_rep.pir_bands, 4);
    assert!(status_rep.pir_sweeps_completed >= 1);
    assert!(status_rep.pir_slices_sent >= 4);

    // 6. Reset PIR mode via POST /api/screen/pir/reset (Tag 28)
    let (c, b) = tetherly_node::loopback_post(visitor_ui, "/api/screen/pir/reset", b"{}")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let reset_rep: tetherly_core::ScreenPirReport = serde_json::from_slice(&b).unwrap();
    assert_eq!(reset_rep.mode, "disabled");
    assert!(!reset_rep.active_sweep);

    // Verify host synchronizes Tag 28 reset
    let wait_host_reset = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/pir")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let host_pir_rep: tetherly_core::ScreenPirReport = serde_json::from_slice(&b).unwrap();
        if host_pir_rep.mode == "disabled" && !host_pir_rep.active_sweep {
            break;
        }
        if wait_host_reset.elapsed() > Duration::from_secs(5) {
            panic!(
                "host never synchronized PIR reset from visitor signal: {:?}",
                host_pir_rep
            );
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 7. Verify status telemetry reflects disabled PIR mode
    let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let status_rep: tetherly_node::ScreenStatusReport = serde_json::from_slice(&b).unwrap();
    assert!(!status_rep.pir_enabled);
    assert_eq!(status_rep.pir_mode, "disabled");

    // 8. Clean shutdown
    host.shutdown();
    visitor.shutdown();
    drop(ga);
    drop(gb);
}

#[tokio::test]
async fn m5_37_screen_ltr_long_term_reference_pipeline() {
    let (da, ga) = temp_dir("host-ltr");
    let (db, gb) = temp_dir("visitor-ltr");
    let source = Arc::new(ControllableDeltaScreenSource::new(64, 64));
    let host = Node::start(cfg(da, "host", Some(source.clone()))).unwrap();
    let visitor = Node::start(cfg(db, "visitor", None)).unwrap();
    host.spawn_listeners().await.unwrap();
    visitor.spawn_listeners().await.unwrap();
    let pin = tetherly_core::pin::parse_pin("25170396").unwrap();
    host.set_pin(pin);
    visitor.set_pin(pin);
    host.dial(visitor.control_addr()).await.unwrap();
    wait_live(&host, Duration::from_secs(25)).await;
    wait_live(&visitor, Duration::from_secs(25)).await;

    let host_ui = host.spawn_ui().await.unwrap();
    let visitor_ui = visitor.spawn_ui().await.unwrap();
    let host_id = host.identity().device_id().clone();
    let visitor_id = visitor.identity().device_id().clone();

    wait_screen_port(&host, &visitor_id, Duration::from_secs(3)).await;
    wait_screen_port(&visitor, &host_id, Duration::from_secs(3)).await;

    // 1. Establish screen streaming session
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

    // 2. Initial state: LTR is disabled by default
    let (c, b) = tetherly_node::loopback_get(visitor_ui, "/api/screen/ltr")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let ltr_rep: tetherly_core::ScreenLtrReport = serde_json::from_slice(&b).unwrap();
    assert!(ltr_rep.ok);
    assert!(!ltr_rep.enabled);
    assert_eq!(
        ltr_rep.auto_interval,
        tetherly_core::SCREEN_LTR_DEFAULT_AUTO_INTERVAL
    );
    assert_eq!(ltr_rep.current_ltr_seq, 0);
    assert_eq!(ltr_rep.anchors_established, 0);
    assert_eq!(ltr_rep.recoveries_requested, 0);
    assert_eq!(ltr_rep.recoveries_completed, 0);
    assert_eq!(ltr_rep.saved_burst_bytes, 0);

    // Check status telemetry initially shows ltr_enabled = false
    let (c, b) = tetherly_node::loopback_get(visitor_ui, "/api/screen")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let status_rep: tetherly_node::ScreenStatusReport = serde_json::from_slice(&b).unwrap();
    assert!(!status_rep.ltr_enabled);
    assert_eq!(
        status_rep.ltr_auto_interval,
        tetherly_core::SCREEN_LTR_DEFAULT_AUTO_INTERVAL
    );

    // 3. Visitor configures LTR mode via POST /api/screen/ltr with auto_interval = 20 (Tag 29)
    let ltr_req = serde_json::json!({
        "enabled": true,
        "auto_interval": 20
    });
    let (c, b) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/ltr",
        &serde_json::to_vec(&ltr_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let ltr_rep: tetherly_core::ScreenLtrReport = serde_json::from_slice(&b).unwrap();
    assert!(ltr_rep.enabled);
    assert_eq!(ltr_rep.auto_interval, 20);

    // Verify host receives Tag 29 and synchronizes LTR config
    let wait_host_ltr = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/ltr")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let host_ltr_rep: tetherly_core::ScreenLtrReport = serde_json::from_slice(&b).unwrap();
        if host_ltr_rep.enabled && host_ltr_rep.auto_interval == 20 {
            break;
        }
        if wait_host_ltr.elapsed() > Duration::from_secs(5) {
            panic!(
                "host never synchronized LTR mode from visitor signal: {:?}",
                host_ltr_rep
            );
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 4. Send initial base keyframe to establish canvas and golden LTR anchor
    assert!(host.screen_send_frame().await.unwrap());
    let wait_init_frame = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() >= 1 {
            break;
        }
        if wait_init_frame.elapsed() > Duration::from_secs(5) {
            panic!("visitor never received initial keyframe");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // Verify host established golden LTR anchor
    let wait_anchor = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/ltr")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let host_ltr_rep: tetherly_core::ScreenLtrReport = serde_json::from_slice(&b).unwrap();
        if host_ltr_rep.anchors_established >= 1 && host_ltr_rep.current_ltr_seq > 0 {
            break;
        }
        if wait_anchor.elapsed() > Duration::from_secs(5) {
            panic!(
                "host never established golden LTR anchor: {:?}",
                host_ltr_rep
            );
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 5. Host pushes intermediate frames with damaged region
    for i in 0..3 {
        tokio::time::sleep(PACER_SETTLE).await;
        source.update_rect(10 + i * 2, 10 + i * 2, 16, 16, [0x80, 0x10, 0x40, 0xff]);
        let prev = visitor.screen_sink_snapshot().count();
        assert!(host.screen_send_frame().await.unwrap());
        let wait_frame = std::time::Instant::now();
        loop {
            if visitor.screen_sink_snapshot().count() > prev {
                break;
            }
            if wait_frame.elapsed() > Duration::from_secs(5) {
                panic!(
                    "visitor frame timeout during intermediate streaming, iter {}",
                    i
                );
            }
            tokio::time::sleep(Duration::from_millis(15)).await;
        }
    }

    // 6. Visitor requests LTR differential gap recovery via POST /api/screen/ltr/recover (Tag 31)
    let (c, b) = tetherly_node::loopback_post(visitor_ui, "/api/screen/ltr/recover", b"{}")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let visitor_ltr_rep: tetherly_core::ScreenLtrReport = serde_json::from_slice(&b).unwrap();
    assert!(visitor_ltr_rep.recoveries_requested >= 1);

    // Verify host receives Tag 31 recovery request
    let wait_host_rec_req = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/ltr")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let host_ltr_rep: tetherly_core::ScreenLtrReport = serde_json::from_slice(&b).unwrap();
        if host_ltr_rep.recoveries_requested >= 1 {
            break;
        }
        if wait_host_rec_req.elapsed() > Duration::from_secs(5) {
            panic!(
                "host never received LTR recovery request: {:?}",
                host_ltr_rep
            );
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // Host pushes next frame which triggers differential LTR recovery patch (SCREEN_COMPRESS_LTR = 5)
    source.update_rect(20, 20, 16, 16, [0x00, 0xee, 0x11, 0xff]);
    let prev_rec = visitor.screen_sink_snapshot().count();
    assert!(host.screen_send_frame().await.unwrap());
    let wait_rec_frame = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() > prev_rec {
            break;
        }
        if wait_rec_frame.elapsed() > Duration::from_secs(5) {
            panic!("visitor never received LTR recovery frame");
        }
        tokio::time::sleep(Duration::from_millis(15)).await;
    }

    // Verify host reports recovery completed and burst bytes saved
    let wait_rec_comp = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/ltr")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let host_ltr_rep: tetherly_core::ScreenLtrReport = serde_json::from_slice(&b).unwrap();
        if host_ltr_rep.recoveries_completed >= 1 && host_ltr_rep.saved_burst_bytes > 0 {
            break;
        }
        if wait_rec_comp.elapsed() > Duration::from_secs(5) {
            panic!(
                "host LTR recovery never reported completed: {:?}",
                host_ltr_rep
            );
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // Verify visitor telemetry shows recovery completed
    let wait_vis_rec = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(visitor_ui, "/api/screen/ltr")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let vis_ltr_rep: tetherly_core::ScreenLtrReport = serde_json::from_slice(&b).unwrap();
        if vis_ltr_rep.recoveries_completed >= 1 {
            break;
        }
        if wait_vis_rec.elapsed() > Duration::from_secs(5) {
            panic!(
                "visitor LTR recovery completion not synchronized: {:?}",
                vis_ltr_rep
            );
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // Verify status telemetry shows active LTR telemetry
    let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let status_rep: tetherly_node::ScreenStatusReport = serde_json::from_slice(&b).unwrap();
    assert!(status_rep.ltr_enabled);
    assert_eq!(status_rep.ltr_auto_interval, 20);
    assert!(status_rep.ltr_anchors_established >= 1);
    assert!(status_rep.ltr_recoveries_completed >= 1);

    // 7. Manual anchor via POST /api/screen/ltr/anchor (Tag 32)
    let (c, b) = tetherly_node::loopback_post(host_ui, "/api/screen/ltr/anchor", b"{}")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let manual_rep: tetherly_core::ScreenLtrReport = serde_json::from_slice(&b).unwrap();
    assert!(manual_rep.anchors_established >= 2);

    // 8. Reset LTR mode via POST /api/screen/ltr/reset (Tag 30)
    let (c, b) = tetherly_node::loopback_post(visitor_ui, "/api/screen/ltr/reset", b"{}")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let reset_rep: tetherly_core::ScreenLtrReport = serde_json::from_slice(&b).unwrap();
    assert!(!reset_rep.enabled);
    assert_eq!(reset_rep.current_ltr_seq, 0);

    // Verify host synchronizes Tag 30 reset
    let wait_host_reset = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/ltr")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let host_ltr_rep: tetherly_core::ScreenLtrReport = serde_json::from_slice(&b).unwrap();
        if !host_ltr_rep.enabled && host_ltr_rep.current_ltr_seq == 0 {
            break;
        }
        if wait_host_reset.elapsed() > Duration::from_secs(5) {
            panic!(
                "host never synchronized LTR reset from visitor signal: {:?}",
                host_ltr_rep
            );
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 9. Verify status telemetry reflects disabled LTR mode
    let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let status_rep: tetherly_node::ScreenStatusReport = serde_json::from_slice(&b).unwrap();
    assert!(!status_rep.ltr_enabled);

    // 10. Clean shutdown
    host.shutdown();
    visitor.shutdown();
    drop(ga);
    drop(gb);
}

#[tokio::test]
async fn m5_38_screen_temporal_scalability_svc_pipeline() {
    let (da, ga) = temp_dir("host-svc");
    let (db, gb) = temp_dir("visitor-svc");
    let source = Arc::new(ControllableDeltaScreenSource::with_seq(64, 64, 0));
    let host = Node::start(cfg(da, "host", Some(source.clone()))).unwrap();
    let visitor = Node::start(cfg(db, "visitor", None)).unwrap();
    host.spawn_listeners().await.unwrap();
    visitor.spawn_listeners().await.unwrap();
    let pin = tetherly_core::pin::parse_pin("25170396").unwrap();
    host.set_pin(pin);
    visitor.set_pin(pin);
    host.dial(visitor.control_addr()).await.unwrap();
    wait_live(&host, Duration::from_secs(25)).await;
    wait_live(&visitor, Duration::from_secs(25)).await;

    let host_ui = host.spawn_ui().await.unwrap();
    let visitor_ui = visitor.spawn_ui().await.unwrap();
    let host_id = host.identity().device_id().clone();
    let visitor_id = visitor.identity().device_id().clone();

    wait_screen_port(&host, &visitor_id, Duration::from_secs(3)).await;
    wait_screen_port(&visitor, &host_id, Duration::from_secs(3)).await;

    // 1. Establish screen streaming session
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

    // 2. Initial state: SVC is disabled by default
    let (c, b) = tetherly_node::loopback_get(visitor_ui, "/api/screen/svc")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let svc_rep: tetherly_core::ScreenSvcReport = serde_json::from_slice(&b).unwrap();
    assert!(svc_rep.ok);
    assert!(!svc_rep.enabled);
    assert_eq!(svc_rep.mode, "disabled");
    assert!(!svc_rep.drop_t1);
    assert_eq!(svc_rep.t0_frames_sent, 0);
    assert_eq!(svc_rep.t1_frames_sent, 0);
    assert_eq!(svc_rep.t1_frames_dropped, 0);
    assert_eq!(svc_rep.saved_bandwidth_bytes, 0);

    // Check status telemetry initially shows svc_enabled = false
    let (c, b) = tetherly_node::loopback_get(visitor_ui, "/api/screen")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let status_rep: tetherly_node::ScreenStatusReport = serde_json::from_slice(&b).unwrap();
    assert!(!status_rep.svc_enabled);
    assert_eq!(status_rep.svc_mode, "disabled");

    // 3. Visitor configures SVC mode via POST /api/screen/svc (mode: adaptive, drop_t1: false) (Tag 33)
    let svc_req = serde_json::json!({
        "mode": "adaptive",
        "drop_t1": false
    });
    let (c, b) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/svc",
        &serde_json::to_vec(&svc_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let svc_rep: tetherly_core::ScreenSvcReport = serde_json::from_slice(&b).unwrap();
    assert!(svc_rep.enabled);
    assert_eq!(svc_rep.mode, "adaptive");
    assert!(!svc_rep.drop_t1);

    // Verify host receives Tag 33 and synchronizes SVC config
    let wait_host_svc = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/svc")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let host_svc_rep: tetherly_core::ScreenSvcReport = serde_json::from_slice(&b).unwrap();
        if host_svc_rep.enabled && host_svc_rep.mode == "adaptive" && !host_svc_rep.drop_t1 {
            break;
        }
        if wait_host_svc.elapsed() > Duration::from_secs(5) {
            panic!(
                "host never synchronized SVC mode from visitor signal: {:?}",
                host_svc_rep
            );
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 4. Host pushes alternating T0 and T1 frames
    // Frame 0: Base Layer T0 (seq 0 % 2 == 0)
    assert!(host.screen_send_frame().await.unwrap());
    let wait_f0 = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() >= 1 {
            break;
        }
        if wait_f0.elapsed() > Duration::from_secs(5) {
            panic!("visitor never received Frame 0");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // Frame 1: Enhancement Layer T1 (seq 1 % 2 == 1)
    tokio::time::sleep(PACER_SETTLE).await;
    source.update_rect(10, 10, 16, 16, [0x11, 0x22, 0x33, 0xff]);
    let prev1 = visitor.screen_sink_snapshot().count();
    assert!(host.screen_send_frame().await.unwrap());
    let wait_f1 = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() > prev1 {
            break;
        }
        if wait_f1.elapsed() > Duration::from_secs(5) {
            panic!("visitor never received Frame 1 (T1)");
        }
        tokio::time::sleep(Duration::from_millis(15)).await;
    }

    // Frame 2: Base Layer T0 (seq 2 % 2 == 0)
    tokio::time::sleep(PACER_SETTLE).await;
    source.update_rect(12, 12, 16, 16, [0x44, 0x55, 0x66, 0xff]);
    let prev2 = visitor.screen_sink_snapshot().count();
    assert!(host.screen_send_frame().await.unwrap());
    let wait_f2 = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() > prev2 {
            break;
        }
        if wait_f2.elapsed() > Duration::from_secs(5) {
            panic!("visitor never received Frame 2 (T0)");
        }
        tokio::time::sleep(Duration::from_millis(15)).await;
    }

    // Frame 3: Enhancement Layer T1 (seq 3 % 2 == 1)
    tokio::time::sleep(PACER_SETTLE).await;
    source.update_rect(14, 14, 16, 16, [0x77, 0x88, 0x99, 0xff]);
    let prev3 = visitor.screen_sink_snapshot().count();
    assert!(host.screen_send_frame().await.unwrap());
    let wait_f3 = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() > prev3 {
            break;
        }
        if wait_f3.elapsed() > Duration::from_secs(5) {
            panic!("visitor never received Frame 3 (T1)");
        }
        tokio::time::sleep(Duration::from_millis(15)).await;
    }

    // Verify host reports alternating T0 and T1 frames emitted
    let wait_frames_stat = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/svc")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let host_svc_rep: tetherly_core::ScreenSvcReport = serde_json::from_slice(&b).unwrap();
        if host_svc_rep.t0_frames_sent >= 2 && host_svc_rep.t1_frames_sent >= 2 {
            break;
        }
        if wait_frames_stat.elapsed() > Duration::from_secs(5) {
            panic!(
                "host never reported >= 2 T0 and >= 2 T1 frames sent: {:?}",
                host_svc_rep
            );
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 5. Congestion simulation: trigger Drop T1 via POST /api/screen/svc/drop-t1 (Tag 33)
    let drop_req = serde_json::json!({ "drop_t1": true });
    let (c, b) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/svc/drop-t1",
        &serde_json::to_vec(&drop_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let vis_svc_rep: tetherly_core::ScreenSvcReport = serde_json::from_slice(&b).unwrap();
    assert!(vis_svc_rep.drop_t1);

    // Verify host synchronizes Tag 33 drop_t1 = true
    let wait_drop_t1 = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/svc")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let host_svc_rep: tetherly_core::ScreenSvcReport = serde_json::from_slice(&b).unwrap();
        if host_svc_rep.drop_t1 {
            break;
        }
        if wait_drop_t1.elapsed() > Duration::from_secs(5) {
            panic!("host never synchronized drop_t1 = true: {:?}", host_svc_rep);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // Host pushes Frame 4 (Base Layer T0: seq 4 % 2 == 0) -> MUST be sent and received
    tokio::time::sleep(PACER_SETTLE).await;
    source.update_rect(16, 16, 16, 16, [0xaa, 0xbb, 0xcc, 0xff]);
    let prev4 = visitor.screen_sink_snapshot().count();
    assert!(host.screen_send_frame().await.unwrap());
    let wait_f4 = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() > prev4 {
            break;
        }
        if wait_f4.elapsed() > Duration::from_secs(5) {
            panic!("visitor never received Frame 4 (T0 base under drop_t1)");
        }
        tokio::time::sleep(Duration::from_millis(15)).await;
    }

    // Host pushes Frame 5 (Enhancement Layer T1: seq 5 % 2 == 1) -> MUST be dropped at sender!
    tokio::time::sleep(PACER_SETTLE).await;
    source.update_rect(18, 18, 16, 16, [0xdd, 0xee, 0xff, 0xff]);
    let sent_f5 = host.screen_send_frame().await.unwrap();
    assert!(
        !sent_f5,
        "Frame 5 (T1) was expected to be dropped by SVC-T degradation policy"
    );

    // Verify host reports t1_frames_dropped >= 1 and saved_bandwidth_bytes > 0
    let wait_dropped = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/svc")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let host_svc_rep: tetherly_core::ScreenSvcReport = serde_json::from_slice(&b).unwrap();
        if host_svc_rep.t1_frames_dropped >= 1 && host_svc_rep.saved_bandwidth_bytes > 0 {
            break;
        }
        if wait_dropped.elapsed() > Duration::from_secs(5) {
            panic!("host never reported dropped T1 frame: {:?}", host_svc_rep);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 6. Restore transmission: disable drop_t1 via POST /api/screen/svc/drop-t1
    let restore_req = serde_json::json!({ "drop_t1": false });
    let (c, _) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/svc/drop-t1",
        &serde_json::to_vec(&restore_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);

    // Verify host synchronizes Tag 33 drop_t1 = false
    let wait_restore = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/svc")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let host_svc_rep: tetherly_core::ScreenSvcReport = serde_json::from_slice(&b).unwrap();
        if !host_svc_rep.drop_t1 {
            break;
        }
        if wait_restore.elapsed() > Duration::from_secs(5) {
            panic!(
                "host never synchronized drop_t1 = false: {:?}",
                host_svc_rep
            );
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // Host pushes Frame 6 (T0) and Frame 7 (T1) -> Both must be sent now
    tokio::time::sleep(PACER_SETTLE).await;
    source.update_rect(20, 20, 16, 16, [0x12, 0x34, 0x56, 0xff]);
    let prev6 = visitor.screen_sink_snapshot().count();
    assert!(host.screen_send_frame().await.unwrap());
    let wait_f6 = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() > prev6 {
            break;
        }
        if wait_f6.elapsed() > Duration::from_secs(5) {
            panic!("visitor never received Frame 6 (T0)");
        }
        tokio::time::sleep(Duration::from_millis(15)).await;
    }

    tokio::time::sleep(PACER_SETTLE).await;
    source.update_rect(22, 22, 16, 16, [0x78, 0x9a, 0xbc, 0xff]);
    let prev7 = visitor.screen_sink_snapshot().count();
    let sent_f7 = host.screen_send_frame().await.unwrap();
    assert!(
        sent_f7,
        "Frame 7 (T1) should be transmitted after drop_t1 disabled"
    );
    let wait_f7 = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() > prev7 {
            break;
        }
        if wait_f7.elapsed() > Duration::from_secs(5) {
            panic!("visitor never received Frame 7 (T1)");
        }
        tokio::time::sleep(Duration::from_millis(15)).await;
    }

    // 7. Verify status report telemetry reflects active SVC metrics
    let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let status_rep: tetherly_node::ScreenStatusReport = serde_json::from_slice(&b).unwrap();
    assert!(status_rep.svc_enabled);
    assert_eq!(status_rep.svc_mode, "adaptive");
    assert!(!status_rep.svc_drop_t1);
    assert!(status_rep.svc_t0_sent >= 3);
    assert!(status_rep.svc_t1_sent >= 2);
    assert!(status_rep.svc_t1_dropped >= 1);

    // 8. Reset SVC mode via POST /api/screen/svc/reset (Tag 34)
    let (c, b) = tetherly_node::loopback_post(visitor_ui, "/api/screen/svc/reset", b"{}")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let reset_rep: tetherly_core::ScreenSvcReport = serde_json::from_slice(&b).unwrap();
    assert!(!reset_rep.enabled);
    assert_eq!(reset_rep.mode, "disabled");

    // Verify host synchronizes Tag 34 reset
    let wait_host_reset = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/svc")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let host_svc_rep: tetherly_core::ScreenSvcReport = serde_json::from_slice(&b).unwrap();
        if !host_svc_rep.enabled && host_svc_rep.mode == "disabled" {
            break;
        }
        if wait_host_reset.elapsed() > Duration::from_secs(5) {
            panic!("host never synchronized SVC reset: {:?}", host_svc_rep);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 9. Clean shutdown
    host.shutdown();
    visitor.shutdown();
    drop(ga);
    drop(gb);
}

#[tokio::test]
async fn m5_39_screen_motion_vector_estimation_and_scroll_acceleration_pipeline() {
    let (da, ga) = temp_dir("host-motion");
    let (db, gb) = temp_dir("visitor-motion");
    let source = Arc::new(ControllableDeltaScreenSource::with_seq(64, 64, 0));
    source.set_pattern();
    let host = Node::start(cfg(da, "host", Some(source.clone()))).unwrap();
    let visitor = Node::start(cfg(db, "visitor", None)).unwrap();
    host.spawn_listeners().await.unwrap();
    visitor.spawn_listeners().await.unwrap();
    let pin = tetherly_core::pin::parse_pin("25170397").unwrap();
    host.set_pin(pin);
    visitor.set_pin(pin);
    host.dial(visitor.control_addr()).await.unwrap();
    wait_live(&host, Duration::from_secs(25)).await;
    wait_live(&visitor, Duration::from_secs(25)).await;

    let host_ui = host.spawn_ui().await.unwrap();
    let visitor_ui = visitor.spawn_ui().await.unwrap();
    let host_id = host.identity().device_id().clone();
    let visitor_id = visitor.identity().device_id().clone();

    wait_screen_port(&host, &visitor_id, Duration::from_secs(3)).await;
    wait_screen_port(&visitor, &host_id, Duration::from_secs(3)).await;

    // 1. Establish screen streaming session
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

    // 2. Initial state: Motion estimation is disabled by default
    let (c, b) = tetherly_node::loopback_get(visitor_ui, "/api/screen/motion")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let motion_rep: tetherly_core::ScreenMotionReport = serde_json::from_slice(&b).unwrap();
    assert!(motion_rep.ok);
    assert!(!motion_rep.enabled);
    assert_eq!(
        motion_rep.max_displacement,
        tetherly_core::SCREEN_MOTION_DEFAULT_MAX_DISPLACEMENT
    );
    assert_eq!(motion_rep.motion_frames_sent, 0);
    assert_eq!(motion_rep.fallback_frames_sent, 0);
    assert_eq!(motion_rep.saved_bandwidth_bytes, 0);

    // Initial status report telemetry
    let (c, b) = tetherly_node::loopback_get(visitor_ui, "/api/screen")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let status_rep: tetherly_node::ScreenStatusReport = serde_json::from_slice(&b).unwrap();
    assert!(!status_rep.motion_enabled);

    // Send initial baseline keyframe
    assert!(host.screen_send_frame().await.unwrap());
    let wait_base = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() >= 1 {
            break;
        }
        if wait_base.elapsed() > Duration::from_secs(5) {
            panic!("visitor never received baseline keyframe");
        }
        tokio::time::sleep(Duration::from_millis(15)).await;
    }
    // Verify baseline keyframe matches source
    let baseline_frame = visitor.screen_sink_snapshot().last().unwrap();
    assert_eq!(baseline_frame.bytes, source.pixels_snapshot());

    // 3. Visitor configures Motion Mode via POST /api/screen/motion (Tag 36)
    let motion_req = serde_json::json!({
        "enabled": true,
        "max_displacement": 32
    });
    let (c, b) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/motion",
        &serde_json::to_vec(&motion_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let vis_rep: tetherly_core::ScreenMotionReport = serde_json::from_slice(&b).unwrap();
    assert!(vis_rep.enabled);
    assert_eq!(vis_rep.max_displacement, 32);

    // Verify host receives Tag 36 and synchronizes motion config
    let wait_host_motion = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/motion")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let host_rep: tetherly_core::ScreenMotionReport = serde_json::from_slice(&b).unwrap();
        if host_rep.enabled && host_rep.max_displacement == 32 {
            break;
        }
        if wait_host_motion.elapsed() > Duration::from_secs(5) {
            panic!("host never synchronized motion config: {:?}", host_rep);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 4. Upward scroll: dy = -8 (scroll up 8 pixels, newly exposed bottom 8 rows)
    tokio::time::sleep(PACER_SETTLE).await;
    source.scroll_vertical(-8, [0xFE, 0xDC, 0xBA, 0xFF]);
    let prev_count = visitor.screen_sink_snapshot().count();
    assert!(host.screen_send_frame().await.unwrap());

    let wait_scroll1 = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() > prev_count {
            break;
        }
        if wait_scroll1.elapsed() > Duration::from_secs(5) {
            panic!("visitor never received upward scroll frame");
        }
        tokio::time::sleep(Duration::from_millis(15)).await;
    }

    // Verify host reports motion_frames_sent >= 1 and saved_bandwidth_bytes > 0
    let wait_motion_stat = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/motion")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let rep: tetherly_core::ScreenMotionReport = serde_json::from_slice(&b).unwrap();
        if rep.motion_frames_sent >= 1 && rep.saved_bandwidth_bytes > 0 {
            break;
        }
        if wait_motion_stat.elapsed() > Duration::from_secs(5) {
            panic!("host never incremented motion_frames_sent: {:?}", rep);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // Bit-exact verification: receiver reconstructed canvas matches host source 100%
    let reconstructed1 = visitor.screen_sink_snapshot().last().unwrap();
    assert_eq!(
        reconstructed1.bytes,
        source.pixels_snapshot(),
        "reconstructed frame after upward scroll must match source exactly"
    );

    // 5. Downward scroll: dy = 8 (scroll down 8 pixels, newly exposed top 8 rows)
    tokio::time::sleep(PACER_SETTLE).await;
    source.scroll_vertical(8, [0x12, 0x34, 0x56, 0xFF]);
    let prev_count2 = visitor.screen_sink_snapshot().count();
    assert!(host.screen_send_frame().await.unwrap());

    let wait_scroll2 = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() > prev_count2 {
            break;
        }
        if wait_scroll2.elapsed() > Duration::from_secs(5) {
            panic!("visitor never received downward scroll frame");
        }
        tokio::time::sleep(Duration::from_millis(15)).await;
    }

    // Verify host reports motion_frames_sent >= 2
    let wait_motion_stat2 = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/motion")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let rep: tetherly_core::ScreenMotionReport = serde_json::from_slice(&b).unwrap();
        if rep.motion_frames_sent >= 2 {
            break;
        }
        if wait_motion_stat2.elapsed() > Duration::from_secs(5) {
            panic!("host never reached motion_frames_sent >= 2: {:?}", rep);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // Bit-exact verification: receiver reconstructed canvas matches host source 100%
    let reconstructed2 = visitor.screen_sink_snapshot().last().unwrap();
    assert_eq!(
        reconstructed2.bytes,
        source.pixels_snapshot(),
        "reconstructed frame after downward scroll must match source exactly"
    );

    // 6. Non-scroll edit: fallback to standard differential/keyframe pipeline
    tokio::time::sleep(PACER_SETTLE).await;
    source.update_rect(16, 16, 8, 8, [0x99, 0x88, 0x77, 0xFF]);
    let prev_count3 = visitor.screen_sink_snapshot().count();
    assert!(host.screen_send_frame().await.unwrap());

    let wait_edit = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() > prev_count3 {
            break;
        }
        if wait_edit.elapsed() > Duration::from_secs(5) {
            panic!("visitor never received non-scroll fallback frame");
        }
        tokio::time::sleep(Duration::from_millis(15)).await;
    }

    // Verify host reports fallback_frames_sent >= 1
    let wait_fallback = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/motion")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let rep: tetherly_core::ScreenMotionReport = serde_json::from_slice(&b).unwrap();
        if rep.fallback_frames_sent >= 1 {
            break;
        }
        if wait_fallback.elapsed() > Duration::from_secs(5) {
            panic!("host never incremented fallback_frames_sent: {:?}", rep);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let reconstructed3 = visitor.screen_sink_snapshot().last().unwrap();
    assert_eq!(
        reconstructed3.bytes,
        source.pixels_snapshot(),
        "reconstructed frame after non-scroll edit must match source exactly"
    );

    // 7. Verify status report telemetry reflects motion metrics
    let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let status_rep: tetherly_node::ScreenStatusReport = serde_json::from_slice(&b).unwrap();
    assert!(status_rep.motion_enabled);
    assert_eq!(status_rep.motion_max_displacement, 32);
    assert!(status_rep.motion_frames_sent >= 2);
    assert!(status_rep.motion_fallback_frames_sent >= 1);
    assert!(status_rep.motion_saved_bytes > 0);

    // 8. Reset motion mode via POST /api/screen/motion/reset (Tag 37)
    let (c, b) = tetherly_node::loopback_post(visitor_ui, "/api/screen/motion/reset", b"{}")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let reset_rep: tetherly_core::ScreenMotionReport = serde_json::from_slice(&b).unwrap();
    assert!(!reset_rep.enabled);
    assert_eq!(
        reset_rep.max_displacement,
        tetherly_core::SCREEN_MOTION_DEFAULT_MAX_DISPLACEMENT
    );

    // Verify host synchronizes Tag 37 reset
    let wait_host_reset = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/motion")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let host_rep: tetherly_core::ScreenMotionReport = serde_json::from_slice(&b).unwrap();
        if !host_rep.enabled {
            break;
        }
        if wait_host_reset.elapsed() > Duration::from_secs(5) {
            panic!("host never synchronized motion reset: {:?}", host_rep);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 9. Clean shutdown
    host.shutdown();
    visitor.shutdown();
    drop(ga);
    drop(gb);
}

#[tokio::test]
async fn m5_40_screen_spatial_tile_grid_decomposition_and_streaming_pipeline() {
    let (da, ga) = temp_dir("host-tile");
    let (db, gb) = temp_dir("visitor-tile");
    let source = Arc::new(ControllableDeltaScreenSource::with_seq(128, 128, 0));
    source.set_pattern();
    let host = Node::start(cfg(da, "host", Some(source.clone()))).unwrap();
    let visitor = Node::start(cfg(db, "visitor", None)).unwrap();
    host.spawn_listeners().await.unwrap();
    visitor.spawn_listeners().await.unwrap();
    let pin = tetherly_core::pin::parse_pin("25170397").unwrap();
    host.set_pin(pin);
    visitor.set_pin(pin);
    host.dial(visitor.control_addr()).await.unwrap();
    wait_live(&host, Duration::from_secs(25)).await;
    wait_live(&visitor, Duration::from_secs(25)).await;

    let host_ui = host.spawn_ui().await.unwrap();
    let visitor_ui = visitor.spawn_ui().await.unwrap();
    let host_id = host.identity().device_id().clone();
    let visitor_id = visitor.identity().device_id().clone();

    wait_screen_port(&host, &visitor_id, Duration::from_secs(3)).await;
    wait_screen_port(&visitor, &host_id, Duration::from_secs(3)).await;

    // 1. Establish screen streaming session
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

    // 2. Initial state: Spatial Tile-Grid mode is disabled by default
    let (c, b) = tetherly_node::loopback_get(visitor_ui, "/api/screen/tile")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let tile_rep: tetherly_core::ScreenTileReport = serde_json::from_slice(&b).unwrap();
    assert!(tile_rep.ok);
    assert!(!tile_rep.enabled);
    assert_eq!(tile_rep.tile_size, tetherly_core::SCREEN_TILE_DEFAULT_SIZE);
    assert_eq!(tile_rep.grid_frames_sent, 0);
    assert_eq!(tile_rep.dirty_tiles_sent, 0);
    assert_eq!(tile_rep.skipped_tiles_count, 0);
    assert_eq!(tile_rep.saved_bandwidth_bytes, 0);

    // Initial status report telemetry
    let (c, b) = tetherly_node::loopback_get(visitor_ui, "/api/screen")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let status_rep: tetherly_node::ScreenStatusReport = serde_json::from_slice(&b).unwrap();
    assert!(!status_rep.tile_enabled);

    // Send initial baseline keyframe
    assert!(host.screen_send_frame().await.unwrap());
    let wait_base = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() >= 1 {
            break;
        }
        if wait_base.elapsed() > Duration::from_secs(5) {
            panic!("visitor never received baseline keyframe");
        }
        tokio::time::sleep(Duration::from_millis(15)).await;
    }
    // Verify baseline keyframe matches source
    let baseline_frame = visitor.screen_sink_snapshot().last().unwrap();
    assert_eq!(baseline_frame.bytes, source.pixels_snapshot());

    // 3. Visitor configures Spatial Tile-Grid mode via POST /api/screen/tile (Tag 39)
    let tile_req = serde_json::json!({
        "enabled": true,
        "tile_size": 32
    });
    let (c, b) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/tile",
        &serde_json::to_vec(&tile_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let vis_rep: tetherly_core::ScreenTileReport = serde_json::from_slice(&b).unwrap();
    assert!(vis_rep.enabled);
    assert_eq!(vis_rep.tile_size, 32);

    // Verify host receives Tag 39 and synchronizes tile config
    let wait_host_tile = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/tile")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let host_rep: tetherly_core::ScreenTileReport = serde_json::from_slice(&b).unwrap();
        if host_rep.enabled && host_rep.tile_size == 32 {
            break;
        }
        if wait_host_tile.elapsed() > Duration::from_secs(5) {
            panic!("host never synchronized tile config: {:?}", host_rep);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 4. Disjoint discrete micro-updates: modify top-left (Tile 0) and bottom-right (Tile 15)
    // In a 128x128 screen with tile_size = 32, there are (128/32)*(128/32) = 16 tiles total.
    tokio::time::sleep(PACER_SETTLE).await;
    source.update_rect(2, 2, 4, 4, [0xAA, 0xBB, 0xCC, 0xFF]);
    source.update_rect(120, 120, 4, 4, [0x11, 0x22, 0x33, 0xFF]);
    let prev_count = visitor.screen_sink_snapshot().count();
    assert!(host.screen_send_frame().await.unwrap());

    let wait_tile1 = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() > prev_count {
            break;
        }
        if wait_tile1.elapsed() > Duration::from_secs(5) {
            panic!("visitor never received tile grid frame");
        }
        tokio::time::sleep(Duration::from_millis(15)).await;
    }

    // Verify host reports grid_frames_sent >= 1, dirty_tiles_sent == 2, skipped_tiles_count == 14, saved_bandwidth_bytes > 0
    let wait_tile_stat = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/tile")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let rep: tetherly_core::ScreenTileReport = serde_json::from_slice(&b).unwrap();
        if rep.grid_frames_sent >= 1
            && rep.dirty_tiles_sent >= 2
            && rep.skipped_tiles_count >= 14
            && rep.saved_bandwidth_bytes > 0
        {
            break;
        }
        if wait_tile_stat.elapsed() > Duration::from_secs(5) {
            panic!("host never reported tile streaming stats: {:?}", rep);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // Bit-exact verification: receiver reconstructed canvas matches host source 100%
    let reconstructed1 = visitor.screen_sink_snapshot().last().unwrap();
    assert_eq!(
        reconstructed1.bytes,
        source.pixels_snapshot(),
        "reconstructed frame after tile streaming must match source exactly"
    );

    // 5. Another discrete update: single tile update at Tile 5 (x=36, y=36, w=4, h=4)
    tokio::time::sleep(PACER_SETTLE).await;
    source.update_rect(36, 36, 4, 4, [0x55, 0x66, 0x77, 0xFF]);
    let prev_count2 = visitor.screen_sink_snapshot().count();
    assert!(host.screen_send_frame().await.unwrap());

    let wait_tile2 = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() > prev_count2 {
            break;
        }
        if wait_tile2.elapsed() > Duration::from_secs(5) {
            panic!("visitor never received second tile grid frame");
        }
        tokio::time::sleep(Duration::from_millis(15)).await;
    }

    // Verify host reports grid_frames_sent >= 2, dirty_tiles_sent >= 3, skipped_tiles_count >= 29
    let wait_tile_stat2 = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/tile")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let rep: tetherly_core::ScreenTileReport = serde_json::from_slice(&b).unwrap();
        if rep.grid_frames_sent >= 2 && rep.dirty_tiles_sent >= 3 && rep.skipped_tiles_count >= 29 {
            break;
        }
        if wait_tile_stat2.elapsed() > Duration::from_secs(5) {
            panic!("host never reached grid_frames_sent >= 2: {:?}", rep);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let reconstructed2 = visitor.screen_sink_snapshot().last().unwrap();
    assert_eq!(
        reconstructed2.bytes,
        source.pixels_snapshot(),
        "reconstructed frame after second tile update must match source exactly"
    );

    // 6. Verify status report telemetry reflects tile metrics
    let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let status_rep: tetherly_node::ScreenStatusReport = serde_json::from_slice(&b).unwrap();
    assert!(status_rep.tile_enabled);
    assert_eq!(status_rep.tile_size, 32);
    assert!(status_rep.tile_frames_sent >= 2);
    assert!(status_rep.dirty_tiles_sent >= 3);
    assert!(status_rep.skipped_tiles_count >= 29);
    assert!(status_rep.tile_saved_bytes > 0);

    // 7. Reset tile mode via POST /api/screen/tile/reset (Tag 40)
    let (c, b) = tetherly_node::loopback_post(visitor_ui, "/api/screen/tile/reset", b"{}")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let reset_rep: tetherly_core::ScreenTileReport = serde_json::from_slice(&b).unwrap();
    assert!(!reset_rep.enabled);
    assert_eq!(reset_rep.tile_size, tetherly_core::SCREEN_TILE_DEFAULT_SIZE);

    // Verify host synchronizes Tag 40 reset
    let wait_host_reset = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/tile")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let host_rep: tetherly_core::ScreenTileReport = serde_json::from_slice(&b).unwrap();
        if !host_rep.enabled {
            break;
        }
        if wait_host_reset.elapsed() > Duration::from_secs(5) {
            panic!("host never synchronized tile reset: {:?}", host_rep);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 8. Clean shutdown
    host.shutdown();
    visitor.shutdown();
    drop(ga);
    drop(gb);
}

#[tokio::test]
async fn m5_41_screen_tile_cache_deduplication_and_streaming_pipeline() {
    let (da, ga) = temp_dir("host-tile-cache");
    let (db, gb) = temp_dir("visitor-tile-cache");
    let source = Arc::new(ControllableDeltaScreenSource::with_seq(128, 128, 0));
    source.set_pattern();
    let host = Node::start(cfg(da, "host", Some(source.clone()))).unwrap();
    let visitor = Node::start(cfg(db, "visitor", None)).unwrap();
    host.spawn_listeners().await.unwrap();
    visitor.spawn_listeners().await.unwrap();
    let pin = tetherly_core::pin::parse_pin("25170398").unwrap();
    host.set_pin(pin);
    visitor.set_pin(pin);
    host.dial(visitor.control_addr()).await.unwrap();
    wait_live(&host, Duration::from_secs(25)).await;
    wait_live(&visitor, Duration::from_secs(25)).await;

    let host_ui = host.spawn_ui().await.unwrap();
    let visitor_ui = visitor.spawn_ui().await.unwrap();
    let host_id = host.identity().device_id().clone();
    let visitor_id = visitor.identity().device_id().clone();

    wait_screen_port(&host, &visitor_id, Duration::from_secs(3)).await;
    wait_screen_port(&visitor, &host_id, Duration::from_secs(3)).await;

    // 1. Establish screen streaming session
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

    // 2. Initial state: Tile dictionary cache is disabled by default
    let (c, b) = tetherly_node::loopback_get(visitor_ui, "/api/screen/tile/cache")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let cache_rep: tetherly_core::ScreenTileCacheReport = serde_json::from_slice(&b).unwrap();
    assert!(cache_rep.ok);
    assert!(!cache_rep.enabled);
    assert_eq!(
        cache_rep.max_entries,
        tetherly_core::SCREEN_TILE_CACHE_DEFAULT_MAX_ENTRIES
    );
    assert_eq!(cache_rep.current_entries, 0);
    assert_eq!(cache_rep.cache_hits, 0);
    assert_eq!(cache_rep.cache_misses, 0);
    assert_eq!(cache_rep.saved_bandwidth_bytes, 0);

    // Initial status report telemetry check
    let (c, b) = tetherly_node::loopback_get(visitor_ui, "/api/screen")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let status_rep: tetherly_node::ScreenStatusReport = serde_json::from_slice(&b).unwrap();
    assert!(!status_rep.tile_cache_enabled);

    // Send initial baseline keyframe
    assert!(host.screen_send_frame().await.unwrap());
    let wait_base = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() >= 1 {
            break;
        }
        if wait_base.elapsed() > Duration::from_secs(5) {
            panic!("visitor never received baseline keyframe");
        }
        tokio::time::sleep(Duration::from_millis(15)).await;
    }
    // Verify baseline keyframe matches source
    let baseline_frame = visitor.screen_sink_snapshot().last().unwrap();
    assert_eq!(baseline_frame.bytes, source.pixels_snapshot());

    // 3. Visitor enables spatial tile-grid streaming (tile_size = 32)
    let tile_req = serde_json::json!({
        "enabled": true,
        "tile_size": 32
    });
    let (c, b) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/tile",
        &serde_json::to_vec(&tile_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let tile_rep: tetherly_core::ScreenTileReport = serde_json::from_slice(&b).unwrap();
    assert!(tile_rep.enabled);

    // Verify host receives Tag 39 and synchronizes tile config
    let wait_host_tile = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/tile")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let host_rep: tetherly_core::ScreenTileReport = serde_json::from_slice(&b).unwrap();
        if host_rep.enabled && host_rep.tile_size == 32 {
            break;
        }
        if wait_host_tile.elapsed() > Duration::from_secs(5) {
            panic!("host never synchronized tile config: {:?}", host_rep);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 4. Visitor configures Tile Dictionary Cache via POST /api/screen/tile/cache (Tag 42)
    let cache_req = serde_json::json!({
        "enabled": true,
        "max_entries": 64
    });
    let (c, b) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/tile/cache",
        &serde_json::to_vec(&cache_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let vis_cache_rep: tetherly_core::ScreenTileCacheReport = serde_json::from_slice(&b).unwrap();
    assert!(vis_cache_rep.enabled);
    assert_eq!(vis_cache_rep.max_entries, 64);

    // Verify host receives Tag 42 and synchronizes tile cache config
    let wait_host_cache = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/tile/cache")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let host_rep: tetherly_core::ScreenTileCacheReport = serde_json::from_slice(&b).unwrap();
        if host_rep.enabled && host_rep.max_entries == 64 {
            break;
        }
        if wait_host_cache.elapsed() > Duration::from_secs(5) {
            panic!("host never synchronized tile cache config: {:?}", host_rep);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 5. First Frame: Cache Miss & Dynamic Learning
    // Update Tile 0 (x=0..32, y=0..32) with a distinctive block pattern
    tokio::time::sleep(PACER_SETTLE).await;
    source.update_rect(0, 0, 32, 32, [0x11, 0x22, 0x33, 0xFF]);
    let prev_count1 = visitor.screen_sink_snapshot().count();
    assert!(host.screen_send_frame().await.unwrap());

    let wait_frame1 = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() > prev_count1 {
            break;
        }
        if wait_frame1.elapsed() > Duration::from_secs(5) {
            panic!("visitor never received Frame 1 (tile cache miss)");
        }
        tokio::time::sleep(Duration::from_millis(15)).await;
    }

    // Verify host reports cache_misses >= 1 and current_entries >= 1
    let wait_miss_stat = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/tile/cache")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let rep: tetherly_core::ScreenTileCacheReport = serde_json::from_slice(&b).unwrap();
        if rep.cache_misses >= 1 && rep.current_entries >= 1 {
            break;
        }
        if wait_miss_stat.elapsed() > Duration::from_secs(5) {
            panic!("host never reported tile cache miss: {:?}", rep);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // Bit-exact canvas check: receiver reconstructed canvas matches host source 100%
    let reconstructed1 = visitor.screen_sink_snapshot().last().unwrap();
    assert_eq!(
        reconstructed1.bytes,
        source.pixels_snapshot(),
        "reconstructed frame after tile cache miss must match source exactly"
    );

    // 6. Second Frame: Spatial Translation (Window Movement Simulation) -> Cache Hit!
    // Move the exact same 32x32 block from Tile 0 to Tile 15 (x=96..128, y=96..128), and reset Tile 0 to dark gray
    tokio::time::sleep(PACER_SETTLE).await;
    source.update_rect(0, 0, 32, 32, [0x05, 0x05, 0x05, 0xFF]);
    source.update_rect(96, 96, 32, 32, [0x11, 0x22, 0x33, 0xFF]);
    let prev_count2 = visitor.screen_sink_snapshot().count();
    assert!(host.screen_send_frame().await.unwrap());

    let wait_frame2 = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() > prev_count2 {
            break;
        }
        if wait_frame2.elapsed() > Duration::from_secs(5) {
            panic!("visitor never received Frame 2 (tile cache hit)");
        }
        tokio::time::sleep(Duration::from_millis(15)).await;
    }

    // Verify host reports cache_hits >= 1 and saved_bandwidth_bytes > 0!
    let wait_hit_stat = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/tile/cache")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let rep: tetherly_core::ScreenTileCacheReport = serde_json::from_slice(&b).unwrap();
        if rep.cache_hits >= 1 && rep.saved_bandwidth_bytes > 0 {
            break;
        }
        if wait_hit_stat.elapsed() > Duration::from_secs(5) {
            panic!("host never reported tile cache hit: {:?}", rep);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // Bit-exact canvas check: receiver reconstructed canvas with cache hit matches source 100%!
    let reconstructed2 = visitor.screen_sink_snapshot().last().unwrap();
    assert_eq!(
        reconstructed2.bytes,
        source.pixels_snapshot(),
        "reconstructed frame with spatial cache hit must match source exactly"
    );

    // 7. Third Frame: Intra-Frame Spatial Pattern Repetition (Multi-Tile Deduplication)
    // Simultaneously update Tile 1 (x=32..64, y=0..32) and Tile 2 (x=64..96, y=0..32) with identical pixels
    tokio::time::sleep(PACER_SETTLE).await;
    source.update_rect(32, 0, 32, 32, [0x77, 0x88, 0x99, 0xFF]);
    source.update_rect(64, 0, 32, 32, [0x77, 0x88, 0x99, 0xFF]);
    let prev_count3 = visitor.screen_sink_snapshot().count();
    assert!(host.screen_send_frame().await.unwrap());

    let wait_frame3 = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() > prev_count3 {
            break;
        }
        if wait_frame3.elapsed() > Duration::from_secs(5) {
            panic!("visitor never received Frame 3 (intra-frame tile deduplication)");
        }
        tokio::time::sleep(Duration::from_millis(15)).await;
    }

    // Verify host reports increased cache_hits (at least 2 hits now)
    let wait_hit_stat2 = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/tile/cache")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let rep: tetherly_core::ScreenTileCacheReport = serde_json::from_slice(&b).unwrap();
        if rep.cache_hits >= 2 {
            assert!(rep.hit_ratio_pct > 0.0);
            break;
        }
        if wait_hit_stat2.elapsed() > Duration::from_secs(5) {
            panic!("host never reached cache_hits >= 2: {:?}", rep);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // Bit-exact canvas check: receiver canvas after intra-frame deduplication matches source 100%!
    let reconstructed3 = visitor.screen_sink_snapshot().last().unwrap();
    assert_eq!(
        reconstructed3.bytes,
        source.pixels_snapshot(),
        "reconstructed frame after intra-frame deduplication must match source exactly"
    );

    // 8. Verify status report telemetry reflects active tile cache metrics
    let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let status_rep: tetherly_node::ScreenStatusReport = serde_json::from_slice(&b).unwrap();
    assert!(status_rep.tile_cache_enabled);
    assert!(status_rep.tile_cache_entries >= 2);
    assert!(status_rep.tile_cache_hits >= 2);
    assert!(status_rep.tile_cache_misses >= 1);
    assert!(status_rep.tile_cache_saved_bytes > 0);

    // 9. Reset tile cache mode via POST /api/screen/tile/cache/reset (Tag 43)
    let (c, b) = tetherly_node::loopback_post(visitor_ui, "/api/screen/tile/cache/reset", b"{}")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let reset_rep: tetherly_core::ScreenTileCacheReport = serde_json::from_slice(&b).unwrap();
    assert!(!reset_rep.enabled);
    assert_eq!(reset_rep.current_entries, 0);
    assert_eq!(reset_rep.cache_hits, 0);
    assert_eq!(reset_rep.cache_misses, 0);
    assert_eq!(reset_rep.saved_bandwidth_bytes, 0);

    // Verify host synchronizes Tag 43 reset
    let wait_host_reset = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/tile/cache")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let host_rep: tetherly_core::ScreenTileCacheReport = serde_json::from_slice(&b).unwrap();
        if !host_rep.enabled && host_rep.current_entries == 0 {
            break;
        }
        if wait_host_reset.elapsed() > Duration::from_secs(5) {
            panic!("host never synchronized tile cache reset: {:?}", host_rep);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 10. Clean shutdown
    host.shutdown();
    visitor.shutdown();
    drop(ga);
    drop(gb);
}

#[tokio::test]
async fn m5_42_screen_foveated_focus_priority_scheduling_pipeline() {
    let (da, ga) = temp_dir("host-focus");
    let (db, gb) = temp_dir("visitor-focus");
    let source = Arc::new(ControllableDeltaScreenSource::with_seq(128, 128, 0));
    source.set_pattern();
    let host = Node::start(cfg(da, "host", Some(source.clone()))).unwrap();
    let visitor = Node::start(cfg(db, "visitor", None)).unwrap();
    host.spawn_listeners().await.unwrap();
    visitor.spawn_listeners().await.unwrap();
    let pin = tetherly_core::pin::parse_pin("25170398").unwrap();
    host.set_pin(pin);
    visitor.set_pin(pin);
    host.dial(visitor.control_addr()).await.unwrap();
    wait_live(&host, Duration::from_secs(25)).await;
    wait_live(&visitor, Duration::from_secs(25)).await;

    let host_ui = host.spawn_ui().await.unwrap();
    let visitor_ui = visitor.spawn_ui().await.unwrap();
    let host_id = host.identity().device_id().clone();
    let visitor_id = visitor.identity().device_id().clone();

    wait_screen_port(&host, &visitor_id, Duration::from_secs(3)).await;
    wait_screen_port(&visitor, &host_id, Duration::from_secs(3)).await;

    // 1. Establish screen streaming session
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

    // 2. Initial state: Foveated focus is disabled by default
    let (c, b) = tetherly_node::loopback_get(visitor_ui, "/api/screen/focus")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let focus_rep: tetherly_core::ScreenFocusReport = serde_json::from_slice(&b).unwrap();
    assert!(focus_rep.ok);
    assert!(!focus_rep.enabled);
    assert_eq!(focus_rep.mode, tetherly_core::ScreenFocusMode::Cursor);
    assert_eq!(focus_rep.focus_tiles_sent, 0);
    assert_eq!(focus_rep.peripheral_tiles_sent, 0);
    assert_eq!(focus_rep.peripheral_tiles_skipped, 0);
    assert_eq!(focus_rep.saved_bandwidth_bytes, 0);

    // Initial status report telemetry check
    let (c, b) = tetherly_node::loopback_get(visitor_ui, "/api/screen")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let status_rep: tetherly_node::ScreenStatusReport = serde_json::from_slice(&b).unwrap();
    assert!(!status_rep.focus_enabled);

    // 3. Send initial baseline keyframe
    assert!(host.screen_send_frame().await.unwrap());
    let wait_base = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() >= 1 {
            break;
        }
        if wait_base.elapsed() > Duration::from_secs(5) {
            panic!("visitor never received baseline keyframe");
        }
        tokio::time::sleep(Duration::from_millis(15)).await;
    }
    let baseline_frame = visitor.screen_sink_snapshot().last().unwrap();
    assert_eq!(baseline_frame.bytes, source.pixels_snapshot());

    // 4. Visitor enables spatial tile-grid streaming (tile_size = 32)
    let tile_req = serde_json::json!({
        "enabled": true,
        "tile_size": 32
    });
    let (c, b) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/tile",
        &serde_json::to_vec(&tile_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let tile_rep: tetherly_core::ScreenTileReport = serde_json::from_slice(&b).unwrap();
    assert!(tile_rep.enabled);

    // Wait for host to synchronize tile config
    let wait_host_tile = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/tile")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let host_rep: tetherly_core::ScreenTileReport = serde_json::from_slice(&b).unwrap();
        if host_rep.enabled && host_rep.tile_size == 32 {
            break;
        }
        if wait_host_tile.elapsed() > Duration::from_secs(5) {
            panic!("host never synchronized tile config: {:?}", host_rep);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 5. Visitor configures Foveated Focus in Box mode (Tag 45 & Tag 46)
    // Box: x=0..32, y=0..32 (Tile 0), peripheral_skip_ratio = 2 (cadence = 3 frames)
    let focus_req = serde_json::json!({
        "enabled": true,
        "mode": "box",
        "box_x": 0,
        "box_y": 0,
        "box_w": 32,
        "box_h": 32,
        "peripheral_skip_ratio": 2
    });
    let (c, b) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/focus",
        &serde_json::to_vec(&focus_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let vis_focus_rep: tetherly_core::ScreenFocusReport = serde_json::from_slice(&b).unwrap();
    assert!(vis_focus_rep.enabled);
    assert_eq!(vis_focus_rep.mode, tetherly_core::ScreenFocusMode::Box);
    assert_eq!(vis_focus_rep.box_w, 32);
    assert_eq!(vis_focus_rep.peripheral_skip_ratio, 2);

    // Verify host receives Tag 45 & 46 and synchronizes focus config
    let wait_host_focus = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/focus")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let host_rep: tetherly_core::ScreenFocusReport = serde_json::from_slice(&b).unwrap();
        if host_rep.enabled
            && host_rep.mode == tetherly_core::ScreenFocusMode::Box
            && host_rep.box_w == 32
        {
            break;
        }
        if wait_host_focus.elapsed() > Duration::from_secs(5) {
            panic!("host never synchronized focus config: {:?}", host_rep);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 6. Frame 1 (seq = 1, seq % 3 != 0): Focus vs Peripheral Spatial Priority
    // Update Focus zone (Tile 0, x=0..32, y=0..32) and Peripheral zone (Tile 15, x=96..128, y=96..128)
    tokio::time::sleep(PACER_SETTLE).await;
    source.update_rect(0, 0, 32, 32, [0xAA, 0xBB, 0xCC, 0xFF]);
    source.update_rect(96, 96, 32, 32, [0x11, 0x22, 0x33, 0xFF]);
    let prev_count1 = visitor.screen_sink_snapshot().count();
    assert!(host.screen_send_frame().await.unwrap());

    let wait_frame1 = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() > prev_count1 {
            break;
        }
        if wait_frame1.elapsed() > Duration::from_secs(5) {
            panic!("visitor never received Frame 1");
        }
        tokio::time::sleep(Duration::from_millis(15)).await;
    }

    // Verify host reports: focus_tiles_sent >= 1, peripheral_tiles_skipped >= 1, saved_bandwidth_bytes > 0
    let wait_stat1 = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/focus")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let rep: tetherly_core::ScreenFocusReport = serde_json::from_slice(&b).unwrap();
        if rep.focus_tiles_sent >= 1
            && rep.peripheral_tiles_skipped >= 1
            && rep.saved_bandwidth_bytes > 0
        {
            break;
        }
        if wait_stat1.elapsed() > Duration::from_secs(5) {
            panic!(
                "host never reported focus sent and peripheral skipped: {:?}",
                rep
            );
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // Check receiver canvas after Frame 1:
    // Focus zone (Tile 0) is updated, while Peripheral zone (Tile 15) was skipped and holds baseline pixels!
    let reconstructed1 = visitor.screen_sink_snapshot().last().unwrap();
    let focus_pixel = &reconstructed1.bytes[0..4];
    assert_eq!(
        focus_pixel,
        &[0xAA, 0xBB, 0xCC, 0xFF],
        "Focus zone pixel must be updated in Frame 1"
    );
    // Tile 15 start pixel index: (96 * 128 + 96) * 4 = 12384 * 4 = 49536
    let periph_idx = ((96 * 128 + 96) * 4) as usize;
    let periph_pixel = &reconstructed1.bytes[periph_idx..periph_idx + 4];
    assert_ne!(
        periph_pixel,
        &[0x11, 0x22, 0x33, 0xFF],
        "Peripheral pixel must be skipped in Frame 1"
    );

    // 7. Frame 2 (seq = 2, seq % 3 != 0): Peripheral tile remains skipped
    tokio::time::sleep(PACER_SETTLE).await;
    assert!(host.screen_send_frame().await.unwrap());
    // Give time for frame processing
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Verify peripheral_tiles_skipped increased
    let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/focus")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let rep: tetherly_core::ScreenFocusReport = serde_json::from_slice(&b).unwrap();
    assert!(
        rep.peripheral_tiles_skipped >= 2,
        "peripheral tile skipped count must increment on Frame 2"
    );

    // 8. Frame 3 (seq = 3, seq % 3 == 0): Cadence tick! Peripheral tile is transmitted!
    tokio::time::sleep(PACER_SETTLE).await;
    let prev_count3 = visitor.screen_sink_snapshot().count();
    assert!(host.screen_send_frame().await.unwrap());

    let wait_frame3 = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() > prev_count3 {
            break;
        }
        if wait_frame3.elapsed() > Duration::from_secs(5) {
            panic!("visitor never received Frame 3 (cadence tick)");
        }
        tokio::time::sleep(Duration::from_millis(15)).await;
    }

    // Verify host reports peripheral_tiles_sent >= 1
    let wait_stat3 = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/focus")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let rep: tetherly_core::ScreenFocusReport = serde_json::from_slice(&b).unwrap();
        if rep.peripheral_tiles_sent >= 1 {
            break;
        }
        if wait_stat3.elapsed() > Duration::from_secs(5) {
            panic!("host never reported peripheral_tiles_sent: {:?}", rep);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // Bit-exact canvas check: After cadence tick, visitor reconstructed canvas matches source 100%!
    let reconstructed3 = visitor.screen_sink_snapshot().last().unwrap();
    assert_eq!(
        reconstructed3.bytes,
        source.pixels_snapshot(),
        "reconstructed frame after cadence tick must match source exactly (Canvas Invariance)"
    );

    // 9. Switch to Cursor mode via POST /api/screen/focus
    let cursor_req = serde_json::json!({
        "enabled": true,
        "mode": "cursor",
        "radius": 128,
        "peripheral_skip_ratio": 1
    });
    let (c, b) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/focus",
        &serde_json::to_vec(&cursor_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let cur_rep: tetherly_core::ScreenFocusReport = serde_json::from_slice(&b).unwrap();
    assert!(cur_rep.enabled);
    assert_eq!(cur_rep.mode, tetherly_core::ScreenFocusMode::Cursor);
    assert_eq!(cur_rep.radius, 128);
    assert_eq!(cur_rep.peripheral_skip_ratio, 1);

    // Verify host synchronizes Cursor mode
    let wait_cursor_sync = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/focus")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let host_rep: tetherly_core::ScreenFocusReport = serde_json::from_slice(&b).unwrap();
        if host_rep.mode == tetherly_core::ScreenFocusMode::Cursor
            && host_rep.radius == 128
            && host_rep.peripheral_skip_ratio == 1
        {
            break;
        }
        if wait_cursor_sync.elapsed() > Duration::from_secs(5) {
            panic!("host never synchronized cursor mode: {:?}", host_rep);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 10. Verify host status report contains focus telemetry
    let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let status_rep: tetherly_node::ScreenStatusReport = serde_json::from_slice(&b).unwrap();
    assert!(status_rep.focus_enabled);
    assert_eq!(status_rep.focus_mode, "cursor");
    assert_eq!(status_rep.focus_radius, 128);
    assert_eq!(status_rep.focus_peripheral_skip_ratio, 1);
    assert!(status_rep.focus_tiles_sent >= 1);
    assert!(status_rep.peripheral_tiles_sent >= 1);
    assert!(status_rep.peripheral_tiles_skipped >= 2);
    assert!(status_rep.focus_saved_bandwidth_bytes > 0);

    // 11. Reset Focus mode via POST /api/screen/focus/reset (Tag 47)
    let (c, b) = tetherly_node::loopback_post(visitor_ui, "/api/screen/focus/reset", b"{}")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let reset_rep: tetherly_core::ScreenFocusReport = serde_json::from_slice(&b).unwrap();
    assert!(!reset_rep.enabled);
    assert_eq!(reset_rep.focus_tiles_sent, 0);
    assert_eq!(reset_rep.peripheral_tiles_sent, 0);
    assert_eq!(reset_rep.peripheral_tiles_skipped, 0);
    assert_eq!(reset_rep.saved_bandwidth_bytes, 0);

    // Verify host synchronizes Tag 47 reset
    let wait_host_reset = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/focus")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let host_rep: tetherly_core::ScreenFocusReport = serde_json::from_slice(&b).unwrap();
        if !host_rep.enabled
            && host_rep.focus_tiles_sent == 0
            && host_rep.peripheral_tiles_skipped == 0
        {
            break;
        }
        if wait_host_reset.elapsed() > Duration::from_secs(5) {
            panic!("host never synchronized focus reset: {:?}", host_rep);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 12. Clean shutdown
    host.shutdown();
    visitor.shutdown();
    drop(ga);
    drop(gb);
}

#[tokio::test]
async fn m5_43_screen_tile_palette_adaptive_compression_pipeline() {
    let (da, ga) = temp_dir("host-palette");
    let (db, gb) = temp_dir("visitor-palette");
    let source = Arc::new(ControllableDeltaScreenSource::with_seq(128, 128, 0));
    source.set_pattern();
    let host = Node::start(cfg(da, "host", Some(source.clone()))).unwrap();
    let visitor = Node::start(cfg(db, "visitor", None)).unwrap();
    host.spawn_listeners().await.unwrap();
    visitor.spawn_listeners().await.unwrap();
    let pin = tetherly_core::pin::parse_pin("25170398").unwrap();
    host.set_pin(pin);
    visitor.set_pin(pin);
    host.dial(visitor.control_addr()).await.unwrap();
    wait_live(&host, Duration::from_secs(25)).await;
    wait_live(&visitor, Duration::from_secs(25)).await;

    let host_ui = host.spawn_ui().await.unwrap();
    let visitor_ui = visitor.spawn_ui().await.unwrap();
    let host_id = host.identity().device_id().clone();
    let visitor_id = visitor.identity().device_id().clone();

    wait_screen_port(&host, &visitor_id, Duration::from_secs(3)).await;
    wait_screen_port(&visitor, &host_id, Duration::from_secs(3)).await;

    // 1. Establish screen streaming session
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

    // 2. Initial state: Tile Palette is disabled by default
    let (c, b) = tetherly_node::loopback_get(visitor_ui, "/api/screen/tile/palette")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let pal_rep: tetherly_core::ScreenTilePaletteReport = serde_json::from_slice(&b).unwrap();
    assert!(pal_rep.ok);
    assert!(!pal_rep.enabled);
    assert_eq!(pal_rep.max_colors, 16);
    assert_eq!(pal_rep.solid_tiles_count, 0);
    assert_eq!(pal_rep.bilevel_tiles_count, 0);
    assert_eq!(pal_rep.palette_tiles_count, 0);
    assert_eq!(pal_rep.truecolor_tiles_count, 0);
    assert_eq!(pal_rep.saved_bandwidth_bytes, 0);

    // Initial status report telemetry check
    let (c, b) = tetherly_node::loopback_get(visitor_ui, "/api/screen")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let status_rep: tetherly_node::ScreenStatusReport = serde_json::from_slice(&b).unwrap();
    assert!(!status_rep.tile_palette_enabled);

    // 3. Send initial baseline keyframe
    assert!(host.screen_send_frame().await.unwrap());
    let wait_base = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() >= 1 {
            break;
        }
        if wait_base.elapsed() > Duration::from_secs(5) {
            panic!("visitor never received baseline keyframe");
        }
        tokio::time::sleep(Duration::from_millis(15)).await;
    }
    let baseline_frame = visitor.screen_sink_snapshot().last().unwrap();
    assert_eq!(baseline_frame.bytes, source.pixels_snapshot());

    // 4. Visitor enables spatial tile-grid streaming (tile_size = 64)
    let tile_req = serde_json::json!({
        "enabled": true,
        "tile_size": 64
    });
    let (c, b) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/tile",
        &serde_json::to_vec(&tile_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let tile_rep: tetherly_core::ScreenTileReport = serde_json::from_slice(&b).unwrap();
    assert!(tile_rep.enabled);

    // Wait for host to synchronize tile config
    let wait_host_tile = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/tile")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let host_rep: tetherly_core::ScreenTileReport = serde_json::from_slice(&b).unwrap();
        if host_rep.enabled && host_rep.tile_size == 64 {
            break;
        }
        if wait_host_tile.elapsed() > Duration::from_secs(5) {
            panic!("host never synchronized tile config: {:?}", host_rep);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 5. Visitor enables Tile-Level Adaptive Palette Compression (Tag 50)
    let pal_req = serde_json::json!({
        "enabled": true,
        "max_colors": 16,
        "enable_rle": false
    });
    let (c, b) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/tile/palette",
        &serde_json::to_vec(&pal_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let vis_pal_rep: tetherly_core::ScreenTilePaletteReport = serde_json::from_slice(&b).unwrap();
    assert!(vis_pal_rep.enabled);
    assert_eq!(vis_pal_rep.max_colors, 16);

    // Verify host receives Tag 50 and synchronizes palette config
    let wait_host_pal = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/tile/palette")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let host_rep: tetherly_core::ScreenTilePaletteReport = serde_json::from_slice(&b).unwrap();
        if host_rep.enabled && host_rep.max_colors == 16 {
            break;
        }
        if wait_host_pal.elapsed() > Duration::from_secs(5) {
            panic!("host never synchronized palette config: {:?}", host_rep);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 6. Frame 1 (Solid Color Tile, K=1):
    // Update Tile 0 (x=0..64, y=0..64) with solid blue
    tokio::time::sleep(PACER_SETTLE).await;
    source.update_rect(0, 0, 64, 64, [0x00, 0x00, 0xFF, 0xFF]);
    let prev_count1 = visitor.screen_sink_snapshot().count();
    assert!(host.screen_send_frame().await.unwrap());

    let wait_frame1 = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() > prev_count1 {
            break;
        }
        if wait_frame1.elapsed() > Duration::from_secs(5) {
            panic!("visitor never received Frame 1 (solid tile)");
        }
        tokio::time::sleep(Duration::from_millis(15)).await;
    }

    // Verify host reports solid_tiles_count >= 1 and saved_bandwidth_bytes > 0
    let wait_stat1 = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/tile/palette")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let rep: tetherly_core::ScreenTilePaletteReport = serde_json::from_slice(&b).unwrap();
        if rep.solid_tiles_count >= 1 && rep.saved_bandwidth_bytes > 0 {
            break;
        }
        if wait_stat1.elapsed() > Duration::from_secs(5) {
            panic!("host never reported solid tile: {:?}", rep);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // Canvas Invariance check for Frame 1
    let rec1 = visitor.screen_sink_snapshot().last().unwrap();
    assert_eq!(
        rec1.bytes,
        source.pixels_snapshot(),
        "reconstructed frame 1 must match source exactly (Canvas Invariance)"
    );
    assert_eq!(&rec1.bytes[0..4], &[0x00, 0x00, 0xFF, 0xFF]);

    // 7. Frame 2 (Bi-Level Tile, K=2):
    // Update Tile 1 (x=64..128, y=0..64) with alternating black and white pattern
    tokio::time::sleep(PACER_SETTLE).await;
    source.update_rect_with(64, 0, 64, 64, |col, row| {
        if (col + row) % 2 == 0 {
            [0x00, 0x00, 0x00, 0xFF]
        } else {
            [0xFF, 0xFF, 0xFF, 0xFF]
        }
    });
    let prev_count2 = visitor.screen_sink_snapshot().count();
    assert!(host.screen_send_frame().await.unwrap());

    let wait_frame2 = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() > prev_count2 {
            break;
        }
        if wait_frame2.elapsed() > Duration::from_secs(5) {
            panic!("visitor never received Frame 2 (bilevel tile)");
        }
        tokio::time::sleep(Duration::from_millis(15)).await;
    }

    // Verify host reports bilevel_tiles_count >= 1
    let wait_stat2 = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/tile/palette")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let rep: tetherly_core::ScreenTilePaletteReport = serde_json::from_slice(&b).unwrap();
        if rep.bilevel_tiles_count >= 1 {
            break;
        }
        if wait_stat2.elapsed() > Duration::from_secs(5) {
            panic!("host never reported bilevel tile: {:?}", rep);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // Canvas Invariance check for Frame 2
    let rec2 = visitor.screen_sink_snapshot().last().unwrap();
    assert_eq!(
        rec2.bytes,
        source.pixels_snapshot(),
        "reconstructed frame 2 must match source exactly (Canvas Invariance)"
    );

    // 8. Frame 3 (16-Color Indexed Palette Tile, 3 <= K <= 16):
    // Update Tile 2 (x=0..64, y=64..128) with 4-color syntax highlighting blocks
    tokio::time::sleep(PACER_SETTLE).await;
    source.update_rect_with(0, 64, 64, 64, |col, row| {
        let colors = [
            [0xFF, 0x00, 0x00, 0xFF],
            [0x00, 0xFF, 0x00, 0xFF],
            [0x00, 0x00, 0xFF, 0xFF],
            [0xFF, 0xFF, 0x00, 0xFF],
        ];
        colors[(col / 16 + row / 16) % 4]
    });
    let prev_count3 = visitor.screen_sink_snapshot().count();
    assert!(host.screen_send_frame().await.unwrap());

    let wait_frame3 = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() > prev_count3 {
            break;
        }
        if wait_frame3.elapsed() > Duration::from_secs(5) {
            panic!("visitor never received Frame 3 (palette tile)");
        }
        tokio::time::sleep(Duration::from_millis(15)).await;
    }

    // Verify host reports palette_tiles_count >= 1
    let wait_stat3 = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/tile/palette")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let rep: tetherly_core::ScreenTilePaletteReport = serde_json::from_slice(&b).unwrap();
        if rep.palette_tiles_count >= 1 {
            break;
        }
        if wait_stat3.elapsed() > Duration::from_secs(5) {
            panic!("host never reported palette tile: {:?}", rep);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // Canvas Invariance check for Frame 3
    let rec3 = visitor.screen_sink_snapshot().last().unwrap();
    assert_eq!(
        rec3.bytes,
        source.pixels_snapshot(),
        "reconstructed frame 3 must match source exactly (Canvas Invariance)"
    );

    // 9. Frame 4 (TrueColor Fallback, K > 16):
    // Update Tile 3 (x=64..128, y=64..128) with gradient (> 16 colors)
    tokio::time::sleep(PACER_SETTLE).await;
    source.update_rect_with(64, 64, 64, 64, |col, row| {
        [
            ((col * 4) & 0xFF) as u8,
            ((row * 4) & 0xFF) as u8,
            ((col ^ row) & 0xFF) as u8,
            0xFF,
        ]
    });
    let prev_count4 = visitor.screen_sink_snapshot().count();
    assert!(host.screen_send_frame().await.unwrap());

    let wait_frame4 = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() > prev_count4 {
            break;
        }
        if wait_frame4.elapsed() > Duration::from_secs(5) {
            panic!("visitor never received Frame 4 (truecolor fallback)");
        }
        tokio::time::sleep(Duration::from_millis(15)).await;
    }

    // Verify host reports truecolor_tiles_count >= 1
    let wait_stat4 = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/tile/palette")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let rep: tetherly_core::ScreenTilePaletteReport = serde_json::from_slice(&b).unwrap();
        if rep.truecolor_tiles_count >= 1 {
            break;
        }
        if wait_stat4.elapsed() > Duration::from_secs(5) {
            panic!("host never reported truecolor tile: {:?}", rep);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // Canvas Invariance check for Frame 4
    let rec4 = visitor.screen_sink_snapshot().last().unwrap();
    assert_eq!(
        rec4.bytes,
        source.pixels_snapshot(),
        "reconstructed frame 4 must match source exactly (Canvas Invariance)"
    );

    // 10. Verify host status report contains tile palette telemetry
    let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let status_rep: tetherly_node::ScreenStatusReport = serde_json::from_slice(&b).unwrap();
    assert!(status_rep.tile_palette_enabled);
    assert_eq!(status_rep.tile_palette_max_colors, 16);
    assert!(status_rep.tile_palette_solid_tiles >= 1);
    assert!(status_rep.tile_palette_bilevel_tiles >= 1);
    assert!(status_rep.tile_palette_palette_tiles >= 1);
    assert!(status_rep.tile_palette_truecolor_tiles >= 1);
    assert!(status_rep.tile_palette_saved_bytes > 0);

    // 11. Reset Tile Palette mode via POST /api/screen/tile/palette/reset (Tag 51)
    let (c, b) = tetherly_node::loopback_post(visitor_ui, "/api/screen/tile/palette/reset", b"{}")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let reset_rep: tetherly_core::ScreenTilePaletteReport = serde_json::from_slice(&b).unwrap();
    assert!(!reset_rep.enabled);
    assert_eq!(reset_rep.solid_tiles_count, 0);
    assert_eq!(reset_rep.bilevel_tiles_count, 0);
    assert_eq!(reset_rep.palette_tiles_count, 0);
    assert_eq!(reset_rep.truecolor_tiles_count, 0);
    assert_eq!(reset_rep.saved_bandwidth_bytes, 0);

    // Verify host synchronizes Tag 51 reset
    let wait_host_reset = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/tile/palette")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let host_rep: tetherly_core::ScreenTilePaletteReport = serde_json::from_slice(&b).unwrap();
        if !host_rep.enabled && host_rep.solid_tiles_count == 0 {
            break;
        }
        if wait_host_reset.elapsed() > Duration::from_secs(5) {
            panic!("host never synchronized palette reset: {:?}", host_rep);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 12. Clean shutdown
    host.shutdown();
    visitor.shutdown();
    drop(ga);
    drop(gb);
}

#[tokio::test]
async fn m5_44_screen_tile_predictive_compression_pipeline() {
    let (da, ga) = temp_dir("host-predict");
    let (db, gb) = temp_dir("visitor-predict");
    let source = Arc::new(ControllableDeltaScreenSource::with_seq(128, 128, 0));
    source.set_pattern();
    let host = Node::start(cfg(da, "host", Some(source.clone()))).unwrap();
    let visitor = Node::start(cfg(db, "visitor", None)).unwrap();
    host.spawn_listeners().await.unwrap();
    visitor.spawn_listeners().await.unwrap();
    let pin = tetherly_core::pin::parse_pin("25170398").unwrap();
    host.set_pin(pin);
    visitor.set_pin(pin);
    host.dial(visitor.control_addr()).await.unwrap();
    wait_live(&host, Duration::from_secs(25)).await;
    wait_live(&visitor, Duration::from_secs(25)).await;

    let host_ui = host.spawn_ui().await.unwrap();
    let visitor_ui = visitor.spawn_ui().await.unwrap();
    let host_id = host.identity().device_id().clone();
    let visitor_id = visitor.identity().device_id().clone();

    wait_screen_port(&host, &visitor_id, Duration::from_secs(3)).await;
    wait_screen_port(&visitor, &host_id, Duration::from_secs(3)).await;

    // 1. Establish screen streaming session
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
    wait_host_streaming(host_ui).await;

    // Initial status report telemetry check: Predict should be disabled
    let (c, b) = tetherly_node::loopback_get(visitor_ui, "/api/screen/tile/predict")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let init_rep: tetherly_core::ScreenTilePredictReport = serde_json::from_slice(&b).unwrap();
    assert!(!init_rep.enabled);

    // Send initial baseline keyframe
    assert!(host.screen_send_frame().await.unwrap());
    let wait_base = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() >= 1 {
            break;
        }
        if wait_base.elapsed() > Duration::from_secs(5) {
            panic!("visitor never received baseline keyframe");
        }
        tokio::time::sleep(Duration::from_millis(15)).await;
    }
    // Verify baseline keyframe matches source
    let baseline_frame = visitor.screen_sink_snapshot().last().unwrap();
    assert_eq!(baseline_frame.bytes, source.pixels_snapshot());

    // 2. Visitor enables spatial tile-grid streaming (tile_size = 64)
    let tile_req = serde_json::json!({
        "enabled": true,
        "tile_size": 64
    });
    let (c, b) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/tile",
        &serde_json::to_vec(&tile_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let tile_rep: tetherly_core::ScreenTileReport = serde_json::from_slice(&b).unwrap();
    assert!(tile_rep.enabled);

    // Verify host receives Tag 39 and synchronizes tile config
    let wait_host_tile = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/tile")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let host_rep: tetherly_core::ScreenTileReport = serde_json::from_slice(&b).unwrap();
        if host_rep.enabled && host_rep.tile_size == 64 {
            break;
        }
        if wait_host_tile.elapsed() > Duration::from_secs(5) {
            panic!("host never synchronized tile config: {:?}", host_rep);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 3. Visitor configures Predictive Filter via POST /api/screen/tile/predict (Tag 53)
    let predict_req = serde_json::json!({
        "enabled": true,
        "filter_mode": "auto",
        "enable_rct": true
    });
    let (c, b) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/tile/predict",
        &serde_json::to_vec(&predict_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let vis_pred_rep: tetherly_core::ScreenTilePredictReport = serde_json::from_slice(&b).unwrap();
    assert!(vis_pred_rep.enabled);
    assert_eq!(
        vis_pred_rep.filter_mode,
        tetherly_core::ScreenTilePredictFilter::Auto
    );
    assert!(vis_pred_rep.enable_rct);

    // Verify host receives Tag 53 and synchronizes predict config
    let wait_host_pred = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/tile/predict")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let host_rep: tetherly_core::ScreenTilePredictReport = serde_json::from_slice(&b).unwrap();
        if host_rep.enabled
            && host_rep.filter_mode == tetherly_core::ScreenTilePredictFilter::Auto
            && host_rep.enable_rct
        {
            break;
        }
        if wait_host_pred.elapsed() > Duration::from_secs(5) {
            panic!("host never synchronized predict config: {:?}", host_rep);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 4. Frame 1 (Horizontal Continuous Gradient Tile, K > 16):
    // Update Tile 0 (x=0..64, y=0..64) with smooth horizontal gradient
    tokio::time::sleep(PACER_SETTLE).await;
    source.update_rect_with(0, 0, 64, 64, |col, row| {
        let base_r = (col * 3) as u8;
        let base_g = (col * 3 + (row % 4)) as u8;
        let base_b = (col * 3 + 12) as u8;
        [base_r, base_g, base_b, 0xFF]
    });
    let prev_count1 = visitor.screen_sink_snapshot().count();
    assert!(host.screen_send_frame().await.unwrap());

    let wait_frame1 = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() > prev_count1 {
            break;
        }
        if wait_frame1.elapsed() > Duration::from_secs(5) {
            panic!("visitor never received Frame 1 (predict tile)");
        }
        tokio::time::sleep(Duration::from_millis(15)).await;
    }

    // Verify host reports predict_tiles_count >= 1 and saved_bandwidth_bytes > 0
    let wait_stat1 = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/tile/predict")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let rep: tetherly_core::ScreenTilePredictReport = serde_json::from_slice(&b).unwrap();
        if rep.predict_tiles_count >= 1 && rep.saved_bandwidth_bytes > 0 {
            break;
        }
        if wait_stat1.elapsed() > Duration::from_secs(5) {
            panic!("host never reported predict tile stats: {:?}", rep);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // Canvas Invariance check for Frame 1
    let rec1 = visitor.screen_sink_snapshot().last().unwrap();
    assert_eq!(
        rec1.bytes,
        source.pixels_snapshot(),
        "reconstructed frame 1 must match source exactly (Canvas Invariance)"
    );

    // 5. Frame 2 (Vertical Continuous Gradient Tile, K > 16):
    // Update Tile 1 (x=64..128, y=0..64) with vertical linear gradient
    tokio::time::sleep(PACER_SETTLE).await;
    source.update_rect_with(64, 0, 64, 64, |_col, row| {
        let base_r = (row * 3 + 20) as u8;
        let base_g = (row * 3 + 25) as u8;
        let base_b = (row * 3 + 30) as u8;
        [base_r, base_g, base_b, 0xFF]
    });
    let prev_count2 = visitor.screen_sink_snapshot().count();
    assert!(host.screen_send_frame().await.unwrap());

    let wait_frame2 = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() > prev_count2 {
            break;
        }
        if wait_frame2.elapsed() > Duration::from_secs(5) {
            panic!("visitor never received Frame 2");
        }
        tokio::time::sleep(Duration::from_millis(15)).await;
    }

    // Verify host reports predict_tiles_count >= 2
    let wait_stat2 = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/tile/predict")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let rep: tetherly_core::ScreenTilePredictReport = serde_json::from_slice(&b).unwrap();
        if rep.predict_tiles_count >= 2 {
            break;
        }
        if wait_stat2.elapsed() > Duration::from_secs(5) {
            panic!("host never reported predict tile count >= 2: {:?}", rep);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // Canvas Invariance check for Frame 2
    let rec2 = visitor.screen_sink_snapshot().last().unwrap();
    assert_eq!(
        rec2.bytes,
        source.pixels_snapshot(),
        "reconstructed frame 2 must match source exactly (Canvas Invariance)"
    );

    // 6. Frame 3 (Two-Dimensional Smooth Surface with Paeth / Diagonal Gradient):
    // Update Tile 2 (x=0..64, y=64..128)
    tokio::time::sleep(PACER_SETTLE).await;
    source.update_rect_with(0, 64, 64, 64, |col, row| {
        let v = ((col + row) * 2) as u8;
        [v, v.wrapping_add(10), v.wrapping_add(20), 0xFF]
    });
    let prev_count3 = visitor.screen_sink_snapshot().count();
    assert!(host.screen_send_frame().await.unwrap());

    let wait_frame3 = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() > prev_count3 {
            break;
        }
        if wait_frame3.elapsed() > Duration::from_secs(5) {
            panic!("visitor never received Frame 3");
        }
        tokio::time::sleep(Duration::from_millis(15)).await;
    }

    // Verify host reports predict_tiles_count >= 3
    let wait_stat3 = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/tile/predict")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let rep: tetherly_core::ScreenTilePredictReport = serde_json::from_slice(&b).unwrap();
        if rep.predict_tiles_count >= 3 {
            break;
        }
        if wait_stat3.elapsed() > Duration::from_secs(5) {
            panic!("host never reported predict tile count >= 3: {:?}", rep);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // Canvas Invariance check for Frame 3
    let rec3 = visitor.screen_sink_snapshot().last().unwrap();
    assert_eq!(
        rec3.bytes,
        source.pixels_snapshot(),
        "reconstructed frame 3 must match source exactly (Canvas Invariance)"
    );

    // 7. Verify host status report contains tile predict telemetry
    let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let status_rep: tetherly_node::ScreenStatusReport = serde_json::from_slice(&b).unwrap();
    assert!(status_rep.tile_predict_enabled);
    assert_eq!(status_rep.tile_predict_filter_mode, "auto");
    assert!(status_rep.tile_predict_enable_rct);
    assert!(status_rep.tile_predict_tiles_sent >= 3);
    assert!(status_rep.tile_predict_saved_bytes > 0);

    // 8. Reset Tile Predict mode via POST /api/screen/tile/predict/reset (Tag 54)
    let (c, b) = tetherly_node::loopback_post(visitor_ui, "/api/screen/tile/predict/reset", b"{}")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let reset_rep: tetherly_core::ScreenTilePredictReport = serde_json::from_slice(&b).unwrap();
    assert!(!reset_rep.enabled);
    assert_eq!(reset_rep.predict_tiles_count, 0);
    assert_eq!(reset_rep.saved_bandwidth_bytes, 0);

    // Verify host synchronizes Tag 54 reset
    let wait_host_reset = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/tile/predict")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let host_rep: tetherly_core::ScreenTilePredictReport = serde_json::from_slice(&b).unwrap();
        if !host_rep.enabled && host_rep.predict_tiles_count == 0 {
            break;
        }
        if wait_host_reset.elapsed() > Duration::from_secs(5) {
            panic!("host never synchronized predict reset: {:?}", host_rep);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 9. Clean shutdown
    host.shutdown();
    visitor.shutdown();
    drop(ga);
    drop(gb);
}

#[tokio::test]
async fn m5_45_screen_tile_presentation_transaction_pipeline() {
    let (da, ga) = temp_dir("host-present");
    let (db, gb) = temp_dir("visitor-present");
    let source = Arc::new(ControllableDeltaScreenSource::with_seq(128, 128, 0));
    source.set_pattern();
    let host = Node::start(cfg(da, "host", Some(source.clone()))).unwrap();
    let visitor = Node::start(cfg(db, "visitor", None)).unwrap();
    host.spawn_listeners().await.unwrap();
    visitor.spawn_listeners().await.unwrap();
    let pin = tetherly_core::pin::parse_pin("25170398").unwrap();
    host.set_pin(pin);
    visitor.set_pin(pin);
    host.dial(visitor.control_addr()).await.unwrap();
    wait_live(&host, Duration::from_secs(25)).await;
    wait_live(&visitor, Duration::from_secs(25)).await;

    let host_ui = host.spawn_ui().await.unwrap();
    let visitor_ui = visitor.spawn_ui().await.unwrap();
    let host_id = host.identity().device_id().clone();
    let visitor_id = visitor.identity().device_id().clone();

    wait_screen_port(&host, &visitor_id, Duration::from_secs(3)).await;
    wait_screen_port(&visitor, &host_id, Duration::from_secs(3)).await;

    // 1. Establish screen streaming session
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
    wait_host_streaming(host_ui).await;

    // Initial status report telemetry check: Present should be disabled
    let (c, b) = tetherly_node::loopback_get(visitor_ui, "/api/screen/tile/present")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let init_rep: tetherly_core::ScreenTilePresentReport = serde_json::from_slice(&b).unwrap();
    assert!(!init_rep.enabled);
    assert_eq!(init_rep.committed_transactions, 0);
    assert_eq!(init_rep.swapped_frames, 0);

    // Send initial baseline keyframe
    assert!(host.screen_send_frame().await.unwrap());
    let wait_base = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() >= 1 {
            break;
        }
        if wait_base.elapsed() > Duration::from_secs(5) {
            panic!("visitor never received baseline keyframe");
        }
        tokio::time::sleep(Duration::from_millis(15)).await;
    }
    // Verify baseline keyframe matches source
    let baseline_frame = visitor.screen_sink_snapshot().last().unwrap();
    assert_eq!(baseline_frame.bytes, source.pixels_snapshot());

    // 2. Visitor enables spatial tile-grid streaming (tile_size = 64)
    let tile_req = serde_json::json!({
        "enabled": true,
        "tile_size": 64
    });
    let (c, b) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/tile",
        &serde_json::to_vec(&tile_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let tile_rep: tetherly_core::ScreenTileReport = serde_json::from_slice(&b).unwrap();
    assert!(tile_rep.enabled);

    // Verify host receives Tag 39 and synchronizes tile config
    let wait_host_tile = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/tile")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let host_rep: tetherly_core::ScreenTileReport = serde_json::from_slice(&b).unwrap();
        if host_rep.enabled && host_rep.tile_size == 64 {
            break;
        }
        if wait_host_tile.elapsed() > Duration::from_secs(5) {
            panic!("host never synchronized tile config: {:?}", host_rep);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 3. Visitor configures Double-Buffered Presentation Transaction & Anti-Tearing V-Sync via POST /api/screen/tile/present (Tag 56)
    let present_req = serde_json::json!({
        "enabled": true,
        "vsync_mode": "vsync60",
        "double_buffer": true,
        "tear_prevention": true
    });
    let (c, b) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/tile/present",
        &serde_json::to_vec(&present_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let vis_pres_rep: tetherly_core::ScreenTilePresentReport = serde_json::from_slice(&b).unwrap();
    assert!(vis_pres_rep.enabled);
    assert_eq!(vis_pres_rep.vsync_mode, "vsync60");
    assert!(vis_pres_rep.double_buffer);
    assert!(vis_pres_rep.tear_prevention);

    // Verify host receives Tag 56 and synchronizes present config
    let wait_host_pres = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/tile/present")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let host_rep: tetherly_core::ScreenTilePresentReport = serde_json::from_slice(&b).unwrap();
        if host_rep.enabled && host_rep.vsync_mode == "vsync60" && host_rep.double_buffer {
            break;
        }
        if wait_host_pres.elapsed() > Duration::from_secs(5) {
            panic!("host never synchronized present config: {:?}", host_rep);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 4. Frame 1 (Multi-Tile Update: Tile 0 [0..64, 0..64] and Tile 3 [64..128, 64..128]):
    // Two dirty tiles in a single frame to verify atomic staging, anti-tearing guard and front-swap
    tokio::time::sleep(PACER_SETTLE).await;
    source.update_rect_with(0, 0, 64, 64, |col, row| {
        let r = (col * 3 + 10) as u8;
        let g = (row * 3 + 20) as u8;
        let b = 180u8;
        [r, g, b, 0xFF]
    });
    source.update_rect_with(64, 64, 64, 64, |col, row| {
        let r = 70u8;
        let g = (col * 2 + 50) as u8;
        let b = (row * 2 + 60) as u8;
        [r, g, b, 0xFF]
    });
    let prev_count1 = visitor.screen_sink_snapshot().count();
    assert!(host.screen_send_frame().await.unwrap());

    let wait_frame1 = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() > prev_count1 {
            break;
        }
        if wait_frame1.elapsed() > Duration::from_secs(5) {
            panic!("visitor never received Frame 1 (present transaction)");
        }
        tokio::time::sleep(Duration::from_millis(15)).await;
    }

    // Verify visitor reports presentation transaction telemetry:
    // committed_transactions >= 1, swapped_frames >= 1, torn_frames_prevented >= 1 (due to multi-tile dirty frame)
    let wait_stat1 = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(visitor_ui, "/api/screen/tile/present")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let rep: tetherly_core::ScreenTilePresentReport = serde_json::from_slice(&b).unwrap();
        if rep.committed_transactions >= 1
            && rep.swapped_frames >= 1
            && rep.torn_frames_prevented >= 1
        {
            break;
        }
        if wait_stat1.elapsed() > Duration::from_secs(5) {
            panic!(
                "visitor never reported present transaction metrics: {:?}",
                rep
            );
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // Canvas Invariance check for Frame 1
    let rec1 = visitor.screen_sink_snapshot().last().unwrap();
    assert_eq!(
        rec1.bytes,
        source.pixels_snapshot(),
        "reconstructed frame 1 must match source exactly (Canvas Invariance)"
    );

    // 5. Frame 2 (Another Multi-Tile Update: Tile 1 [64..128, 0..64] and Tile 2 [0..64, 64..128]):
    tokio::time::sleep(PACER_SETTLE).await;
    source.update_rect_with(64, 0, 64, 64, |col, row| {
        let r = (col * 2 + 80) as u8;
        let g = 210u8;
        let b = (row * 2 + 90) as u8;
        [r, g, b, 0xFF]
    });
    source.update_rect_with(0, 64, 64, 64, |col, row| {
        let r = 240u8;
        let g = (col * 3 + 30) as u8;
        let b = (row * 3 + 40) as u8;
        [r, g, b, 0xFF]
    });
    let prev_count2 = visitor.screen_sink_snapshot().count();
    assert!(host.screen_send_frame().await.unwrap());

    let wait_frame2 = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() > prev_count2 {
            break;
        }
        if wait_frame2.elapsed() > Duration::from_secs(5) {
            panic!("visitor never received Frame 2");
        }
        tokio::time::sleep(Duration::from_millis(15)).await;
    }

    // Verify visitor reports committed_transactions >= 2, swapped_frames >= 2, torn_frames_prevented >= 2
    let wait_stat2 = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(visitor_ui, "/api/screen/tile/present")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let rep: tetherly_core::ScreenTilePresentReport = serde_json::from_slice(&b).unwrap();
        if rep.committed_transactions >= 2
            && rep.swapped_frames >= 2
            && rep.torn_frames_prevented >= 2
        {
            break;
        }
        if wait_stat2.elapsed() > Duration::from_secs(5) {
            panic!("visitor never reported present count >= 2: {:?}", rep);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // Canvas Invariance check for Frame 2
    let rec2 = visitor.screen_sink_snapshot().last().unwrap();
    assert_eq!(
        rec2.bytes,
        source.pixels_snapshot(),
        "reconstructed frame 2 must match source exactly (Canvas Invariance)"
    );

    // 6. Verify visitor screen status report contains tile present telemetry
    let (c, b) = tetherly_node::loopback_get(visitor_ui, "/api/screen")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let status_rep: tetherly_node::ScreenStatusReport = serde_json::from_slice(&b).unwrap();
    assert!(status_rep.tile_present_enabled);
    assert_eq!(status_rep.tile_present_vsync_mode, "vsync60");
    assert!(status_rep.tile_present_double_buffer);
    assert!(status_rep.tile_present_tear_prevention);
    assert!(status_rep.tile_present_committed_transactions >= 2);
    assert!(status_rep.tile_present_swapped_frames >= 2);
    assert!(status_rep.tile_present_torn_frames_prevented >= 2);

    // 7. Reset Tile Present mode via POST /api/screen/tile/present/reset (Tag 57)
    let (c, b) = tetherly_node::loopback_post(visitor_ui, "/api/screen/tile/present/reset", b"{}")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let reset_rep: tetherly_core::ScreenTilePresentReport = serde_json::from_slice(&b).unwrap();
    assert!(!reset_rep.enabled);
    assert_eq!(reset_rep.committed_transactions, 0);
    assert_eq!(reset_rep.swapped_frames, 0);
    assert_eq!(reset_rep.torn_frames_prevented, 0);

    // Verify host synchronizes Tag 57 reset
    let wait_host_reset = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/tile/present")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let host_rep: tetherly_core::ScreenTilePresentReport = serde_json::from_slice(&b).unwrap();
        if !host_rep.enabled {
            break;
        }
        if wait_host_reset.elapsed() > Duration::from_secs(5) {
            panic!("host never synchronized present reset: {:?}", host_rep);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 8. Clean shutdown
    host.shutdown();
    visitor.shutdown();
    drop(ga);
    drop(gb);
}

#[tokio::test]
async fn m5_46_screen_tile_nack_recovery_pipeline() {
    let (da, ga) = temp_dir("host-nack");
    let (db, gb) = temp_dir("visitor-nack");
    let source = Arc::new(ControllableDeltaScreenSource::with_seq(128, 128, 0));
    source.set_pattern();
    let host = Node::start(cfg(da, "host", Some(source.clone()))).unwrap();
    let visitor = Node::start(cfg(db, "visitor", None)).unwrap();
    host.spawn_listeners().await.unwrap();
    visitor.spawn_listeners().await.unwrap();
    let pin = tetherly_core::pin::parse_pin("25170398").unwrap();
    host.set_pin(pin);
    visitor.set_pin(pin);
    host.dial(visitor.control_addr()).await.unwrap();
    wait_live(&host, Duration::from_secs(25)).await;
    wait_live(&visitor, Duration::from_secs(25)).await;

    let host_ui = host.spawn_ui().await.unwrap();
    let visitor_ui = visitor.spawn_ui().await.unwrap();
    let host_id = host.identity().device_id().clone();
    let visitor_id = visitor.identity().device_id().clone();

    wait_screen_port(&host, &visitor_id, Duration::from_secs(3)).await;
    wait_screen_port(&visitor, &host_id, Duration::from_secs(3)).await;

    // 1. Establish screen streaming session
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
    wait_host_streaming(host_ui).await;

    // Initial status report telemetry check: Tile NACK should be disabled by default
    let (c, b) = tetherly_node::loopback_get(visitor_ui, "/api/screen/tile/nack")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let init_rep: tetherly_core::ScreenTileNackReport = serde_json::from_slice(&b).unwrap();
    assert!(!init_rep.enabled);
    assert_eq!(init_rep.timeout_ms, 20);
    assert_eq!(init_rep.max_retries, 2);
    assert_eq!(init_rep.nack_requests_sent, 0);
    assert_eq!(init_rep.tiles_retransmitted, 0);
    assert_eq!(init_rep.degraded_commits, 0);

    // Send initial baseline keyframe
    assert!(host.screen_send_frame().await.unwrap());
    let wait_base = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() >= 1 {
            break;
        }
        if wait_base.elapsed() > Duration::from_secs(5) {
            panic!("visitor never received baseline keyframe");
        }
        tokio::time::sleep(Duration::from_millis(15)).await;
    }
    // Verify baseline keyframe matches source
    let baseline_frame = visitor.screen_sink_snapshot().last().unwrap();
    assert_eq!(baseline_frame.bytes, source.pixels_snapshot());

    // 2. Visitor enables spatial tile-grid streaming (tile_size = 64)
    let tile_req = serde_json::json!({
        "enabled": true,
        "tile_size": 64
    });
    let (c, b) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/tile",
        &serde_json::to_vec(&tile_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let tile_rep: tetherly_core::ScreenTileReport = serde_json::from_slice(&b).unwrap();
    assert!(tile_rep.enabled);

    // Verify host receives Tag 39 and synchronizes tile config
    let wait_host_tile = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/tile")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let host_rep: tetherly_core::ScreenTileReport = serde_json::from_slice(&b).unwrap();
        if host_rep.enabled && host_rep.tile_size == 64 {
            break;
        }
        if wait_host_tile.elapsed() > Duration::from_secs(5) {
            panic!("host never synchronized tile config: {:?}", host_rep);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 3. Visitor configures Tile Micro-NACK Fast Retransmission via POST /api/screen/tile/nack
    let nack_req = serde_json::json!({
        "enabled": true,
        "timeout_ms": 30,
        "max_retries": 3
    });
    let (c, b) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/tile/nack",
        &serde_json::to_vec(&nack_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let vis_nack_rep: tetherly_core::ScreenTileNackReport = serde_json::from_slice(&b).unwrap();
    assert!(vis_nack_rep.enabled);
    assert_eq!(vis_nack_rep.timeout_ms, 30);
    assert_eq!(vis_nack_rep.max_retries, 3);

    // 4. Send multi-tile update frame (Frame 1):
    // Updates Tile 0 (0..64, 0..64) and Tile 3 (64..128, 64..128)
    tokio::time::sleep(PACER_SETTLE).await;
    source.update_rect_with(0, 0, 64, 64, |col, row| {
        let r = (col * 3 + 15) as u8;
        let g = (row * 3 + 25) as u8;
        let b = 195u8;
        [r, g, b, 0xFF]
    });
    source.update_rect_with(64, 64, 64, 64, |col, row| {
        let r = 85u8;
        let g = (col * 2 + 55) as u8;
        let b = (row * 2 + 65) as u8;
        [r, g, b, 0xFF]
    });
    let prev_count1 = visitor.screen_sink_snapshot().count();
    assert!(host.screen_send_frame().await.unwrap());

    let wait_frame1 = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() > prev_count1 {
            break;
        }
        if wait_frame1.elapsed() > Duration::from_secs(5) {
            panic!("visitor never received Frame 1 (tile grid frame)");
        }
        tokio::time::sleep(Duration::from_millis(15)).await;
    }

    // Verify Canvas Invariance for Frame 1
    let rec1 = visitor.screen_sink_snapshot().last().unwrap();
    assert_eq!(
        rec1.bytes,
        source.pixels_snapshot(),
        "reconstructed frame 1 must match source exactly (Canvas Invariance)"
    );

    // 5. Verify Host-side Ring-Buffered Fast Replay:
    // Visitor sends Tag 59 (ControlMsg::TileNackRequest) targeting Tile 0 of Frame 1.
    // Host must find the dirty tile in its screen_tile_resend_buffer, fast replay it,
    // and increment tiles_retransmitted.
    let nack_msg = tetherly_core::ControlMsg::TileNackRequest {
        seq: rec1.seq as u32,
        tile_index: 0,
        retry_count: 1,
    };
    visitor.send_screen_control(nack_msg).await.unwrap();

    let wait_host_retransmit = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/tile/nack")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let host_rep: tetherly_core::ScreenTileNackReport = serde_json::from_slice(&b).unwrap();
        if host_rep.tiles_retransmitted >= 1 {
            break;
        }
        if wait_host_retransmit.elapsed() > Duration::from_secs(5) {
            panic!(
                "host never recorded tiles_retransmitted >= 1: {:?}",
                host_rep
            );
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 6. Verify full screen status telemetry includes Tile Micro-NACK fields
    let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let status_rep: tetherly_node::ScreenStatusReport = serde_json::from_slice(&b).unwrap();
    assert!(status_rep.tile_nack_tiles_retransmitted >= 1);

    // 7. Reset Tile NACK mode via POST /api/screen/tile/nack/reset (Tag 60)
    let (c, b) = tetherly_node::loopback_post(visitor_ui, "/api/screen/tile/nack/reset", b"{}")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let reset_rep: tetherly_core::ScreenTileNackReport = serde_json::from_slice(&b).unwrap();
    assert!(!reset_rep.enabled);
    assert_eq!(reset_rep.timeout_ms, 20);
    assert_eq!(reset_rep.max_retries, 2);
    assert_eq!(reset_rep.nack_requests_sent, 0);
    assert_eq!(reset_rep.tiles_retransmitted, 0);
    assert_eq!(reset_rep.degraded_commits, 0);

    // Verify host synchronizes Tag 60 reset
    let wait_host_reset = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/tile/nack")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let host_rep: tetherly_core::ScreenTileNackReport = serde_json::from_slice(&b).unwrap();
        if !host_rep.enabled && host_rep.tiles_retransmitted == 0 {
            break;
        }
        if wait_host_reset.elapsed() > Duration::from_secs(5) {
            panic!("host never synchronized tile nack reset: {:?}", host_rep);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 8. Clean shutdown
    host.shutdown();
    visitor.shutdown();
    drop(ga);
    drop(gb);
}

// --------------------------------------------------------------------------
// M5.47 — Screen Streaming Dynamic Tile Saliency, Semantic Importance Tiering
// & Per-Tile Priority Protection Pipeline (ADR-123 / v2.37)
// --------------------------------------------------------------------------
#[tokio::test(flavor = "multi_thread")]
async fn m5_47_screen_tile_saliency_tiering_pipeline() {
    let (da, ga) = temp_dir("host-saliency");
    let (db, gb) = temp_dir("visitor-saliency");
    let source = Arc::new(ControllableDeltaScreenSource::with_seq(128, 128, 0));
    source.set_pattern();
    let host = Node::start(cfg(da, "host", Some(source.clone()))).unwrap();
    let visitor = Node::start(cfg(db, "visitor", None)).unwrap();
    host.spawn_listeners().await.unwrap();
    visitor.spawn_listeners().await.unwrap();
    let pin = tetherly_core::pin::parse_pin("25170399").unwrap();
    host.set_pin(pin);
    visitor.set_pin(pin);
    host.dial(visitor.control_addr()).await.unwrap();
    wait_live(&host, Duration::from_secs(25)).await;
    wait_live(&visitor, Duration::from_secs(25)).await;

    let host_ui = host.spawn_ui().await.unwrap();
    let visitor_ui = visitor.spawn_ui().await.unwrap();
    let host_id = host.identity().device_id().clone();
    let visitor_id = visitor.identity().device_id().clone();

    wait_screen_port(&host, &visitor_id, Duration::from_secs(3)).await;
    wait_screen_port(&visitor, &host_id, Duration::from_secs(3)).await;

    // 1. Establish screen streaming session
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
    wait_host_streaming(host_ui).await;

    // Initial status report check: Tile Saliency should be disabled by default
    let (c, b) = tetherly_node::loopback_get(visitor_ui, "/api/screen/tile/saliency")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let init_rep: tetherly_core::ScreenTileSaliencyReport = serde_json::from_slice(&b).unwrap();
    assert!(!init_rep.enabled);
    assert_eq!(init_rep.high_threshold, 120);
    assert_eq!(init_rep.low_threshold, 40);
    assert!(init_rep.drop_background_on_congestion);
    assert_eq!(init_rep.critical_tiles_sent, 0);
    assert_eq!(init_rep.normal_tiles_sent, 0);
    assert_eq!(init_rep.background_tiles_sent, 0);
    assert_eq!(init_rep.background_tiles_skipped, 0);

    let wait_streaming = std::time::Instant::now();
    loop {
        if host.screen_state() == tetherly_core::ScreenState::Streaming {
            break;
        }
        if wait_streaming.elapsed() > Duration::from_secs(5) {
            panic!(
                "host never entered streaming state: {:?}",
                host.screen_state()
            );
        }
        tokio::time::sleep(Duration::from_millis(15)).await;
    }

    // Send initial baseline keyframe
    assert!(host.screen_send_frame().await.unwrap());
    let wait_base = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() >= 1 {
            break;
        }
        if wait_base.elapsed() > Duration::from_secs(5) {
            panic!("visitor never received baseline keyframe");
        }
        tokio::time::sleep(Duration::from_millis(15)).await;
    }
    let baseline_frame = visitor.screen_sink_snapshot().last().unwrap();
    assert_eq!(baseline_frame.bytes, source.pixels_snapshot());

    // 2. Visitor enables spatial tile-grid streaming (tile_size = 64)
    let tile_req = serde_json::json!({
        "enabled": true,
        "tile_size": 64
    });
    let (c, b) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/tile",
        &serde_json::to_vec(&tile_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let tile_rep: tetherly_core::ScreenTileReport = serde_json::from_slice(&b).unwrap();
    assert!(tile_rep.enabled);

    // Verify host receives Tag 39 and synchronizes tile config
    let wait_host_tile = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/tile")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let host_rep: tetherly_core::ScreenTileReport = serde_json::from_slice(&b).unwrap();
        if host_rep.enabled && host_rep.tile_size == 64 {
            break;
        }
        if wait_host_tile.elapsed() > Duration::from_secs(5) {
            panic!("host never synchronized tile config: {:?}", host_rep);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 3. Visitor configures Tile Saliency Mode via POST /api/screen/tile/saliency (Tag 62)
    let saliency_req = serde_json::json!({
        "enabled": true,
        "high_threshold": 120,
        "low_threshold": 40,
        "drop_background_on_congestion": true
    });
    let (c, b) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/tile/saliency",
        &serde_json::to_vec(&saliency_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let vis_saliency_rep: tetherly_core::ScreenTileSaliencyReport =
        serde_json::from_slice(&b).unwrap();
    assert!(vis_saliency_rep.enabled);
    assert_eq!(vis_saliency_rep.high_threshold, 120);
    assert_eq!(vis_saliency_rep.low_threshold, 40);
    assert!(vis_saliency_rep.drop_background_on_congestion);

    // Verify host receives Tag 62 and synchronizes saliency config
    let wait_host_saliency = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/tile/saliency")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let host_rep: tetherly_core::ScreenTileSaliencyReport = serde_json::from_slice(&b).unwrap();
        if host_rep.enabled && host_rep.high_threshold == 120 && host_rep.low_threshold == 40 {
            break;
        }
        if wait_host_saliency.elapsed() > Duration::from_secs(5) {
            panic!("host never synchronized saliency config: {:?}", host_rep);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 4. Send multi-tier dirty tiles frame (Frame 1):
    // Updates Tile 0 (0..64, 0..64) with high contrast alternating pattern -> Critical
    // Updates Tile 3 (64..128, 64..128) with flat smooth pattern -> Background
    tokio::time::sleep(PACER_SETTLE).await;
    source.update_rect_with(0, 0, 64, 64, |col, row| {
        if (col / 4 + row / 4) % 2 == 0 {
            [255, 255, 255, 0xFF]
        } else {
            [0, 0, 0, 0xFF]
        }
    });
    source.update_rect_with(64, 64, 64, 64, |_col, _row| [30, 30, 30, 0xFF]);
    let prev_count1 = visitor.screen_sink_snapshot().count();
    assert!(host.screen_send_frame().await.unwrap());

    let wait_frame1 = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() > prev_count1 {
            break;
        }
        if wait_frame1.elapsed() > Duration::from_secs(5) {
            panic!("visitor never received Frame 1 (multi-tier frame)");
        }
        tokio::time::sleep(Duration::from_millis(15)).await;
    }

    // Verify Canvas Invariance for Frame 1
    let rec1 = visitor.screen_sink_snapshot().last().unwrap();
    assert_eq!(
        rec1.bytes,
        source.pixels_snapshot(),
        "reconstructed frame 1 must match source exactly (Canvas Invariance)"
    );

    // Query host saliency telemetry: Critical and Background tiles tracked, no skips yet
    let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/tile/saliency")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let host_rep1: tetherly_core::ScreenTileSaliencyReport = serde_json::from_slice(&b).unwrap();
    assert!(
        host_rep1.critical_tiles_sent >= 1,
        "expected critical_tiles_sent >= 1, got {:?}",
        host_rep1
    );
    assert!(
        host_rep1.background_tiles_sent >= 1,
        "expected background_tiles_sent >= 1, got {:?}",
        host_rep1
    );
    assert_eq!(
        host_rep1.background_tiles_skipped, 0,
        "no skips before congestion"
    );

    // 5. Simulate Link Congestion (Frame 2):
    // Under congestion, background tiles should be dropped/skipped, while critical tiles are sent.
    host.set_screen_congestion_override(Some(true));
    assert!(host.is_screen_congested());

    // Update Tile 0 with high-contrast text pattern (Critical)
    source.update_rect_with(0, 0, 64, 64, |col, row| {
        if (col / 4 + row / 4) % 2 == 0 {
            [240, 240, 240, 0xFF]
        } else {
            [10, 10, 10, 0xFF]
        }
    });
    // Update Tile 3 with flat smooth color (Background)
    source.update_rect_with(64, 64, 64, 64, |_col, _row| [50, 50, 50, 0xFF]);

    let prev_count2 = visitor.screen_sink_snapshot().count();
    assert!(host.screen_send_frame().await.unwrap());

    let wait_frame2 = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() > prev_count2 {
            break;
        }
        if wait_frame2.elapsed() > Duration::from_secs(5) {
            panic!("visitor never received Frame 2 (congested critical frame)");
        }
        tokio::time::sleep(Duration::from_millis(15)).await;
    }

    // Verify host saliency telemetry recorded background tile skip under congestion
    let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/tile/saliency")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let host_rep2: tetherly_core::ScreenTileSaliencyReport = serde_json::from_slice(&b).unwrap();
    assert!(
        host_rep2.background_tiles_skipped >= 1,
        "background tiles must be skipped during congestion: {:?}",
        host_rep2
    );

    // 6. Clear Congestion & Verify Eventual Consistency (Frame 3):
    // Skipped background tile was kept dirty in host canvas, and should now heal cleanly.
    host.set_screen_congestion_override(None);
    assert!(!host.is_screen_congested());

    let prev_count3 = visitor.screen_sink_snapshot().count();
    assert!(host.screen_send_frame().await.unwrap());

    let wait_frame3 = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() > prev_count3 {
            break;
        }
        if wait_frame3.elapsed() > Duration::from_secs(5) {
            panic!("visitor never received Frame 3 (recovery frame)");
        }
        tokio::time::sleep(Duration::from_millis(15)).await;
    }

    let rec3 = visitor.screen_sink_snapshot().last().unwrap();
    assert_eq!(
        rec3.bytes,
        source.pixels_snapshot(),
        "reconstructed frame 3 must match source exactly after congestion clears (Eventual Consistency)"
    );

    // 7. Verify full screen status telemetry includes Tile Saliency fields
    let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let status_rep: tetherly_node::ScreenStatusReport = serde_json::from_slice(&b).unwrap();
    assert!(status_rep.tile_saliency_enabled);
    assert_eq!(status_rep.tile_saliency_high_threshold, 120);
    assert_eq!(status_rep.tile_saliency_low_threshold, 40);
    assert!(status_rep.tile_saliency_drop_background_on_congestion);
    assert!(status_rep.tile_saliency_critical_tiles >= 2);
    assert!(status_rep.tile_saliency_background_skipped >= 1);

    // 8. Reset Tile Saliency mode via POST /api/screen/tile/saliency/reset (Tag 63)
    let (c, b) = tetherly_node::loopback_post(visitor_ui, "/api/screen/tile/saliency/reset", b"{}")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let reset_rep: tetherly_core::ScreenTileSaliencyReport = serde_json::from_slice(&b).unwrap();
    assert!(!reset_rep.enabled);
    assert_eq!(reset_rep.high_threshold, 120);
    assert_eq!(reset_rep.low_threshold, 40);
    assert!(reset_rep.drop_background_on_congestion);
    assert_eq!(reset_rep.critical_tiles_sent, 0);
    assert_eq!(reset_rep.normal_tiles_sent, 0);
    assert_eq!(reset_rep.background_tiles_sent, 0);
    assert_eq!(reset_rep.background_tiles_skipped, 0);

    // Verify host synchronizes Tag 63 reset
    let wait_host_reset = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/tile/saliency")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let host_rep: tetherly_core::ScreenTileSaliencyReport = serde_json::from_slice(&b).unwrap();
        if !host_rep.enabled && host_rep.critical_tiles_sent == 0 {
            break;
        }
        if wait_host_reset.elapsed() > Duration::from_secs(5) {
            panic!(
                "host never synchronized tile saliency reset: {:?}",
                host_rep
            );
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 9. Clean shutdown
    host.shutdown();
    visitor.shutdown();
    drop(ga);
    drop(gb);
}

// --------------------------------------------------------------------------
// M5.48 — Screen Streaming Dynamic Spatial Tile Multi-Resolution Level-of-Detail
// Downsampling & Progressive Bilinear Reconstruction Pipeline (ADR-124 / v2.38)
// --------------------------------------------------------------------------
#[tokio::test(flavor = "multi_thread")]
async fn m5_48_screen_tile_lod_pipeline() {
    let (da, ga) = temp_dir("host-lod");
    let (db, gb) = temp_dir("visitor-lod");
    let source = Arc::new(ControllableDeltaScreenSource::with_seq(128, 128, 0));
    source.set_pattern();
    let host = Node::start(cfg(da, "host", Some(source.clone()))).unwrap();
    let visitor = Node::start(cfg(db, "visitor", None)).unwrap();
    host.spawn_listeners().await.unwrap();
    visitor.spawn_listeners().await.unwrap();
    let pin = tetherly_core::pin::parse_pin("25170399").unwrap();
    host.set_pin(pin);
    visitor.set_pin(pin);
    host.dial(visitor.control_addr()).await.unwrap();
    wait_live(&host, Duration::from_secs(25)).await;
    wait_live(&visitor, Duration::from_secs(25)).await;

    let host_ui = host.spawn_ui().await.unwrap();
    let visitor_ui = visitor.spawn_ui().await.unwrap();
    let host_id = host.identity().device_id().clone();
    let visitor_id = visitor.identity().device_id().clone();

    wait_screen_port(&host, &visitor_id, Duration::from_secs(3)).await;
    wait_screen_port(&visitor, &host_id, Duration::from_secs(3)).await;

    // 1. Establish screen streaming session
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
    wait_host_streaming(host_ui).await;

    // Initial status report check: Tile LoD should be disabled by default
    let (c, b) = tetherly_node::loopback_get(visitor_ui, "/api/screen/tile/lod")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let init_rep: tetherly_core::ScreenTileLodReport = serde_json::from_slice(&b).unwrap();
    assert!(!init_rep.enabled);
    assert_eq!(init_rep.max_lod, 1);
    assert!(init_rep.auto_lod_on_congestion);
    assert_eq!(init_rep.lod0_tiles_sent, 0);
    assert_eq!(init_rep.lod1_tiles_sent, 0);
    assert_eq!(init_rep.lod2_tiles_sent, 0);
    assert_eq!(init_rep.bytes_saved, 0);

    let wait_streaming = std::time::Instant::now();
    loop {
        if host.screen_state() == tetherly_core::ScreenState::Streaming {
            break;
        }
        if wait_streaming.elapsed() > Duration::from_secs(5) {
            panic!(
                "host never entered streaming state: {:?}",
                host.screen_state()
            );
        }
        tokio::time::sleep(Duration::from_millis(15)).await;
    }

    // Send initial baseline keyframe
    assert!(host.screen_send_frame().await.unwrap());
    let wait_base = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() >= 1 {
            break;
        }
        if wait_base.elapsed() > Duration::from_secs(5) {
            panic!("visitor never received baseline keyframe");
        }
        tokio::time::sleep(Duration::from_millis(15)).await;
    }
    let baseline_frame = visitor.screen_sink_snapshot().last().unwrap();
    assert_eq!(baseline_frame.bytes, source.pixels_snapshot());

    // 2. Visitor enables spatial tile-grid streaming (tile_size = 64)
    let tile_req = serde_json::json!({
        "enabled": true,
        "tile_size": 64
    });
    let (c, b) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/tile",
        &serde_json::to_vec(&tile_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let tile_rep: tetherly_core::ScreenTileReport = serde_json::from_slice(&b).unwrap();
    assert!(tile_rep.enabled);

    // Verify host receives Tag 39 and synchronizes tile config
    let wait_host_tile = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/tile")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let host_rep: tetherly_core::ScreenTileReport = serde_json::from_slice(&b).unwrap();
        if host_rep.enabled && host_rep.tile_size == 64 {
            break;
        }
        if wait_host_tile.elapsed() > Duration::from_secs(5) {
            panic!("host never synchronized tile config: {:?}", host_rep);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 3. Visitor enables Saliency awareness with drop_background = false
    let saliency_req = serde_json::json!({
        "enabled": true,
        "high_threshold": 120,
        "low_threshold": 40,
        "drop_background_on_congestion": false
    });
    let (c, _) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/tile/saliency",
        &serde_json::to_vec(&saliency_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);

    // 4. Visitor configures Tile LoD Mode via POST /api/screen/tile/lod (Tag 65)
    let lod_req = serde_json::json!({
        "enabled": true,
        "max_lod": 2,
        "auto_lod_on_congestion": true
    });
    let (c, b) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/tile/lod",
        &serde_json::to_vec(&lod_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let vis_lod_rep: tetherly_core::ScreenTileLodReport = serde_json::from_slice(&b).unwrap();
    assert!(vis_lod_rep.enabled);
    assert_eq!(vis_lod_rep.max_lod, 2);
    assert!(vis_lod_rep.auto_lod_on_congestion);

    // Verify host receives Tag 65 and synchronizes LoD config
    let wait_host_lod = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/tile/lod")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let host_rep: tetherly_core::ScreenTileLodReport = serde_json::from_slice(&b).unwrap();
        if host_rep.enabled && host_rep.max_lod == 2 && host_rep.auto_lod_on_congestion {
            break;
        }
        if wait_host_lod.elapsed() > Duration::from_secs(5) {
            panic!("host never synchronized lod config: {:?}", host_rep);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 5. Send multi-tier dirty tiles frame without congestion (Frame 1):
    // Updates Tile 0 (0..64, 0..64) with high contrast alternating pattern -> Critical
    // Updates Tile 3 (64..128, 64..128) with flat smooth pattern -> Background
    tokio::time::sleep(PACER_SETTLE).await;
    source.update_rect_with(0, 0, 64, 64, |col, row| {
        if (col / 4 + row / 4) % 2 == 0 {
            [255, 255, 255, 0xFF]
        } else {
            [0, 0, 0, 0xFF]
        }
    });
    source.update_rect_with(64, 64, 64, 64, |_col, _row| [30, 30, 30, 0xFF]);
    let prev_count1 = visitor.screen_sink_snapshot().count();
    assert!(host.screen_send_frame().await.unwrap());

    let wait_frame1 = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() > prev_count1 {
            break;
        }
        if wait_frame1.elapsed() > Duration::from_secs(5) {
            panic!("visitor never received Frame 1 (uncongested frame)");
        }
        tokio::time::sleep(Duration::from_millis(15)).await;
    }

    // Verify Canvas Invariance for Frame 1
    let rec1 = visitor.screen_sink_snapshot().last().unwrap();
    assert_eq!(
        rec1.bytes,
        source.pixels_snapshot(),
        "reconstructed frame 1 must match source exactly (Canvas Invariance)"
    );

    // Query host LoD telemetry: uncongested frame transmits full resolution (Lod0)
    let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/tile/lod")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let host_rep1: tetherly_core::ScreenTileLodReport = serde_json::from_slice(&b).unwrap();
    assert!(
        host_rep1.lod0_tiles_sent >= 1,
        "expected lod0_tiles_sent >= 1, got {:?}",
        host_rep1
    );

    // 6. Simulate Link Congestion (Frame 2):
    // Under congestion, background tiles should be downsampled (Lod2), while critical tiles remain Lod0.
    host.set_screen_congestion_override(Some(true));
    assert!(host.is_screen_congested());

    // Update Tile 0 with high-contrast text pattern (Critical)
    source.update_rect_with(0, 0, 64, 64, |col, row| {
        if (col / 4 + row / 4) % 2 == 0 {
            [240, 240, 240, 0xFF]
        } else {
            [10, 10, 10, 0xFF]
        }
    });
    // Update Tile 3 with flat smooth color (Background)
    source.update_rect_with(64, 64, 64, 64, |_col, _row| [50, 50, 50, 0xFF]);

    let prev_count2 = visitor.screen_sink_snapshot().count();
    assert!(host.screen_send_frame().await.unwrap());

    let wait_frame2 = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() > prev_count2 {
            break;
        }
        if wait_frame2.elapsed() > Duration::from_secs(5) {
            panic!("visitor never received Frame 2 (congested lod downsampled frame)");
        }
        tokio::time::sleep(Duration::from_millis(15)).await;
    }

    // Verify host LoD telemetry recorded downsampled tiles and bandwidth savings
    let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/tile/lod")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let host_rep2: tetherly_core::ScreenTileLodReport = serde_json::from_slice(&b).unwrap();
    assert!(
        host_rep2.lod1_tiles_sent + host_rep2.lod2_tiles_sent >= 1,
        "downsampled tiles must be sent during congestion: {:?}",
        host_rep2
    );
    assert!(
        host_rep2.bytes_saved > 0,
        "bytes_saved must be positive: {:?}",
        host_rep2
    );

    // 7. Clear Congestion & Verify Eventual Consistency (Frame 3):
    // Downsampled background tile was flagged for healing on host, and should now be retransmitted as Lod0 lossless.
    host.set_screen_congestion_override(None);
    assert!(!host.is_screen_congested());

    let prev_count3 = visitor.screen_sink_snapshot().count();
    assert!(host.screen_send_frame().await.unwrap());

    let wait_frame3 = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() > prev_count3 {
            break;
        }
        if wait_frame3.elapsed() > Duration::from_secs(5) {
            panic!("visitor never received Frame 3 (recovery frame)");
        }
        tokio::time::sleep(Duration::from_millis(15)).await;
    }

    let rec3 = visitor.screen_sink_snapshot().last().unwrap();
    assert_eq!(
        rec3.bytes,
        source.pixels_snapshot(),
        "reconstructed frame 3 must match source exactly after congestion clears (Eventual Consistency & Canvas Invariance)"
    );

    // 8. Verify full screen status telemetry includes Tile LoD fields
    let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let status_rep: tetherly_node::ScreenStatusReport = serde_json::from_slice(&b).unwrap();
    assert!(status_rep.tile_lod_enabled);
    assert_eq!(status_rep.tile_lod_max_lod, 2);
    assert!(status_rep.tile_lod_auto_lod_on_congestion);
    assert!(status_rep.tile_lod0_tiles >= 1);
    assert!(status_rep.tile_lod1_tiles + status_rep.tile_lod2_tiles >= 1);
    assert!(status_rep.tile_lod_bytes_saved > 0);

    // 9. Reset Tile LoD mode via POST /api/screen/tile/lod/reset (Tag 66)
    let (c, b) = tetherly_node::loopback_post(visitor_ui, "/api/screen/tile/lod/reset", b"{}")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let reset_rep: tetherly_core::ScreenTileLodReport = serde_json::from_slice(&b).unwrap();
    assert!(!reset_rep.enabled);
    assert_eq!(reset_rep.max_lod, 1);
    assert!(reset_rep.auto_lod_on_congestion);
    assert_eq!(reset_rep.lod0_tiles_sent, 0);
    assert_eq!(reset_rep.lod1_tiles_sent, 0);
    assert_eq!(reset_rep.lod2_tiles_sent, 0);
    assert_eq!(reset_rep.bytes_saved, 0);

    // Verify host synchronizes Tag 66 reset
    let wait_host_reset = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/tile/lod")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let host_rep: tetherly_core::ScreenTileLodReport = serde_json::from_slice(&b).unwrap();
        if !host_rep.enabled && host_rep.lod0_tiles_sent == 0 {
            break;
        }
        if wait_host_reset.elapsed() > Duration::from_secs(5) {
            panic!("host never synchronized tile lod reset: {:?}", host_rep);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 10. Clean shutdown
    host.shutdown();
    visitor.shutdown();
    drop(ga);
    drop(gb);
}

// --------------------------------------------------------------------------
// M5.49: Screen Streaming Dynamic Tile Batching, MTU Chunking & Coalesced
// Pacing Pipeline with Dual CLI Isomorphism (ADR-125 / v2.39)
// --------------------------------------------------------------------------
#[tokio::test(flavor = "multi_thread")]
async fn m5_49_screen_tile_batching_pipeline() {
    let (da, ga) = temp_dir("host-batch");
    let (db, gb) = temp_dir("visitor-batch");
    let source = Arc::new(ControllableDeltaScreenSource::with_seq(128, 128, 0));
    source.set_pattern();
    let host = Node::start(cfg(da, "host", Some(source.clone()))).unwrap();
    let visitor = Node::start(cfg(db, "visitor", None)).unwrap();
    host.spawn_listeners().await.unwrap();
    visitor.spawn_listeners().await.unwrap();
    let pin = tetherly_core::pin::parse_pin("25170399").unwrap();
    host.set_pin(pin);
    visitor.set_pin(pin);
    host.dial(visitor.control_addr()).await.unwrap();
    wait_live(&host, Duration::from_secs(25)).await;
    wait_live(&visitor, Duration::from_secs(25)).await;

    let host_ui = host.spawn_ui().await.unwrap();
    let visitor_ui = visitor.spawn_ui().await.unwrap();
    let host_id = host.identity().device_id().clone();
    let visitor_id = visitor.identity().device_id().clone();

    wait_screen_port(&host, &visitor_id, Duration::from_secs(3)).await;
    wait_screen_port(&visitor, &host_id, Duration::from_secs(3)).await;

    // 1. Establish screen streaming session
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
    wait_host_streaming(host_ui).await;

    // Initial status report check: Tile Batching should be disabled by default
    let (c, b) = tetherly_node::loopback_get(visitor_ui, "/api/screen/tile/batch")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let init_rep: tetherly_core::ScreenTileBatchReport = serde_json::from_slice(&b).unwrap();
    assert!(!init_rep.enabled);
    assert_eq!(init_rep.target_chunk_bytes, 1200);
    assert_eq!(init_rep.pacing_interval_us, 200);
    assert_eq!(init_rep.batches_sent, 0);
    assert_eq!(init_rep.chunks_sent, 0);
    assert_eq!(init_rep.avg_chunk_bytes, 0);
    assert_eq!(init_rep.pacing_pauses, 0);

    let wait_streaming = std::time::Instant::now();
    loop {
        if host.screen_state() == tetherly_core::ScreenState::Streaming {
            break;
        }
        if wait_streaming.elapsed() > Duration::from_secs(5) {
            panic!(
                "host never entered streaming state: {:?}",
                host.screen_state()
            );
        }
        tokio::time::sleep(Duration::from_millis(15)).await;
    }

    // Send initial baseline keyframe
    assert!(host.screen_send_frame().await.unwrap());
    let wait_base = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() >= 1 {
            break;
        }
        if wait_base.elapsed() > Duration::from_secs(5) {
            panic!("visitor never received baseline keyframe");
        }
        tokio::time::sleep(Duration::from_millis(15)).await;
    }
    let baseline_frame = visitor.screen_sink_snapshot().last().unwrap();
    assert_eq!(baseline_frame.bytes, source.pixels_snapshot());

    // 2. Visitor enables spatial tile-grid streaming (tile_size = 64)
    let tile_req = serde_json::json!({
        "enabled": true,
        "tile_size": 64
    });
    let (c, b) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/tile",
        &serde_json::to_vec(&tile_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let tile_rep: tetherly_core::ScreenTileReport = serde_json::from_slice(&b).unwrap();
    assert!(tile_rep.enabled);

    // Verify host receives Tag 39 and synchronizes tile config
    let wait_host_tile = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/tile")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let host_rep: tetherly_core::ScreenTileReport = serde_json::from_slice(&b).unwrap();
        if host_rep.enabled && host_rep.tile_size == 64 {
            break;
        }
        if wait_host_tile.elapsed() > Duration::from_secs(5) {
            panic!("host never synchronized tile config: {:?}", host_rep);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 3. Visitor configures Tile Batch Mode via POST /api/screen/tile/batch (Tag 68)
    let batch_req = serde_json::json!({
        "enabled": true,
        "target_chunk_bytes": 600,
        "pacing_interval_us": 100
    });
    let (c, b) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/tile/batch",
        &serde_json::to_vec(&batch_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let vis_batch_rep: tetherly_core::ScreenTileBatchReport = serde_json::from_slice(&b).unwrap();
    assert!(vis_batch_rep.enabled);
    assert_eq!(vis_batch_rep.target_chunk_bytes, 600);
    assert_eq!(vis_batch_rep.pacing_interval_us, 100);

    // Verify host receives Tag 68 and synchronizes Batch config
    let wait_host_batch = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/tile/batch")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let host_rep: tetherly_core::ScreenTileBatchReport = serde_json::from_slice(&b).unwrap();
        if host_rep.enabled
            && host_rep.target_chunk_bytes == 600
            && host_rep.pacing_interval_us == 100
        {
            break;
        }
        if wait_host_batch.elapsed() > Duration::from_secs(5) {
            panic!("host never synchronized batch config: {:?}", host_rep);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 4. Update all 4 tiles with distinct patterns to trigger multi-chunk batching:
    // Tile (0, 0): (0..64, 0..64)
    // Tile (1, 0): (64..128, 0..64)
    // Tile (0, 1): (0..64, 64..128)
    // Tile (1, 1): (64..128, 64..128)
    tokio::time::sleep(PACER_SETTLE).await;
    source.update_rect_with(0, 0, 64, 64, |col, row| {
        if (col / 4 + row / 4) % 2 == 0 {
            [220, 50, 50, 0xFF]
        } else {
            [30, 20, 20, 0xFF]
        }
    });
    source.update_rect_with(64, 0, 64, 64, |col, row| {
        if (col / 3 + row / 3) % 2 == 0 {
            [50, 220, 50, 0xFF]
        } else {
            [20, 40, 20, 0xFF]
        }
    });
    source.update_rect_with(0, 64, 64, 64, |col, row| {
        if (col / 5 + row / 5) % 2 == 0 {
            [50, 50, 220, 0xFF]
        } else {
            [20, 20, 50, 0xFF]
        }
    });
    source.update_rect_with(64, 64, 64, 64, |col, row| {
        if (col / 2 + row / 2) % 2 == 0 {
            [200, 200, 50, 0xFF]
        } else {
            [40, 40, 10, 0xFF]
        }
    });

    let prev_count = visitor.screen_sink_snapshot().count();
    assert!(host.screen_send_frame().await.unwrap());

    let wait_batched = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() > prev_count {
            break;
        }
        if wait_batched.elapsed() > Duration::from_secs(5) {
            panic!("visitor never received batched & paced frame");
        }
        tokio::time::sleep(Duration::from_millis(15)).await;
    }

    // Verify Canvas Invariance: the reconstructed image on the receiver
    // matches the source exactly pixel-by-pixel after chunk reassembly and double-buffered presentation.
    let rec_batched = visitor.screen_sink_snapshot().last().unwrap();
    assert_eq!(
        rec_batched.bytes,
        source.pixels_snapshot(),
        "reconstructed frame must match source exactly (Canvas Invariance across MTU chunks)"
    );

    // 5. Verify host tile batch telemetry:
    // Should have sent 1 batch with >= 2 chunks, positive avg chunk bytes, and >= 1 pacing pauses.
    let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/tile/batch")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let host_batch_rep: tetherly_core::ScreenTileBatchReport = serde_json::from_slice(&b).unwrap();
    assert!(
        host_batch_rep.batches_sent >= 1,
        "expected batches_sent >= 1, got {:?}",
        host_batch_rep
    );
    assert!(
        host_batch_rep.chunks_sent > host_batch_rep.batches_sent,
        "chunks_sent ({}) must exceed batches_sent ({}) demonstrating sub-MTU chunking",
        host_batch_rep.chunks_sent,
        host_batch_rep.batches_sent
    );
    assert!(
        host_batch_rep.avg_chunk_bytes > 0,
        "avg_chunk_bytes must be positive"
    );
    assert!(
        host_batch_rep.pacing_pauses >= 1,
        "pacing_pauses must be >= 1 for multi-chunk batch dispersion"
    );

    // 6. Verify full ScreenStatusReport includes Tile Batch telemetry
    let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let status_rep: tetherly_node::ScreenStatusReport = serde_json::from_slice(&b).unwrap();
    assert!(status_rep.tile_batch_enabled);
    assert_eq!(status_rep.tile_batch_target_chunk_bytes, 600);
    assert_eq!(status_rep.tile_batch_pacing_interval_us, 100);
    assert!(status_rep.tile_batches_sent >= 1);
    assert!(status_rep.tile_chunks_sent >= 2);
    assert!(status_rep.tile_batch_avg_chunk_bytes > 0);
    assert!(status_rep.tile_batch_pacing_pauses >= 1);

    // 7. Reset Tile Batch mode via POST /api/screen/tile/batch/reset (Tag 69)
    let (c, b) = tetherly_node::loopback_post(visitor_ui, "/api/screen/tile/batch/reset", b"{}")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let reset_rep: tetherly_core::ScreenTileBatchReport = serde_json::from_slice(&b).unwrap();
    assert!(!reset_rep.enabled);
    assert_eq!(reset_rep.target_chunk_bytes, 1200);
    assert_eq!(reset_rep.pacing_interval_us, 200);
    assert_eq!(reset_rep.batches_sent, 0);
    assert_eq!(reset_rep.chunks_sent, 0);
    assert_eq!(reset_rep.pacing_pauses, 0);

    // Verify host synchronizes Tag 69 reset
    let wait_host_reset = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/tile/batch")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let host_rep: tetherly_core::ScreenTileBatchReport = serde_json::from_slice(&b).unwrap();
        if !host_rep.enabled && host_rep.batches_sent == 0 {
            break;
        }
        if wait_host_reset.elapsed() > Duration::from_secs(5) {
            panic!("host never synchronized tile batch reset: {:?}", host_rep);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 8. Clean shutdown
    host.shutdown();
    visitor.shutdown();
    drop(ga);
    drop(gb);
}

// --------------------------------------------------------------------------
// M5.50: Screen Streaming Dynamic Tile Local Motion Vector Estimation &
// Inter-Tile Displacement Compensation Pipeline with Dual CLI Isomorphism (ADR-126 / v2.40)
// --------------------------------------------------------------------------
#[tokio::test(flavor = "multi_thread")]
async fn m5_50_screen_tile_motion_vector_pipeline() {
    let (da, ga) = temp_dir("host-motion");
    let (db, gb) = temp_dir("visitor-motion");
    let source = Arc::new(ControllableDeltaScreenSource::with_seq(128, 128, 0));
    source.set_pattern();
    let host = Node::start(cfg(da, "host", Some(source.clone()))).unwrap();
    let visitor = Node::start(cfg(db, "visitor", None)).unwrap();
    host.spawn_listeners().await.unwrap();
    visitor.spawn_listeners().await.unwrap();
    let pin = tetherly_core::pin::parse_pin("25170399").unwrap();
    host.set_pin(pin);
    visitor.set_pin(pin);
    host.dial(visitor.control_addr()).await.unwrap();
    wait_live(&host, Duration::from_secs(25)).await;
    wait_live(&visitor, Duration::from_secs(25)).await;

    let host_ui = host.spawn_ui().await.unwrap();
    let visitor_ui = visitor.spawn_ui().await.unwrap();
    let host_id = host.identity().device_id().clone();
    let visitor_id = visitor.identity().device_id().clone();

    wait_screen_port(&host, &visitor_id, Duration::from_secs(3)).await;
    wait_screen_port(&visitor, &host_id, Duration::from_secs(3)).await;

    // 1. Establish screen streaming session
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
    wait_host_streaming(host_ui).await;

    // Initial status report check: Tile Motion Estimation should be disabled by default
    let (c, b) = tetherly_node::loopback_get(visitor_ui, "/api/screen/tile/motion")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let init_rep: tetherly_core::ScreenTileMotionReport = serde_json::from_slice(&b).unwrap();
    assert!(!init_rep.enabled);
    assert_eq!(init_rep.max_displacement, 32);
    assert_eq!(init_rep.min_match_ratio, 90);
    assert_eq!(init_rep.motion_tiles_sent, 0);
    assert_eq!(init_rep.fallback_tiles_sent, 0);
    assert_eq!(init_rep.bytes_saved, 0);

    let wait_streaming = std::time::Instant::now();
    loop {
        if host.screen_state() == tetherly_core::ScreenState::Streaming {
            break;
        }
        if wait_streaming.elapsed() > Duration::from_secs(5) {
            panic!(
                "host never entered streaming state: {:?}",
                host.screen_state()
            );
        }
        tokio::time::sleep(Duration::from_millis(15)).await;
    }

    // Send initial baseline keyframe
    assert!(host.screen_send_frame().await.unwrap());
    let wait_base = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() >= 1 {
            break;
        }
        if wait_base.elapsed() > Duration::from_secs(5) {
            panic!("visitor never received baseline keyframe");
        }
        tokio::time::sleep(Duration::from_millis(15)).await;
    }
    let baseline_frame = visitor.screen_sink_snapshot().last().unwrap();
    assert_eq!(baseline_frame.bytes, source.pixels_snapshot());

    // 2. Visitor enables spatial tile-grid streaming (tile_size = 64)
    let tile_req = serde_json::json!({
        "enabled": true,
        "tile_size": 64
    });
    let (c, b) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/tile",
        &serde_json::to_vec(&tile_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let tile_rep: tetherly_core::ScreenTileReport = serde_json::from_slice(&b).unwrap();
    assert!(tile_rep.enabled);

    // Verify host receives Tag 39 and synchronizes tile config
    let wait_host_tile = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/tile")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let host_rep: tetherly_core::ScreenTileReport = serde_json::from_slice(&b).unwrap();
        if host_rep.enabled && host_rep.tile_size == 64 {
            break;
        }
        if wait_host_tile.elapsed() > Duration::from_secs(5) {
            panic!("host never synchronized tile config: {:?}", host_rep);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 3. Visitor configures Tile Motion Mode via POST /api/screen/tile/motion (Tag 71)
    let motion_req = serde_json::json!({
        "enabled": true,
        "max_displacement": 32,
        "min_match_ratio": 90
    });
    let (c, b) = tetherly_node::loopback_post(
        visitor_ui,
        "/api/screen/tile/motion",
        &serde_json::to_vec(&motion_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c, 200);
    let vis_motion_rep: tetherly_core::ScreenTileMotionReport = serde_json::from_slice(&b).unwrap();
    assert!(vis_motion_rep.enabled);
    assert_eq!(vis_motion_rep.max_displacement, 32);
    assert_eq!(vis_motion_rep.min_match_ratio, 90);

    // Verify host receives Tag 71 and synchronizes Motion config
    let wait_host_motion = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/tile/motion")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let host_rep: tetherly_core::ScreenTileMotionReport = serde_json::from_slice(&b).unwrap();
        if host_rep.enabled && host_rep.max_displacement == 32 && host_rep.min_match_ratio == 90 {
            break;
        }
        if wait_host_motion.elapsed() > Duration::from_secs(5) {
            panic!("host never synchronized motion config: {:?}", host_rep);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 4. Update tile (0, 1) at (0, 64) with a local displacement (vertical shift dy = 10 px) from previous frame:
    // Tile (0, 1) covers x: 0..64, y: 64..128.
    // In reference canvas, pattern was P(x, y).
    // With dy = 10, ref_y = tile_y - dy = 64 - 10 = 54.
    // Now we set tile (0, 1) to P(col, 54 + row).
    tokio::time::sleep(PACER_SETTLE).await;
    source.update_rect_with(0, 64, 64, 64, |col, row| {
        let x = col;
        let y = 54 + row;
        [
            ((x * 3) & 0xFF) as u8,
            ((y * 7) & 0xFF) as u8,
            ((x ^ y) & 0xFF) as u8,
            0xFF,
        ]
    });

    let prev_count = visitor.screen_sink_snapshot().count();
    assert!(host.screen_send_frame().await.unwrap());

    let wait_motion_frame = std::time::Instant::now();
    loop {
        if visitor.screen_sink_snapshot().count() > prev_count {
            break;
        }
        if wait_motion_frame.elapsed() > Duration::from_secs(5) {
            panic!("visitor never received motion compensated tile frame");
        }
        tokio::time::sleep(Duration::from_millis(15)).await;
    }

    // Verify Canvas Invariance: the reconstructed image on the receiver
    // matches the source exactly pixel-by-pixel after zero-copy displacement blit.
    let rec_frame = visitor.screen_sink_snapshot().last().unwrap();
    assert_eq!(
        rec_frame.bytes,
        source.pixels_snapshot(),
        "reconstructed frame must match source exactly (Canvas Invariance across motion displacement blit)"
    );

    // 5. Verify host tile motion telemetry:
    // At least 1 motion tile was identified and sent, with positive bytes saved.
    let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/tile/motion")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let host_motion_rep: tetherly_core::ScreenTileMotionReport =
        serde_json::from_slice(&b).unwrap();
    assert!(
        host_motion_rep.motion_tiles_sent >= 1,
        "expected motion_tiles_sent >= 1, got {:?}",
        host_motion_rep
    );
    assert!(
        host_motion_rep.bytes_saved > 0,
        "bytes_saved must be positive, got {:?}",
        host_motion_rep
    );

    // 6. Verify full ScreenStatusReport includes Tile Motion telemetry
    let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let status_rep: tetherly_node::ScreenStatusReport = serde_json::from_slice(&b).unwrap();
    assert!(status_rep.tile_motion_enabled);
    assert_eq!(status_rep.tile_motion_max_displacement, 32);
    assert_eq!(status_rep.tile_motion_min_match_ratio, 90);
    assert!(status_rep.tile_motion_tiles_sent >= 1);
    assert!(status_rep.tile_motion_bytes_saved > 0);

    // 7. Reset Tile Motion mode via POST /api/screen/tile/motion/reset (Tag 72)
    let (c, b) = tetherly_node::loopback_post(visitor_ui, "/api/screen/tile/motion/reset", b"{}")
        .await
        .unwrap();
    assert_eq!(c, 200);
    let reset_rep: tetherly_core::ScreenTileMotionReport = serde_json::from_slice(&b).unwrap();
    assert!(!reset_rep.enabled);
    assert_eq!(reset_rep.max_displacement, 32);
    assert_eq!(reset_rep.min_match_ratio, 90);
    assert_eq!(reset_rep.motion_tiles_sent, 0);
    assert_eq!(reset_rep.bytes_saved, 0);

    // Verify host synchronizes Tag 72 reset
    let wait_host_reset = std::time::Instant::now();
    loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/screen/tile/motion")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let host_rep: tetherly_core::ScreenTileMotionReport = serde_json::from_slice(&b).unwrap();
        if !host_rep.enabled && host_rep.motion_tiles_sent == 0 {
            break;
        }
        if wait_host_reset.elapsed() > Duration::from_secs(5) {
            panic!("host never synchronized tile motion reset: {:?}", host_rep);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 8. Clean shutdown
    host.shutdown();
    visitor.shutdown();
    drop(ga);
    drop(gb);
} // --------------------------------------------------------------------------
  // ADR-138 remote desktop session pipeline (m6_1 ~ m6_6)
  // --------------------------------------------------------------------------

/// Poll until the viewer reports Controlling: the host answered the session
/// request, the viewer sent its Session Enter through 45719. Shares the
/// wait-shape of `wait_host_streaming` — poll the state you assert about,
/// never sleep a guessed interval.
async fn wait_session_controlling(viewer: &Node, timeout: Duration) {
    let start = std::time::Instant::now();
    loop {
        let rep = viewer.remote_session_report();
        if rep.message == "controlling" {
            return;
        }
        if start.elapsed() > timeout {
            panic!(
                "session never reached controlling: {:?} / {:?}",
                rep.message, rep.session_id
            );
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn wait_session_gone(node: &Node, timeout: Duration) {
    let start = std::time::Instant::now();
    while node.remote_session_active() {
        if start.elapsed() > timeout {
            panic!("remote session slot never cleared");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// m6_1 — request → accept → host MemorySink receives the Session Enter+Move
/// sequence. The host auto-allows screen consent (ADR-138 §2.4), registers as
/// SessionTarget, and the viewer's resp-driven Enter parks the host sink
/// cursor at screen center with seat=Remote.
#[tokio::test(flavor = "multi_thread")]
async fn m6_1_remote_session_request_accept_enter_pipeline() {
    let (host, viewer, _ga, _gb) = pair_view().await;
    let host_id = host.identity().device_id().clone();

    let rep = viewer
        .remote_session_start(
            host_id.to_string().as_str(),
            tetherly_core::RemoteSessionReason::UserInitiated,
        )
        .await
        .expect("session start");
    assert!(rep.ok, "start report must be ok");
    assert_eq!(rep.message, "remote session controlling");
    let session_id = rep.session_id.clone().expect("session id in report");
    assert_eq!(rep.peer_id.as_deref(), Some(host_id.to_string().as_str()));
    assert!(rep.screen_width.unwrap_or(0) > 0);
    assert!(rep.screen_height.unwrap_or(0) > 0);

    // Host: SessionTarget registered, screen consent auto-opened.
    assert!(host.remote_session_active());
    let target = host.remote_session_status().expect("host session state");
    assert_eq!(target.session_id, session_id);
    assert_eq!(target.role, tetherly_core::InputTunnelRole::SessionTarget);
    let start = std::time::Instant::now();
    loop {
        // Allowed is consent; Streaming is consent + viewer Start landed.
        if matches!(
            host.screen_state(),
            ScreenState::Allowed | ScreenState::Streaming
        ) {
            break;
        }
        if start.elapsed() > Duration::from_secs(5) {
            panic!(
                "host screen consent was not auto-allowed, state={:?}",
                host.screen_state()
            );
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // Host MemorySink: the session Enter+Move(center) sequence landed through
    // 45719 — focused, parked at center, seat Remote. Note: the MemorySink
    // observer never records Enter (the engine consumes it internally: focus
    // + release_all_inputs), so the observable trace is the center Move.
    let start = std::time::Instant::now();
    loop {
        let sink = host.input_sink_snapshot();
        let centered = sink.applied.iter().any(|k| match k {
            tetherly_core::InputKind::Move { x, y } => {
                *x == tetherly_core::SCREEN_UNITS / 2 && *y == tetherly_core::SCREEN_UNITS / 2
            }
            _ => false,
        });
        if centered {
            break;
        }
        if start.elapsed() > Duration::from_secs(5) {
            panic!(
                "host sink never received the session center Move (tail: {:?})",
                sink.applied.iter().rev().take(8).collect::<Vec<_>>()
            );
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let sink = host.input_sink_snapshot();
    let last_move = sink
        .applied
        .iter()
        .rev()
        .find_map(|k| match k {
            tetherly_core::InputKind::Move { x, y } => Some((*x, *y)),
            _ => None,
        })
        .expect("move after enter");
    assert_eq!(
        last_move,
        (
            tetherly_core::SCREEN_UNITS / 2,
            tetherly_core::SCREEN_UNITS / 2
        ),
        "session Enter parks the cursor at screen center"
    );
    assert_eq!(
        host.input_seat(),
        Some(tetherly_core::CursorSeat::Remote),
        "session Enter focuses the host client (input_seat, not the local engine)"
    );

    // Graceful stop: close frame flows, both slots clear, host seat back to
    // Local with a Leave-driven release.
    viewer
        .remote_session_stop("m6_1 end")
        .await
        .expect("session stop");
    wait_session_gone(&viewer, Duration::from_secs(3)).await;
    wait_session_gone(&host, Duration::from_secs(3)).await;
    assert_eq!(
        host.input_seat(),
        Some(tetherly_core::CursorSeat::Local),
        "session stop restores the host seat to local"
    );

    host.shutdown();
    viewer.shutdown();
    drop(_ga);
    drop(_gb);
}

/// m6_2 — policy reject: `allow_inbound_control=false` on the host answers
/// `PolicyDisabled`, no session state is created on either side.
#[tokio::test(flavor = "multi_thread")]
async fn m6_2_remote_session_policy_reject() {
    let (host, viewer, _ga, _gb) = pair_view().await;
    let host_id = host.identity().device_id().clone();

    let mut policy = host.input_policy();
    policy.allow_inbound_control = false;
    host.set_input_policy(policy).unwrap();

    let err = viewer
        .remote_session_start(
            host_id.to_string().as_str(),
            tetherly_core::RemoteSessionReason::UserInitiated,
        )
        .await
        .expect_err("policy-disabled host must reject");
    assert!(
        err.to_string().contains("rejected or closed by peer"),
        "unexpected error: {err}"
    );

    // Neither side leaks state; the reject reason reached the wire correctly.
    wait_session_gone(&viewer, Duration::from_secs(2)).await;
    wait_session_gone(&host, Duration::from_secs(2)).await;

    host.shutdown();
    viewer.shutdown();
    drop(_ga);
    drop(_gb);
}

/// m6_3 — Session roaming-guard: while a session drives the host
/// (SessionTarget), local `drive_input` edge pushes are suppressed entirely —
/// the deliberate edge-push takeover must not fire, and the watcher-style
/// `active_tunnel_role()`/`remote_session_active()` predicates the watcher
/// consults report the session so auto-attach is skipped.
#[tokio::test(flavor = "multi_thread")]
async fn m6_3_remote_session_roaming_guard_skips_takeover() {
    let (host, viewer, _ga, _gb) = pair_view().await;
    let host_id = host.identity().device_id().clone();

    viewer
        .remote_session_start(
            host_id.to_string().as_str(),
            tetherly_core::RemoteSessionReason::UserInitiated,
        )
        .await
        .expect("session start");
    wait_session_controlling(&viewer, Duration::from_secs(5)).await;

    // Deliberate edge push on the host while being session-controlled: the
    // guard must swallow it (no edge, no takeover, seat unchanged).
    let edge = host.drive_input(0, 10, 100, 50).unwrap();
    assert_eq!(
        edge, None,
        "edge-push takeover must be suppressed during a session"
    );
    assert_eq!(
        host.input_seat(),
        Some(tetherly_core::CursorSeat::Remote),
        "session keeps the host seat remote"
    );
    assert!(host.remote_session_active());
    // The watcher consults exactly these predicates; the session's receiving
    // role lives in the remote_session slot (active_input_tunnel stays None).
    assert_eq!(
        host.remote_session_status().map(|s| s.role),
        Some(tetherly_core::InputTunnelRole::SessionTarget)
    );
    assert!(host
        .remote_session_status()
        .map(|s| s.role.is_receiving())
        .unwrap_or(false));

    viewer.remote_session_stop("m6_3 end").await.expect("stop");
    wait_session_gone(&host, Duration::from_secs(3)).await;

    host.shutdown();
    viewer.shutdown();
    drop(_ga);
    drop(_gb);
}

/// m6_4 — idle timeout: with `idle_timeout_secs` at the floor, a session the
/// heartbeat no longer touches is force-closed by `remote_session_idle_check`,
/// the host's held keys are flushed (MemorySink keys_down emptied via the
/// Leave-driven release) and the slot clears.
#[tokio::test(flavor = "multi_thread")]
async fn m6_4_remote_session_idle_timeout_flushes_inputs() {
    let (host, viewer, _ga, _gb) = pair_view().await;
    let host_id = host.identity().device_id().clone();

    viewer
        .remote_session_start(
            host_id.to_string().as_str(),
            tetherly_core::RemoteSessionReason::UserInitiated,
        )
        .await
        .expect("session start");
    wait_session_controlling(&viewer, Duration::from_secs(5)).await;

    // Hold a key so the flush assertion has something to observe.
    host.drive_input_key(0x1E, true).unwrap();

    // Floor the idle budget after the key is held; the heartbeat keeps the
    // tunnel table alive but the session sweep uses its own clock.
    let mut policy = host.input_policy();
    policy.idle_timeout_secs = 1;
    host.set_input_policy(policy).unwrap();

    // Simulate silence: pause the controller's session heartbeats — they are
    // what refreshes the host slot now, so real silence means no heartbeat
    // traffic at all. (The policy floor clamps to 10s, hence the 15s budget
    // below.) Then run the same sweep the watcher tick runs every second.
    viewer.set_session_heartbeat_paused(true);
    let started = std::time::Instant::now();
    loop {
        let closed = host.remote_session_idle_check();
        if closed {
            break;
        }
        if started.elapsed() > Duration::from_secs(15) {
            let host_rep = host.remote_session_report();
            panic!(
                "idle sweep never force-closed the session; host_session={:?} viewer_session={:?}",
                host_rep.session_id,
                viewer.remote_session_report().session_id
            );
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    wait_session_gone(&host, Duration::from_secs(3)).await;
    wait_session_gone(&viewer, Duration::from_secs(3)).await;

    // Held key flushed: the Leave-driven release emptied keys_down.
    let start = std::time::Instant::now();
    loop {
        let sink = host.input_sink_snapshot();
        if sink.keys_down.is_empty() {
            break;
        }
        if start.elapsed() > Duration::from_secs(3) {
            panic!(
                "keys_down not flushed after idle close: {:?}",
                sink.keys_down
            );
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(host.input_engine_seat(), tetherly_core::CursorSeat::Local);

    host.shutdown();
    viewer.shutdown();
    drop(_ga);
    drop(_gb);
}

/// m6_5 — a session whose screen flow is refused rolls the viewer back to
/// Idle, clears both slots, and the host learns via Close rather than idling
/// Busy. Forced deterministically: consent is revoked on the host *between*
/// its session accept (auto-allow) and the viewer's screen Start, so the
/// host's Start-refusal `Stop` reply reaches the viewer and `screen_start`
/// errors — the viewer tears down (slot cleared + Close sent) and the host's
/// session slot is freed by the received Close.
#[tokio::test(flavor = "multi_thread")]
async fn m6_5_remote_session_screen_failure_returns_to_idle() {
    let (host, viewer, _ga, _gb) = pair_view().await;
    let host_id = host.identity().device_id().clone();

    // Revoke consent while the request is in flight: the accept handler's
    // auto-allow wins the race, then this revoke closes it before the
    // viewer's screen Start arrives.
    let revoke_host = host.clone_handle();
    tokio::spawn(async move {
        let start = std::time::Instant::now();
        loop {
            if matches!(
                revoke_host.screen_state(),
                ScreenState::Allowed | ScreenState::Streaming
            ) {
                revoke_host.screen_revoke();
                return;
            }
            if start.elapsed() > Duration::from_secs(5) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    });

    let err = viewer
        .remote_session_start(
            host_id.to_string().as_str(),
            tetherly_core::RemoteSessionReason::UserInitiated,
        )
        .await
        .expect_err("refused screen flow must fail the session start");
    assert!(
        err.to_string().contains("refused") || err.to_string().contains("screen"),
        "unexpected failure mode: {err}"
    );

    wait_session_gone(&viewer, Duration::from_secs(3)).await;
    wait_session_gone(&host, Duration::from_secs(3)).await;
    assert_eq!(
        host.input_seat(),
        Some(tetherly_core::CursorSeat::Local),
        "host seat must stay local — no session Enter ever landed"
    );

    host.shutdown();
    viewer.shutdown();
    drop(_ga);
    drop(_gb);
}

/// m6_7 — session-scoped screen consent (root fix #9): the host's accept-path
/// auto-allow is revoked when the session ends, so the peer cannot re-dial
/// 45720 without a fresh grant; an operator allow that predates the session
/// survives the teardown. Verified via `screen_state()` on the host.
#[tokio::test(flavor = "multi_thread")]
async fn m6_7_remote_session_teardown_revokes_auto_allowed_screen() {
    let (host, viewer, _ga, _gb) = pair_view().await;
    let host_id = host.identity().device_id().clone();

    // Full session lifecycle: request → accept (auto-allow) → controlling.
    viewer
        .remote_session_start(
            host_id.to_string().as_str(),
            tetherly_core::RemoteSessionReason::UserInitiated,
        )
        .await
        .expect("session start must succeed");
    wait_session_controlling(&viewer, Duration::from_secs(10)).await;
    assert!(
        matches!(
            host.screen_state(),
            ScreenState::Allowed | ScreenState::Streaming
        ),
        "host screen must be allowed while the session is active, got {:?}",
        host.screen_state()
    );

    // Controller-initiated stop → host receives Close → auto-allow revoked.
    viewer
        .remote_session_stop("m6_7")
        .await
        .expect("session stop must succeed");
    wait_session_gone(&host, Duration::from_secs(3)).await;
    assert_eq!(
        host.screen_state(),
        ScreenState::Requested,
        "session-scoped screen consent must be revoked after the session — \
         the peer must not stream again without a fresh operator grant. \
         Revocation lands in `Requested` (not the channel-death `Idle`): the \
         45720 channel survives the session and is reused by the next one, \
         and an Idle engine would refuse every later Start while the \
         still-attached receiver would black-screen on a reset next_seq"
    );

    // Operator-granted consent survives a session teardown. The operator
    // grants for a *connected* peer: a real 45720 dial moves the engine to
    // Requested (the accept handler runs on_peer_connected), then allow —
    // the same shape as the m5_2 real-handshake path.
    viewer
        .open_screen(host.identity().device_id())
        .await
        .expect("viewer must reach the host's 45720 screen channel");
    let start = std::time::Instant::now();
    loop {
        if host.screen_state() == ScreenState::Requested {
            break;
        }
        if start.elapsed() > Duration::from_secs(3) {
            panic!(
                "host never reached Requested after the operator dial, state={:?}",
                host.screen_state()
            );
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    host.screen_allow();
    assert!(
        matches!(host.screen_state(), ScreenState::Allowed),
        "operator allow on a connected peer must grant consent, got {:?}",
        host.screen_state()
    );
    viewer
        .remote_session_start(
            host_id.to_string().as_str(),
            tetherly_core::RemoteSessionReason::UserInitiated,
        )
        .await
        .expect("second session start must succeed");
    wait_session_controlling(&viewer, Duration::from_secs(10)).await;
    viewer
        .remote_session_stop("m6_7b")
        .await
        .expect("second session stop must succeed");
    wait_session_gone(&host, Duration::from_secs(3)).await;
    assert!(
        matches!(host.screen_state(), ScreenState::Allowed),
        "operator consent must survive session teardown, got {:?}",
        host.screen_state()
    );

    host.shutdown();
    viewer.shutdown();
    drop(_ga);
    drop(_gb);
}

/// m6_8 — the cross-session 45720 channel-reuse path must survive REAL frame
/// flow (the m6 series previously only exercised the control plane). Session
/// 1 streams frames, ends, consent is revoked, and session 2 REUSES the
/// still-open channel (no re-dial). Two regression classes live here:
/// 1. session 2 must start at all — revocation must land in `Requested`
///    (re-allowable), not the channel-death `Idle` that refuses every Start;
/// 2. no seq cliff — revocation must keep the sender's `next_seq` monotonic
///    because the receiver instance is NOT recreated on reuse and would
///    silently drop every frame until seq caught up with `last_seq`.
#[tokio::test(flavor = "multi_thread")]
async fn m6_8_reused_channel_second_session_streams_frames() {
    let (host, viewer, _ga, _gb) = pair_view().await;
    let host_id = host.identity().device_id().clone();

    // ---- Session 1: full lifecycle with real frames on the wire. ----
    viewer
        .remote_session_start(
            host_id.to_string().as_str(),
            tetherly_core::RemoteSessionReason::UserInitiated,
        )
        .await
        .expect("session 1 start must succeed");
    wait_session_controlling(&viewer, Duration::from_secs(10)).await;

    let start = std::time::Instant::now();
    loop {
        host.screen_send_frame()
            .await
            .expect("session 1 frame send");
        let rx = viewer.screen_rx_stats();
        if rx.received >= 3 {
            break;
        }
        if start.elapsed() > Duration::from_secs(5) {
            panic!(
                "session 1 never streamed frames to the viewer; received={}",
                rx.received
            );
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let session1_received = viewer.screen_rx_stats().received;
    assert!(
        session1_received >= 3,
        "precondition: session 1 must have advanced the receiver's sequence"
    );

    viewer
        .remote_session_stop("m6_8 s1")
        .await
        .expect("session 1 stop must succeed");
    wait_session_gone(&host, Duration::from_secs(3)).await;
    assert_eq!(
        host.screen_state(),
        ScreenState::Requested,
        "revocation must leave the engine re-allowable (Requested), not Idle"
    );

    // ---- Session 2: the reuse path (screen_ctl + screen_peer still point
    // at the host) — must start without a fresh dial and keep streaming. ----
    viewer
        .remote_session_start(
            host_id.to_string().as_str(),
            tetherly_core::RemoteSessionReason::UserInitiated,
        )
        .await
        .expect(
            "session 2 start MUST succeed on the reused channel — an Idle \
             engine or a seq reset here is the regression this test pins",
        );
    wait_session_controlling(&viewer, Duration::from_secs(10)).await;

    let start = std::time::Instant::now();
    loop {
        host.screen_send_frame()
            .await
            .expect("session 2 frame send");
        let rx = viewer.screen_rx_stats();
        if rx.received >= session1_received + 3 {
            break;
        }
        if start.elapsed() > Duration::from_secs(5) {
            panic!(
                "session 2 frames not arriving after channel reuse; \
                 received={} (session 1 got {session1_received}), dropped={}",
                rx.received, rx.dropped
            );
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let rx = viewer.screen_rx_stats();
    assert_eq!(
        rx.dropped, 0,
        "reused-channel session must not drop a stale-seq run — the sender \
         sequence must continue monotonically across sessions"
    );

    viewer
        .remote_session_stop("m6_8 s2")
        .await
        .expect("session 2 stop must succeed");
    wait_session_gone(&host, Duration::from_secs(3)).await;

    host.shutdown();
    viewer.shutdown();
    drop(_ga);
    drop(_gb);
}

/// m6_9 — the controller's session heartbeat keeps the HOST's session slot
/// alive. Before the heartbeat→`remote_session.last_activity_ms` wiring, the
/// host-side timestamp was written only once at accept and the idle sweep
/// force-closed EVERY session after `idle_timeout_secs`, active or not.
#[tokio::test(flavor = "multi_thread")]
async fn m6_9_session_heartbeat_keeps_host_slot_alive() {
    let (host, viewer, _ga, _gb) = pair_view().await;
    let host_id = host.identity().device_id().clone();

    viewer
        .remote_session_start(
            host_id.to_string().as_str(),
            tetherly_core::RemoteSessionReason::UserInitiated,
        )
        .await
        .expect("session start");
    wait_session_controlling(&viewer, Duration::from_secs(10)).await;

    // The policy floor clamps `idle_timeout_secs` to 10s. Without the
    // heartbeat wiring the sweep closes the session at accept+10s; with it,
    // the 2s heartbeats keep the host slot fresh forever. Run past 12s —
    // past the floor — and demand both slots are still alive.
    let mut policy = host.input_policy();
    policy.idle_timeout_secs = 2;
    host.set_input_policy(policy).unwrap();

    tokio::time::sleep(Duration::from_secs(12)).await;
    let viewer_rep = viewer.remote_session_report();
    assert!(
        viewer_rep.session_id.is_some(),
        "controller session slot must survive while heartbeats flow"
    );
    let host_rep = host.remote_session_report();
    assert!(
        host_rep.session_id.is_some(),
        "host session slot must be kept alive by the controller's heartbeats — \
         a 300s-style force close during active use is the P1 regression"
    );
    // And the sweep itself must agree the session is not expired.
    assert!(
        !host.remote_session_idle_check(),
        "idle sweep must not fire while the controller's heartbeats keep \
         the host slot fresh"
    );

    viewer
        .remote_session_stop("m6_9")
        .await
        .expect("session stop");
    wait_session_gone(&host, Duration::from_secs(3)).await;

    host.shutdown();
    viewer.shutdown();
    drop(_ga);
    drop(_gb);
}

/// m6_6 — keymap end-to-end: the host suggests `mac_to_windows`; the session
/// resp carries it; a Mac-source VK_LWIN from the viewer remaps to
/// VK_LCONTROL on the wire and the host MemorySink records VK_LCONTROL.
#[tokio::test(flavor = "multi_thread")]
async fn m6_6_remote_session_keymap_end_to_end() {
    let (host, viewer, _ga, _gb) = pair_view().await;
    let host_id = host.identity().device_id().clone();

    let mut policy = host.input_policy();
    policy.keymap = Some("mac_to_windows".to_string());
    host.set_input_policy(policy).unwrap();

    let rep = viewer
        .remote_session_start(
            host_id.to_string().as_str(),
            tetherly_core::RemoteSessionReason::UserInitiated,
        )
        .await
        .expect("session start");
    assert_eq!(rep.message, "remote session controlling");

    // The viewer drives a Mac Command press (VK_LWIN) in Session mode; the
    // host-side policy remap applies on the wire and lands as VK_LCONTROL.
    const VK_LWIN: u16 = 0x5B;
    const VK_LCONTROL: u16 = 0xA2;
    viewer.drive_input_key(VK_LWIN, true).unwrap();
    viewer.drive_input_key(VK_LWIN, false).unwrap();

    let start = std::time::Instant::now();
    loop {
        let sink = host.input_sink_snapshot();
        let remapped = sink.applied.iter().any(|k| {
            matches!(
                k,
                tetherly_core::InputKind::Key {
                    code: VK_LCONTROL,
                    down: true,
                    ..
                }
            )
        });
        if remapped {
            break;
        }
        if start.elapsed() > Duration::from_secs(5) {
            panic!("host sink never received remapped VK_LCONTROL");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    viewer.remote_session_stop("m6_6 end").await.expect("stop");
    wait_session_gone(&host, Duration::from_secs(3)).await;

    host.shutdown();
    viewer.shutdown();
    drop(_ga);
    drop(_gb);
}
