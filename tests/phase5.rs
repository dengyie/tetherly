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

/// Wait until the *host* has consumed the visitor's `Start` and reports
/// `streaming`.
///
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

// --------------------------------------------------------------------------
// v1.54 / ADR-047 — Screen & input lifecycle governance, typed reports, active
// peer tracking, and event bus integration.
// --------------------------------------------------------------------------

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

// --------------------------------------------------------------------------
// M5.7 — Dynamic FPS & Adaptive Frame Pacing Controller (v1.97 / ADR-083)
// --------------------------------------------------------------------------

// --------------------------------------------------------------------------
// v1.99 / ADR-085 — Screen dynamic content profiles & QP quality controller.
// --------------------------------------------------------------------------

// --------------------------------------------------------------------------
// v2.00 / ADR-086 — Screen streaming 4D autonomous adaptation matrix coordinator.
// --------------------------------------------------------------------------

// --------------------------------------------------------------------------
// M5.11-playout-dejitter — Receiver Playout Jitter Buffer & Dejittering
// Controller (v2.01 / ADR-087)
// --------------------------------------------------------------------------

// ADR-100 Screen Streaming Bottleneck-Driven Autonomous Tuning Closed Loop

// ADR-101 Screen Streaming Session Recording Container & Replay Pipeline

// --------------------------------------------------------------------------
// M5.47 — Screen Streaming Dynamic Tile Saliency, Semantic Importance Tiering
// & Per-Tile Priority Protection Pipeline (ADR-123 / v2.37)
// --------------------------------------------------------------------------

// --------------------------------------------------------------------------
// M5.48 — Screen Streaming Dynamic Spatial Tile Multi-Resolution Level-of-Detail
// Downsampling & Progressive Bilinear Reconstruction Pipeline (ADR-124 / v2.38)
// --------------------------------------------------------------------------

// --------------------------------------------------------------------------
// M5.49: Screen Streaming Dynamic Tile Batching, MTU Chunking & Coalesced
// Pacing Pipeline with Dual CLI Isomorphism (ADR-125 / v2.39)
// --------------------------------------------------------------------------

// --------------------------------------------------------------------------
// M5.50: Screen Streaming Dynamic Tile Local Motion Vector Estimation &
// Inter-Tile Displacement Compensation Pipeline with Dual CLI Isomorphism (ADR-126 / v2.40)
// --------------------------------------------------------------------------
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

    // Stream a substantial run (24 frames): the seq-cliff failure mode drops
    // the WHOLE previous run (>= 24 stale drops) while slow-runner HARQ
    // duplicate delivery contributes only 2-4 counted drops — a full-order
    // of magnitude of assertion margin between the two.
    let start = std::time::Instant::now();
    loop {
        host.screen_send_frame()
            .await
            .expect("session 1 frame send");
        let rx = viewer.screen_rx_stats();
        if rx.received >= 24 {
            break;
        }
        if start.elapsed() > Duration::from_secs(15) {
            panic!(
                "session 1 never streamed frames to the viewer; received={}",
                rx.received
            );
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let session1_received = viewer.screen_rx_stats().received;
    assert!(
        session1_received >= 24,
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
    // Cliff signature, decided from the seq-decision trace: a DEEP drop is a
    // rejected seq far below the accepted high-water mark (the sender
    // restarted). Per-frame duplicate deliveries — the same seq applied once
    // then rejected once, (n,true),(n,false) — are legitimate FEC/HARQ
    // retransmits of an already-applied frame under load and must NOT fail
    // the test. A real reset produces a descending run of deep drops covering
    // the whole previous run.
    let trace = viewer.screen_seq_trace();
    let mut high_water = 0u64;
    let mut deep_drops = 0usize;
    for (seq, accepted) in &trace {
        if *accepted {
            high_water = (*seq).max(high_water);
        } else if *seq + 8 < high_water {
            deep_drops += 1;
        }
    }
    assert!(
        deep_drops < session1_received as usize / 2,
        "reused-channel session hit a stale-seq cliff — a reset next_seq \
         drops deep below the accepted high-water mark for the whole \
         previous run (deep_drops={deep_drops}, session1={session1_received}). \
         Stats: received={}, dropped={}. seq-trace tail: {:?}",
        rx.received,
        rx.dropped,
        trace.iter().rev().take(48).rev().collect::<Vec<_>>()
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

/// m6_10 — the session CONTROLLER's physical mouse must reach the target.
/// The m6_3 target-suppression guard in `drive_input_delta` used to fire
/// unconditionally on `remote_session_active()`, which also dead-ended the
/// controller's seat-Remote delta forwarding: the session's keyboard
/// (drive_input_key, never guarded) worked while the mouse did not — the
/// "Windows mouse cannot control the Mac" production bug.
#[tokio::test(flavor = "multi_thread")]
async fn m6_10_session_controller_mouse_delta_reaches_target() {
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

    // The controller's engine must be in the Remote seat once Controlling
    // (same precondition drive_input_key relies on for m6_6 keys).
    let sink_before = host.input_sink_snapshot().applied.len();

    // Drive a controller-side mouse delta (position + displacement) while
    // the session is live. This is exactly what the macOS CGEventTap feeds
    // on real hardware.
    viewer
        .drive_input_delta(1000, 500, 12, 6, 1920, 1080)
        .expect("controller delta must not error under the session guard");

    let start = std::time::Instant::now();
    loop {
        let sink = host.input_sink_snapshot();
        let moved = sink
            .applied
            .iter()
            .skip(sink_before)
            .any(|k| matches!(k, tetherly_core::InputKind::Move { .. }));
        if moved {
            break;
        }
        if start.elapsed() > Duration::from_secs(5) {
            panic!(
                "session controller's mouse delta never reached the target \
                 sink — the seat-Remote forward path is blocked again"
            );
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    viewer
        .remote_session_stop("m6_10")
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
