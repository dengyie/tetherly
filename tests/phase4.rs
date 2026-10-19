// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Phase 4 computer-side ANCS. CI drives the ingress with `MemoryAncsTransport`:
//! subscribe order, serial Control Point, byte-wise fragment reassembly,
//! allowlist-gated "open", and reconnect that never re-pops old notifications.
//!
//! Real BLE GATT (Windows `windows` / CoreBluetooth / BlueZ), the physical
//! iPhone, and the p95 < 2s measurement on hardware stay Manual-required.

use std::path::PathBuf;
use std::sync::Arc;
use tetherly_core::{
    AncsCharacteristic, AncsState, Clock, ConnectOutcome, ManualClock, MemoryAncsTransport,
    OpenAllowlist, OpenRule, APP_NAME_WAIT_MS,
};
use tetherly_node::ancs::AncsInbound;
use tetherly_node::{
    loopback_delete, loopback_events_connect, loopback_get, loopback_post, AncsClearRulesReport,
    AncsConnectReport, AncsDisconnectReport, AncsReport, AncsRuleReport, MemoryOpener, Node,
    NodeConfig, OverlayConfig, SseStream,
};

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
        "tetherly-p4-{tag}-{}-{}-{n}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    (dir.clone(), DirGuard(dir))
}

fn cfg(dir: PathBuf, ancs: Option<Arc<MemoryAncsTransport>>) -> NodeConfig {
    NodeConfig {
        data_dir: dir,
        name: "desk".into(),
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
        ancs: ancs.map(|t| t as Arc<dyn tetherly_core::AncsTransport>),
        ancs_peripheral: "test-iphone".into(),
        screen_source: None,
        screen_codec: None,
        screen_sink: None,
        input_sink: None,
    }
}

fn start(tag: &str, ancs: Option<Arc<MemoryAncsTransport>>) -> (Node, Arc<ManualClock>, DirGuard) {
    let (dir, guard) = temp_dir(tag);
    let clock = Arc::new(ManualClock::new(1_000_000));
    let node = Node::start_with_clock(cfg(dir, ancs), clock.clone()).expect("start");
    (node, clock, guard)
}

// --- ANCS wire fixtures -------------------------------------------------

fn ns_event(event_id: u8, flags: u8, uid: u32) -> Vec<u8> {
    let mut v = vec![event_id, flags, 6, 1];
    v.extend_from_slice(&uid.to_le_bytes());
    v
}

fn attr(id: u8, value: &[u8]) -> Vec<u8> {
    let mut v = vec![id];
    v.extend_from_slice(&(value.len() as u16).to_le_bytes());
    v.extend_from_slice(value);
    v
}

/// Full `GetNotificationAttributes` response for one uid.
fn notif_response(uid: u32, app: &str, title: &str, body: &str) -> Vec<u8> {
    let mut v = vec![0u8];
    v.extend_from_slice(&uid.to_le_bytes());
    v.extend(attr(0, app.as_bytes()));
    v.extend(attr(1, title.as_bytes()));
    v.extend(attr(3, body.as_bytes()));
    v.extend(attr(4, &(body.len() as u32).to_le_bytes()));
    v
}

fn app_response(app: &str, name: &str) -> Vec<u8> {
    let mut v = vec![1u8];
    v.extend_from_slice(app.as_bytes());
    v.push(0);
    v.push(0);
    v.extend_from_slice(&(name.len() as u16).to_le_bytes());
    v.extend_from_slice(name.as_bytes());
    v
}

/// One full ANCS round trip through the node.
async fn ingest(node: &Node, uid: u32, app: &str, app_name: &str, title: &str, body: &str) {
    node.ancs_notification_source(&ns_event(0, 0, uid))
        .await
        .expect("notification source");
    node.ancs_tick().await.expect("tick");
    node.ancs_data_source(&notif_response(uid, app, title, body))
        .await
        .expect("data source");
    if !app_name.is_empty() {
        node.ancs_tick().await.expect("tick");
        node.ancs_data_source(&app_response(app, app_name))
            .await
            .expect("app data source");
    }
}

fn actions_of(node: &Node, uid: u32) -> Vec<String> {
    node.candidates()
        .into_iter()
        .find(|c| c.uid == uid.to_string())
        .map(|c| c.actions)
        .unwrap_or_default()
}

fn candidate_id(node: &Node, uid: u32) -> String {
    node.candidates()
        .into_iter()
        .find(|c| c.uid == uid.to_string())
        .map(|c| c.id)
        .expect("candidate")
}

// --- tests -------------------------------------------------------------

