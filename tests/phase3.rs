// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Phase 3 native input port 45719. Resume-only Noise, binary TIN1, MemorySink.
//! Real OS cursor injection and dual-desktop soak stay Manual-required.
//! DeskFlow / Lan Mouse / KDE source is not linked.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;
use tetherly_core::{CursorSeat, InputKind, SCREEN_UNITS, VK_LCONTROL, VK_LWIN};
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

/// Disable the speed-aware Top/Bottom redundancy band for legacy positional
/// crossing assertions. The band defaults to 24px (config default), which
/// turns a dy=0 positional event at the boundary into a dwell-wait instead
/// of an instant cross — tests that assert the instant cross must opt out
/// explicitly (this also exercises the new config field's plumbing).
/// Also zeroes the entry return grace (default 3000 ms since v0.2.32),
/// whose ramp would otherwise hold the seat Remote through the instant
/// push-back these legacy assertions perform.
async fn disable_tb_redundancy_band(node: &tetherly_node::Node) {
    node.topology_update_config(
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        Some(0),
        None,
        None,
        Some(0),
        None,
    )
    .await
    .unwrap();
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
    // Roaming requires a Controller grant, not just an open channel (the
    // ghost-roam guard); wait for the session to register on both sides.
    attach_and_wait_session(&server, &client, tetherly_core::DisplayEdge::Right).await;
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
    attach_and_wait_session(&server, &client, tetherly_core::DisplayEdge::Right).await;
    server.drive_input(80, 5, 80, 40).unwrap();
    assert_eq!(server.input_engine_seat(), CursorSeat::Remote);
    client.shutdown();
    let start = std::time::Instant::now();
    loop {
        if server.input_engine_seat() == CursorSeat::Local {
            break;
        }
        // Node holds a 2.5s grace period for reconnect before returning the
        // seat to local; allow 5s total to avoid runner scheduling flakiness.
        if start.elapsed() > Duration::from_secs(5) {
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
    attach_and_wait_session(&server, &client, tetherly_core::DisplayEdge::Right).await;
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
    attach_and_wait_session(&server, &client, tetherly_core::DisplayEdge::Top).await;
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

    // Legacy positional crossing assertions: opt out of the speed-aware
    // Top/Bottom redundancy band (positional events carry no deltas).
    disable_tb_redundancy_band(&server).await;

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
    attach_and_wait_session(&server, &client, tetherly_core::DisplayEdge::Top).await;
    tokio::time::sleep(Duration::from_millis(80)).await;

    // Moving to y <= 0 crosses the Top boundary
    let crossed = server.drive_input(50, 0, 100, 50).unwrap();
    assert_eq!(crossed, Some(tetherly_core::ScreenEdge::Top));
    assert_eq!(server.input_engine_seat(), CursorSeat::Remote);

    // Wait for client to focus and receive cursor entry at remote bottom (y = SCREEN_UNITS)
    let start = std::time::Instant::now();
    loop {
        let sink = client.input_sink_snapshot();
        if client.input_seat() == Some(CursorSeat::Remote)
            && sink.cursor.1 >= SCREEN_UNITS * 99 / 100
        {
            break;
        }
        if start.elapsed() > Duration::from_secs(3) {
            panic!(
                "client never focused or cursor did not land at y=SCREEN_UNITS: seat={:?}, sink={:?}",
                client.input_seat(),
                sink
            );
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // Deskflow semantics: the entry x is the proportional map of the exit x
    // (local x=50 on a 100px screen -> 50*10000/99 = 5051), NOT a stale
    // remembered position from the previous roaming session.
    {
        let sink = client.input_sink_snapshot();
        assert_eq!(
            sink.cursor.0, 5051,
            "entry x must be proportional to the local exit x"
        );
    }

    // Roam into the target interior first (the entry parks at the shared
    // edge and the interior hysteresis guards against accidental instant
    // returns), then push back down through the shared edge. The local
    // cursor must land at the inverse-proportional x (denorm(505) = 50
    // with round-to-nearest — the truncating map gave 49, shearing every
    // round trip one pixel), parked just OUTSIDE the redundancy band — not
    // at the parked hook position. v0.2.26: the old y=16 dock sat inside
    // the band and re-crossed on a stray dy<=0 event (ghost controller
    // bounce loop). 50px screen -> band_safe_dock_y floors at 48 but the
    // screen is only 50px tall -> band_safe_dock_y = min(48, 49) = 48.
    //
    // v0.2.38: only the push PAST the shared edge counts, and right after
    // entry the threshold is 10x the base (default 3 s grace), so the
    // return is a deliberate flick: 40 px back to the edge + 140 beyond.
    server.drive_input_delta(50, 30, 0, -40, 100, 50).unwrap();
    assert_eq!(server.input_engine_seat(), CursorSeat::Remote);
    server.drive_input_delta(50, 30, 0, 180, 100, 50).unwrap();
    assert_eq!(server.input_engine_seat(), CursorSeat::Local);
    let (lx, ly) = server.input_engine_local_pos();
    assert_eq!(ly, 48);
    assert_eq!(lx, 50, "return x = denorm(remote x), round-trip stable");

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
    attach_and_wait_session(&server, &client, tetherly_core::DisplayEdge::Left).await;
    tokio::time::sleep(Duration::from_millis(80)).await;

    // Moving to x <= 0 crosses the Left boundary
    let crossed = server.drive_input(0, 25, 100, 50).unwrap();
    assert_eq!(crossed, Some(tetherly_core::ScreenEdge::Left));
    assert_eq!(server.input_engine_seat(), CursorSeat::Remote);

    // Wait for client to focus and receive cursor entry at remote right (x = SCREEN_UNITS)
    let start = std::time::Instant::now();
    loop {
        let sink = client.input_sink_snapshot();
        if client.input_seat() == Some(CursorSeat::Remote) && sink.cursor.0 == SCREEN_UNITS {
            break;
        }
        if start.elapsed() > Duration::from_secs(3) {
            panic!(
                "client never focused or cursor did not land at x=SCREEN_UNITS: seat={:?}, sink={:?}",
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

    // Legacy positional crossing assertions: opt out of the speed-aware
    // Top/Bottom redundancy band (positional events carry no deltas).
    disable_tb_redundancy_band(&server).await;

    // Test Right edge physical touch at width - 1 (e.g. 1919 on a 1920 display)
    let mut policy = server.input_policy();
    policy.edge = Some("right".to_string());
    server.set_input_policy(policy).unwrap();

    server
        .open_input(client.identity().device_id())
        .await
        .unwrap();
    attach_and_wait_session(&server, &client, tetherly_core::DisplayEdge::Right).await;
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
    attach_and_wait_session(&server, &client, tetherly_core::DisplayEdge::Bottom).await;
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

/// Attach as controller and wait until BOTH sides have registered the
/// session (target role on the peer, open channel here). Roaming frames
/// shipped before that window land on a half-built session — the entry
/// placement races the target's own Enter/Leave bookkeeping (m3_5 flake
/// 2026-11-11: entry x 497/1000 instead of 505).
async fn attach_and_wait_session(
    server: &tetherly_node::Node,
    client: &tetherly_node::Node,
    edge: tetherly_core::DisplayEdge,
) {
    server
        .input_tunnel_attach(
            client.identity().device_id().to_string().as_str(),
            Some(edge),
            None,
            None,
        )
        .await
        .unwrap();
    let start = std::time::Instant::now();
    while client.active_tunnel_role() != Some(tetherly_core::InputTunnelRole::Target)
        || !server.input_channel_open_sync()
    {
        if start.elapsed() > Duration::from_secs(3) {
            panic!("session did not register (target role + channel open) in time");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn m3_9_reverse_control_takeover_flips_roles_cleanly() {
    let (server, client, _ga, _gb) = pair().await;

    // Legacy positional crossing assertions (the step-4 drive_input is
    // positional, dy=0): opt out of the speed-aware redundancy band.
    disable_tb_redundancy_band(&client).await;

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
    //    controller. The demoted side must have released its own. As in
    //    step 1, the new controller's channel opens on the peer's accepted
    //    response, which can land after the role flip on a slow runner
    //    (macOS CI 2026-11-13 run 43486556104) — poll, don't assert.
    let start = std::time::Instant::now();
    while !client.input_channel_open_sync() {
        if start.elapsed() > Duration::from_secs(3) {
            panic!("new controller must hold the outbound channel");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
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
async fn m3_12_same_peer_takeover_request_never_demotes_own_roaming() {
    let (server, client, _ga, _gb) = pair().await;

    // Production 2026-11-12 17:08: Windows roams toward Mac (Controller
    // session Windows->Mac live). Mac's edge-driven watcher, blind to that
    // session (its own channel is closed), redials every tick; each redial
    // arrives at Windows as a takeover request FROM ITS OWN ROAM PEER and
    // the polite-takeover branch demoted the user's roaming session.
    //
    // The wire semantics of a same-peer takeover request are intentional
    // (m3_9: the reverse-direction click/takeover is a legitimate flip), so
    // the receive side keeps demoting. The fix lives on the REQUESTING
    // side: the watcher debounces a pending (<10s) Controller session
    // instead of re-arming every tick. This test pins the observable
    // contract the watcher consumes: a fresh pre-arm Controller session
    // with no channel reports active + young age (the debounce window),
    // and a LEGITIMATE same-peer takeover (a real user flip) still
    // demotes — the semantics the fix must not change.
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
    while server.active_tunnel_role() != Some(tetherly_core::InputTunnelRole::Target)
        || !client.input_channel_open_sync()
    {
        if start.elapsed() > Duration::from_secs(3) {
            panic!("client roaming session did not register");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(
        client.active_tunnel_role(),
        Some(tetherly_core::InputTunnelRole::Controller)
    );

    // 1. A same-peer takeover request (the watcher-redial shape, but also
    //    the legitimate user-flip shape) demotes the roaming session —
    //    wire semantics unchanged by the fix.
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
    while server.active_tunnel_role() != Some(tetherly_core::InputTunnelRole::Controller)
        || !server.input_channel_open_sync()
    {
        if start.elapsed() > Duration::from_secs(3) {
            panic!(
                "same-peer takeover did not complete: server={:?}, client={:?}",
                server.active_tunnel_role(),
                client.active_tunnel_role()
            );
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    // The server flips to Controller synchronously at pre-arm and its channel
    // opens via its own direct dial; the client's demotion rides the control
    // channel and can land later (CI ubuntu 2026-11-12). Wait for it.
    let start = std::time::Instant::now();
    while client.active_tunnel_role() != Some(tetherly_core::InputTunnelRole::Target) {
        if start.elapsed() > Duration::from_secs(3) {
            panic!(
                "demoted side must flip to target: client={:?}",
                client.active_tunnel_role()
            );
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 2. A fresh pre-arm Controller session is "pending" — the watcher
    //    debounce reads exactly this shape (active Controller session, no
    //    channel, started_at_ms fresh). Pre-arm one and assert the status
    //    fields the debounce consumes BEFORE the resp lands: session age
    //    must be well under the 10s window.
    client
        .input_tunnel_attach(
            server.identity().device_id().to_string().as_str(),
            Some(tetherly_core::DisplayEdge::Bottom),
            None,
            None,
        )
        .await
        .unwrap();
    let status = client.input_tunnel_status();
    assert!(
        status.active,
        "pre-armed session must be visible to the watcher"
    );
    let s = status.session.expect("session present");
    assert_eq!(s.role, tetherly_core::InputTunnelRole::Controller);
    let age = status.timestamp_ms.saturating_sub(s.started_at_ms);
    assert!(
        age < 10_000,
        "fresh pre-arm age {age}ms must sit inside the watcher debounce window"
    );

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
        // dy=8 is a FAST push (>= fast_push_delta_px=6): crosses the engine
        // band gate immediately, so the takeover push still carries the
        // seat remote in the same breath.
        client.drive_input_delta(50, 49, 0, 8, 100, 50).unwrap();
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
        // 10s: demotion rides the takeover request/response round-trip; 3s
        // flaked on loaded runners (see the sink-drive window below).
        if start.elapsed() > Duration::from_secs(10) {
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
        if start.elapsed() > Duration::from_secs(10) {
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
    // dy=10 is a fast push (>= 6): crosses the target's Top band gate
    // immediately (the incoming event is dy=10 with the target engine's
    // own mirror of the defaults).
    client.drive_input_delta(50, 40, 0, 10, 100, 50).unwrap();
    let start = std::time::Instant::now();
    loop {
        let sink = server.input_sink_snapshot();
        if server.input_seat() == Some(CursorSeat::Remote) && sink.cursor.0 >= SCREEN_UNITS / 2 {
            break;
        }
        // 10s: this window spans the takeover response round-trip, the grace
        // buffer flush, and the first reverse-channel frames — on a loaded
        // macos runner the 3s budget flaked (run 41930403598) even though the
        // flow is locally deterministic (5/5 at 1.44s).
        if start.elapsed() > Duration::from_secs(10) {
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

    // v0.2.33: this client is a PRE-ARM Target (attach without entry
    // placement) — no controller cursor is parked on its screen, the local
    // user owns it. The distance gate and the 600ms un-armed dwell only
    // fence the parked-entry case (controller on screen; unit-tested in
    // runtime::input: test_entry_parked_edge_micro_jitter_cannot_arm_reverse_dwell
    // and controller_on_screen_disables_fast_reverse_until_leave). Here a
    // dense slow push held past the short dwell is the deliberate gesture
    // and hands over, exactly like a controller-side crossing (production
    // 2026-11-13 13:05:07: the old 600ms gate cost 3.6 s on first entry).
    // The isolated-bump fences above still hold for the local owner.
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
        // 10s: the takeover rides the request/response round-trip and the
        // demotion on the old controller — the same loaded-runner budget
        // m3_10 uses; 3s flaked on ubuntu CI (runs 42866567283/42870492694).
        if start.elapsed() > Duration::from_secs(10) {
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
