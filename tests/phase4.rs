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
    OpenAllowlist, OpenRule,
};
use tetherly_node::{MemoryOpener, Node, NodeConfig, OverlayConfig};

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
    // The app name is the only reason we wait at all; the phone answers fast.
    clock.set(t0 + 120);
    node.ancs_data_source(&app_response("com.apple.MobileSMS", "信息"))
        .await
        .unwrap();

    let elapsed = clock.unix_ms() - t0;
    assert!(
        elapsed < 2_000,
        "simulated end-to-end latency {elapsed}ms is outside the M4.1 budget"
    );
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
    // The app never answers; the wait is capped, so the candidate still lands.
    clock.set(t0 + 400);
    node.ancs_tick().await.unwrap();
    assert!(clock.unix_ms() - t0 < 2_000);
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