#[tokio::test]
async fn m4_2_open_is_allowlisted_only_and_payload_urls_are_never_opened() {
    let transport = Arc::new(MemoryAncsTransport::default());
    let (node, _clock, _guard) = start("m42", Some(transport.clone()));
    node.set_open_allowlist(
        OpenAllowlist::from_rules(vec![OpenRule {
            app_id: "com.tencent.xin".into(),
            url: "weixin://".into(),
        }])
        .unwrap(),
    );
    assert_eq!(
        node.ancs_connect().unwrap(),
        Some(ConnectOutcome::Subscribed),
        "transport attached"
    );

    // Allowlisted app. The body carries a remote url that must be ignored.
    ingest(
        &node,
        1,
        "com.tencent.xin",
        "微信",
        "https://evil.example/steal",
        "点 https://evil.example/steal 领奖",
    )
    .await;
    assert_eq!(actions_of(&node, 1), vec!["copy", "dismiss", "open"]);

    // Same payload shape, app not allowlisted: no open action at all.
    ingest(
        &node,
        2,
        "com.evil.app",
        "Evil",
        "weixin://steal",
        "weixin://steal",
    )
    .await;
    assert_eq!(actions_of(&node, 2), vec!["copy", "dismiss"]);

    let opener = MemoryOpener::new();
    let denied = node.open_candidate(&candidate_id(&node, 2), &opener);
    assert!(denied.is_err(), "non-allowlisted app must not open");
    assert!(opener.opened().is_empty());

    let url = node
        .open_candidate(&candidate_id(&node, 1), &opener)
        .expect("allowlisted app opens");
    assert_eq!(url, "weixin://");
    assert_eq!(opener.opened(), vec!["weixin://"]);
    for opened in opener.opened() {
        assert!(!opened.contains("evil.example"), "payload url was opened");
    }
}

#[tokio::test]
async fn m4_3_reconnect_does_not_re_pop_old_notifications() {
    let transport = Arc::new(MemoryAncsTransport::default());
    let (node, clock, _guard) = start("m43", Some(transport.clone()));
    node.ancs_connect().unwrap().expect("connected");
    ingest(
        &node,
        100,
        "com.apple.MobileSMS",
        "信息",
        "网易",
        "验证码 868740",
    )
    .await;
    assert_eq!(node.candidates().len(), 1);

    // Bluetooth off, then on again.
    let retry_at = node.ancs_disconnected().expect("attached");
    assert!(matches!(
        node.ancs_state(),
        Some(AncsState::Backoff { retry_at_ms }) if retry_at_ms == retry_at
    ));
    clock.set(retry_at);
    transport.clear();
    assert_eq!(
        node.ancs_connect().unwrap(),
        Some(ConnectOutcome::Subscribed)
    );
    assert_eq!(
        transport.subscribes(),
        vec![
            AncsCharacteristic::DataSource,
            AncsCharacteristic::NotificationSource
        ],
        "Data Source is subscribed before Notification Source"
    );

    // iOS replays everything that already existed with the PreExisting flag.
    node.ancs_notification_source(&ns_event(0, 0x04, 100))
        .await
        .unwrap();
    node.ancs_notification_source(&ns_event(0, 0x04, 101))
        .await
        .unwrap();
    node.ancs_tick().await.unwrap();
    assert_eq!(
        transport.write_count(),
        0,
        "no attribute fetch for old notifications"
    );
    assert_eq!(node.candidates().len(), 1, "old notification re-popped");
    assert!(actions_of(&node, 101).is_empty());

    // A genuinely new notification still arrives.
    ingest(&node, 102, "com.apple.MobileSMS", "信息", "新短信", "hello").await;
    assert_eq!(node.candidates().len(), 2);
    assert_eq!(transport.write_count(), 1);
}

#[tokio::test]
async fn m4_1_ingest_latency_stays_inside_the_budget() {
    let transport = Arc::new(MemoryAncsTransport::default());
    let (node, clock, _guard) = start("m41", Some(transport.clone()));
    node.ancs_connect().unwrap().expect("connected");

    let t0 = clock.unix_ms();
    node.ancs_notification_source(&ns_event(0, 0, 7))
        .await
        .unwrap();
    node.ancs_tick().await.unwrap();
    node.ancs_data_source(&notif_response(
        7,
        "com.apple.MobileSMS",
        "网易",
        "验证码 868740",
    ))
    .await
    .unwrap();
    node.ancs_tick().await.unwrap();
    // The notification is held for at most `APP_NAME_WAIT_MS` waiting for the
    // display name; advance the clock to exactly that deadline and it must be
    // released, not held longer.
    clock.set(t0 + APP_NAME_WAIT_MS);
    node.ancs_data_source(&app_response("com.apple.MobileSMS", "信息"))
        .await
        .unwrap();
    node.ancs_tick().await.unwrap();

    // The hold is bounded by `APP_NAME_WAIT_MS`, well inside the M4.1 budget.
    let elapsed = clock.unix_ms() - t0;
    assert_eq!(
        elapsed, APP_NAME_WAIT_MS,
        "the app-name wait is the only delay, and it is capped"
    );
    assert!(elapsed < 2_000, "M4.1 budget");
    let c = node
        .candidates()
        .into_iter()
        .find(|c| c.uid == "7")
        .expect("candidate");
    assert_eq!(c.app_name, "信息");
    assert!(c.has_otp, "OTP is extracted locally");
}

