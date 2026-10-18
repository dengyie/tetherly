// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Phase 2 EasyTier sidecar: unicast overlay, LAN preference, absent sidecar.
//! Physical cellular p95 remains Manual-required. Never links easytier*.

use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::time::Duration;
use tetherly_core::{NotifyPush, PathKind};
use tetherly_node::{
    loopback_events_connect, loopback_get, loopback_post, NetworkRefreshReport, NetworkReport,
    Node, NodeConfig, OverlayConfig, OverlaySource, SseStream, UiEvent,
};

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
        screen_port: 0,
        advertise: false,
        memory_clip: true,
        loopback_only: true,
        reconnect: false,
        ui_port: 0,
        overlay,
        ancs: None,
        ancs_peripheral: "test-iphone".into(),
        screen_source: None,
        screen_codec: None,
        screen_sink: None,
        input_sink: None,
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
async fn m2_6_notify_reply_reaches_the_phone_and_is_consumed_once() {
    let (da, ga) = temp_dir("a6");
    let (db, gb) = temp_dir("b6");
    let a = Node::start(cfg(da, "desk", overlay_off())).unwrap();
    let b = Node::start(cfg(db, "phone", overlay_off())).unwrap();
    pair(&a, &b).await;
    // Phone → desktop notification, so the desktop has a candidate to answer.
    let dest = a.identity().device_id().clone();
    let start = std::time::Instant::now();
    loop {
        if b.send_notify(&dest, otp_push("u6")).await.is_ok() {
            break;
        }
        if start.elapsed() > Duration::from_secs(3) {
            panic!("session not ready");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    wait_otp(&a, Duration::from_secs(3)).await;
    let cand = a
        .candidates()
        .into_iter()
        .find(|c| c.uid == "u6")
        .expect("candidate");
    // Desktop answers in place; the text travels back to the phone's inbox.
    a.reply_to_candidate(&cand.id, "收到,马上回电")
        .await
        .unwrap();
    let start = std::time::Instant::now();
    let view = loop {
        let v = b.reply_views();
        if !v.is_empty() {
            break v.into_iter().next().unwrap();
        }
        if start.elapsed() > Duration::from_secs(3) {
            panic!("reply did not reach the phone");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert_eq!(view.uid, "u6");
    assert_eq!(view.app_id, "com.example.sms");
    let (view, text) = b.take_reply(&view.id).expect("queued reply");
    assert_eq!(view.uid, "u6");
    assert_eq!(text, "收到,马上回电");
    // Drained exactly once.
    assert!(b.take_reply(&view.id).is_none());
    assert!(b.reply_views().is_empty());
    // An empty reply is refused locally, not sent.
    assert!(a.reply_to_candidate(&cand.id, "   ").await.is_err());
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

#[tokio::test(flavor = "multi_thread")]
async fn network_interface_classification_and_sidecar_inspection_via_loopback_api() {
    // 1. Inspect default node with overlay disabled
    let (d1, g1) = temp_dir("net1");
    let n1 = Node::start(cfg(d1, "desk-net", overlay_off())).expect("start node 1");
    n1.spawn_listeners().await.expect("spawn listeners 1");
    let ui1 = n1.spawn_ui().await.expect("spawn ui 1");

    let (code1, resp1) = tetherly_node::loopback_get(ui1, "/api/net")
        .await
        .expect("get /api/net on node 1");
    assert_eq!(code1, 200);
    let v1: serde_json::Value = serde_json::from_slice(&resp1).expect("valid json 1");
    assert_eq!(v1["ok"], true);

    let ifaces = v1["interfaces"].as_array().expect("interfaces array");
    assert!(!ifaces.is_empty());
    let lo = ifaces
        .iter()
        .find(|i| i["ip"] == "127.0.0.1")
        .expect("127.0.0.1 interface present");
    assert_eq!(lo["listen"], true);
    assert!(lo["reason"].is_null());

    for iface in ifaces {
        assert!(iface["iface"].is_string());
        assert!(iface["ip"].is_string());
        let listen = iface["listen"].as_bool().expect("listen bool");
        if !listen {
            assert!(
                iface["reason"].is_string(),
                "skipped interface must provide classification reason"
            );
        }
    }

    assert_eq!(v1["overlay"]["enabled"], false);
    assert_eq!(v1["overlay"]["present"], false);
    assert_eq!(v1["overlay"]["source"], "Absent");
    assert_eq!(v1["overlay"]["peer_count"], 0);
    assert!(v1["overlay"]["local_ips"].as_array().unwrap().is_empty());
    assert!(v1["overlay"]["peers"].as_array().unwrap().is_empty());

    n1.shutdown();
    drop(g1);

    // 2. Inspect node with enabled overlay and manual extra peers
    let (d2, g2) = temp_dir("net2");
    let mut ov_cfg = overlay_absent();
    ov_cfg.extra_peers = vec![
        "10.144.144.88:45717".parse().unwrap(),
        "10.144.144.99:45717".parse().unwrap(),
    ];
    let n2 = Node::start(cfg(d2, "desk-ov", ov_cfg)).expect("start node 2");
    n2.spawn_listeners().await.expect("spawn listeners 2");
    tokio::time::sleep(Duration::from_millis(50)).await;
    let ui2 = n2.spawn_ui().await.expect("spawn ui 2");

    let (code2, resp2) = tetherly_node::loopback_get(ui2, "/api/net")
        .await
        .expect("get /api/net on node 2");
    assert_eq!(code2, 200);
    let v2: serde_json::Value = serde_json::from_slice(&resp2).expect("valid json 2");
    assert_eq!(v2["ok"], true);
    assert_eq!(v2["overlay"]["enabled"], true);
    assert_eq!(v2["overlay"]["present"], true);
    assert_eq!(v2["overlay"]["source"], "Manual");
    assert_eq!(v2["overlay"]["peer_count"], 2);

    let peers = v2["overlay"]["peers"].as_array().expect("peers array");
    assert_eq!(peers.len(), 2);
    assert!(peers.iter().any(|p| p == "10.144.144.88"));
    assert!(peers.iter().any(|p| p == "10.144.144.99"));

    n2.shutdown();
    drop(g2);
}

// --------------------------------------------------------------------------
// M2.4 — Dynamic network rescan, typed reports, and SSE event synchronization.
// --------------------------------------------------------------------------
#[tokio::test(flavor = "multi_thread")]
async fn dynamic_network_rescan_and_overlay_events_via_loopback_api() {
    let (d, _guard) = temp_dir("rescan-e2e");
    let mut ov_cfg = overlay_absent();
    ov_cfg.extra_peers = vec!["10.144.144.55:45717".parse().unwrap()];

    let node = Node::start(cfg(d, "desk-rescan", ov_cfg)).expect("start node");
    node.spawn_listeners().await.expect("spawn listeners");
    let ui_port = node.spawn_ui().await.expect("spawn ui");
    tokio::time::sleep(Duration::from_millis(50)).await;

    // 1. Establish SSE event stream
    let mut sse = loopback_events_connect(ui_port).await.expect("connect sse");

    // 2. Query initial GET /api/net and deserialize strongly-typed NetworkReport
    let (code, resp) = loopback_get(ui_port, "/api/net")
        .await
        .expect("get /api/net");
    assert_eq!(code, 200);
    let net_rep: NetworkReport = serde_json::from_slice(&resp).expect("valid NetworkReport DTO");
    assert!(net_rep.ok);
    assert!(!net_rep.interfaces.is_empty());
    let lo = net_rep
        .interfaces
        .iter()
        .find(|i| i.ip == "127.0.0.1")
        .expect("loopback present");
    assert!(lo.listen);
    assert!(net_rep.overlay.enabled);
    assert!(net_rep.overlay.present);
    assert_eq!(net_rep.overlay.source, "Manual");
    assert_eq!(net_rep.overlay.peer_count, 1);
    assert_eq!(net_rep.overlay.peers, vec!["10.144.144.55"]);

    // 3. Verify security invariant: reject external Host header with HTTP 403 Forbidden
    {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut raw_stream =
            tokio::net::TcpStream::connect(SocketAddr::from(([127, 0, 0, 1], ui_port)))
                .await
                .expect("raw connect");
        let raw_req =
            "POST /api/net/refresh HTTP/1.1\r\nHost: evil.attacker.com\r\nContent-Length: 2\r\n\r\n{}";
        raw_stream
            .write_all(raw_req.as_bytes())
            .await
            .expect("write raw");
        let mut buf = vec![0u8; 1024];
        let n = raw_stream.read(&mut buf).await.expect("read raw");
        let resp_str = String::from_utf8_lossy(&buf[..n]);
        assert!(
            resp_str.contains("403 Forbidden") && resp_str.contains("loopback only"),
            "non-loopback host must be rejected with 403: {resp_str}"
        );
    }

    // 4. Trigger on-demand dynamic rescan via POST /api/net/refresh
    let (code_ref, resp_ref) = loopback_post(ui_port, "/api/net/refresh", b"{}")
        .await
        .expect("post /api/net/refresh");
    assert_eq!(code_ref, 200);
    let refresh_rep: NetworkRefreshReport =
        serde_json::from_slice(&resp_ref).expect("valid NetworkRefreshReport DTO");
    assert!(refresh_rep.ok);
    assert_eq!(refresh_rep.action, "refreshed");
    assert!(refresh_rep.ifaces_scanned >= 1);
    assert!(refresh_rep.ifaces_listening >= 1);
    assert!(refresh_rep.overlay_present);
    assert_eq!(refresh_rep.overlay_source, "Manual");
    assert_eq!(refresh_rep.overlay_peers_count, 1);
    assert_eq!(refresh_rep.overlay_local_ips, Vec::<String>::new());
    assert!(refresh_rep.timestamp_ms > 0);

    async fn wait_for_network_refreshed(
        sse: &mut SseStream,
        timeout: Duration,
    ) -> (usize, usize, bool, Option<String>) {
        let deadline = tokio::time::Instant::now() + timeout;
        while tokio::time::Instant::now() < deadline {
            let remain = deadline.saturating_duration_since(tokio::time::Instant::now());
            match tokio::time::timeout(remain.min(Duration::from_millis(500)), sse.next_ui_event())
                .await
            {
                Ok(Ok(Some(UiEvent::NetworkRefreshed {
                    iface_count,
                    listen_count,
                    overlay_present,
                    overlay_source,
                }))) => {
                    return (iface_count, listen_count, overlay_present, overlay_source);
                }
                Ok(Ok(Some(_other))) => {}
                Ok(Ok(None)) | Ok(Err(_)) | Err(_) => {}
            }
        }
        panic!("timed out waiting for NetworkRefreshed event on SSE");
    }

    // 5. Verify SSE stream receives UiEvent::NetworkRefreshed
    let (iface_count, listen_count, overlay_present, overlay_source) =
        wait_for_network_refreshed(&mut sse, Duration::from_secs(10)).await;
    assert!(iface_count >= 1);
    assert!(listen_count >= 1);
    assert!(overlay_present);
    assert_eq!(overlay_source.as_deref(), Some("Manual"));

    // 6. Test alias POST /api/net/probe returns same typed report and dispatches event
    let (code_probe, resp_probe) = loopback_post(ui_port, "/api/net/probe", b"{}")
        .await
        .expect("post /api/net/probe");
    assert_eq!(code_probe, 200);
    let probe_rep: NetworkRefreshReport =
        serde_json::from_slice(&resp_probe).expect("valid probe NetworkRefreshReport DTO");
    assert!(probe_rep.ok);
    assert_eq!(probe_rep.action, "refreshed");

    let (p_iface, p_listen, p_present, p_src) =
        wait_for_network_refreshed(&mut sse, Duration::from_secs(10)).await;
    assert!(p_iface >= 1);
    assert!(p_listen >= 1);
    assert!(p_present);
    assert_eq!(p_src.as_deref(), Some("Manual"));

    // 7. Verify direct Rust Node API invocation: node.refresh_network()
    let direct_rep = node.refresh_network().await;
    assert!(direct_rep.ok);
    assert_eq!(direct_rep.action, "refreshed");
    assert!(direct_rep.ifaces_scanned >= 1);
    assert!(direct_rep.ifaces_listening >= 1);

    let (d_iface, d_listen, _d_present, _d_src) =
        wait_for_network_refreshed(&mut sse, Duration::from_secs(10)).await;
    assert!(d_iface >= 1);
    assert!(d_listen >= 1);

    node.shutdown();
}
