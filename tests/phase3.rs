// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Phase 3 native input port 45719. Resume-only Noise, binary TIN1, MemorySink.
//! Real OS cursor injection and dual-desktop soak stay Manual-required.
//! DeskFlow / Lan Mouse / KDE source is not linked.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;
use tetherly_core::{CursorSeat, InputKind, VK_LCONTROL, VK_LWIN};
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
        "tetherly-p3-{tag}-{}-{}-{n}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    (dir.clone(), DirGuard(dir))
}

fn cfg(dir: PathBuf, name: &str) -> NodeConfig {
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
        screen_source: None,
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

async fn pair() -> (Node, Node, DirGuard, DirGuard) {
    let (da, ga) = temp_dir("a");
    let (db, gb) = temp_dir("b");
    let a = Node::start(cfg(da, "server")).unwrap();
    let b = Node::start(cfg(db, "client")).unwrap();
    a.spawn_listeners().await.unwrap();
    b.spawn_listeners().await.unwrap();
    let pin = tetherly_core::pin::parse_pin("25170394").unwrap();
    a.set_pin(pin);
    b.set_pin(pin);
    a.dial(b.control_addr()).await.unwrap();
    wait_live(&a, Duration::from_secs(25)).await;
    wait_live(&b, Duration::from_secs(25)).await;
    wait_input_port(&a, b.identity().device_id(), Duration::from_secs(3)).await;
    (a, b, ga, gb)
}

#[tokio::test(flavor = "multi_thread")]
async fn unpaired_45719_refuses_pairing() {
    let (dir, _g) = temp_dir("unpaired");
    let host = Node::start(cfg(dir, "host")).unwrap();
    host.spawn_listeners().await.unwrap();
    let stream = TcpStream::connect(host.input_addr()).await.unwrap();
    let addr = stream.peer_addr().unwrap();
    let stranger = tetherly_crypto::Identity::generate().unwrap();
    let cfg = SessionConfig {
        identity: stranger,
        name: "stranger".into(),
        platform: "windows".into(),
        trust: std::sync::Arc::new(std::sync::Mutex::new(Default::default())),
        lockout: std::sync::Arc::new(std::sync::Mutex::new(Default::default())),
        tcp_limiter: std::sync::Arc::new(std::sync::Mutex::new(Default::default())),
        pin: Some(tetherly_core::pin::parse_pin("25170394").unwrap()),
        pin_created_ms: 1,
        clock: std::sync::Arc::new(tetherly_core::SystemClock),
        handshake_timeout: HANDSHAKE_TIMEOUT,
        hello_override: None,
        resume_only: true,
        screen_only: false,
        caps: None,
    };
    let err = match dial_session(stream, addr, &cfg).await {
        Err(e) => e,
        Ok(_) => panic!("unpaired input dial must fail"),
    };
    assert!(
        matches!(err, NetError::InputRequiresTrust),
        "unpaired 45719 must refuse pairing, got {err}"
    );
    // The listener bound an ephemeral port (config asked for 0); a zero here
    // would mean it never came up. `input_addr` dials this same port.
    assert_ne!(host.input_port(), 0, "input listener did not bind");
    let _ = SocketAddr::from(([127, 0, 0, 1], host.input_port()));
}

#[tokio::test(flavor = "multi_thread")]
async fn m3_1_edge_and_key_follow_on_45719() {
    let (server, client, _ga, _gb) = pair().await;
    server
        .open_input(client.identity().device_id())
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(80)).await;

    let edge = server.drive_input(100, 10, 100, 50).unwrap();
    assert_eq!(edge, Some(tetherly_core::ScreenEdge::Right));
    server.drive_input_key(0x1E, true).unwrap();
    server.drive_input_key(0x1E, false).unwrap();

    let start = std::time::Instant::now();
    loop {
        let sink = client.input_sink_snapshot();
        let keyed = sink.applied.iter().any(|k| {
            matches!(
                k,
                InputKind::Key {
                    code: 0x1E,
                    down: true,
                    ..
                }
            )
        });
        if keyed && sink.cursor.0 == 0 {
            break;
        }
        if start.elapsed() > Duration::from_secs(3) {
            panic!("input did not land: {sink:?}");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    server.shutdown();
    client.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn m3_2_peer_gone_returns_cursor() {
    let (server, client, _ga, _gb) = pair().await;
    server
        .open_input(client.identity().device_id())
        .await
        .unwrap();
    server.drive_input(80, 5, 80, 40).unwrap();
    assert_eq!(server.input_engine_seat(), CursorSeat::Remote);
    client.shutdown();
    let start = std::time::Instant::now();
    loop {
        if server.input_engine_seat() == CursorSeat::Local {
            break;
        }
        if start.elapsed() > Duration::from_secs(3) {
            panic!("cursor did not return home");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    server.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn m3_3_clip_still_works_while_input_focused() {
    let (server, client, _ga, _gb) = pair().await;
    server
        .open_input(client.identity().device_id())
        .await
        .unwrap();
    server.drive_input(120, 20, 120, 60).unwrap();
    let start = std::time::Instant::now();
    while client.input_seat() != Some(CursorSeat::Remote) {
        if start.elapsed() > Duration::from_secs(3) {
            panic!("client never focused");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    server.clip().set_text("still-clip").unwrap();
    server.send_clipboard().await.unwrap();
    let start = std::time::Instant::now();
    loop {
        if client.clip().get_text().unwrap().as_deref() == Some("still-clip") {
            break;
        }
        if start.elapsed() > Duration::from_secs(3) {
            panic!("clip.set failed while input focused");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    server.shutdown();
    client.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn m3_4_input_keymap_remapping() {
    let (server, client, _ga, _gb) = pair().await;
    // Configure keymap profile to mac_to_windows on server
    let mut policy = server.input_policy();
    policy.keymap = Some("mac_to_windows".to_string());
    server.set_input_policy(policy).unwrap();

    server
        .open_input(client.identity().device_id())
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(80)).await;

    // Cross edge to set seat to Remote
    let edge = server.drive_input(100, 10, 100, 50).unwrap();
    assert_eq!(edge, Some(tetherly_core::ScreenEdge::Right));

    // Send VK_LWIN (Mac Command) -> should be remapped to VK_LCONTROL on wire
    server.drive_input_key(VK_LWIN, true).unwrap();
    server.drive_input_key(VK_LWIN, false).unwrap();

    let start = std::time::Instant::now();
    loop {
        let sink = client.input_sink_snapshot();
        let mapped = sink.applied.iter().any(|k| {
            matches!(
                k,
                InputKind::Key {
                    code: VK_LCONTROL,
                    down: true,
                    ..
                }
            )
        });
        if mapped {
            break;
        }
        if start.elapsed() > Duration::from_secs(3) {
            panic!("remapped key VK_LCONTROL not received: {sink:?}");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // Negative: an unknown keymap name must fall back to identity — the
    // untouched VK_LWIN must arrive verbatim (and only once, no double-send
    // of the raw key next to a remapped copy).
    let mut policy = server.input_policy();
    policy.keymap = Some("bogus_profile".to_string());
    server.set_input_policy(policy).unwrap();
    // set_input_policy flushed the remote session on the keymap change
    // (Leave+Enter+Move); wait out the reconnect settle window.
    tokio::time::sleep(Duration::from_millis(80)).await;
    server.drive_input_key(VK_LWIN, true).unwrap();
    server.drive_input_key(VK_LWIN, false).unwrap();
    let start = std::time::Instant::now();
    loop {
        let sink = client.input_sink_snapshot();
        let raw = sink.applied.iter().any(|k| {
            matches!(
                k,
                InputKind::Key {
                    code: VK_LWIN,
                    down: true,
                    ..
                }
            )
        });
        if raw {
            break;
        }
        if start.elapsed() > Duration::from_secs(3) {
            panic!("unknown keymap did not fall back to identity: {sink:?}");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    server.shutdown();
    client.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn m3_5_four_directional_edge_roaming() {
    let (server, client, _ga, _gb) = pair().await;

    // 1. Configure policy to Top edge roaming (e.g. laptop below monitor/desktop)
    let mut policy = server.input_policy();
    policy.edge = Some("top".to_string());
    server.set_input_policy(policy).unwrap();
    assert_eq!(
        server.input_policy().resolved_edge(),
        tetherly_core::ScreenEdge::Top
    );

    // Negative: from the LOCAL seat, driving into the RIGHT boundary must
    // NOT cross while edge=Top. If the engine ignored the configured edge
    // and fell back to the historic Right default, this would return
    // Some(Right).
    let crossed = server.drive_input(100, 25, 100, 50).unwrap();
    assert_eq!(crossed, None, "right edge must not cross under edge=Top");
    assert_eq!(server.input_engine_seat(), CursorSeat::Local);

    server
        .open_input(client.identity().device_id())
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(80)).await;

    // Moving to y <= 0 crosses the Top boundary
    let crossed = server.drive_input(50, 0, 100, 50).unwrap();
    assert_eq!(crossed, Some(tetherly_core::ScreenEdge::Top));
    assert_eq!(server.input_engine_seat(), CursorSeat::Remote);

    // Wait for client to focus and receive cursor entry at remote bottom (y = 1000)
    let start = std::time::Instant::now();
    loop {
        let sink = client.input_sink_snapshot();
        if client.input_seat() == Some(CursorSeat::Remote) && sink.cursor.1 >= 990 {
            break;
        }
        if start.elapsed() > Duration::from_secs(3) {
            panic!(
                "client never focused or cursor did not land at y=1000: seat={:?}, sink={:?}",
                client.input_seat(),
                sink
            );
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // Deskflow semantics: the entry x is the proportional map of the exit x
    // (local x=50 on a 100px screen -> 50*1000/99 = 505), NOT a stale
    // remembered position from the previous roaming session.
    {
        let sink = client.input_sink_snapshot();
        assert_eq!(
            sink.cursor.0, 505,
            "entry x must be proportional to the local exit x"
        );
    }

    // Roam into the target interior first (the entry parks at the shared
    // edge and the interior hysteresis guards against accidental instant
    // returns), then push back down through the shared edge. The local
    // cursor must land at the inverse-proportional x (505*99/1000 = 49),
    // parked 16px below the top edge — not at the parked hook position.
    server.drive_input_delta(50, 30, 0, -40, 100, 50).unwrap();
    assert_eq!(server.input_engine_seat(), CursorSeat::Remote);
    server.drive_input_delta(50, 30, 0, 50, 100, 50).unwrap();
    assert_eq!(server.input_engine_seat(), CursorSeat::Local);
    let (lx, ly) = server.input_engine_local_pos();
    assert_eq!(ly, 16);
    assert_eq!(lx, 49, "return x = denorm(remote x)");

    server.shutdown();
    client.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn m3_6_left_edge_roaming() {
    let (server, client, _ga, _gb) = pair().await;

    let mut policy = server.input_policy();
    policy.edge = Some("left".to_string());
    server.set_input_policy(policy).unwrap();
    assert_eq!(
        server.input_policy().resolved_edge(),
        tetherly_core::ScreenEdge::Left
    );

    server
        .open_input(client.identity().device_id())
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(80)).await;

    // Moving to x <= 0 crosses the Left boundary
    let crossed = server.drive_input(0, 25, 100, 50).unwrap();
    assert_eq!(crossed, Some(tetherly_core::ScreenEdge::Left));
    assert_eq!(server.input_engine_seat(), CursorSeat::Remote);

    // Wait for client to focus and receive cursor entry at remote right (x = 1000)
    let start = std::time::Instant::now();
    loop {
        let sink = client.input_sink_snapshot();
        if client.input_seat() == Some(CursorSeat::Remote) && sink.cursor.0 == 1000 {
            break;
        }
        if start.elapsed() > Duration::from_secs(3) {
            panic!(
                "client never focused or cursor did not land at x=1000: seat={:?}, sink={:?}",
                client.input_seat(),
                sink
            );
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    server.shutdown();
    client.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn m3_7_physical_boundary_touch_crossing() {
    let (server, client, _ga, _gb) = pair().await;

    // Test Right edge physical touch at width - 1 (e.g. 1919 on a 1920 display)
    let mut policy = server.input_policy();
    policy.edge = Some("right".to_string());
    server.set_input_policy(policy).unwrap();

    server
        .open_input(client.identity().device_id())
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(80)).await;

    // Moving to width - 1 (1919) must trigger crossing
    let crossed = server.drive_input(1919, 500, 1920, 1080).unwrap();
    assert_eq!(crossed, Some(tetherly_core::ScreenEdge::Right));
    assert_eq!(server.input_engine_seat(), CursorSeat::Remote);

    // Switch policy to Bottom edge roaming and verify height - 1 (1079) touch
    let mut policy = server.input_policy();
    policy.edge = Some("bottom".to_string());
    server.set_input_policy(policy).unwrap();

    // Reset cursor locally
    server.input_stop().await.unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    server
        .open_input(client.identity().device_id())
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(80)).await;

    let crossed = server.drive_input(500, 1079, 1920, 1080).unwrap();
    assert_eq!(crossed, Some(tetherly_core::ScreenEdge::Bottom));
    assert_eq!(server.input_engine_seat(), CursorSeat::Remote);

    server.shutdown();
    client.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn m3_8_wheel_speed_policy_mutation() {
    let (server, _client, _ga, _gb) = pair().await;

    // 1. Initial default wheel_speed is 1.0
    let policy = server.input_policy();
    assert!((policy.wheel_speed - 1.0).abs() < f64::EPSILON);

    // 2. Direct mutation via set_input_policy
    let mut policy = server.input_policy();
    policy.wheel_speed = 1.75;
    server.set_input_policy(policy).unwrap();
    assert!((server.input_policy().wheel_speed - 1.75).abs() < f64::EPSILON);

    // 3. Mutation via update_input_policy helper
    let update = tetherly_node::InputTunnelPolicyUpdate {
        wheel_speed: Some(2.5),
        ..Default::default()
    };
    server.update_input_policy(update).unwrap();
    assert!((server.input_policy().wheel_speed - 2.5).abs() < f64::EPSILON);

    // 4. Boundary guard: out-of-range (>20.0 or <=0.0) is rejected/ignored
    let invalid_update = tetherly_node::InputTunnelPolicyUpdate {
        wheel_speed: Some(25.0),
        ..Default::default()
    };
    server.update_input_policy(invalid_update).unwrap();
    // Must remain 2.5
    assert!((server.input_policy().wheel_speed - 2.5).abs() < f64::EPSILON);

    server.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn m3_9_reverse_control_takeover_flips_roles_cleanly() {
    let (server, client, _ga, _gb) = pair().await;

    // 1. Forward direction: server (Mac) attaches as controller.
    server
        .input_tunnel_attach(
            client.identity().device_id().to_string().as_str(),
            Some(tetherly_core::DisplayEdge::Top),
            None,
            None,
        )
        .await
        .unwrap();
    let start = std::time::Instant::now();
    while client.active_tunnel_role() != Some(tetherly_core::InputTunnelRole::Target) {
        if start.elapsed() > Duration::from_secs(3) {
            panic!("target never registered the inbound tunnel");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(
        server.active_tunnel_role(),
        Some(tetherly_core::InputTunnelRole::Controller)
    );
    // role=Controller is recorded when the request is SENT; the data channel
    // opens on the peer's accepted response — poll instead of asserting.
    let start = std::time::Instant::now();
    while !server.input_channel_open_sync() {
        if start.elapsed() > Duration::from_secs(3) {
            panic!("controller never opened its outbound channel");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 2. Reverse control: client (Windows) attaches. The polite-takeover
    //    path must demote the old controller FIRST, leaving exactly one
    //    outbound channel in the new direction — never two.
    client
        .input_tunnel_attach(
            server.identity().device_id().to_string().as_str(),
            Some(tetherly_core::DisplayEdge::Bottom),
            None,
            None,
        )
        .await
        .unwrap();

    let start = std::time::Instant::now();
    while server.active_tunnel_role() != Some(tetherly_core::InputTunnelRole::Target) {
        if start.elapsed() > Duration::from_secs(3) {
            panic!(
                "old controller never demoted to target; roles: server={:?}, client={:?}",
                server.active_tunnel_role(),
                client.active_tunnel_role()
            );
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(
        client.active_tunnel_role(),
        Some(tetherly_core::InputTunnelRole::Controller)
    );

    // 3. Exactly one outbound channel exists, and it belongs to the NEW
    //    controller. The demoted side must have released its own.
    assert!(
        client.input_channel_open_sync(),
        "new controller must hold the outbound channel"
    );
    assert!(
        !server.input_channel_open_sync(),
        "demoted controller must have released its outbound channel"
    );

    // 4. The new direction actually drives the old controller's screen:
    //    the client (now controller) crosses its own bottom edge (its
    //    policy edge points down toward the Mac) and the server receives
    //    the entry frame.
    let mut pol = client.input_policy();
    pol.edge = Some("bottom".into());
    client.set_input_policy(pol).unwrap();
    client
        .open_input(server.identity().device_id())
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(80)).await;
    let edge = client.drive_input(50, 50, 100, 50).unwrap();
    assert_eq!(edge, Some(tetherly_core::ScreenEdge::Bottom));

    server.shutdown();
    client.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn m3_10_auto_reverse_edge_push_takes_over_without_click() {
    let (server, client, _ga, _gb) = pair().await;

    // 1. Forward direction: server attaches as controller (Mac drives Windows).
    server
        .input_tunnel_attach(
            client.identity().device_id().to_string().as_str(),
            Some(tetherly_core::DisplayEdge::Top),
            None,
            None,
        )
        .await
        .unwrap();
    let start = std::time::Instant::now();
    while client.active_tunnel_role() != Some(tetherly_core::InputTunnelRole::Target) {
        if start.elapsed() > Duration::from_secs(3) {
            panic!("target never registered the inbound tunnel");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 2. The target declares its own reverse direction (Windows bottom edge
    //    points down toward the Mac).
    let mut pol = client.input_policy();
    pol.edge = Some("bottom".into());
    client.set_input_policy(pol).unwrap();

    // 3. Deskflow switch-at-the-edge: the mouse first proves real use by
    //    traversing the arming range (300px), then a sustained push through
    //    the target's own edge (touch events arriving within the gap
    //    window for >= delay_ms) fires the polite takeover — no UI click.
    for i in 0..13 {
        client
            .drive_input_delta(10 + i * 20, 40, 0, 0, 100, 50)
            .unwrap();
    }
    for _ in 0..8 {
        client.drive_input_delta(50, 49, 0, 5, 100, 50).unwrap();
        tokio::time::sleep(Duration::from_millis(30)).await;
    }

    let start = std::time::Instant::now();
    while client.active_tunnel_role() != Some(tetherly_core::InputTunnelRole::Controller) {
        if start.elapsed() > Duration::from_secs(3) {
            panic!(
                "edge-push takeover never happened; server={:?}, client={:?}",
                server.active_tunnel_role(),
                client.active_tunnel_role()
            );
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let start = std::time::Instant::now();
    while server.active_tunnel_role() != Some(tetherly_core::InputTunnelRole::Target) {
        if start.elapsed() > Duration::from_secs(3) {
            panic!("old controller never demoted to target");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 4. Single-outbound-channel invariant still holds after the flip.
    // The client records role=Controller when SENDING the takeover request;
    // its data channel opens once the peer's accepted response lands, so
    // poll for it (slow CI runners race the immediate assert).
    let start = std::time::Instant::now();
    while !client.input_channel_open_sync() {
        if start.elapsed() > Duration::from_secs(3) {
            panic!("new controller never opened its outbound channel");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(!server.input_channel_open_sync());

    // 5. The new direction actually drives the old controller's screen.
    //    During the takeover the sustained push already carried the engine
    //    into the Remote seat (the crossing frames sat in the grace buffer
    //    until the fresh channel flushed them), so further pushes continue
    //    free roaming instead of re-crossing — what must be asserted is
    //    that the Mac's sink receives the motion, polled below.
    // Downward (deeper into the Mac): the entry parked at the Mac's top
    // edge (rem_y = 0), pushing up would auto-return to Windows.
    client.drive_input_delta(50, 40, 0, 10, 100, 50).unwrap();
    let start = std::time::Instant::now();
    loop {
        let sink = server.input_sink_snapshot();
        if server.input_seat() == Some(CursorSeat::Remote) && sink.cursor.0 >= 500 {
            break;
        }
        if start.elapsed() > Duration::from_secs(3) {
            panic!(
                "reverse direction never drove the old controller's screen: seat={:?}, sink={:?}",
                server.input_seat(),
                sink
            );
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    server.shutdown();
    client.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn m3_11_auto_reverse_gates_block_accidental_flips() {
    let (server, client, _ga, _gb) = pair().await;

    // Forward direction: server is controller, client is target.
    server
        .input_tunnel_attach(
            client.identity().device_id().to_string().as_str(),
            Some(tetherly_core::DisplayEdge::Top),
            None,
            None,
        )
        .await
        .unwrap();
    let start = std::time::Instant::now();
    while client.active_tunnel_role() != Some(tetherly_core::InputTunnelRole::Target) {
        if start.elapsed() > Duration::from_secs(3) {
            panic!("target never registered the inbound tunnel");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let mut pol = client.input_policy();
    pol.edge = Some("bottom".into());
    client.set_input_policy(pol).unwrap();

    // Gate: a single touch of the shared edge must NOT flip. The dwell
    // requires touch events to keep arriving for delay_ms; one isolated
    // push has no continuation, so no takeover may occur.
    client.drive_input_delta(50, 49, 0, 5, 100, 50).unwrap();
    tokio::time::sleep(Duration::from_millis(700)).await;
    assert_eq!(
        client.active_tunnel_role(),
        Some(tetherly_core::InputTunnelRole::Target),
        "single isolated edge touch must not take over"
    );

    // Gate: two isolated bumps 300ms apart (gap > TOUCH_GAP_MS) must NOT
    // flip either — this is exactly the accidental parked-mouse-bump
    // pattern the gap-continuity dwell exists to fence. (A plain
    // elapsed-since-first-touch dwell would flip here.)
    client.drive_input_delta(50, 49, 0, 5, 100, 50).unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    client.drive_input_delta(50, 49, 0, 5, 100, 50).unwrap();
    tokio::time::sleep(Duration::from_millis(700)).await;
    assert_eq!(
        client.active_tunnel_role(),
        Some(tetherly_core::InputTunnelRole::Target),
        "two isolated bumps with a 300ms gap must not take over"
    );

    // Gate: distance arming. After an idle gap (2s) the accumulator is
    // reset — a freshly plugged-in mouse that only reaches the edge with a
    // few px of jitter can never start the takeover, no matter how long it
    // pushes.
    tokio::time::sleep(Duration::from_millis(2200)).await;
    for _ in 0..10 {
        client.drive_input_delta(50, 49, 0, 3, 100, 50).unwrap();
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    tokio::time::sleep(Duration::from_millis(700)).await;
    assert_eq!(
        client.active_tunnel_role(),
        Some(tetherly_core::InputTunnelRole::Target),
        "a mouse that never traversed the arming range must not take over"
    );

    // The sustained push still works once the mouse has proven real use: a
    // warmup traversal crosses the arming range, then a dense event stream
    // (30ms apart, well under TOUCH_GAP_MS) held past delay_ms fires the
    // takeover.
    for i in 0..16 {
        client
            .drive_input_delta(10 + i * 12, 40, 0, 0, 100, 50)
            .unwrap();
    }
    for _ in 0..12 {
        client.drive_input_delta(50, 49, 0, 5, 100, 50).unwrap();
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    let start = std::time::Instant::now();
    while client.active_tunnel_role() != Some(tetherly_core::InputTunnelRole::Controller) {
        if start.elapsed() > Duration::from_secs(3) {
            panic!(
                "sustained push never took over; server={:?}, client={:?}",
                server.active_tunnel_role(),
                client.active_tunnel_role()
            );
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    server.shutdown();
    client.shutdown();
}