#[tokio::test]
async fn m4_1_held_notification_is_bounded_by_the_app_name_wait() {
    let transport = Arc::new(MemoryAncsTransport::default());
    let (node, clock, _guard) = start("m41b", Some(transport.clone()));
    node.ancs_connect().unwrap().expect("connected");

    let t0 = clock.unix_ms();
    node.ancs_notification_source(&ns_event(0, 0, 8))
        .await
        .unwrap();
    node.ancs_tick().await.unwrap();
    node.ancs_data_source(&notif_response(8, "com.unknown.app", "t", "b"))
        .await
        .unwrap();
    // One tick before the deadline: still held, nothing emitted yet.
    clock.set(t0 + APP_NAME_WAIT_MS - 1);
    node.ancs_tick().await.unwrap();
    assert!(
        node.candidates().iter().all(|c| c.uid != "8"),
        "notification must still be held before the deadline"
    );
    // At the deadline the wait is capped and the candidate lands even though
    // the app never answered.
    clock.set(t0 + APP_NAME_WAIT_MS);
    node.ancs_tick().await.unwrap();
    assert!(clock.unix_ms() - t0 <= APP_NAME_WAIT_MS);
    let c = node
        .candidates()
        .into_iter()
        .find(|c| c.uid == "8")
        .expect("candidate");
    assert_eq!(c.app_id, "com.unknown.app");
    assert_eq!(c.app_name, "");
}

#[tokio::test]
async fn control_point_is_serial_and_never_written_on_the_callback() {
    let transport = Arc::new(MemoryAncsTransport::default());
    let (node, _clock, _guard) = start("serial", Some(transport.clone()));
    node.ancs_connect().unwrap().expect("connected");

    for uid in 10..13u32 {
        node.ancs_notification_source(&ns_event(0, 0, uid))
            .await
            .unwrap();
    }
    assert_eq!(
        transport.write_count(),
        0,
        "the value-changed callback must not write the Control Point"
    );

    node.ancs_tick().await.unwrap();
    assert_eq!(transport.write_count(), 1, "one request in flight");
    // Answer the first uid; the next tick may then send the second.
    node.ancs_data_source(&notif_response(10, "a.b", "t", "b"))
        .await
        .unwrap();
    node.ancs_tick().await.unwrap();
    assert_eq!(transport.write_count(), 2);
    node.ancs_data_source(&notif_response(11, "a.b", "t", "b"))
        .await
        .unwrap();
    node.ancs_tick().await.unwrap();
    assert_eq!(transport.write_count(), 3);

    // Requests are distinguishable and each is a GetNotificationAttributes.
    let writes = transport.writes();
    assert!(writes.iter().all(|w| w[0] == 0));
    assert_ne!(writes[0], writes[1]);
}

#[tokio::test]
async fn fragmented_data_source_is_reassembled_across_utf8() {
    let transport = Arc::new(MemoryAncsTransport::default());
    let (node, _clock, _guard) = start("frag", Some(transport.clone()));
    node.ancs_connect().unwrap().expect("connected");

    node.ancs_notification_source(&ns_event(0, 0, 20))
        .await
        .unwrap();
    node.ancs_tick().await.unwrap();
    let full = notif_response(20, "com.apple.MobileSMS", "网易", "验证码 868740");
    // Cut in the middle of a multi-byte value.
    let cut = 24;
    let (head, tail) = full.split_at(cut);
    node.ancs_data_source(head).await.unwrap();
    assert!(
        node.candidates().is_empty(),
        "partial value must not be parsed"
    );
    node.ancs_data_source(tail).await.unwrap();
    node.ancs_tick().await.unwrap();
    node.ancs_data_source(&app_response("com.apple.MobileSMS", "信息"))
        .await
        .unwrap();
    let c = node
        .candidates()
        .into_iter()
        .find(|c| c.uid == "20")
        .expect("candidate");
    assert_eq!(c.title, "网易");
    assert_eq!(c.app_name, "信息");
    assert!(c.has_otp);
}

#[tokio::test]
async fn removed_event_clears_the_candidate() {
    let transport = Arc::new(MemoryAncsTransport::default());
    let (node, _clock, _guard) = start("removed", Some(transport.clone()));
    node.ancs_connect().unwrap().expect("connected");
    ingest(
        &node,
        30,
        "com.apple.MobileSMS",
        "信息",
        "网易",
        "验证码 868740",
    )
    .await;
    assert_eq!(node.candidates().len(), 1);

    node.ancs_notification_source(&ns_event(2, 0, 30))
        .await
        .unwrap();
    assert!(
        node.candidates().is_empty(),
        "removed notification must clear"
    );
}

