// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Phase 3 native input port 45719. Resume-only Noise, binary TIN1, MemorySink.
//! Real OS cursor injection and dual-desktop soak stay Manual-required.
//! DeskFlow / Lan Mouse / KDE source is not linked.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;
use tetherly_core::{CursorSeat, InputKind};
use tetherly_net::session::{dial_session, SessionConfig, HANDSHAKE_TIMEOUT, INPUT_PORT};
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
    let dir = std::env::temp_dir().join(format!(
        "tetherly-p3-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
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
        advertise: false,
        memory_clip: true,
        loopback_only: true,
        reconnect: false,
        ui_port: 0,
        overlay: OverlayConfig {
            enabled: false,
            ..OverlayConfig::default()
        },
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
    wait_live(&a, Duration::from_secs(8)).await;
    wait_live(&b, Duration::from_secs(8)).await;
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
    };
    let err = match dial_session(stream, addr, &cfg).await {
        Err(e) => e,
        Ok(_) => panic!("unpaired input dial must fail"),
    };
    assert!(
        matches!(err, NetError::InputRequiresTrust),
        "unpaired 45719 must refuse pairing, got {err}"
    );
    assert_ne!(host.input_port(), INPUT_PORT);
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

    let edge = server.drive_input(100, 10, 100, 50).await.unwrap();
    assert_eq!(edge, Some(tetherly_core::ScreenEdge::Right));
    server.drive_input_key(0x1E, true).await.unwrap();
    server.drive_input_key(0x1E, false).await.unwrap();

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
    server.drive_input(80, 5, 80, 40).await.unwrap();
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
    server.drive_input(120, 20, 120, 60).await.unwrap();
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
