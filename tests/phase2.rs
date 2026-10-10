// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Phase 2 EasyTier sidecar: unicast overlay, LAN preference, absent sidecar.
//! Physical cellular p95 remains Manual-required. Never links easytier*.

use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::time::Duration;
use tetherly_core::{NotifyPush, PathKind};
use tetherly_node::{Node, NodeConfig, OverlayConfig, OverlaySource};

const OTP_BODY: &str = "【测试】验证码：524681，您正在登录";

struct DirGuard(PathBuf);

impl Drop for DirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn temp_dir(tag: &str) -> (PathBuf, DirGuard) {
    let dir = std::env::temp_dir().join(format!(
        "tetherly-p2-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    (dir.clone(), DirGuard(dir))
}

fn overlay_off() -> OverlayConfig {
    OverlayConfig {
        enabled: false,
        cli: PathBuf::from("tetherly-no-such-easytier-cli"),
        rpc_addr: SocketAddr::from((Ipv4Addr::LOCALHOST, 1)),
        ..OverlayConfig::default()
    }
}

fn overlay_absent() -> OverlayConfig {
    OverlayConfig {
        enabled: true,
        cidrs: vec![("10.199.199.0".parse().unwrap(), 24)],
        cli: PathBuf::from("tetherly-no-such-easytier-cli"),
        rpc_addr: SocketAddr::from((Ipv4Addr::LOCALHOST, 1)),
        extra_peers: Vec::new(),
    }
}

fn cfg(dir: PathBuf, name: &str, overlay: OverlayConfig) -> NodeConfig {
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
        overlay,
        ancs: None,
        ancs_peripheral: "test-iphone".into(),
    }
}

fn otp_push(uid: &str) -> NotifyPush {
    NotifyPush {
        uid: uid.into(),
        app_id: "com.example.sms".into(),
        app_name: "信息".into(),
        title: "网易".into(),
        body: OTP_BODY.into(),
        ts: 1,
        actions: vec!["copy".into()],
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

async fn wait_otp(node: &Node, timeout: Duration) {
    let start = std::time::Instant::now();
    loop {
        if node.candidates().iter().any(|c| c.has_otp) {
            return;
        }
        if start.elapsed() > timeout {
            panic!("notify did not land");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn pair(a: &Node, b: &Node) {
    let pin = tetherly_core::pin::parse_pin("25170394").unwrap();
    a.set_pin(pin);
    b.set_pin(pin);
    a.spawn_listeners().await.unwrap();
    b.spawn_listeners().await.unwrap();
    tokio::time::sleep(Duration::from_millis(40)).await;
    b.dial(a.control_addr()).await.unwrap();
    wait_live(a, Duration::from_secs(25)).await;
    wait_live(b, Duration::from_secs(25)).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn m2_4_absent_sidecar_phase1_unchanged() {
    let (da, ga) = temp_dir("a4");
    let (db, gb) = temp_dir("b4");
    let a = Node::start(cfg(da, "desk", overlay_absent())).unwrap();
    let b = Node::start(cfg(db, "phone", overlay_absent())).unwrap();
    assert!(a.lan_only());
    assert_eq!(a.overlay_status().source, OverlaySource::Absent);
    pair(&a, &b).await;
    let dest = a.identity().device_id().clone();
    let start = std::time::Instant::now();
    loop {
        if b.send_notify(&dest, otp_push("u4")).await.is_ok() {
            break;
        }
        if start.elapsed() > Duration::from_secs(3) {
            panic!("session not ready");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    wait_otp(&a, Duration::from_secs(3)).await;
    assert!(a.lan_only());
    a.shutdown();
    b.shutdown();
    drop((ga, gb));
}

#[tokio::test(flavor = "multi_thread")]
async fn m2_3_stop_sidecar_lan_only_no_crash() {
    let (d, g) = temp_dir("stop");
    let node = Node::start(cfg(d, "desk", overlay_absent())).unwrap();
    node.spawn_listeners().await.unwrap();
    tokio::time::sleep(Duration::from_millis(80)).await;
    let st = node.overlay_status();
    assert!(!st.present);
    assert!(node.lan_only());
    node.shutdown();
    drop(g);
}

#[tokio::test(flavor = "multi_thread")]
async fn m2_5_unicast_peer_list_without_mdns() {
    let (da, ga) = temp_dir("a5");
    let (db, gb) = temp_dir("b5");
    let pin = tetherly_core::pin::parse_pin("25170394").unwrap();
    let a = Node::start(cfg(da, "desk", overlay_off())).unwrap();
    a.set_pin(pin);
    a.spawn_listeners().await.unwrap();
    tokio::time::sleep(Duration::from_millis(40)).await;
    let mut ob = overlay_absent();
    ob.extra_peers = vec![a.control_addr()];
    let b = Node::start(cfg(db, "phone", ob)).unwrap();
    b.set_pin(pin);
    b.spawn_listeners().await.unwrap();
    wait_live(&a, Duration::from_secs(25)).await;
    wait_live(&b, Duration::from_secs(25)).await;
    let dest = a.identity().device_id().clone();
    let start = std::time::Instant::now();
    loop {
        if b.send_notify(&dest, otp_push("u5")).await.is_ok() {
            break;
        }
        if start.elapsed() > Duration::from_secs(3) {
            panic!("unicast session not ready");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    wait_otp(&a, Duration::from_secs(3)).await;
    a.shutdown();
    b.shutdown();
    drop((ga, gb));
}

#[tokio::test(flavor = "multi_thread")]
async fn m2_2_keep_lan_when_overlay_scan_repeats() {
    let (da, ga) = temp_dir("a2");
    let (db, gb) = temp_dir("b2");
    let a = Node::start(cfg(da, "desk", overlay_off())).unwrap();
    let b = Node::start(cfg(db, "phone", overlay_off())).unwrap();
    pair(&a, &b).await;
    assert_eq!(a.live_peers()[0].path, PathKind::Lan);
    let id = a.live_peers()[0].device_id.clone();
    a.dial(b.control_addr()).await.unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    let live = a.live_peers();
    assert_eq!(live.len(), 1);
    assert_eq!(live[0].device_id, id);
    assert_eq!(live[0].path, PathKind::Lan);
    a.shutdown();
    b.shutdown();
    drop((ga, gb));
}