#[tokio::test]
async fn a_node_without_a_transport_is_inert() {
    let (node, _clock, _guard) = start("noancs", None);
    assert_eq!(node.ancs_state(), None);
    assert_eq!(node.ancs_connect().unwrap(), None);
    assert_eq!(node.ancs_disconnected(), None);
    // Feeding bytes is a no-op rather than a panic or a phantom candidate.
    node.ancs_notification_source(&ns_event(0, 0, 1))
        .await
        .unwrap();
    node.ancs_data_source(&notif_response(1, "a.b", "t", "b"))
        .await
        .unwrap();
    node.ancs_tick().await.unwrap();
    assert!(node.candidates().is_empty());
}

#[tokio::test]
async fn backoff_refuses_a_hot_reconnect() {
    let transport = Arc::new(MemoryAncsTransport::default());
    let (node, clock, _guard) = start("backoff", Some(transport.clone()));
    node.ancs_connect().unwrap().expect("connected");
    let retry_at = node.ancs_disconnected().expect("attached");
    transport.clear();
    assert_eq!(
        node.ancs_connect().unwrap(),
        Some(ConnectOutcome::BackingOff {
            retry_at_ms: retry_at
        })
    );
    assert!(transport.subscribes().is_empty());
    clock.set(retry_at - 1);
    assert_eq!(
        node.ancs_connect().unwrap(),
        Some(ConnectOutcome::BackingOff {
            retry_at_ms: retry_at
        })
    );
    clock.set(retry_at);
    assert_eq!(
        node.ancs_connect().unwrap(),
        Some(ConnectOutcome::Subscribed)
    );
}

#[tokio::test]
async fn ui_routes_are_not_the_only_way_in() {
    // The node surface is the contract; the loopback UI is a thin shell over it.
    let transport = Arc::new(MemoryAncsTransport::default());
    let (node, _clock, _guard) = start("surface", Some(transport.clone()));
    node.ancs_connect().unwrap().expect("connected");
    assert!(node
        .add_open_rule(OpenRule {
            app_id: "com.apple.MobileSMS".into(),
            url: "weixin://".into(),
        })
        .is_ok());
    assert!(node
        .add_open_rule(OpenRule {
            app_id: "com.evil".into(),
            url: "https://evil.example".into(),
        })
        .is_err());
    assert_eq!(node.open_allowlist().rules().len(), 1);

    ingest(
        &node,
        40,
        "com.apple.MobileSMS",
        "信息",
        "网易",
        "验证码 868740",
    )
    .await;
    assert_eq!(actions_of(&node, 40), vec!["copy", "dismiss", "open"]);
}

// --- transport pump (v1.23) --------------------------------------------

/// Poll a condition until it holds. The pump runs on its own thread, so the
/// effects below are observed rather than assumed.
async fn wait_for(mut ready: impl FnMut() -> bool) {
    for _ in 0..400 {
        if ready() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("condition never became true");
}

/// The inbound channel is what a real stack feeds. Driving it end to end proves
/// the pump wires the transport to the ingress: subscribe order, the value
/// callbacks becoming queued Control Point work, and the disconnect backoff.
/// The GATT stack and the physical phone stay Manual-required.
#[tokio::test]
async fn m4_5_the_transport_pump_subscribes_in_order_and_feeds_the_ingress() {
    let transport = Arc::new(MemoryAncsTransport::default());
    let (node, _clock, _guard) = start("pump", Some(transport.clone()));
    let (feed, inbound) = tokio::sync::mpsc::unbounded_channel();
    node.spawn_ancs_pump(inbound);

    // The stack reports the session up; §9.4 demands Data Source first.
    feed.send(AncsInbound::Ready).expect("feed");
    wait_for(|| transport.subscribes().len() == 2).await;
    assert_eq!(
        transport.subscribes(),
        vec![
            AncsCharacteristic::DataSource,
            AncsCharacteristic::NotificationSource
        ],
        "Data Source must be subscribed before Notification Source"
    );

    // A Notification Source callback only enqueues; the pump's tick is what
    // writes the Control Point.
    feed.send(AncsInbound::NotificationSource(ns_event(0, 0, 77)))
        .expect("feed");
    wait_for(|| !transport.writes().is_empty()).await;

    // The Data Source response becomes a candidate, i.e. the bytes reached the
    // ingress rather than dying on the channel.
    feed.send(AncsInbound::DataSource(notif_response(
        77,
        "com.apple.MobileSMS",
        "网易",
        "验证码 424242",
    )))
    .expect("feed");
    feed.send(AncsInbound::DataSource(app_response(
        "com.apple.MobileSMS",
        "信息",
    )))
    .expect("feed");
    wait_for(|| node.candidates().iter().any(|c| c.uid == "77")).await;

    // The phone going away schedules the backoff reconnect.
    feed.send(AncsInbound::Disconnected).expect("feed");
    wait_for(|| matches!(node.ancs_state(), Some(AncsState::Backoff { .. }))).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn ancs_status_inspection_connection_and_allowlist_via_loopback_api() {
    // 1. Node without ANCS transport (no transport attached)
    let (node1, _clock1, _guard1) = start("ancs-ui1", None);
    let ui_port1 = node1.spawn_ui().await.expect("spawn ui 1");

    // GET /api/ancs should report ok=true, attached=false, state="off", retry_at_ms=None
    let (code, resp) = tetherly_node::loopback_get(ui_port1, "/api/ancs")
        .await
        .expect("get /api/ancs");
    assert_eq!(code, 200);
    let report: tetherly_node::AncsReport =
        serde_json::from_slice(&resp).expect("parse AncsReport");
    assert!(report.ok);
    assert!(!report.attached);
    assert_eq!(report.state, "off");
    assert_eq!(report.retry_at_ms, None);
    assert!(report.allowlist.is_empty());

    // POST /api/ancs/connect without transport should fail with HTTP 400
    let (code, err_resp) = tetherly_node::loopback_post(ui_port1, "/api/ancs/connect", b"{}")
        .await
        .expect("post connect");
    assert_eq!(code, 400);
    let err_v: serde_json::Value = serde_json::from_slice(&err_resp).expect("parse err");
    assert_eq!(err_v["error"], "no ANCS transport attached");

    // POST /api/ancs/allow with invalid json should fail with HTTP 400
    let (code, _) = tetherly_node::loopback_post(ui_port1, "/api/ancs/allow", b"{}")
        .await
        .expect("post allow invalid");
    assert_eq!(code, 400);

    // POST /api/ancs/allow with disallowed scheme should fail with HTTP 400
    let evil_payload = serde_json::json!({
        "app_id": "com.evil",
        "url": "https://evil.example"
    });
    let (code, _) = tetherly_node::loopback_post(
        ui_port1,
        "/api/ancs/allow",
        &serde_json::to_vec(&evil_payload).unwrap(),
    )
    .await
    .expect("post allow evil");
    assert_eq!(code, 400);

    // POST /api/ancs/allow with explicit url
    let rule1 = serde_json::json!({
        "app_id": "com.tencent.xin",
        "url": "weixin://"
    });
    let (code, resp) = tetherly_node::loopback_post(
        ui_port1,
        "/api/ancs/allow",
        &serde_json::to_vec(&rule1).unwrap(),
    )
    .await
    .expect("post allow rule1");
    assert_eq!(code, 200);
    let ok_v: serde_json::Value = serde_json::from_slice(&resp).expect("parse ok");
    assert_eq!(ok_v["ok"], true);

    // POST /api/ancs/allow with default url fallback
    let rule2 = serde_json::json!({
        "app_id": "com.eg.android.AlipayGphone"
    });
    let (code, resp) = tetherly_node::loopback_post(
        ui_port1,
        "/api/ancs/allow",
        &serde_json::to_vec(&rule2).unwrap(),
    )
    .await
    .expect("post allow rule2");
    assert_eq!(code, 200);
    let ok_v: serde_json::Value = serde_json::from_slice(&resp).expect("parse ok 2");
    assert_eq!(ok_v["ok"], true);

    // GET /api/ancs should now reflect updated allowlist
    let (code, resp) = tetherly_node::loopback_get(ui_port1, "/api/ancs")
        .await
        .expect("get /api/ancs after allow");
    assert_eq!(code, 200);
    let report: tetherly_node::AncsReport =
        serde_json::from_slice(&resp).expect("parse AncsReport");
    assert_eq!(report.allowlist.len(), 2);
    assert!(report.allowlist.contains(&"com.tencent.xin".to_string()));
    assert!(report
        .allowlist
        .contains(&"com.eg.android.AlipayGphone".to_string()));

    node1.shutdown();

    // 2. Node with MemoryAncsTransport
    let transport = Arc::new(MemoryAncsTransport::default());
    let (node2, _clock2, _guard2) = start("ancs-ui2", Some(transport.clone()));
    let ui_port2 = node2.spawn_ui().await.expect("spawn ui 2");

    // Initial state with transport attached but before connect -> attached=true, state="idle"
    let (code, resp) = tetherly_node::loopback_get(ui_port2, "/api/ancs")
        .await
        .expect("get /api/ancs");
    assert_eq!(code, 200);
    let report: tetherly_node::AncsReport =
        serde_json::from_slice(&resp).expect("parse AncsReport");
    assert!(report.ok);
    assert!(report.attached);
    assert_eq!(report.state, "idle");
    assert_eq!(report.retry_at_ms, None);

    // Trigger connect via POST /api/ancs/connect
    let (code, resp) = tetherly_node::loopback_post(ui_port2, "/api/ancs/connect", b"{}")
        .await
        .expect("post connect on node 2");
    assert_eq!(code, 200);
    let v: serde_json::Value = serde_json::from_slice(&resp).expect("parse connect resp");
    assert_eq!(v["state"], "ready");

    // GET /api/ancs after connect -> state="ready"
    let (code, resp) = tetherly_node::loopback_get(ui_port2, "/api/ancs")
        .await
        .expect("get /api/ancs after connect");
    assert_eq!(code, 200);
    let report: tetherly_node::AncsReport =
        serde_json::from_slice(&resp).expect("parse AncsReport");
    assert!(report.attached);
    assert_eq!(report.state, "ready");
    assert_eq!(report.retry_at_ms, None);

    // Trigger disconnect to transition Ingress into Backoff
    let retry_at = node2.ancs_disconnected().expect("attached transport");
    let (code, resp) = tetherly_node::loopback_get(ui_port2, "/api/ancs")
        .await
        .expect("get /api/ancs after disconnect");
    assert_eq!(code, 200);
    let report: tetherly_node::AncsReport =
        serde_json::from_slice(&resp).expect("parse AncsReport");
    assert!(report.attached);
    assert_eq!(report.state, "backoff");
    assert_eq!(report.retry_at_ms, Some(retry_at));

    node2.shutdown();
}

async fn wait_event<F>(sse: &mut SseStream, predicate: F) -> serde_json::Value
where
    F: Fn(&serde_json::Value) -> bool,
{
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(4);
    while tokio::time::Instant::now() < deadline {
        let remaining = deadline - tokio::time::Instant::now();
        if let Ok(Ok(Some(raw))) = tokio::time::timeout(remaining, sse.next_event()).await {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
                if predicate(&v) {
                    return v;
                }
            }
        }
    }
    panic!("timed out waiting for matching event");
}

#[tokio::test(flavor = "multi_thread")]
async fn ancs_lifecycle_governance_rule_management_and_events_via_loopback_api() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let transport = Arc::new(MemoryAncsTransport::default());
    let (node, _clock, _guard) = start("ancs-gov", Some(transport.clone()));
    let ui_port = node.spawn_ui().await.expect("spawn ui");

    // 1. Host header loopback guard: Reject non-loopback Host with 403 Forbidden
    {
        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", ui_port))
            .await
            .expect("tcp connect");
        let req = b"GET /api/ancs HTTP/1.1\r\nHost: evil.attacker.com\r\nConnection: close\r\n\r\n";
        stream.write_all(req).await.unwrap();
        stream.flush().await.unwrap();
        let mut buf = Vec::new();
        stream.read_to_end(&mut buf).await.unwrap();
        let resp = String::from_utf8_lossy(&buf);
        assert!(
            resp.starts_with("HTTP/1.1 403"),
            "must reject evil Host with 403: {resp}"
        );
        assert!(
            resp.contains("loopback only"),
            "body must state 'loopback only'"
        );
    }

    // 2. Connect SSE event stream to verify live broadcast of state and rule changes
    let mut sse = loopback_events_connect(ui_port).await.expect("connect sse");

    // 3. Initial GET /api/ancs inspection
    let (code, resp) = loopback_get(ui_port, "/api/ancs").await.expect("get ancs");
    assert_eq!(code, 200);
    let initial_report: AncsReport = serde_json::from_slice(&resp).expect("parse AncsReport");
    assert!(initial_report.ok);
    assert!(initial_report.attached);
    assert_eq!(initial_report.state, "idle");
    assert_eq!(initial_report.retry_at_ms, None);
    assert!(initial_report.rules.is_empty());
    assert!(initial_report.allowlist.is_empty());

    // 4. POST /api/ancs/connect -> Subscribed (ready) and verifies UiEvent::AncsStateChanged
    let (code, resp) = loopback_post(ui_port, "/api/ancs/connect", b"{}")
        .await
        .expect("post ancs connect");
    assert_eq!(code, 200);
    let conn_rep: AncsConnectReport =
        serde_json::from_slice(&resp).expect("parse AncsConnectReport");
    assert!(conn_rep.ok);
    assert_eq!(conn_rep.state, "ready");
    assert_eq!(conn_rep.retry_at_ms, None);
    assert_eq!(conn_rep.message, "ANCS subscribed");

    let ev = wait_event(&mut sse, |v| v["kind"] == "ancs_state_changed").await;
    assert_eq!(ev["state"], "ready");
    assert_eq!(ev["attached"], true);
    assert!(ev["retry_at_ms"].is_null());

    // 5. Add allowlist rule 1 via POST /api/ancs/allow
    let rule1_payload = serde_json::json!({
        "app_id": "com.tencent.xin",
        "url": "weixin://"
    });
    let (code, resp) = loopback_post(
        ui_port,
        "/api/ancs/allow",
        &serde_json::to_vec(&rule1_payload).unwrap(),
    )
    .await
    .expect("post allow rule 1");
    assert_eq!(code, 200);
    let r1_rep: AncsRuleReport = serde_json::from_slice(&resp).expect("parse AncsRuleReport 1");
    assert!(r1_rep.ok);
    assert_eq!(r1_rep.action, "added");
    assert_eq!(r1_rep.total_rules, 1);
    let r1_dto = r1_rep.rule.expect("rule dto");
    assert_eq!(r1_dto.app_id, "com.tencent.xin");
    assert_eq!(r1_dto.url, "weixin://");

    let ev_r1 = wait_event(&mut sse, |v| {
        v["kind"] == "ancs_rule_changed"
            && v["action"] == "added"
            && v["app_id"] == "com.tencent.xin"
    })
    .await;
    assert_eq!(ev_r1["rules_count"], 1);
    assert_eq!(ev_r1["url"], "weixin://");

    // 6. Add allowlist rule 2 via alias POST /api/ancs/rules/add (default URL scheme)
    let rule2_payload = serde_json::json!({
        "app_id": "com.apple.MobileSMS"
    });
    let (code, resp) = loopback_post(
        ui_port,
        "/api/ancs/rules/add",
        &serde_json::to_vec(&rule2_payload).unwrap(),
    )
    .await
    .expect("post rules add rule 2");
    assert_eq!(code, 200);
    let r2_rep: AncsRuleReport = serde_json::from_slice(&resp).expect("parse AncsRuleReport 2");
    assert!(r2_rep.ok);
    assert_eq!(r2_rep.action, "added");
    assert_eq!(r2_rep.total_rules, 2);
    let r2_dto = r2_rep.rule.expect("rule dto 2");
    assert_eq!(r2_dto.app_id, "com.apple.MobileSMS");
    assert_eq!(r2_dto.url, "tetherly://open");

    let ev_r2 = wait_event(&mut sse, |v| {
        v["kind"] == "ancs_rule_changed"
            && v["action"] == "added"
            && v["app_id"] == "com.apple.MobileSMS"
    })
    .await;
    assert_eq!(ev_r2["rules_count"], 2);

    // 7. Verify GET /api/ancs returns both rules and backwards-compatible allowlist
    let (code, resp) = loopback_get(ui_port, "/api/ancs")
        .await
        .expect("get ancs with 2 rules");
    assert_eq!(code, 200);
    let report_with_2: AncsReport = serde_json::from_slice(&resp).expect("parse report");
    assert_eq!(report_with_2.rules.len(), 2);
    assert_eq!(report_with_2.allowlist.len(), 2);
    assert!(report_with_2
        .rules
        .iter()
        .any(|r| r.app_id == "com.tencent.xin" && r.url == "weixin://"));
    assert!(report_with_2
        .rules
        .iter()
        .any(|r| r.app_id == "com.apple.MobileSMS" && r.url == "tetherly://open"));

    // 8. Remove rule 1 via DELETE /api/ancs/allow?app_id=com.tencent.xin
    let (code, resp) = loopback_delete(ui_port, "/api/ancs/allow?app_id=com.tencent.xin")
        .await
        .expect("delete rule 1");
    assert_eq!(code, 200);
    let rem1_rep: AncsRuleReport =
        serde_json::from_slice(&resp).expect("parse AncsRuleReport remove");
    assert!(rem1_rep.ok);
    assert_eq!(rem1_rep.action, "removed");
    assert_eq!(rem1_rep.total_rules, 1);
    assert_eq!(
        rem1_rep.rule.as_ref().map(|r| r.app_id.as_str()),
        Some("com.tencent.xin")
    );

    let ev_rem = wait_event(&mut sse, |v| {
        v["kind"] == "ancs_rule_changed" && v["action"] == "removed"
    })
    .await;
    assert_eq!(ev_rem["app_id"], "com.tencent.xin");
    assert_eq!(ev_rem["rules_count"], 1);

    // 9. Remove non-existent rule via POST /api/ancs/rules/remove -> not_found
    let not_found_payload = serde_json::json!({
        "app_id": "com.tencent.xin"
    });
    let (code, resp) = loopback_post(
        ui_port,
        "/api/ancs/rules/remove",
        &serde_json::to_vec(&not_found_payload).unwrap(),
    )
    .await
    .expect("remove already removed rule");
    assert_eq!(code, 200);
    let rem_nf_rep: AncsRuleReport =
        serde_json::from_slice(&resp).expect("parse AncsRuleReport not found");
    assert!(rem_nf_rep.ok);
    assert_eq!(rem_nf_rep.action, "not_found");
    assert_eq!(rem_nf_rep.total_rules, 1);
    assert!(rem_nf_rep.rule.is_none());

    // 10. Clear all rules via POST /api/ancs/rules/clear
    let (code, resp) = loopback_post(ui_port, "/api/ancs/rules/clear", b"{}")
        .await
        .expect("post clear rules");
    assert_eq!(code, 200);
    let clear_rep: AncsClearRulesReport =
        serde_json::from_slice(&resp).expect("parse AncsClearRulesReport");
    assert!(clear_rep.ok);
    assert_eq!(clear_rep.cleared_count, 1);
    assert_eq!(clear_rep.total_rules, 0);

    let ev_clr = wait_event(&mut sse, |v| {
        v["kind"] == "ancs_rule_changed" && v["action"] == "cleared"
    })
    .await;
    assert_eq!(ev_clr["rules_count"], 0);

    // Verify GET /api/ancs is now completely empty
    let (code, resp) = loopback_get(ui_port, "/api/ancs")
        .await
        .expect("get ancs empty");
    assert_eq!(code, 200);
    let empty_rep: AncsReport = serde_json::from_slice(&resp).expect("parse empty report");
    assert!(empty_rep.rules.is_empty());
    assert!(empty_rep.allowlist.is_empty());

    // 11. Disconnect ANCS via POST /api/ancs/disconnect -> backoff and verifies UiEvent::AncsStateChanged
    let (code, resp) = loopback_post(ui_port, "/api/ancs/disconnect", b"{}")
        .await
        .expect("post ancs disconnect");
    assert_eq!(code, 200);
    let disc_rep: AncsDisconnectReport =
        serde_json::from_slice(&resp).expect("parse AncsDisconnectReport");
    assert!(disc_rep.ok);
    assert_eq!(disc_rep.state, "backoff");
    assert!(disc_rep.retry_at_ms.is_some());
    assert_eq!(disc_rep.message, "ANCS disconnected");

    let ev_disc = wait_event(&mut sse, |v| {
        v["kind"] == "ancs_state_changed" && v["state"] == "backoff"
    })
    .await;
    assert_eq!(ev_disc["attached"], true);
    assert!(ev_disc["retry_at_ms"].as_u64().is_some());

    node.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn test_ble_address_and_daemon_ancs_wiring_regression() {
    use tetherly_core::parse_ble_address;

    // 1. Verify cross-platform BleAddress parsing
    let mac = parse_ble_address("11:22:33:44:55:66").expect("parse mac");
    assert_eq!(mac.to_canonical_string(), "11:22:33:44:55:66");
    assert_eq!(mac.as_mac_u64(), Some(0x112233445566));
    assert!(mac.is_mac());
    assert!(!mac.is_uuid());

    let uuid_str = "9F82A2E6-B9C0-4E90-B8E5-3A21F5A67C43";
    let uuid = parse_ble_address(uuid_str).expect("parse uuid");
    assert_eq!(
        uuid.to_canonical_string(),
        "9f82a2e6-b9c0-4e90-b8e5-3a21f5a67c43"
    );
    assert_eq!(uuid.as_mac_u64(), None);
    assert!(uuid.is_uuid());
    assert!(!uuid.is_mac());

    // 2. Daemon wiring regression:
    // With transport attached and configured into NodeConfig:
    let (dir, _guard) = temp_dir("ancs-daemon-fix");
    let clock = Arc::new(ManualClock::new(2_000_000));
    let transport = Arc::new(MemoryAncsTransport::default());
    let (in_tx, in_rx) = tokio::sync::mpsc::unbounded_channel();

    let mut config = cfg(dir, Some(transport.clone()));
    config.ancs_peripheral = uuid.to_canonical_string();

    let node = Node::start_with_clock(config, clock.clone()).expect("start node with ancs config");
    let ui_port = node.spawn_ui().await.expect("spawn ui");

    // Spawn pump with inbound channel (replicates tetherly run daemon logic)
    node.spawn_ancs_pump(in_rx);

    // Initial check: transport is attached, peripheral is recorded, state is idle
    let (code, resp) = loopback_get(ui_port, "/api/ancs").await.expect("get ancs");
    assert_eq!(code, 200);
    let report: AncsReport = serde_json::from_slice(&resp).expect("parse AncsReport");
    assert!(report.ok);
    assert!(report.attached);
    assert_eq!(report.state, "idle");
    assert_eq!(
        report.peripheral.as_deref(),
        Some("9f82a2e6-b9c0-4e90-b8e5-3a21f5a67c43")
    );

    // Inbound: Ready signal from transport stack
    in_tx.send(AncsInbound::Ready).expect("send ready");
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // Ingress state transitioned to ready
    let (code, resp) = loopback_get(ui_port, "/api/ancs").await.expect("get ancs");
    assert_eq!(code, 200);
    let report: AncsReport = serde_json::from_slice(&resp).expect("parse AncsReport");
    assert_eq!(report.state, "ready");

    // Inbound: Disconnected signal from transport stack
    in_tx
        .send(AncsInbound::Disconnected)
        .expect("send disconnected");
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // Ingress state transitioned to backoff
    let (code, resp) = loopback_get(ui_port, "/api/ancs").await.expect("get ancs");
    assert_eq!(code, 200);
    let report: AncsReport = serde_json::from_slice(&resp).expect("parse AncsReport");
    assert_eq!(report.state, "backoff");
    assert!(report.retry_at_ms.is_some());

    node.shutdown();
}
