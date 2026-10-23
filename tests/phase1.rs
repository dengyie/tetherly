// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Phase 1 LAN loopback: notify/OTP, clip, persist reconnect, file confirm.
//! Physical Android p95 / 8h soak remain Manual-required.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tetherly_core::{
    hex_sha256, FileMeta, FileOffer, ManualClock, NotifyPush, TrustStore, CANDIDATE_TTL_MS,
};
use tetherly_node::{
    ActiveTransfer, MemoryInsertor, Node, NodeConfig, OverlayConfig, TransferDirection,
    TransferPhase, UiEvent,
};

const OTP_BODY: &str = "【测试】验证码：524681，您正在登录";
const OTP: &str = "524681";

struct DirGuard(PathBuf);

impl DirGuard {
    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for DirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn temp_dir(tag: &str) -> (PathBuf, DirGuard) {
    let dir = std::env::temp_dir().join(format!(
        "tetherly-p1-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    (dir.clone(), DirGuard(dir))
}

fn cfg(dir: PathBuf, name: &str, platform: &str) -> NodeConfig {
    NodeConfig {
        data_dir: dir,
        name: name.into(),
        platform: platform.into(),
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
    loop {
        let peers = node.live_peers();
        if !peers.is_empty() && peers.iter().all(|p| p.file_port.is_some()) {
            return;
        }
        if start.elapsed() > timeout {
            panic!("peer did not come up with file_port");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn wait_offered(node: &Node, timeout: Duration) {
    let start = std::time::Instant::now();
    while node.offered_files().is_empty() {
        if start.elapsed() > timeout {
            panic!("file offer did not arrive");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn wait_inbox(dir: &std::path::Path, name: &str, timeout: Duration) -> Vec<u8> {
    let path = dir.join("inbox").join(name);
    let start = std::time::Instant::now();
    loop {
        if let Ok(bytes) = std::fs::read(&path) {
            if !bytes.is_empty() {
                return bytes;
            }
        }
        if start.elapsed() > timeout {
            panic!("inbox file missing");
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
}

async fn pair_phone_desktop() -> (Node, Node, DirGuard, DirGuard) {
    let (phone_dir, g1) = temp_dir("phone");
    let (desk_dir, g2) = temp_dir("desk");
    let phone = Node::start(cfg(phone_dir, "phone", "android")).unwrap();
    let desk = Node::start(cfg(desk_dir, "desk", "windows")).unwrap();
    phone.spawn_listeners().await.unwrap();
    desk.spawn_listeners().await.unwrap();
    let pin = tetherly_core::pin::parse_pin("25170394").unwrap();
    phone.set_pin(pin);
    desk.set_pin(pin);
    phone.dial(desk.control_addr()).await.unwrap();
    wait_live(&phone, Duration::from_secs(45)).await;
    wait_live(&desk, Duration::from_secs(45)).await;
    (phone, desk, g1, g2)
}

#[tokio::test(flavor = "multi_thread")]
async fn m1_1_simulated_notify_push_to_desktop() {
    let (phone, desk, _g1, _g2) = pair_phone_desktop().await;
    let mut rx = desk.subscribe();
    let t0 = std::time::Instant::now();
    phone
        .send_notify(desk.identity().device_id(), otp_push("uid-sim"))
        .await
        .unwrap();
    let mut seen = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while tokio::time::Instant::now() < deadline {
        if let Ok(Ok(UiEvent::Candidate { candidate })) =
            tokio::time::timeout(Duration::from_millis(200), rx.recv()).await
        {
            assert!(candidate.has_otp);
            assert_eq!(candidate.title, "网易");
            seen = true;
            break;
        }
    }
    assert!(seen, "desktop never showed a candidate");
    assert!(
        t0.elapsed() < Duration::from_secs(1),
        "simulated notify exceeded 1s: {:?}",
        t0.elapsed()
    );
    phone.shutdown();
    desk.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn m1_2_copy_clipboard_and_ttl() {
    let clock = Arc::new(ManualClock::new(5_000));
    let (dir, _g) = temp_dir("copy");
    let mut cfg = cfg(dir, "desk", "windows");
    cfg.control_port = 0;
    let desk = Node::start_with_clock(cfg, clock.clone()).unwrap();
    match desk
        .ingest_local_push(desk.identity().device_id().clone(), otp_push("uid-copy"))
        .await
    {
        tetherly_core::IngestOutcome::Candidate(v) => {
            desk.copy_candidate(&v.id).await.unwrap();
            assert_eq!(desk.clip().get_text().unwrap().as_deref(), Some(OTP));
            clock.set(5_000 + CANDIDATE_TTL_MS + 1);
            assert!(
                desk.candidates().is_empty(),
                "120s candidate must disappear"
            );
        }
        other => panic!("expected candidate, {other:?}"),
    }
}

#[tokio::test]
async fn m1_3_insert_empty_and_readonly() {
    let (dir, _g) = temp_dir("ins");
    let desk = Node::start(cfg(dir, "desk", "windows")).unwrap();
    match desk
        .ingest_local_push(desk.identity().device_id().clone(), otp_push("uid-ins"))
        .await
    {
        tetherly_core::IngestOutcome::Candidate(v) => {
            let empty = MemoryInsertor::new("", false);
            desk.insert_candidate_with(&v.id, &empty).unwrap();
            assert_eq!(empty.current.lock().unwrap().as_str(), OTP);

            let ro = MemoryInsertor::new("", true);
            let err = desk.insert_candidate_with(&v.id, &ro).unwrap_err();
            assert!(matches!(err, tetherly_core::CoreError::InsertRefused(_)));
            assert!(ro.current.lock().unwrap().is_empty());
        }
        other => panic!("expected candidate, {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn m1_4_win_loopback_clip_set() {
    let (a_dir, _g1) = temp_dir("clip-a");
    let (b_dir, _g2) = temp_dir("clip-b");
    let a = Node::start(cfg(a_dir, "a", "windows")).unwrap();
    let b = Node::start(cfg(b_dir, "b", "windows")).unwrap();
    a.spawn_listeners().await.unwrap();
    b.spawn_listeners().await.unwrap();
    let pin = tetherly_core::pin::parse_pin("25170394").unwrap();
    a.set_pin(pin);
    b.set_pin(pin);
    a.dial(b.control_addr()).await.unwrap();
    wait_live(&a, Duration::from_secs(45)).await;
    wait_live(&b, Duration::from_secs(45)).await;

    a.clip().set_text("hello-clip").unwrap();
    a.send_clipboard().await.unwrap();
    let start = std::time::Instant::now();
    loop {
        if a.clip().get_text().unwrap().as_deref() == Some("hello-clip")
            && b.clip().get_text().unwrap().as_deref() == Some("hello-clip")
        {
            break;
        }
        if start.elapsed() > Duration::from_secs(3) {
            panic!(
                "clip did not land; a={:?} b={:?}",
                a.clip().get_text().unwrap(),
                b.clip().get_text().unwrap()
            );
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    a.shutdown();
    b.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn m1_5_reconnect_without_repair() {
    let (phone, desk, g1, _g2) = pair_phone_desktop().await;
    let phone_dir = g1.0.clone();
    let desk_addr = desk.control_addr();
    let desk_id = desk.identity().device_id().clone();
    assert!(!phone.trusted().is_empty());
    assert!(!desk.trusted().is_empty());
    phone.shutdown();
    drop(phone);

    let phone2 = Node::start(cfg(phone_dir, "phone", "android")).unwrap();
    phone2.spawn_listeners().await.unwrap();
    phone2.dial(desk_addr).await.unwrap();
    wait_live(&phone2, Duration::from_secs(45)).await;
    wait_live(&desk, Duration::from_secs(45)).await;
    assert_eq!(phone2.live_peers()[0].device_id, desk_id);
    phone2
        .send_notify(desk.identity().device_id(), otp_push("uid-re"))
        .await
        .unwrap();
    let start = std::time::Instant::now();
    loop {
        if desk.candidates().iter().any(|c| c.has_otp) {
            break;
        }
        if start.elapsed() > Duration::from_secs(3) {
            panic!("notify after resume did not land");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    phone2.shutdown();
    desk.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn m1_6_logs_do_not_contain_otp_digits() {
    // The default EnvFilter hides debug/trace, which is exactly where a leak
    // would land, so widen it to the most verbose level: every event the
    // ingest path can emit is captured.
    let buf = tracing_capture::Capture::install_with("trace");
    let (phone, desk, _g1, _g2) = pair_phone_desktop().await;
    phone
        .send_notify(desk.identity().device_id(), otp_push("uid-log"))
        .await
        .unwrap();
    let start = std::time::Instant::now();
    while desk.candidates().is_empty() && start.elapsed() < Duration::from_secs(3) {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let dumped = buf.dump();
    // Assert the ingest actually logged, so a missing subscriber can never make
    // the redaction checks below pass vacuously.
    assert!(
        dumped.contains("notify ingested"),
        "ingest must log (capture was empty), dumping {} bytes",
        dumped.len()
    );
    assert!(!dumped.contains(OTP), "logs leaked OTP digits");
    assert!(!dumped.contains(OTP_BODY), "logs leaked notify body");
    phone.shutdown();
    desk.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn m1_7_file_reject_then_accept_sha256() {
    let (a_dir, _g1) = temp_dir("file-a");
    let (b_dir, _g2) = temp_dir("file-b");
    let a = Node::start(cfg(a_dir, "a", "windows")).unwrap();
    let b = Node::start(cfg(b_dir.clone(), "b", "windows")).unwrap();
    a.spawn_listeners().await.unwrap();
    b.spawn_listeners().await.unwrap();
    let pin = tetherly_core::pin::parse_pin("25170394").unwrap();
    a.set_pin(pin);
    b.set_pin(pin);
    a.dial(b.control_addr()).await.unwrap();
    wait_live(&a, Duration::from_secs(45)).await;
    wait_live(&b, Duration::from_secs(45)).await;
    let start = std::time::Instant::now();
    loop {
        if a.live_peers().iter().any(|p| p.file_port.is_some())
            && b.live_peers().iter().any(|p| p.file_port.is_some())
        {
            break;
        }
        if start.elapsed() > Duration::from_secs(3) {
            panic!("fileport caps.update never arrived");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let payload = vec![0x5Au8; 10 * 1024 * 1024];
    let digest = hex_sha256(&payload);
    let peer_b = b.identity().device_id().clone();

    let id_reject = a
        .offer_and_send_bytes(&peer_b, "secret.bin", &payload, false)
        .await
        .unwrap();
    wait_offered(&b, Duration::from_secs(3)).await;
    assert!(!std::path::Path::new(&b_dir)
        .join("inbox")
        .join("secret.bin")
        .exists());
    b.user_reject_file(&id_reject).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(!std::path::Path::new(&b_dir)
        .join("inbox")
        .join("secret.bin")
        .exists());

    let id_ok = a
        .offer_and_send_bytes(&peer_b, "ok.bin", &payload, false)
        .await
        .unwrap();
    wait_offered(&b, Duration::from_secs(3)).await;
    b.user_accept_file(&id_ok).await.unwrap();
    let got = wait_inbox(&b_dir, "ok.bin", Duration::from_secs(20)).await;
    assert_eq!(hex_sha256(&got), digest);
    a.shutdown();
    b.shutdown();
}

/// M1.7c — an offer whose declared size exceeds `FILE_MAX` is refused at the
/// receiver before the user is ever asked, and the refusal must not be fatal to
/// the session (the file hub rejects the offer, not the connection).
#[tokio::test(flavor = "multi_thread")]
async fn an_oversized_offer_is_refused_without_dropping_the_session() {
    let (a_dir, _g1) = temp_dir("cap-a");
    let (b_dir, _g2) = temp_dir("cap-b");
    let a = Node::start(cfg(a_dir, "a", "windows")).unwrap();
    let b = Node::start(cfg(b_dir.clone(), "b", "windows")).unwrap();
    a.spawn_listeners().await.unwrap();
    b.spawn_listeners().await.unwrap();
    let pin = tetherly_core::pin::parse_pin("25170394").unwrap();
    a.set_pin(pin);
    b.set_pin(pin);
    a.dial(b.control_addr()).await.unwrap();
    wait_live(&a, Duration::from_secs(45)).await;
    wait_live(&b, Duration::from_secs(45)).await;
    let start = std::time::Instant::now();
    while !a.live_peers().iter().any(|p| p.file_port.is_some())
        || !b.live_peers().iter().any(|p| p.file_port.is_some())
    {
        if start.elapsed() > Duration::from_secs(3) {
            panic!("fileport caps.update never arrived");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let peer_b = b.identity().device_id().clone();
    // The digest is well-formed, so only the size check can refuse this offer.
    let oversized = FileOffer {
        transfer_id: "tr_oversized".into(),
        files: vec![FileMeta {
            name: "huge.bin".into(),
            size: tetherly_core::FILE_MAX + 1,
            sha256: "d".repeat(64),
        }],
    };
    a.send_to(&peer_b, oversized.to_frame().unwrap())
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        b.offered_files().is_empty(),
        "an oversized offer must never become a candidate the user can accept"
    );
    // The session must still be usable: a normal transfer lands on it.
    let payload = vec![0x11u8; 64 * 1024];
    let id = a
        .offer_and_send_bytes(&peer_b, "small.bin", &payload, false)
        .await
        .unwrap();
    wait_offered(&b, Duration::from_secs(3)).await;
    b.user_accept_file(&id).await.unwrap();
    let got = wait_inbox(&b_dir, "small.bin", Duration::from_secs(20)).await;
    assert_eq!(hex_sha256(&got), hex_sha256(&payload));
    a.shutdown();
    b.shutdown();
}
/// M1.7b — an interrupted incoming transfer resumes from a persisted,
/// chunk-aligned offset after a restart, sending only the missing tail.
#[tokio::test(flavor = "multi_thread")]
async fn m1_8_file_resumes_from_persisted_offset() {
    let (a_dir, _g1) = temp_dir("resume-a");
    let (b_dir, _g2) = temp_dir("resume-b");
    // A's device id is needed to forge the receiver's resume sidecar before B
    // starts (the sidecar is what the restart-scan reads).
    let a = Node::start(cfg(a_dir, "a", "windows")).unwrap();
    let a_id = a.identity().device_id().clone();

    let chunk = tetherly_core::FILE_CHUNK;
    let payload: Vec<u8> = (0..(3 * chunk + 5000)).map(|i| (i % 251) as u8).collect();
    let total = payload.len() as u64;
    // Content-addressed id must match `offer_and_send_bytes`.
    let transfer_id = format!("tr_{}", &hex_sha256(&payload)[..16]);
    let kept = 2 * chunk; // chunk-aligned partial the receiver already holds

    // Seed B's inbox with the durable partial + sidecar, then start B so the
    // restart scan rebuilds the resume table.
    let inbox = b_dir.join("inbox");
    std::fs::create_dir_all(&inbox).unwrap();
    std::fs::write(inbox.join(format!(".{transfer_id}.part")), &payload[..kept]).unwrap();
    std::fs::write(
        inbox.join(format!(".{transfer_id}.resume.json")),
        format!(
            r#"{{"device_id":"{}","total":{},"offset":{}}}"#,
            a_id.as_str(),
            total,
            kept
        ),
    )
    .unwrap();

    let b = Node::start(cfg(b_dir.clone(), "b", "windows")).unwrap();
    a.spawn_listeners().await.unwrap();
    b.spawn_listeners().await.unwrap();
    let pin = tetherly_core::pin::parse_pin("25170394").unwrap();
    a.set_pin(pin);
    b.set_pin(pin);
    a.dial(b.control_addr()).await.unwrap();
    wait_live(&a, Duration::from_secs(45)).await;
    wait_live(&b, Duration::from_secs(45)).await;
    let start = std::time::Instant::now();
    while !a.live_peers().iter().any(|p| p.file_port.is_some())
        || !b.live_peers().iter().any(|p| p.file_port.is_some())
    {
        if start.elapsed() > Duration::from_secs(3) {
            panic!("fileport caps.update never arrived");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let peer_b = b.identity().device_id().clone();
    let id = a
        .offer_and_send_bytes(&peer_b, "resume.bin", &payload, false)
        .await
        .unwrap();
    assert_eq!(id, transfer_id, "transfer id must be content-addressed");
    wait_offered(&b, Duration::from_secs(3)).await;
    // The receiver saw the offer and knows it can resume at the persisted offset.
    let offered = b.offered_files();
    assert_eq!(offered[0].resume_from, Some(kept as u64));
    // Nothing final is written before the user confirms.
    assert!(!inbox.join("resume.bin").exists());

    b.user_accept_file(&id).await.unwrap();
    let got = wait_inbox(&b_dir, "resume.bin", Duration::from_secs(20)).await;
    assert_eq!(hex_sha256(&got), hex_sha256(&payload));
    // The partial + sidecar are cleaned up once the transfer completes.
    assert!(!inbox.join(format!(".{transfer_id}.part")).exists());
    assert!(!inbox.join(format!(".{transfer_id}.resume.json")).exists());
    a.shutdown();
    b.shutdown();
}
/// A PIN only means anything to the process that armed it. `tetherly pin`
/// therefore arms the *live* node through its loopback API; a second process
/// printing its own PIN would advertise a code no listener can ever accept.
/// This drives that exact route: the PIN handed back by `/api/pin` must be the
/// one the running listener accepts.
#[tokio::test(flavor = "multi_thread")]
async fn pin_shown_by_the_live_node_api_is_the_one_that_pairs() {
    let (host_dir, _g1) = temp_dir("livenode-host");
    let (peer_dir, _g2) = temp_dir("livenode-peer");
    let host = Node::start(cfg(host_dir, "host", "windows")).unwrap();
    let peer = Node::start(cfg(peer_dir, "peer", "android")).unwrap();
    host.spawn_listeners().await.unwrap();
    peer.spawn_listeners().await.unwrap();

    // The route `tetherly pin` uses. Any other port has no listener.
    let ui_port = host.spawn_ui().await.unwrap();
    assert_ne!(ui_port, 0, "test node must serve the loopback UI API");
    let (code, body) = tetherly_node::loopback_post(ui_port, "/api/pin", b"{}")
        .await
        .expect("live node API reachable");
    assert_eq!(code, 200, "body: {}", String::from_utf8_lossy(&body));
    let pin_str = serde_json::from_slice::<serde_json::Value>(&body)
        .unwrap()
        .get("pin")
        .and_then(|p| p.as_str().map(str::to_string))
        .expect("pin field");
    peer.set_pin(tetherly_core::pin::parse_pin(&pin_str).unwrap());

    peer.dial(host.control_addr()).await.unwrap();
    wait_live(&host, Duration::from_secs(45)).await;
    wait_live(&peer, Duration::from_secs(45)).await;
    assert_eq!(
        host.live_peers().first().map(|p| p.device_id.clone()),
        Some(peer.identity().device_id().clone())
    );

    // A PIN never armed at a live node (the defect's failure mode) is refused,
    // so this assertion cannot pass by accident on a stale/hardcoded code.
    let (other_dir, _g3) = temp_dir("livenode-stale");
    let stale = Node::start(cfg(other_dir, "stale", "windows")).unwrap();
    stale.spawn_listeners().await.unwrap();
    stale.set_pin(tetherly_core::pin::parse_pin("25170394").unwrap());
    assert!(
        peer.dial(stale.control_addr()).await.is_err(),
        "a peer that never served /api/pin must not be pairable"
    );

    host.shutdown();
    peer.shutdown();
    stale.shutdown();
}

/// `tetherly notify` cannot start a second node: only the process holding the
/// Noise session can deliver on it, and a second `Node::start` would collide on
/// the LAN ports ("no LAN address bound"). It posts to the live node's loopback
/// API instead. This drives that route end to end: the text posted to
/// `/api/notify` is what arrives at the paired peer's candidate list.
#[tokio::test(flavor = "multi_thread")]
async fn notify_posted_to_the_live_node_api_reaches_the_paired_peer() {
    let (host, peer, _g1, _g2) = pair_phone_desktop().await;
    let ui_port = host.spawn_ui().await.unwrap();
    assert_ne!(ui_port, 0, "test node must serve the loopback UI API");

    let mut rx = peer.subscribe();
    let payload = serde_json::json!({
        "title": "来自桌面",
        "body": "【测试】验证码：524681",
        "app_id": "com.example.sms",
    });
    let (code, body) = tetherly_node::loopback_post(
        ui_port,
        "/api/notify",
        &serde_json::to_vec(&payload).unwrap(),
    )
    .await
    .expect("live node API reachable");
    assert_eq!(code, 200, "body: {}", String::from_utf8_lossy(&body));

    let mut seen = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while tokio::time::Instant::now() < deadline {
        if let Ok(Ok(UiEvent::Candidate { candidate })) =
            tokio::time::timeout(Duration::from_millis(200), rx.recv()).await
        {
            assert_eq!(candidate.title, "来自桌面");
            assert!(candidate.has_otp, "the peer must extract the code");
            seen = true;
            break;
        }
    }
    assert!(seen, "peer never showed the posted notification");

    // An unknown target is refused, so a 200 cannot be a blanket "accepted".
    let missing = serde_json::json!({
        "title": "x", "body": "y",
        "peer": "tdev_2222222222222222",
    });
    let (code, _) = tetherly_node::loopback_post(
        ui_port,
        "/api/notify",
        &serde_json::to_vec(&missing).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code, 400, "a peer that is not connected must be refused");

    host.shutdown();
    peer.shutdown();
}

/// v1.25 production file-sending loop: a file posted to the live node's loopback
/// API (`POST /api/file/send`) via disk path or base64 payload arrives at the
/// paired peer as a FileOffer, is accepted, and is written to inbox matching SHA-256.
#[tokio::test(flavor = "multi_thread")]
async fn file_sent_via_live_node_api_reaches_the_paired_peer() {
    let (host, peer, _g1, g2) = pair_phone_desktop().await;
    let ui_port = host.spawn_ui().await.unwrap();
    assert_ne!(ui_port, 0, "test node must serve the loopback UI API");

    // 1. Create a temporary test file on disk
    let (send_dir, _g_send) = temp_dir("send-file");
    let file_path = send_dir.join("report.pdf");
    let file_content = b"PDF-1.7 Test Document Content Tetherly v1.25";
    std::fs::write(&file_path, file_content).unwrap();
    let expected_hash = hex_sha256(file_content);

    // 2. Post to /api/file/send with file path
    let payload = serde_json::json!({
        "path": file_path.to_str().unwrap(),
        "name": "report.pdf",
    });
    let (code, body) = tetherly_node::loopback_post(
        ui_port,
        "/api/file/send",
        &serde_json::to_vec(&payload).unwrap(),
    )
    .await
    .expect("live node API reachable");
    assert_eq!(
        code,
        200,
        "send response: {}",
        String::from_utf8_lossy(&body)
    );
    let resp: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let transfer_id = resp["transfer_id"].as_str().unwrap();

    // 3. Peer waits for the offer, then accepts the file
    wait_offered(&peer, Duration::from_secs(10)).await;
    peer.user_accept_file(transfer_id).await.unwrap();

    // 4. Peer receives and saves the file in inbox
    let received = wait_inbox(g2.path(), "report.pdf", Duration::from_secs(25)).await;
    assert_eq!(hex_sha256(&received), expected_hash);

    // 5. Test with base64 data upload directly (the web UI upload route)
    use base64::Engine;
    let b64_content = b"Base64 Upload Test Tetherly v1.25";
    let b64_str = base64::engine::general_purpose::STANDARD.encode(b64_content);
    let payload_b64 = serde_json::json!({
        "data_base64": b64_str,
        "name": "b64_doc.txt",
    });
    let (code, body) = tetherly_node::loopback_post(
        ui_port,
        "/api/file/send",
        &serde_json::to_vec(&payload_b64).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(
        code,
        200,
        "b64 send response: {}",
        String::from_utf8_lossy(&body)
    );
    let resp_b64: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let transfer_id_b64 = resp_b64["transfer_id"].as_str().unwrap();

    wait_offered(&peer, Duration::from_secs(10)).await;
    peer.user_accept_file(transfer_id_b64).await.unwrap();
    let received_b64 = wait_inbox(g2.path(), "b64_doc.txt", Duration::from_secs(25)).await;
    assert_eq!(hex_sha256(&received_b64), hex_sha256(b64_content));

    // 6. Test negative assertion: unknown peer returns 400
    let missing_peer = serde_json::json!({
        "path": file_path.to_str().unwrap(),
        "peer": "tdev_2222222222222222",
    });
    let (code, _) = tetherly_node::loopback_post(
        ui_port,
        "/api/file/send",
        &serde_json::to_vec(&missing_peer).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code, 400, "an unknown peer must be refused with 400");

    // 7. Test negative assertion: missing file returns 400
    let missing_file = serde_json::json!({
        "path": send_dir.join("nonexistent.txt").to_str().unwrap(),
    });
    let (code, _) = tetherly_node::loopback_post(
        ui_port,
        "/api/file/send",
        &serde_json::to_vec(&missing_file).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code, 400, "a missing file must be refused with 400");

    host.shutdown();
    peer.shutdown();
}

/// Multi-chunk streaming file transfer: a file spanning multiple 64 KiB chunks
/// is streamed through `offer_and_send_file` with O(1) memory, received into
/// `.part` with incremental SHA-256 hashing, and atomically placed in the inbox.
#[tokio::test(flavor = "multi_thread")]
async fn multi_chunk_file_streamed_via_live_node_api_reaches_inbox() {
    let (host, peer, _g1, g2) = pair_phone_desktop().await;
    let ui_port = host.spawn_ui().await.unwrap();

    let (send_dir, _g_send) = temp_dir("stream-send");
    let file_path = send_dir.join("large_archive.bin");
    let total_size = 3 * tetherly_core::FILE_CHUNK + 12345;
    let file_content: Vec<u8> = (0..total_size).map(|i| (i % 251) as u8).collect();
    std::fs::write(&file_path, &file_content).unwrap();
    let expected_hash = hex_sha256(&file_content);

    let payload = serde_json::json!({
        "path": file_path.to_str().unwrap(),
        "name": "large_archive.bin",
    });
    let (code, body) = tetherly_node::loopback_post(
        ui_port,
        "/api/file/send",
        &serde_json::to_vec(&payload).unwrap(),
    )
    .await
    .expect("live node API reachable");
    assert_eq!(
        code,
        200,
        "send response: {}",
        String::from_utf8_lossy(&body)
    );
    let resp: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let transfer_id = resp["transfer_id"].as_str().unwrap();

    wait_offered(&peer, Duration::from_secs(10)).await;
    peer.user_accept_file(transfer_id).await.unwrap();

    let received = wait_inbox(g2.path(), "large_archive.bin", Duration::from_secs(25)).await;
    assert_eq!(received.len(), total_size);
    assert_eq!(hex_sha256(&received), expected_hash);

    host.shutdown();
    peer.shutdown();
}

/// Streaming file transfer tracks active transfers, progress accumulation,
/// and terminal completion state both in runtime and via loopback HTTP snapshot.
#[tokio::test(flavor = "multi_thread")]
async fn streaming_file_transfer_tracks_active_progress_and_completion_states() {
    let (host, peer, _g1, g2) = pair_phone_desktop().await;
    let ui_port = host.spawn_ui().await.unwrap();

    let (send_dir, _g_send) = temp_dir("stream-prog");
    let file_path = send_dir.join("large_progress.bin");
    let total_size = 3 * tetherly_core::FILE_CHUNK + 4096;
    let file_content: Vec<u8> = (0..total_size).map(|i| (i % 233) as u8).collect();
    std::fs::write(&file_path, &file_content).unwrap();
    let expected_hash = hex_sha256(&file_content);

    let payload = serde_json::json!({
        "path": file_path.to_str().unwrap(),
        "name": "large_progress.bin",
    });
    let (code, body) = tetherly_node::loopback_post(
        ui_port,
        "/api/file/send",
        &serde_json::to_vec(&payload).unwrap(),
    )
    .await
    .expect("live node API reachable");
    assert_eq!(
        code,
        200,
        "send response: {}",
        String::from_utf8_lossy(&body)
    );
    let resp: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let transfer_id = resp["transfer_id"].as_str().unwrap();

    // Verify initial outbound active transfer in host runtime
    let initial_host_transfers: Vec<ActiveTransfer> = host.active_transfers();
    assert_eq!(initial_host_transfers.len(), 1);
    assert_eq!(initial_host_transfers[0].transfer_id, transfer_id);
    assert_eq!(
        initial_host_transfers[0].direction,
        TransferDirection::Outbound
    );
    assert_eq!(initial_host_transfers[0].total, total_size as u64);
    assert_eq!(initial_host_transfers[0].filename, "large_progress.bin");

    wait_offered(&peer, Duration::from_secs(10)).await;
    peer.user_accept_file(transfer_id).await.unwrap();

    let received = wait_inbox(g2.path(), "large_progress.bin", Duration::from_secs(25)).await;
    assert_eq!(received.len(), total_size);
    assert_eq!(hex_sha256(&received), expected_hash);

    // Host should reflect completed outbound transfer with full progress
    let host_transfers = host.active_transfers();
    assert_eq!(host_transfers.len(), 1);
    assert_eq!(host_transfers[0].phase, TransferPhase::Completed);
    assert_eq!(host_transfers[0].transferred, total_size as u64);

    // Peer should reflect completed inbound transfer with full progress
    let peer_transfers = peer.active_transfers();
    assert_eq!(peer_transfers.len(), 1);
    assert_eq!(peer_transfers[0].phase, TransferPhase::Completed);
    assert_eq!(peer_transfers[0].direction, TransferDirection::Inbound);
    assert_eq!(peer_transfers[0].transferred, total_size as u64);

    // Snapshot HTTP endpoint should expose transfers array
    let (sc_code, sc_body) = {
        let addr = std::net::SocketAddr::from(([127, 0, 0, 1], ui_port));
        let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let head = "GET /api/state HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n";
        stream.write_all(head.as_bytes()).await.unwrap();
        stream.flush().await.unwrap();
        let mut buf = Vec::new();
        stream.read_to_end(&mut buf).await.unwrap();
        let text = String::from_utf8_lossy(&buf);
        let code = text
            .split_whitespace()
            .nth(1)
            .and_then(|c| c.parse().ok())
            .unwrap_or(0);
        let payload = text
            .split("\r\n\r\n")
            .nth(1)
            .map(|b| b.as_bytes().to_vec())
            .unwrap_or_default();
        (code, payload)
    };
    assert_eq!(sc_code, 200);
    let snapshot: serde_json::Value = serde_json::from_slice(&sc_body).unwrap();
    let snap_transfers = snapshot["transfers"].as_array().expect("transfers array");
    assert_eq!(snap_transfers.len(), 1);
    assert_eq!(snap_transfers[0]["transfer_id"], transfer_id);
    assert_eq!(snap_transfers[0]["phase"], "completed");
    assert_eq!(snap_transfers[0]["transferred"], total_size as u64);
    assert_eq!(snap_transfers[0]["total"], total_size as u64);
    assert_eq!(snap_transfers[0]["filename"], "large_progress.bin");

    host.shutdown();
    peer.shutdown();
}

/// v1.28: Enterprise file transfer limits (64 GiB), transfer velocity (speed_bps)
/// tracking, and loopback GET status observability pipeline (ADR-021).
#[tokio::test(flavor = "multi_thread")]
async fn enterprise_file_limits_speed_and_status_observability() {
    let (host, peer, _g1, g2) = pair_phone_desktop().await;
    let ui_port = host.spawn_ui().await.unwrap();

    // 1. Verify loopback_get retrieves valid node state for CLI status command
    let (code, body) = tetherly_node::loopback_get(ui_port, "/api/state")
        .await
        .expect("loopback_get state succeeds");
    assert_eq!(code, 200);
    let state: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        state["device_id"].as_str().unwrap(),
        host.identity().device_id().as_str()
    );
    assert_eq!(state["name"].as_str().unwrap(), "phone");
    let peers_arr = state["peers"].as_array().expect("peers array");
    assert_eq!(peers_arr.len(), 1);
    assert_eq!(
        peers_arr[0]["device_id"].as_str().unwrap(),
        peer.identity().device_id().as_str()
    );

    // 2. Perform a multi-chunk streaming transfer to test progress and velocity tracking
    let (send_dir, _g_send) = temp_dir("obs-send");
    let file_path = send_dir.join("speed_test.bin");
    let total_size = 4 * tetherly_core::FILE_CHUNK + 2048;
    let file_content: Vec<u8> = (0..total_size).map(|i| (i % 179) as u8).collect();
    std::fs::write(&file_path, &file_content).unwrap();

    let payload = serde_json::json!({
        "path": file_path.to_str().unwrap(),
        "name": "speed_test.bin",
    });
    let (send_code, send_body) = tetherly_node::loopback_post(
        ui_port,
        "/api/file/send",
        &serde_json::to_vec(&payload).unwrap(),
    )
    .await
    .expect("send succeeds");
    assert_eq!(send_code, 200);
    let send_resp: serde_json::Value = serde_json::from_slice(&send_body).unwrap();
    let transfer_id = send_resp["transfer_id"].as_str().unwrap();

    // Verify ActiveTransfer structure contains speed_bps field in runtime
    let initial_transfers = host.active_transfers();
    assert_eq!(initial_transfers.len(), 1);
    assert_eq!(initial_transfers[0].transfer_id, transfer_id);
    assert_eq!(initial_transfers[0].speed_bps, 0); // initially 0 before transfer

    wait_offered(&peer, Duration::from_secs(10)).await;
    peer.user_accept_file(transfer_id).await.unwrap();

    let received = wait_inbox(g2.path(), "speed_test.bin", Duration::from_secs(25)).await;
    assert_eq!(received.len(), total_size);

    // 3. Verify loopback_get /api/state reveals updated transfer with completed phase and speed_bps zeroed out
    let (sc_code, sc_body) = tetherly_node::loopback_get(ui_port, "/api/state")
        .await
        .expect("loopback_get state succeeds");
    assert_eq!(sc_code, 200);
    let final_state: serde_json::Value = serde_json::from_slice(&sc_body).unwrap();
    let transfers = final_state["transfers"]
        .as_array()
        .expect("transfers array");
    assert_eq!(transfers.len(), 1);
    assert_eq!(transfers[0]["transfer_id"], transfer_id);
    assert_eq!(transfers[0]["phase"], "completed");
    assert_eq!(transfers[0]["transferred"], total_size as u64);
    assert_eq!(transfers[0]["total"], total_size as u64);
    // On terminal phase, speed_bps must be settled to 0
    assert_eq!(transfers[0]["speed_bps"], 0);

    // 4. Test 64 GiB limit boundary via wire offer to peer
    let peer_dev_id = peer.identity().device_id().clone();
    let sixty_four_gib: u64 = 64 * 1024 * 1024 * 1024;
    let oversized = tetherly_core::FileOffer {
        transfer_id: "tr_64g_plus_1".into(),
        files: vec![tetherly_core::FileMeta {
            name: "huge_virtual_disk.img".into(),
            size: sixty_four_gib + 1,
            sha256: "f".repeat(64),
        }],
    };
    host.send_to(&peer_dev_id, oversized.to_frame().unwrap())
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        peer.offered_files()
            .iter()
            .all(|f| f.transfer_id != "tr_64g_plus_1"),
        "an offer exceeding 64 GiB must be dropped by the receiver"
    );

    host.shutdown();
    peer.shutdown();
}

/// v1.29 ADR-022: In-flight streaming file transfer can be cancelled via loopback API
/// (`POST /api/file/cancel`), immediately aborting the transfer task, resetting speed_bps,
/// setting phase to Failed ("cancelled by user"), without dropping or corrupting the main session.
#[tokio::test(flavor = "multi_thread")]
async fn in_flight_streaming_transfer_can_be_cancelled_via_loopback_api() {
    let (host, peer, _g1, g2) = pair_phone_desktop().await;
    let ui_port = host.spawn_ui().await.unwrap();

    // 1. Prepare a test file
    let (send_dir, _g_send) = temp_dir("cancel-send");
    let file_path = send_dir.join("cancellable.bin");
    let total_size = 10 * tetherly_core::FILE_CHUNK + 1024;
    let file_content: Vec<u8> = vec![0x42; total_size];
    std::fs::write(&file_path, &file_content).unwrap();

    // 2. Initiate send via loopback API
    let payload = serde_json::json!({
        "path": file_path.to_str().unwrap(),
        "name": "cancellable.bin",
    });
    let (send_code, send_body) = tetherly_node::loopback_post(
        ui_port,
        "/api/file/send",
        &serde_json::to_vec(&payload).unwrap(),
    )
    .await
    .expect("send succeeds");
    assert_eq!(send_code, 200);
    let send_resp: serde_json::Value = serde_json::from_slice(&send_body).unwrap();
    let transfer_id = send_resp["transfer_id"].as_str().unwrap().to_string();

    // Peer accepts the offer so the transfer is actively in flight
    wait_offered(&peer, Duration::from_secs(10)).await;
    peer.user_accept_file(&transfer_id).await.unwrap();

    // Wait until the transfer is actively in-flight and streaming chunks
    let start_stream = std::time::Instant::now();
    loop {
        let transfers = host.active_transfers();
        if let Some(t) = transfers.iter().find(|x| x.transfer_id == transfer_id) {
            if t.transferred > 0 {
                break;
            }
        }
        if start_stream.elapsed() > Duration::from_secs(10) {
            panic!("transfer did not begin streaming chunks within 10s");
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    // 3. Cancel the in-flight transfer via loopback API
    let cancel_payload = serde_json::json!({
        "id": transfer_id,
    });
    let (cancel_code, cancel_body) = tetherly_node::loopback_post(
        ui_port,
        "/api/file/cancel",
        &serde_json::to_vec(&cancel_payload).unwrap(),
    )
    .await
    .expect("cancel succeeds");
    assert_eq!(
        cancel_code,
        200,
        "cancel response: {}",
        String::from_utf8_lossy(&cancel_body)
    );
    let cancel_resp: serde_json::Value = serde_json::from_slice(&cancel_body).unwrap();
    assert_eq!(cancel_resp["ok"], true);
    assert_eq!(cancel_resp["transfer_id"], transfer_id);

    // 4. Verify host state shows Failed with "cancelled by user"
    let host_transfers = host.active_transfers();
    let t = host_transfers
        .iter()
        .find(|x| x.transfer_id == transfer_id)
        .expect("transfer in list");
    assert_eq!(t.phase, TransferPhase::Failed);
    assert_eq!(t.error.as_deref(), Some("cancelled by user"));
    assert_eq!(t.speed_bps, 0);

    // Verify peer state also transitions to Failed via bilateral wire rejection
    let start_peer = std::time::Instant::now();
    loop {
        let peer_transfers = peer.active_transfers();
        if let Some(pt) = peer_transfers.iter().find(|x| x.transfer_id == transfer_id) {
            if pt.phase == TransferPhase::Failed {
                break;
            }
        }
        if start_peer.elapsed() > Duration::from_secs(5) {
            panic!("peer transfer failed to settle to Failed state after cancellation");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 5. Verify cancel idempotency / error handling: cancelling again returns 400
    let (cancel_code2, _) = tetherly_node::loopback_post(
        ui_port,
        "/api/file/cancel",
        &serde_json::to_vec(&cancel_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(
        cancel_code2, 400,
        "cancelling non-active transfer should return 400"
    );

    // 6. Verify main control session is unaffected: send a notification over the session
    let notify_payload = serde_json::json!({
        "title": "Session Health Check",
        "body": "Control session remains fully operational",
        "app_id": "com.tetherly.test",
    });
    let (notif_code, notif_body) = tetherly_node::loopback_post(
        ui_port,
        "/api/notify",
        &serde_json::to_vec(&notify_payload).unwrap(),
    )
    .await
    .expect("notify succeeds");
    assert_eq!(
        notif_code,
        200,
        "notify response: {}",
        String::from_utf8_lossy(&notif_body)
    );

    let start = std::time::Instant::now();
    loop {
        if peer
            .candidates()
            .iter()
            .any(|c| c.title == "Session Health Check")
        {
            break;
        }
        if start.elapsed() > Duration::from_secs(5) {
            panic!("notification after cancelled transfer did not arrive");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    // 7. Verify a subsequent file transfer succeeds completely
    let ok_file = send_dir.join("healthy.bin");
    let ok_content = b"Subsequent transfer works perfectly!";
    std::fs::write(&ok_file, ok_content).unwrap();
    let ok_payload = serde_json::json!({
        "path": ok_file.to_str().unwrap(),
        "name": "healthy.bin",
    });
    let (ok_code, ok_body) = tetherly_node::loopback_post(
        ui_port,
        "/api/file/send",
        &serde_json::to_vec(&ok_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(ok_code, 200);
    let ok_resp: serde_json::Value = serde_json::from_slice(&ok_body).unwrap();
    let ok_transfer_id = ok_resp["transfer_id"].as_str().unwrap().to_string();

    let start = std::time::Instant::now();
    while !peer
        .offered_files()
        .iter()
        .any(|t| t.transfer_id == ok_transfer_id)
    {
        if start.elapsed() > Duration::from_secs(5) {
            panic!("subsequent file offer did not arrive");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    peer.user_accept_file(&ok_transfer_id).await.unwrap();

    let start = std::time::Instant::now();
    let received = loop {
        let path = g2.path().join("inbox").join("healthy.bin");
        if let Ok(bytes) = std::fs::read(&path) {
            if !bytes.is_empty() {
                break bytes;
            }
        }
        if start.elapsed() > Duration::from_secs(20) {
            let host_t = host
                .active_transfers()
                .into_iter()
                .find(|t| t.transfer_id == ok_transfer_id);
            let peer_t = peer
                .active_transfers()
                .into_iter()
                .find(|t| t.transfer_id == ok_transfer_id);
            panic!("inbox file missing after 20s: host={host_t:?}, peer={peer_t:?}");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert_eq!(received, ok_content);

    let start = std::time::Instant::now();
    loop {
        let host_done = host
            .active_transfers()
            .into_iter()
            .find(|t| t.transfer_id == ok_transfer_id)
            .map(|t| t.phase == TransferPhase::Completed)
            .unwrap_or(false);
        let peer_done = peer
            .active_transfers()
            .into_iter()
            .find(|t| t.transfer_id == ok_transfer_id)
            .map(|t| t.phase == TransferPhase::Completed)
            .unwrap_or(false);
        if host_done && peer_done {
            break;
        }
        if start.elapsed() > Duration::from_secs(10) {
            let host_t = host
                .active_transfers()
                .into_iter()
                .find(|t| t.transfer_id == ok_transfer_id);
            let peer_t = peer
                .active_transfers()
                .into_iter()
                .find(|t| t.transfer_id == ok_transfer_id);
            panic!(
                "subsequent transfer did not settle to Completed: host={host_t:?}, peer={peer_t:?}"
            );
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    host.shutdown();
    peer.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn transfer_lifecycle_governance_and_stats() {
    let (host, peer, _g1, g2) = pair_phone_desktop().await;
    let ui_port = host.spawn_ui().await.unwrap();

    // 1. Initial transfer stats are zero
    let (code, body) = tetherly_node::loopback_get(ui_port, "/api/state")
        .await
        .expect("get state");
    assert_eq!(code, 200);
    let state_v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let stats = &state_v["transfer_stats"];
    assert_eq!(stats["bytes_sent"], 0);
    assert_eq!(stats["bytes_received"], 0);
    assert_eq!(stats["completed_count"], 0);
    assert_eq!(stats["failed_count"], 0);

    // 2. Perform a successful transfer and check stats accumulation
    let (send_dir, _g_send) = temp_dir("lifecycle-send");
    let file_path = send_dir.join("stats_test.bin");
    let total_size = 2 * tetherly_core::FILE_CHUNK + 512;
    let file_content: Vec<u8> = vec![0x77; total_size];
    std::fs::write(&file_path, &file_content).unwrap();

    let payload = serde_json::json!({
        "path": file_path.to_str().unwrap(),
        "name": "stats_test.bin",
    });
    let (send_code, send_body) = tetherly_node::loopback_post(
        ui_port,
        "/api/file/send",
        &serde_json::to_vec(&payload).unwrap(),
    )
    .await
    .expect("send succeeds");
    assert_eq!(send_code, 200);
    let send_resp: serde_json::Value = serde_json::from_slice(&send_body).unwrap();
    let transfer_id = send_resp["transfer_id"].as_str().unwrap().to_string();

    wait_offered(&peer, Duration::from_secs(10)).await;
    peer.user_accept_file(&transfer_id).await.unwrap();

    // Wait until peer receives the file
    let received = wait_inbox(&g2.0, "stats_test.bin", Duration::from_secs(25)).await;
    assert_eq!(received.len(), total_size);

    // Wait until host transfer settles to Completed
    let start = std::time::Instant::now();
    loop {
        let transfers = host.active_transfers();
        if let Some(t) = transfers.iter().find(|x| x.transfer_id == transfer_id) {
            if t.phase == TransferPhase::Completed {
                break;
            }
        }
        if start.elapsed() > Duration::from_secs(25) {
            panic!("host transfer did not settle to Completed");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    let h_stats = host.transfer_stats();
    assert_eq!(h_stats.bytes_sent, total_size as u64);
    assert_eq!(h_stats.completed_count, 1);
    assert_eq!(h_stats.failed_count, 0);

    let p_stats = peer.transfer_stats();
    assert_eq!(p_stats.bytes_received, total_size as u64);
    assert_eq!(p_stats.completed_count, 1);

    // 3. Perform a rejected transfer and verify failed_count increments
    let file2_path = send_dir.join("reject_test.bin");
    std::fs::write(&file2_path, b"quick fail").unwrap();
    let payload2 = serde_json::json!({
        "path": file2_path.to_str().unwrap(),
        "name": "reject_test.bin",
    });
    let (s2_code, s2_body) = tetherly_node::loopback_post(
        ui_port,
        "/api/file/send",
        &serde_json::to_vec(&payload2).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(s2_code, 200);
    let s2_resp: serde_json::Value = serde_json::from_slice(&s2_body).unwrap();
    let transfer_id2 = s2_resp["transfer_id"].as_str().unwrap().to_string();

    wait_offered(&peer, Duration::from_secs(10)).await;
    peer.user_reject_file(&transfer_id2).await.unwrap();

    let start = std::time::Instant::now();
    loop {
        let transfers = host.active_transfers();
        if let Some(t) = transfers.iter().find(|x| x.transfer_id == transfer_id2) {
            if t.phase == TransferPhase::Failed {
                break;
            }
        }
        if start.elapsed() > Duration::from_secs(5) {
            panic!("host transfer did not settle to Failed on peer rejection");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    let h_stats2 = host.transfer_stats();
    assert_eq!(h_stats2.bytes_sent, total_size as u64);
    assert_eq!(h_stats2.completed_count, 1);
    assert_eq!(h_stats2.failed_count, 1);

    // 4. Test clearing finished transfers via loopback API
    let host_transfers = host.active_transfers();
    assert_eq!(host_transfers.len(), 2);

    let (clear_code, clear_body) = tetherly_node::loopback_post(ui_port, "/api/file/clear", b"{}")
        .await
        .expect("clear succeeds");
    assert_eq!(clear_code, 200);
    let clear_resp: serde_json::Value = serde_json::from_slice(&clear_body).unwrap();
    assert_eq!(clear_resp["ok"], true);
    assert_eq!(clear_resp["cleared"], 2);

    // Transfers list is now empty, but transfer_stats remain preserved
    assert!(host.active_transfers().is_empty());
    let h_stats3 = host.transfer_stats();
    assert_eq!(h_stats3.bytes_sent, total_size as u64);
    assert_eq!(h_stats3.completed_count, 1);
    assert_eq!(h_stats3.failed_count, 1);

    host.shutdown();
    peer.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn peer_unpair_and_trust_revocation() {
    let (host, peer, g1, _g2) = pair_phone_desktop().await;
    let ui_port = host.spawn_ui().await.unwrap();

    let peer_id = peer.identity().device_id().clone();

    // 1. Peer is initially in trusted and live peers
    let trusted_init = host.trusted();
    assert!(trusted_init.iter().any(|p| p.device_id == peer_id));
    assert_eq!(host.live_peers().len(), 1);

    // 2. Unpair via loopback API
    let payload = serde_json::json!({
        "device_id": peer_id.as_str(),
    });
    let (code, body) = tetherly_node::loopback_post(
        ui_port,
        "/api/peers/unpair",
        &serde_json::to_vec(&payload).unwrap(),
    )
    .await
    .expect("unpair post succeeds");
    assert_eq!(code, 200);
    let resp: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(resp["ok"], true);
    assert_eq!(resp["device_id"], peer_id.as_str());
    assert_eq!(resp["unpaired"], true);

    // 3. Peer is immediately removed from in-memory trust store
    let trusted_after = host.trusted();
    assert!(!trusted_after.iter().any(|p| p.device_id == peer_id));

    // 4. Peer is removed from persisted trusted.json on disk
    let loaded_store = tetherly_node::store::load_trust(g1.path()).unwrap();
    assert!(!loaded_store.all().iter().any(|p| p.device_id == peer_id));

    // 5. Active connection is severed
    let start = std::time::Instant::now();
    loop {
        if host.live_peers().is_empty() {
            break;
        }
        if start.elapsed() > Duration::from_secs(5) {
            panic!("live peer was not evicted within 5s after unpair");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    // 6. Idempotent unpair of already-removed peer returns unpaired: false
    let (code2, body2) = tetherly_node::loopback_post(
        ui_port,
        "/api/peers/unpair",
        &serde_json::to_vec(&payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code2, 200);
    let resp2: serde_json::Value = serde_json::from_slice(&body2).unwrap();
    assert_eq!(resp2["unpaired"], false);

    // 7. Invalid device_id returns 400 Bad Request
    let bad_payload = serde_json::json!({
        "device_id": "invalid_id_format",
    });
    let (code3, _) = tetherly_node::loopback_post(
        ui_port,
        "/api/peers/unpair",
        &serde_json::to_vec(&bad_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code3, 400);

    host.shutdown();
    peer.shutdown();
}

/// Tests that clipboard synchronization via live loopback HTTP API correctly
/// inspects local clipboard state, broadcasts text to connected peers, respects
/// echo suppression, handles targeted peer delivery, and rejects oversized payloads.
#[tokio::test(flavor = "multi_thread")]
async fn clip_sync_and_send_via_live_node_api_reaches_peers() {
    let (host, peer, _g1, _g2) = pair_phone_desktop().await;
    let ui_port = host.spawn_ui().await.unwrap();
    let peer_id = peer.identity().device_id().clone();

    // 1. Initial GET /api/clip returns has_text: false when clipboard is empty
    let (code, body) = tetherly_node::loopback_get(ui_port, "/api/clip")
        .await
        .expect("get clip succeeds");
    assert_eq!(code, 200);
    let resp: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(resp["ok"], true);
    assert_eq!(resp["has_text"], false);

    // 2. Setting local clipboard on host reflects in GET /api/clip
    let initial_msg = "tetherly-v1.32-clipboard-sync";
    host.clip().set_text(initial_msg).unwrap();
    let (code, body) = tetherly_node::loopback_get(ui_port, "/api/clip")
        .await
        .unwrap();
    assert_eq!(code, 200);
    let resp: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(resp["ok"], true);
    assert_eq!(resp["has_text"], true);
    assert_eq!(resp["text"], initial_msg);
    assert_eq!(resp["bytes"], initial_msg.len());

    // 3. POST /api/clip/sync broadcasts local clipboard to connected peer
    let (sync_code, sync_body) = tetherly_node::loopback_post(ui_port, "/api/clip/sync", b"{}")
        .await
        .unwrap();
    assert_eq!(sync_code, 200);
    let sync_resp: serde_json::Value = serde_json::from_slice(&sync_body).unwrap();
    assert_eq!(sync_resp["ok"], true);
    assert_eq!(sync_resp["sent_peers"], 1);
    assert_eq!(sync_resp["bytes"], initial_msg.len());

    // Wait until peer receives the clipboard content
    let start = std::time::Instant::now();
    loop {
        if peer.clip().get_text().unwrap().as_deref() == Some(initial_msg) {
            break;
        }
        if start.elapsed() > Duration::from_secs(10) {
            panic!(
                "peer did not receive synced clipboard within 10s; current: {:?}",
                peer.clip().get_text().unwrap()
            );
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }

    // 4. Repeated sync of identical unchanged clipboard is suppressed (sent_peers = 0)
    let (echo_code, echo_body) = tetherly_node::loopback_post(ui_port, "/api/clip/sync", b"{}")
        .await
        .unwrap();
    assert_eq!(echo_code, 200);
    let echo_resp: serde_json::Value = serde_json::from_slice(&echo_body).unwrap();
    assert_eq!(echo_resp["sent_peers"], 0);

    // 5. POST /api/clip/sync with explicit text payload broadcasts and sets local clipboard
    let explicit_text = "explicit broadcast to peer";
    let explicit_payload = serde_json::json!({
        "text": explicit_text,
    });
    let (exp_code, exp_body) = tetherly_node::loopback_post(
        ui_port,
        "/api/clip/sync",
        &serde_json::to_vec(&explicit_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(exp_code, 200);
    let exp_resp: serde_json::Value = serde_json::from_slice(&exp_body).unwrap();
    assert_eq!(exp_resp["ok"], true);
    assert_eq!(exp_resp["sent_peers"], 1);

    let start = std::time::Instant::now();
    loop {
        if peer.clip().get_text().unwrap().as_deref() == Some(explicit_text) {
            break;
        }
        if start.elapsed() > Duration::from_secs(10) {
            panic!("peer did not receive explicit clipboard within 10s");
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    assert_eq!(
        host.clip().get_text().unwrap().as_deref(),
        Some(explicit_text)
    );

    // 6. Targeted sync to specific peer
    let targeted_text = "targeted message to peer";
    let targeted_payload = serde_json::json!({
        "text": targeted_text,
        "peer": peer_id.as_str(),
    });
    let (targ_code, targ_body) = tetherly_node::loopback_post(
        ui_port,
        "/api/clip/sync",
        &serde_json::to_vec(&targeted_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(targ_code, 200);
    let targ_resp: serde_json::Value = serde_json::from_slice(&targ_body).unwrap();
    assert_eq!(targ_resp["sent_peers"], 1);

    // Negative: targeted sync to unknown peer returns 400
    let unknown_payload = serde_json::json!({
        "text": "lost text",
        "peer": "tdev_2222222222222222",
    });
    let (bad_peer_code, _) = tetherly_node::loopback_post(
        ui_port,
        "/api/clip/sync",
        &serde_json::to_vec(&unknown_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(bad_peer_code, 400);

    // 7. Oversized text (>1 MiB CLIP_TEXT_MAX) is refused with 400
    let oversized_text = "o".repeat(tetherly_core::CLIP_TEXT_MAX + 1);
    let oversized_payload = serde_json::json!({
        "text": oversized_text,
    });
    let (oversized_code, _) = tetherly_node::loopback_post(
        ui_port,
        "/api/clip/sync",
        &serde_json::to_vec(&oversized_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(oversized_code, 400);

    host.shutdown();
    peer.shutdown();
}

/// Tests the full notification candidate lifecycle, OTP inspection and copying,
/// quick reply transmission, queued replies querying, and single-use reply consumption
/// via loopback HTTP API and live Noise session.
#[tokio::test(flavor = "multi_thread")]
async fn notify_candidate_lifecycle_and_quick_reply_via_loopback_api() {
    let (host, peer, _g1, _g2) = pair_phone_desktop().await;
    let host_ui = host.spawn_ui().await.unwrap();
    let peer_ui = peer.spawn_ui().await.unwrap();
    let host_id = host.identity().device_id().clone();
    let _peer_id = peer.identity().device_id().clone();

    // 1. Initial candidates on host should be empty
    let (code, body) = tetherly_node::loopback_get(host_ui, "/api/notify/candidates")
        .await
        .unwrap();
    assert_eq!(code, 200);
    let resp: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(resp["ok"], true);
    assert!(resp["candidates"].as_array().unwrap().is_empty());

    // 2. Peer (phone) sends a notification containing an OTP verification code
    let push = tetherly_core::NotifyPush {
        uid: "test-sms-1001".into(),
        app_id: "com.example.bank".into(),
        app_name: "ExampleBank".into(),
        title: "验证码通知".into(),
        body: "【示例银行】您的短信验证码为 849201，5分钟内有效。".into(),
        ts: 1_700_000_000,
        actions: vec!["copy".into(), "dismiss".into()],
    };
    peer.send_notify(&host_id, push).await.unwrap();

    // 3. Wait until host receives the candidate
    let start = std::time::Instant::now();
    let cand_id = loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/notify/candidates")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let v: serde_json::Value = serde_json::from_slice(&b).unwrap();
        let list = v["candidates"].as_array().unwrap();
        if !list.is_empty() {
            let cand = &list[0];
            assert_eq!(cand["app_id"], "com.example.bank");
            assert_eq!(cand["has_otp"], true);
            break cand["id"].as_str().unwrap().to_string();
        }
        if start.elapsed() > Duration::from_secs(10) {
            panic!("host did not ingest candidate within 10s");
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    };

    // 4. Retrieve OTP directly via POST /api/otp/get
    let get_otp_payload = serde_json::json!({ "id": cand_id });
    let (otp_code, otp_body) = tetherly_node::loopback_post(
        host_ui,
        "/api/otp/get",
        &serde_json::to_vec(&get_otp_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(otp_code, 200);
    let otp_resp: serde_json::Value = serde_json::from_slice(&otp_body).unwrap();
    assert_eq!(otp_resp["ok"], true);
    assert_eq!(otp_resp["otp"], "849201");

    // 5. Copy OTP to system clipboard via POST /api/copy
    let (copy_code, copy_body) = tetherly_node::loopback_post(
        host_ui,
        "/api/copy",
        &serde_json::to_vec(&get_otp_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(copy_code, 200);
    let copy_resp: serde_json::Value = serde_json::from_slice(&copy_body).unwrap();
    assert_eq!(copy_resp["ok"], true);
    assert_eq!(host.clip().get_text().unwrap().as_deref(), Some("849201"));

    // 6. Send a quick reply back to peer via POST /api/reply
    let reply_text = "验证码已收到";
    let reply_payload = serde_json::json!({
        "id": cand_id,
        "text": reply_text,
    });
    let (reply_code, reply_body) = tetherly_node::loopback_post(
        host_ui,
        "/api/reply",
        &serde_json::to_vec(&reply_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(reply_code, 200);
    let reply_resp: serde_json::Value = serde_json::from_slice(&reply_body).unwrap();
    assert_eq!(reply_resp["ok"], true);

    // 7. Wait until peer ingests the reply in its replies queue
    let start = std::time::Instant::now();
    let reply_id = loop {
        let (rc, rb) = tetherly_node::loopback_get(peer_ui, "/api/notify/replies")
            .await
            .unwrap();
        assert_eq!(rc, 200);
        let rv: serde_json::Value = serde_json::from_slice(&rb).unwrap();
        let replies = rv["replies"].as_array().unwrap();
        if !replies.is_empty() {
            let r = &replies[0];
            assert_eq!(r["app_id"], "com.example.bank");
            assert_eq!(r["uid"], "test-sms-1001");
            break r["id"].as_str().unwrap().to_string();
        }
        if start.elapsed() > Duration::from_secs(10) {
            panic!("peer did not receive reply within 10s");
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    };

    // 8. Peer takes the reply via POST /api/reply/take
    let take_payload = serde_json::json!({ "id": reply_id });
    let (take_code, take_body) = tetherly_node::loopback_post(
        peer_ui,
        "/api/reply/take",
        &serde_json::to_vec(&take_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(take_code, 200);
    let take_resp: serde_json::Value = serde_json::from_slice(&take_body).unwrap();
    assert_eq!(take_resp["ok"], true);
    assert_eq!(take_resp["text"], reply_text);
    assert_eq!(take_resp["app_id"], "com.example.bank");
    assert_eq!(take_resp["uid"], "test-sms-1001");

    // Single-use: taking the reply again returns 400
    let (take2_code, _) = tetherly_node::loopback_post(
        peer_ui,
        "/api/reply/take",
        &serde_json::to_vec(&take_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(take2_code, 400);

    // 9. Host dismisses candidate via POST /api/dismiss
    let (dismiss_code, _) = tetherly_node::loopback_post(
        host_ui,
        "/api/dismiss",
        &serde_json::to_vec(&get_otp_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(dismiss_code, 200);

    // Candidates list is now empty
    let (c_after, b_after) = tetherly_node::loopback_get(host_ui, "/api/notify/candidates")
        .await
        .unwrap();
    assert_eq!(c_after, 200);
    let v_after: serde_json::Value = serde_json::from_slice(&b_after).unwrap();
    assert!(v_after["candidates"].as_array().unwrap().is_empty());

    // 10. Negative tests: invalid ID returns 400
    let bad_payload = serde_json::json!({ "id": "unknown-cand-id" });
    let (bad_otp_code, _) = tetherly_node::loopback_post(
        host_ui,
        "/api/otp/get",
        &serde_json::to_vec(&bad_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(bad_otp_code, 400);

    let empty_reply_payload = serde_json::json!({
        "id": "unknown-cand-id",
        "text": "   ",
    });
    let (empty_reply_code, _) = tetherly_node::loopback_post(
        host_ui,
        "/api/reply",
        &serde_json::to_vec(&empty_reply_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(empty_reply_code, 400);

    host.shutdown();
    peer.shutdown();
}

/// Tests that peers inspection (`GET /api/peers`) and routes caching inspection (`GET /api/routes`)
/// correctly report live vs trusted devices with online/offline state, expose physical route cache,
/// persist routes across session offline transitions, and clean up both trust and route cache upon unpair.
#[tokio::test(flavor = "multi_thread")]
async fn peers_and_routes_inspection_via_loopback_api() {
    let (host, peer, _g1, _g2) = pair_phone_desktop().await;
    let host_ui = host.spawn_ui().await.expect("host UI spawns");
    let peer_id = peer.identity().device_id().clone();
    let host_id = host.identity().device_id().clone();

    // 1. Initial live check: GET /api/peers on host
    let (code, body) = tetherly_node::loopback_get(host_ui, "/api/peers")
        .await
        .expect("get peers succeeds");
    assert_eq!(code, 200);
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["ok"], true);
    assert_eq!(v["self_id"], host_id.as_str());
    assert_eq!(v["live_count"], 1);
    assert_eq!(v["trusted_count"], 1);

    let live = v["live"].as_array().expect("live array");
    assert_eq!(live.len(), 1);
    assert_eq!(live[0]["device_id"], peer_id.as_str());
    assert_eq!(live[0]["online"], true);
    assert!(live[0].get("file_port").is_some());
    assert!(live[0].get("screen_port").is_some());
    assert!(live[0].get("input_port").is_some());
    assert_eq!(live[0]["path"], "lan");

    let trusted = v["trusted"].as_array().expect("trusted array");
    assert_eq!(trusted.len(), 1);
    assert_eq!(trusted[0]["device_id"], peer_id.as_str());
    assert_eq!(trusted[0]["online"], true);
    assert_eq!(trusted[0]["revoked"], false);

    // 2. Initial route check: GET /api/routes on host
    let (rcode, rbody) = tetherly_node::loopback_get(host_ui, "/api/routes")
        .await
        .expect("get routes succeeds");
    assert_eq!(rcode, 200);
    let rv: serde_json::Value = serde_json::from_slice(&rbody).unwrap();
    assert_eq!(rv["ok"], true);
    assert!(rv["count"].as_u64().unwrap_or(0) >= 1);
    let routes = rv["routes"].as_array().expect("routes array");
    assert!(routes.iter().any(|r| r["device_id"] == peer_id.as_str()));

    // 3. GET /api/state snapshot also exposes trusted inventory
    let (scode, sbody) = tetherly_node::loopback_get(host_ui, "/api/state")
        .await
        .expect("get state succeeds");
    assert_eq!(scode, 200);
    let sv: serde_json::Value = serde_json::from_slice(&sbody).unwrap();
    let snap_trusted = sv["trusted"].as_array().expect("trusted in snapshot");
    assert!(snap_trusted
        .iter()
        .any(|t| t["device_id"] == peer_id.as_str() && t["online"] == true));

    // 4. Simulate peer going offline
    peer.shutdown();

    let start = std::time::Instant::now();
    loop {
        let (pcode, pbody) = tetherly_node::loopback_get(host_ui, "/api/peers")
            .await
            .unwrap();
        assert_eq!(pcode, 200);
        let pv: serde_json::Value = serde_json::from_slice(&pbody).unwrap();
        if pv["live_count"] == 0 {
            // Live peer is severed, but trusted peer persists marked offline
            assert_eq!(pv["trusted_count"], 1);
            let t_arr = pv["trusted"].as_array().unwrap();
            assert_eq!(t_arr[0]["device_id"], peer_id.as_str());
            assert_eq!(t_arr[0]["online"], false);
            break;
        }
        if start.elapsed() > Duration::from_secs(10) {
            panic!("peer was not marked offline in /api/peers within 10s");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    // 5. Verify cached route persists after peer disconnected
    let (r2_code, r2_body) = tetherly_node::loopback_get(host_ui, "/api/routes")
        .await
        .unwrap();
    assert_eq!(r2_code, 200);
    let r2_v: serde_json::Value = serde_json::from_slice(&r2_body).unwrap();
    let r2_routes = r2_v["routes"].as_array().unwrap();
    assert!(r2_routes.iter().any(|r| r["device_id"] == peer_id.as_str()));

    // 6. Unpair offline peer via POST /api/peers/unpair
    let unpair_payload = serde_json::json!({
        "device_id": peer_id.as_str(),
    });
    let (ucode, ubody) = tetherly_node::loopback_post(
        host_ui,
        "/api/peers/unpair",
        &serde_json::to_vec(&unpair_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(ucode, 200);
    let uresp: serde_json::Value = serde_json::from_slice(&ubody).unwrap();
    assert_eq!(uresp["ok"], true);
    assert_eq!(uresp["unpaired"], true);

    // 7. After unpair, trusted inventory and routes are empty
    let (p3_code, p3_body) = tetherly_node::loopback_get(host_ui, "/api/peers")
        .await
        .unwrap();
    assert_eq!(p3_code, 200);
    let p3_v: serde_json::Value = serde_json::from_slice(&p3_body).unwrap();
    assert_eq!(p3_v["trusted_count"], 0);
    assert_eq!(p3_v["live_count"], 0);

    let (r3_code, r3_body) = tetherly_node::loopback_get(host_ui, "/api/routes")
        .await
        .unwrap();
    assert_eq!(r3_code, 200);
    let r3_v: serde_json::Value = serde_json::from_slice(&r3_body).unwrap();
    let r3_routes = r3_v["routes"].as_array().unwrap();
    assert!(!r3_routes.iter().any(|r| r["device_id"] == peer_id.as_str()));

    host.shutdown();
}

/// ADR-031: Node storage inspection and safe orphaned partial garbage collection.
#[tokio::test(flavor = "multi_thread")]
async fn storage_inspection_and_partial_cleanup_via_loopback_api() {
    let (data_dir, _guard) = temp_dir("storage-test");
    let node = Node::start(cfg(data_dir.clone(), "storage-node", "macos")).unwrap();
    let ui_port = node.spawn_ui().await.unwrap();
    assert_ne!(ui_port, 0, "test node must serve loopback UI");

    // 1. Initial state check: clean data_dir and inbox
    let (code, body) = tetherly_node::loopback_get(ui_port, "/api/storage")
        .await
        .expect("storage get");
    assert_eq!(code, 200);
    let mut rep: tetherly_node::StorageReport = serde_json::from_slice(&body).unwrap();
    assert!(rep.ok);
    assert_eq!(rep.inbox_file_count, 0);
    assert_eq!(rep.inbox_total_bytes, 0);
    assert_eq!(rep.partial_count, 0);
    assert_eq!(rep.orphaned_partial_count, 0);

    // 2. Populate inbox with:
    //    - A regular received file: "file1.txt" (50 bytes)
    //    - An orphaned partial transfer: ".tfl_orphaned1.part" (120 bytes)
    //    - A matching orphaned sidecar: ".tfl_orphaned1.resume.json" (40 bytes)
    //    - A lonely orphaned sidecar: ".tfl_orphaned2.resume.json" (30 bytes)
    let inbox_dir = data_dir.join("inbox");
    std::fs::create_dir_all(&inbox_dir).unwrap();
    std::fs::write(inbox_dir.join("file1.txt"), vec![b'A'; 50]).unwrap();
    std::fs::write(inbox_dir.join(".tfl_orphaned1.part"), vec![b'B'; 120]).unwrap();
    std::fs::write(inbox_dir.join(".tfl_orphaned1.resume.json"), vec![b'C'; 40]).unwrap();
    std::fs::write(inbox_dir.join(".tfl_orphaned2.resume.json"), vec![b'D'; 30]).unwrap();

    // 3. Re-query /api/storage to verify metrics
    let (code, body) = tetherly_node::loopback_get(ui_port, "/api/storage")
        .await
        .unwrap();
    assert_eq!(code, 200);
    rep = serde_json::from_slice(&body).unwrap();
    assert_eq!(rep.inbox_file_count, 1);
    assert_eq!(rep.inbox_total_bytes, 50);
    assert_eq!(rep.partial_count, 1);
    assert_eq!(rep.partial_bytes, 120);
    assert_eq!(rep.orphaned_partial_count, 1);
    assert_eq!(rep.orphaned_partial_bytes, 120);
    assert!(rep.meta_bytes >= 70); // 40 + 30 + any identity/trusted files

    // 4. Trigger orphaned-only cleanup via POST /api/storage/clean with {"orphaned_only": true}
    let payload = serde_json::json!({ "orphaned_only": true });
    let (code, body) = tetherly_node::loopback_post(
        ui_port,
        "/api/storage/clean",
        &serde_json::to_vec(&payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code, 200);
    let clean_rep: tetherly_node::StorageCleanReport = serde_json::from_slice(&body).unwrap();
    assert!(clean_rep.ok);
    // Deleted: .tfl_orphaned1.part (120) + .tfl_orphaned1.resume.json (40) + .tfl_orphaned2.resume.json (30) = 190 bytes, 3 files
    assert_eq!(clean_rep.freed_bytes, 190);
    assert_eq!(clean_rep.deleted_files_count, 3);

    // 5. Verify the regular file remains untouched
    assert!(inbox_dir.join("file1.txt").exists());
    assert_eq!(
        std::fs::read(inbox_dir.join("file1.txt")).unwrap().len(),
        50
    );

    // 6. Query /api/storage again to ensure partials are now zero
    let (code, body) = tetherly_node::loopback_get(ui_port, "/api/storage")
        .await
        .unwrap();
    assert_eq!(code, 200);
    rep = serde_json::from_slice(&body).unwrap();
    assert_eq!(rep.inbox_file_count, 1);
    assert_eq!(rep.inbox_total_bytes, 50);
    assert_eq!(rep.partial_count, 0);
    assert_eq!(rep.orphaned_partial_count, 0);

    node.shutdown();
}

/// ADR-032: Node health probe, latency ping, and graceful shutdown via loopback HTTP API.
#[tokio::test(flavor = "multi_thread")]
async fn node_health_probe_ping_and_graceful_shutdown_via_loopback_api() {
    let (data_dir, _guard) = temp_dir("health-test");
    let node = Node::start(cfg(data_dir.clone(), "health-node", "macos")).unwrap();
    let ui_port = node.spawn_ui().await.unwrap();
    assert_ne!(ui_port, 0, "test node must serve loopback UI");

    // 1. Direct Rust API inspection
    let direct_report = node.health_report();
    assert!(direct_report.ok);
    assert_eq!(direct_report.status, "healthy");
    assert_eq!(direct_report.name, "health-node");
    assert_eq!(direct_report.platform, "macos");
    assert_eq!(direct_report.pid, std::process::id());
    assert_eq!(direct_report.live_peers, 0);
    assert_eq!(direct_report.trusted_peers, 0);
    assert_eq!(direct_report.ports.ui, ui_port);

    // 2. Spawn node.run() in background task to verify graceful termination
    let run_node = node.clone_handle();
    let run_handle = tokio::spawn(async move { run_node.run().await });

    // Yield to let run() spawn its listener loops and subscribe to shutdown
    tokio::time::sleep(Duration::from_millis(100)).await;

    // 3. HTTP probe: GET /api/health with RTT measurement
    let start_t = std::time::Instant::now();
    let (code, body) = tetherly_node::loopback_get(ui_port, "/api/health")
        .await
        .expect("health probe get");
    let rtt = start_t.elapsed();
    assert_eq!(code, 200);
    assert!(
        rtt < Duration::from_millis(500),
        "health probe RTT too high: {rtt:?}"
    );

    let rep: tetherly_node::HealthReport =
        serde_json::from_slice(&body).expect("valid health report json");
    assert!(rep.ok);
    assert_eq!(rep.status, "healthy");
    assert_eq!(rep.version, env!("CARGO_PKG_VERSION"));
    assert_eq!(rep.name, "health-node");
    assert_eq!(rep.platform, "macos");
    assert_eq!(rep.pid, std::process::id());
    assert_eq!(rep.ports.ui, ui_port);
    assert_ne!(rep.ports.control, 0);

    // 4. Request graceful shutdown via POST /api/daemon/shutdown
    let (code, body) = tetherly_node::loopback_post(ui_port, "/api/daemon/shutdown", b"{}")
        .await
        .expect("shutdown post");
    assert_eq!(code, 200);
    let val: serde_json::Value = serde_json::from_slice(&body).expect("shutdown resp json");
    assert_eq!(val["ok"], true);
    assert_eq!(val["status"], "shutting_down");

    // 5. Verify that node.run() cleanly unwinds and exits with Ok(())
    let run_res = tokio::time::timeout(Duration::from_secs(3), run_handle)
        .await
        .expect("node run should shutdown within 3s")
        .expect("join handle ok");
    assert!(run_res.is_ok(), "node.run() should exit with Ok(())");

    // 6. Verify that UI listener is stopped
    tokio::time::sleep(Duration::from_millis(50)).await;
    let post_shutdown = tetherly_node::loopback_get(ui_port, "/api/health").await;
    assert!(
        post_shutdown.is_err(),
        "ui server should no longer accept connections"
    );
}

/// ADR-057: Daemon process resources, thread count, file descriptors and host system telemetry.
#[tokio::test(flavor = "multi_thread")]
async fn system_telemetry_metrics_and_probe_via_loopback_api() {
    let (data_dir, _guard) = temp_dir("sys-test");
    let node = Node::start(cfg(data_dir.clone(), "sys-node", "macos")).unwrap();
    let ui_port = node.spawn_ui().await.unwrap();
    assert_ne!(ui_port, 0, "test node must serve loopback UI");

    // 1. Direct Rust API inspection
    let rep = node.system_report().expect("system report");
    assert!(rep.ok);
    assert_eq!(rep.node_name, "sys-node");
    assert_eq!(rep.process.pid, std::process::id());
    assert!(rep.process.rss_bytes > 0, "RSS must be > 0");
    assert!(rep.process.thread_count >= 1, "thread count must be >= 1");
    assert!(rep.host.num_cpus >= 1, "CPU count must be >= 1");

    // 2. HTTP GET /api/sys
    let (code, body) = tetherly_node::loopback_get(ui_port, "/api/sys")
        .await
        .expect("sys get");
    assert_eq!(code, 200);
    let val: serde_json::Value = serde_json::from_slice(&body).expect("valid json");
    assert_eq!(val["ok"], true);
    assert_eq!(val["node_name"], "sys-node");
    assert_eq!(val["process"]["pid"], std::process::id());
    assert!(val["process"]["rss_bytes"].as_u64().unwrap_or(0) > 0);

    // 3. HTTP GET aliases: /api/system and /api/process
    let (code_system, _) = tetherly_node::loopback_get(ui_port, "/api/system")
        .await
        .expect("system get");
    assert_eq!(code_system, 200);

    let (code_proc, _) = tetherly_node::loopback_get(ui_port, "/api/process")
        .await
        .expect("process get");
    assert_eq!(code_proc, 200);

    // 4. Host header validation: non-loopback Host header must return 403 Forbidden
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], ui_port));
    let mut stream = tokio::net::TcpStream::connect(addr)
        .await
        .expect("connect to ui port");
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let evil_req = "GET /api/sys HTTP/1.1\r\nHost: evil.attacker.com\r\nConnection: close\r\n\r\n";
    stream
        .write_all(evil_req.as_bytes())
        .await
        .expect("write req");
    stream.flush().await.expect("flush");
    let mut resp_buf = Vec::new();
    stream.read_to_end(&mut resp_buf).await.expect("read resp");
    let resp_str = String::from_utf8_lossy(&resp_buf);
    assert!(
        resp_str.starts_with("HTTP/1.1 403"),
        "expected 403 Forbidden on invalid Host header, got: {resp_str}"
    );
}

/// ADR-033: Node real-time event streaming via loopback Server-Sent Events (SSE).
#[tokio::test(flavor = "multi_thread")]
async fn node_events_stream_sse_via_loopback_api() {
    let (data_dir, _guard) = temp_dir("sse-test");
    let node = Node::start(cfg(data_dir.clone(), "sse-node", "macos")).unwrap();
    let ui_port = node.spawn_ui().await.unwrap();
    assert_ne!(ui_port, 0, "test node must serve loopback UI");

    // 1. Verify invalid HTTP method on /api/events returns 405 Method Not Allowed
    let (post_code, _) = tetherly_node::loopback_post(ui_port, "/api/events", b"{}")
        .await
        .expect("post events");
    assert_eq!(post_code, 405);

    // 2. Connect SSE event stream over loopback HTTP
    let mut sse = tetherly_node::loopback_events_connect(ui_port)
        .await
        .expect("connect loopback sse");

    // 3. Emit sample UiEvents from node event bus
    let ev1 = tetherly_node::UiEvent::Status {
        message: "node ready".to_string(),
    };
    let ev2 = tetherly_node::UiEvent::PeerUp {
        device_id: "tdev_peer_abc".to_string(),
        name: "remote-laptop".to_string(),
        platform: "macos".to_string(),
    };
    let ev3 = tetherly_node::UiEvent::FileOffered {
        transfer_id: "tfl_12345".to_string(),
        source: "tdev_peer_abc".to_string(),
        files: vec![tetherly_node::events::FileView {
            name: "test_report.pdf".to_string(),
            size: 4096,
        }],
    };
    let ev4 = tetherly_node::UiEvent::FileDone {
        transfer_id: "tfl_12345".to_string(),
    };

    node.emit(ev1.clone());
    node.emit(ev2.clone());
    node.emit(ev3.clone());
    node.emit(ev4.clone());

    // 4. Consume events via SseStream typed next_ui_event()
    let r1 = tokio::time::timeout(Duration::from_secs(10), sse.next_ui_event())
        .await
        .expect("timeout r1")
        .expect("read r1");
    assert_eq!(r1, Some(ev1));

    let r2 = tokio::time::timeout(Duration::from_secs(10), sse.next_ui_event())
        .await
        .expect("timeout r2")
        .expect("read r2");
    assert_eq!(r2, Some(ev2));

    let r3 = tokio::time::timeout(Duration::from_secs(10), sse.next_ui_event())
        .await
        .expect("timeout r3")
        .expect("read r3");
    assert_eq!(r3, Some(ev3));

    let r4 = tokio::time::timeout(Duration::from_secs(10), sse.next_ui_event())
        .await
        .expect("timeout r4")
        .expect("read r4");
    assert_eq!(r4, Some(ev4));

    // 5. Connect a second stream to test raw JSON next_event() and shutdown termination
    let mut sse2 = tetherly_node::loopback_events_connect(ui_port)
        .await
        .expect("connect second sse");

    node.emit(tetherly_node::UiEvent::Status {
        message: "stream2 check".to_string(),
    });

    let raw = tokio::time::timeout(Duration::from_secs(10), sse2.next_event())
        .await
        .expect("timeout sse2")
        .expect("read sse2")
        .expect("some raw event");
    let val: serde_json::Value = serde_json::from_str(&raw).expect("valid json");
    assert_eq!(val["kind"], "status");
    assert_eq!(val["message"], "stream2 check");

    // Both active streams receive the broadcasted event
    let raw1 = tokio::time::timeout(Duration::from_secs(10), sse.next_event())
        .await
        .expect("timeout sse")
        .expect("read sse")
        .expect("some raw event on sse1");
    assert_eq!(raw1, raw);

    // 6. Shutdown node and verify streams cleanly terminate with Ok(None)
    node.shutdown();
    let term1 = tokio::time::timeout(Duration::from_secs(10), sse.next_event())
        .await
        .expect("timeout term1")
        .expect("read term1");
    assert_eq!(term1, None);

    let term2 = tokio::time::timeout(Duration::from_secs(10), sse2.next_event())
        .await
        .expect("timeout term2")
        .expect("read term2");
    assert_eq!(term2, None);
}

/// ADR-034: Active peer round-trip latency (RTT) probing, keepalive telemetry, and live peers latency reporting.
#[tokio::test(flavor = "multi_thread")]
async fn active_peer_rtt_probing_and_latency_reporting_via_loopback_api() {
    let (host, peer, _g1, _g2) = pair_phone_desktop().await;
    let host_ui = host.spawn_ui().await.expect("host UI spawns");
    let peer_id = peer.identity().device_id().clone();

    // 1. Direct Rust API invocation: host pings peer
    let report = host
        .ping_peer(&peer_id)
        .await
        .expect("direct ping_peer succeeds");
    assert!(report.ok);
    assert_eq!(report.device_id, peer_id.as_str());
    assert_eq!(report.path, "lan");
    assert!(
        report.rtt_ms < 500,
        "LAN ping RTT should be fast: {}ms",
        report.rtt_ms
    );

    // 2. Direct Rust API error cases
    let bogus_dev = tetherly_core::DeviceId::parse("tdev_aaaaaaaaaaaaaaaa").unwrap();
    let err_res = host.ping_peer(&bogus_dev).await;
    assert!(
        matches!(
            err_res,
            Err(tetherly_core::error::CoreError::NoConnectedPeer)
        ),
        "pinging disconnected peer should return NoConnectedPeer"
    );

    // 3. HTTP Loopback API: POST /api/peers/ping with specific peer
    let ping_req = serde_json::json!({
        "peer": peer_id.as_str(),
    });
    let (code, body) = tetherly_node::loopback_post(
        host_ui,
        "/api/peers/ping",
        &serde_json::to_vec(&ping_req).unwrap(),
    )
    .await
    .expect("ping post succeeds");
    assert_eq!(code, 200);
    let ping_val: tetherly_node::PeerPingReport =
        serde_json::from_slice(&body).expect("valid ping report");
    assert!(ping_val.ok);
    assert_eq!(ping_val.device_id, peer_id.as_str());
    assert_eq!(ping_val.path, "lan");

    // 4. HTTP Loopback API: POST /api/peers/ping with empty body (auto-resolves single connected peer)
    let (code_auto, body_auto) = tetherly_node::loopback_post(host_ui, "/api/peers/ping", b"{}")
        .await
        .expect("auto ping succeeds");
    assert_eq!(code_auto, 200);
    let auto_val: tetherly_node::PeerPingReport =
        serde_json::from_slice(&body_auto).expect("valid auto ping report");
    assert!(auto_val.ok);
    assert_eq!(auto_val.device_id, peer_id.as_str());

    // 5. HTTP Loopback API error handling: invalid peer id format
    let bad_req = serde_json::json!({
        "peer": "not_a_valid_device_id",
    });
    let (code_bad, body_bad) = tetherly_node::loopback_post(
        host_ui,
        "/api/peers/ping",
        &serde_json::to_vec(&bad_req).unwrap(),
    )
    .await
    .expect("bad ping post");
    assert_eq!(code_bad, 400);
    assert!(String::from_utf8_lossy(&body_bad).contains("bad peer id"));

    // 6. Verify live peer RTT and last_seen_ms in GET /api/peers
    let (p_code, p_body) = tetherly_node::loopback_get(host_ui, "/api/peers")
        .await
        .expect("get peers succeeds");
    assert_eq!(p_code, 200);
    let p_val: serde_json::Value = serde_json::from_slice(&p_body).unwrap();
    let live_peers = p_val["live"].as_array().expect("live peers array");
    assert_eq!(live_peers.len(), 1);
    let peer_entry = &live_peers[0];
    assert_eq!(peer_entry["device_id"], peer_id.as_str());
    assert!(
        peer_entry["rtt_ms"].as_u64().is_some(),
        "live peer should expose measured rtt_ms"
    );
    assert!(
        peer_entry["last_seen_ms"].as_u64().unwrap_or(0) > 0,
        "last_seen_ms should be updated"
    );

    // 7. Verify live peer RTT in GET /api/state snapshot
    let (s_code, s_body) = tetherly_node::loopback_get(host_ui, "/api/state")
        .await
        .expect("get state succeeds");
    assert_eq!(s_code, 200);
    let s_val: serde_json::Value = serde_json::from_slice(&s_body).unwrap();
    let snap_peers = s_val["peers"].as_array().expect("snapshot peers array");
    assert_eq!(snap_peers.len(), 1);
    assert_eq!(snap_peers[0]["device_id"], peer_id.as_str());
    assert!(snap_peers[0]["rtt_ms"].as_u64().is_some());

    // 8. Verify SSE event stream receives UiEvent::PeerLatency
    let mut sse = tetherly_node::loopback_events_connect(host_ui)
        .await
        .expect("connect loopback sse");

    // Trigger ping probe to emit PeerLatency event
    host.ping_peer(&peer_id).await.unwrap();

    let sse_event = tokio::time::timeout(Duration::from_secs(3), sse.next_ui_event())
        .await
        .expect("timeout waiting for sse peer latency event")
        .expect("read sse event")
        .expect("some ui event");

    match sse_event {
        tetherly_node::UiEvent::PeerLatency {
            device_id,
            rtt_ms: _,
        } => {
            assert_eq!(device_id, peer_id.as_str());
        }
        other => panic!("expected PeerLatency event, got: {other:?}"),
    }

    host.shutdown();
    peer.shutdown();
}

/// ADR-035: Lightweight file transfers query, single transfer inspection, cumulative metrics, and history lifecycle.
#[tokio::test(flavor = "multi_thread")]
async fn file_transfers_inspection_and_query_via_loopback_api() {
    let (host, peer, _g1, g2) = pair_phone_desktop().await;
    let ui_port = host.spawn_ui().await.expect("host UI spawns");

    // 1. Initial transfers report: empty list and zero metrics
    let (code, body) = tetherly_node::loopback_get(ui_port, "/api/file/transfers")
        .await
        .expect("get transfers");
    assert_eq!(code, 200);
    let mut rep: tetherly_node::TransfersReport = serde_json::from_slice(&body).unwrap();
    assert!(rep.ok);
    assert_eq!(rep.active_count, 0);
    assert_eq!(rep.terminal_count, 0);
    assert_eq!(rep.total_count, 0);
    assert!(rep.transfers.is_empty());
    assert_eq!(rep.stats.completed_count, 0);
    assert_eq!(rep.stats.failed_count, 0);

    // 2. Query non-existent transfer returns 404
    let (code_404, _) =
        tetherly_node::loopback_get(ui_port, "/api/file/transfers?id=tfl_nonexistent")
            .await
            .expect("get non-existent transfer");
    assert_eq!(code_404, 404);

    // 3. Create a test file and send it from host to peer
    let (send_dir, _g_send) = temp_dir("transfers-query-test");
    let file_path = send_dir.join("query_sample.dat");
    let total_size = tetherly_core::FILE_CHUNK + 2048;
    let file_content: Vec<u8> = (0..total_size).map(|i| (i % 251) as u8).collect();
    std::fs::write(&file_path, &file_content).unwrap();

    let send_payload = serde_json::json!({
        "path": file_path.to_str().unwrap(),
        "name": "query_sample.dat",
    });
    let (send_code, send_body) = tetherly_node::loopback_post(
        ui_port,
        "/api/file/send",
        &serde_json::to_vec(&send_payload).unwrap(),
    )
    .await
    .expect("send file post");
    assert_eq!(send_code, 200);
    let send_resp: serde_json::Value = serde_json::from_slice(&send_body).unwrap();
    let transfer_id = send_resp["transfer_id"].as_str().unwrap().to_string();

    // 4. Query single transfer by ID before peer accepts
    let (code_id, body_id) =
        tetherly_node::loopback_get(ui_port, &format!("/api/file/transfers?id={transfer_id}"))
            .await
            .expect("get single transfer by id");
    assert_eq!(code_id, 200);
    let single_tf: tetherly_node::ActiveTransfer = serde_json::from_slice(&body_id).unwrap();
    assert_eq!(single_tf.transfer_id, transfer_id);
    assert_eq!(single_tf.filename, "query_sample.dat");
    assert_eq!(single_tf.total, total_size as u64);
    assert_eq!(
        single_tf.direction,
        tetherly_node::TransferDirection::Outbound
    );

    // 5. Query with alias endpoint /api/transfers and active filter
    let (code_alias, body_alias) =
        tetherly_node::loopback_get(ui_port, "/api/transfers?active=true")
            .await
            .expect("get alias transfers active=true");
    assert_eq!(code_alias, 200);
    let alias_rep: tetherly_node::TransfersReport = serde_json::from_slice(&body_alias).unwrap();
    assert_eq!(alias_rep.active_count, 1);
    assert_eq!(alias_rep.transfers.len(), 1);
    assert_eq!(alias_rep.transfers[0].transfer_id, transfer_id);

    // 6. Query with terminal filter (should be empty initially)
    let (code_term, body_term) =
        tetherly_node::loopback_get(ui_port, "/api/file/transfers?terminal=true")
            .await
            .expect("get terminal transfers");
    assert_eq!(code_term, 200);
    let term_rep: tetherly_node::TransfersReport = serde_json::from_slice(&body_term).unwrap();
    assert_eq!(term_rep.transfers.len(), 0);

    // 7. Peer accepts file and completes transmission
    wait_offered(&peer, Duration::from_secs(10)).await;
    peer.user_accept_file(&transfer_id).await.unwrap();

    let received = wait_inbox(g2.path(), "query_sample.dat", Duration::from_secs(25)).await;
    assert_eq!(received.len(), total_size);

    // 8. Re-query /api/file/transfers after completion:
    //    - active_count should be 0
    //    - terminal_count should be 1
    //    - stats.completed_count should be 1
    //    - stats.bytes_sent should reflect total_size
    let (code_done, body_done) = tetherly_node::loopback_get(ui_port, "/api/file/transfers")
        .await
        .expect("get completed transfers");
    assert_eq!(code_done, 200);
    rep = serde_json::from_slice(&body_done).unwrap();
    assert_eq!(rep.active_count, 0);
    assert_eq!(rep.terminal_count, 1);
    assert_eq!(rep.total_count, 1);
    assert_eq!(
        rep.transfers[0].phase,
        tetherly_node::TransferPhase::Completed
    );
    assert_eq!(rep.stats.completed_count, 1);
    assert!(rep.stats.bytes_sent >= total_size as u64);

    // 9. Query with direction filter: direction=inbound should yield 0 for host, direction=outbound yields 1
    let (code_in, body_in) =
        tetherly_node::loopback_get(ui_port, "/api/file/transfers?direction=inbound")
            .await
            .expect("query direction inbound");
    assert_eq!(code_in, 200);
    let in_rep: tetherly_node::TransfersReport = serde_json::from_slice(&body_in).unwrap();
    assert_eq!(in_rep.transfers.len(), 0);

    let (code_out, body_out) =
        tetherly_node::loopback_get(ui_port, "/api/file/transfers?direction=outbound")
            .await
            .expect("query direction outbound");
    assert_eq!(code_out, 200);
    let out_rep: tetherly_node::TransfersReport = serde_json::from_slice(&body_out).unwrap();
    assert_eq!(out_rep.transfers.len(), 1);

    // 10. POST /api/file/clear to purge completed transfer history
    let (clear_code, clear_body) = tetherly_node::loopback_post(ui_port, "/api/file/clear", b"{}")
        .await
        .expect("clear completed transfers");
    assert_eq!(clear_code, 200);
    let clear_val: serde_json::Value = serde_json::from_slice(&clear_body).unwrap();
    assert_eq!(clear_val["ok"], true);
    assert_eq!(clear_val["cleared"], 1);

    // 11. Final transfers report check: transfers list empty, but cumulative stats preserved
    let (code_final, body_final) = tetherly_node::loopback_get(ui_port, "/api/file/transfers")
        .await
        .expect("get final transfers");
    assert_eq!(code_final, 200);
    let final_rep: tetherly_node::TransfersReport = serde_json::from_slice(&body_final).unwrap();
    assert_eq!(final_rep.total_count, 0);
    assert!(final_rep.transfers.is_empty());
    assert_eq!(final_rep.stats.completed_count, 1);
    assert!(final_rep.stats.bytes_sent >= total_size as u64);

    host.shutdown();
    peer.shutdown();
}

/// ADR-036: Pairing control plane, PIN security lifecycle, query inspection, and explicit revocation.
#[tokio::test(flavor = "multi_thread")]
async fn pin_lifecycle_inspection_and_revocation_via_loopback_api() {
    let (data_dir, _guard) = temp_dir("pin-lifecycle-test");
    let node = Node::start(cfg(data_dir, "pin-node", "macos")).unwrap();
    let ui_port = node.spawn_ui().await.expect("spawn UI succeeds");

    // 1. Initial PIN inspection: inactive, no digits, 0 remaining seconds
    let (code_init, body_init) = tetherly_node::loopback_get(ui_port, "/api/pin")
        .await
        .expect("initial pin query succeeds");
    assert_eq!(code_init, 200);
    let init_rep: tetherly_node::PinReport = serde_json::from_slice(&body_init).unwrap();
    assert!(init_rep.ok);
    assert!(!init_rep.active);
    assert!(init_rep.pin.is_none());
    assert_eq!(init_rep.expires_in_secs, 0);
    assert_eq!(init_rep.ttl_secs, 180);

    // 2. Connect to loopback SSE event stream
    let mut sse = tetherly_node::loopback_events_connect(ui_port)
        .await
        .expect("connect loopback sse");

    // 3. Arm a new PIN via POST /api/pin
    let (gen_code, gen_body) = tetherly_node::loopback_post(ui_port, "/api/pin", b"{}")
        .await
        .expect("generate pin succeeds");
    assert_eq!(gen_code, 200);
    let gen_val: serde_json::Value = serde_json::from_slice(&gen_body).unwrap();
    assert_eq!(gen_val["ok"], true);
    let pin_str = gen_val["pin"].as_str().expect("pin string");
    assert_eq!(pin_str.len(), 9); // e.g. "2517 0394"
    assert!(gen_val["expires_in_secs"].as_u64().unwrap_or(0) > 0);
    assert_eq!(gen_val["ttl_secs"].as_u64().unwrap_or(0), 180);

    // 4. Verify SSE stream receives UiEvent::PinStateChanged { active: true }
    let sse_event1 = tokio::time::timeout(Duration::from_secs(10), sse.next_ui_event())
        .await
        .expect("timeout waiting for sse pin armed event")
        .expect("read sse event")
        .expect("some ui event");
    match sse_event1 {
        tetherly_node::UiEvent::PinStateChanged { active } => {
            assert!(active);
        }
        other => panic!("expected PinStateChanged active=true, got: {other:?}"),
    }

    // 5. Query /api/pin after generation: active, matching pin, valid TTL
    let (code_act, body_act) = tetherly_node::loopback_get(ui_port, "/api/pin")
        .await
        .expect("active pin query succeeds");
    assert_eq!(code_act, 200);
    let act_rep: tetherly_node::PinReport = serde_json::from_slice(&body_act).unwrap();
    assert!(act_rep.ok);
    assert!(act_rep.active);
    assert_eq!(act_rep.pin.as_deref(), Some(pin_str));
    assert!(act_rep.expires_in_secs > 0);
    assert_eq!(act_rep.ttl_secs, 180);

    // 6. Explicitly revoke and clear PIN via POST /api/pin/clear
    let (clear_code, clear_body) = tetherly_node::loopback_post(ui_port, "/api/pin/clear", b"{}")
        .await
        .expect("clear pin succeeds");
    assert_eq!(clear_code, 200);
    let clear_val: serde_json::Value = serde_json::from_slice(&clear_body).unwrap();
    assert_eq!(clear_val["ok"], true);
    assert_eq!(clear_val["cleared"], true);

    // 7. Verify SSE stream receives UiEvent::PinStateChanged { active: false }
    let mut saw_pin_cleared = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while tokio::time::Instant::now() < deadline {
        if let Ok(Ok(Some(tetherly_node::UiEvent::PinStateChanged { active }))) =
            tokio::time::timeout(Duration::from_millis(500), sse.next_ui_event()).await
        {
            assert!(!active);
            saw_pin_cleared = true;
            break;
        }
    }
    assert!(saw_pin_cleared, "expected PinStateChanged active=false");

    // 8. Query /api/pin after revocation: inactive, no digits, 0 remaining seconds
    let (code_cleared, body_cleared) = tetherly_node::loopback_get(ui_port, "/api/pin")
        .await
        .expect("cleared pin query succeeds");
    assert_eq!(code_cleared, 200);
    let cleared_rep: tetherly_node::PinReport = serde_json::from_slice(&body_cleared).unwrap();
    assert!(cleared_rep.ok);
    assert!(!cleared_rep.active);
    assert!(cleared_rep.pin.is_none());
    assert_eq!(cleared_rep.expires_in_secs, 0);

    // 9. Idempotent clear: calling clear again returns cleared: false
    let (c2_code, c2_body) = tetherly_node::loopback_post(ui_port, "/api/pin/clear", b"{}")
        .await
        .expect("second clear succeeds");
    assert_eq!(c2_code, 200);
    let c2_val: serde_json::Value = serde_json::from_slice(&c2_body).unwrap();
    assert_eq!(c2_val["ok"], true);
    assert_eq!(c2_val["cleared"], false);

    node.shutdown();
}

/// ADR-037: Peer alias governance, friendly name persistence, and smart peer resolution across loopback APIs.
#[tokio::test(flavor = "multi_thread")]
async fn peer_alias_governance_and_smart_resolution_via_loopback_api() {
    let (host, peer, g1, _g2) = pair_phone_desktop().await;
    let host_ui = host.spawn_ui().await.expect("host UI spawns");
    let peer_id = peer.identity().device_id().clone();

    // 1. Initial peers inspection: peer has initial alias (advertised name "laptop")
    let (code_init, body_init) = tetherly_node::loopback_get(host_ui, "/api/peers")
        .await
        .expect("initial peers query succeeds");
    assert_eq!(code_init, 200);
    let init_v: serde_json::Value = serde_json::from_slice(&body_init).unwrap();
    assert_eq!(init_v["live_count"], 1);
    assert_eq!(init_v["trusted_count"], 1);
    let init_trusted = &init_v["trusted"][0];
    assert_eq!(init_trusted["device_id"], peer_id.as_str());
    assert_eq!(init_trusted["alias"], peer_id.as_str());

    // 2. Connect to host loopback SSE stream to verify real-time UiEvent::PeerAliasChanged
    let mut sse = tetherly_node::loopback_events_connect(host_ui)
        .await
        .expect("connect loopback sse");

    // 3. Update peer alias via POST /api/peers/alias using cryptographic device ID
    let new_alias = "MacBook Workstation";
    let alias_payload = serde_json::json!({
        "peer": peer_id.as_str(),
        "alias": new_alias,
    });
    let (code_set, body_set) = tetherly_node::loopback_post(
        host_ui,
        "/api/peers/alias",
        &serde_json::to_vec(&alias_payload).unwrap(),
    )
    .await
    .expect("set alias post succeeds");
    assert_eq!(code_set, 200);
    let set_v: serde_json::Value = serde_json::from_slice(&body_set).unwrap();
    assert_eq!(set_v["ok"], true);
    assert_eq!(set_v["device_id"], peer_id.as_str());
    assert_eq!(set_v["alias"], new_alias);

    // 4. Verify SSE stream receives UiEvent::PeerAliasChanged { device_id, alias }
    let sse_event = tokio::time::timeout(Duration::from_secs(10), sse.next_ui_event())
        .await
        .expect("timeout waiting for sse alias event")
        .expect("read sse event")
        .expect("some ui event");
    match sse_event {
        tetherly_node::UiEvent::PeerAliasChanged { device_id, alias } => {
            assert_eq!(device_id, peer_id.as_str());
            assert_eq!(alias, new_alias);
        }
        other => panic!("expected PeerAliasChanged, got: {other:?}"),
    }

    // 5. Verify alias persistence in runtime memory and on disk (trusted.json)
    let trusted_now = host.trusted();
    assert_eq!(trusted_now.len(), 1);
    assert_eq!(trusted_now[0].alias, new_alias);

    let loaded_store = tetherly_node::store::load_trust(g1.path()).unwrap();
    let reloaded_peer = loaded_store.get(&peer_id).expect("peer present on disk");
    assert_eq!(reloaded_peer.alias, new_alias);

    // 6. Verify GET /api/peers exposes updated alias in both live and trusted views
    let (code_p, body_p) = tetherly_node::loopback_get(host_ui, "/api/peers")
        .await
        .unwrap();
    assert_eq!(code_p, 200);
    let pv: serde_json::Value = serde_json::from_slice(&body_p).unwrap();
    assert_eq!(pv["live"][0]["alias"], new_alias);
    assert_eq!(pv["trusted"][0]["alias"], new_alias);

    // 7. Smart Peer Resolution: ping using exact friendly alias
    let ping_alias_payload = serde_json::json!({
        "peer": "MacBook Workstation",
    });
    let (code_ping, body_ping) = tetherly_node::loopback_post(
        host_ui,
        "/api/peers/ping",
        &serde_json::to_vec(&ping_alias_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_ping, 200);
    let ping_rep: tetherly_node::PeerPingReport = serde_json::from_slice(&body_ping).unwrap();
    assert_eq!(ping_rep.device_id, peer_id.as_str());

    // 8. Smart Peer Resolution: ping using substring / case-insensitive match
    let ping_sub_payload = serde_json::json!({
        "peer": "workstation",
    });
    let (code_sub, body_sub) = tetherly_node::loopback_post(
        host_ui,
        "/api/peers/ping",
        &serde_json::to_vec(&ping_sub_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_sub, 200);
    let sub_rep: tetherly_node::PeerPingReport = serde_json::from_slice(&body_sub).unwrap();
    assert_eq!(sub_rep.device_id, peer_id.as_str());

    // 9. Smart Peer Resolution: targeted clipboard sync by alias
    let clip_payload = serde_json::json!({
        "text": "sync to workstation alias",
        "peer": "MacBook Workstation",
    });
    let (code_clip, body_clip) = tetherly_node::loopback_post(
        host_ui,
        "/api/clip/sync",
        &serde_json::to_vec(&clip_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_clip, 200);
    let clip_v: serde_json::Value = serde_json::from_slice(&body_clip).unwrap();
    assert_eq!(clip_v["ok"], true);
    assert_eq!(clip_v["sent_peers"], 1);

    // 10. Smart Peer Resolution: update alias by referring to existing alias
    let alias2_payload = serde_json::json!({
        "peer": "workstation",
        "alias": "Studio Desktop",
    });
    let (code_set2, body_set2) = tetherly_node::loopback_post(
        host_ui,
        "/api/peers/alias",
        &serde_json::to_vec(&alias2_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_set2, 200);
    let set2_v: serde_json::Value = serde_json::from_slice(&body_set2).unwrap();
    assert_eq!(set2_v["alias"], "Studio Desktop");
    assert_eq!(host.trusted()[0].alias, "Studio Desktop");

    // 11. Validation and negative error paths
    // a. Empty alias returns 400
    let empty_payload = serde_json::json!({
        "peer": peer_id.as_str(),
        "alias": "   ",
    });
    let (code_empty, _) = tetherly_node::loopback_post(
        host_ui,
        "/api/peers/alias",
        &serde_json::to_vec(&empty_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_empty, 400);

    // b. Oversized alias (>64 bytes) returns 400
    let oversized_payload = serde_json::json!({
        "peer": peer_id.as_str(),
        "alias": "x".repeat(65),
    });
    let (code_over, _) = tetherly_node::loopback_post(
        host_ui,
        "/api/peers/alias",
        &serde_json::to_vec(&oversized_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_over, 400);

    // c. Control characters in alias return 400
    let ctrl_payload = serde_json::json!({
        "peer": peer_id.as_str(),
        "alias": "bad\nalias",
    });
    let (code_ctrl, _) = tetherly_node::loopback_post(
        host_ui,
        "/api/peers/alias",
        &serde_json::to_vec(&ctrl_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_ctrl, 400);

    // d. Non-existent peer query returns 404
    let ghost_payload = serde_json::json!({
        "peer": "non_existent_peer_device",
        "alias": "Ghost Alias",
    });
    let (code_ghost, _) = tetherly_node::loopback_post(
        host_ui,
        "/api/peers/alias",
        &serde_json::to_vec(&ghost_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_ghost, 404);

    // 12. Unpair using alias via smart peer resolution
    let unpair_payload = serde_json::json!({
        "device_id": "Studio Desktop",
    });
    let (code_unpair, body_unpair) = tetherly_node::loopback_post(
        host_ui,
        "/api/peers/unpair",
        &serde_json::to_vec(&unpair_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_unpair, 200);
    let unpair_v: serde_json::Value = serde_json::from_slice(&body_unpair).unwrap();
    assert_eq!(unpair_v["ok"], true);
    assert_eq!(unpair_v["device_id"], peer_id.as_str());
    assert_eq!(unpair_v["unpaired"], true);
    assert!(host.trusted().is_empty());

    host.shutdown();
    peer.shutdown();
}

/// ADR-038: Node configuration observability, dynamic runtime flags, and loopback configuration governance.
#[tokio::test(flavor = "multi_thread")]
async fn node_config_inspection_and_dynamic_flags_via_loopback_api() {
    let (data_dir, _guard) = temp_dir("config-node-test");
    let node = Node::start(cfg(data_dir, "config-host", "macos")).unwrap();
    let ui_port = node.spawn_ui().await.expect("spawn UI succeeds");

    // 1. Initial config inspection via GET /api/config
    let (code_init, body_init) = tetherly_node::loopback_get(ui_port, "/api/config")
        .await
        .expect("initial config query succeeds");
    assert_eq!(code_init, 200);
    let init_rep: tetherly_node::NodeConfigReport = serde_json::from_slice(&body_init).unwrap();
    assert!(init_rep.ok);
    assert_eq!(init_rep.device_id, node.identity().device_id().as_str());
    assert_eq!(init_rep.name, "config-host");
    assert_eq!(init_rep.platform, "macos");
    assert!(!init_rep.advertise);
    assert!(!init_rep.reconnect);
    assert!(init_rep.loopback_only);
    assert!(init_rep.memory_clip);
    assert!(init_rep.memory_notifier);
    assert!(!init_rep.overlay_enabled);
    assert!(!init_rep.ancs_attached);
    assert_eq!(init_rep.ui_port, ui_port);

    // 2. Connect to loopback SSE event stream
    let mut sse = tetherly_node::loopback_events_connect(ui_port)
        .await
        .expect("connect loopback sse");

    // 3. Update node friendly name dynamically via POST /api/config
    let rename_payload = serde_json::json!({
        "name": "workstation-pro",
    });
    let (code_ren, body_ren) = tetherly_node::loopback_post(
        ui_port,
        "/api/config",
        &serde_json::to_vec(&rename_payload).unwrap(),
    )
    .await
    .expect("rename post succeeds");
    assert_eq!(code_ren, 200);
    let ren_rep: tetherly_node::NodeConfigReport = serde_json::from_slice(&body_ren).unwrap();
    assert_eq!(ren_rep.name, "workstation-pro");
    assert_eq!(node.name(), "workstation-pro");

    // 4. Verify SSE stream receives UiEvent::ConfigChanged
    let sse_event = tokio::time::timeout(Duration::from_secs(10), sse.next_ui_event())
        .await
        .expect("timeout waiting for sse config event")
        .expect("read sse event")
        .expect("some ui event");
    match sse_event {
        tetherly_node::UiEvent::ConfigChanged {
            name,
            advertise,
            reconnect,
        } => {
            assert_eq!(name, "workstation-pro");
            assert!(!advertise);
            assert!(!reconnect);
        }
        other => panic!("expected ConfigChanged, got: {other:?}"),
    }

    // 5. Verify GET /api/health reflects dynamically updated name
    let (code_hlth, body_hlth) = tetherly_node::loopback_get(ui_port, "/api/health")
        .await
        .expect("health query succeeds");
    assert_eq!(code_hlth, 200);
    let hlth_rep: tetherly_node::HealthReport = serde_json::from_slice(&body_hlth).unwrap();
    assert_eq!(hlth_rep.name, "workstation-pro");

    // 6. Update dynamic flags (advertise & reconnect) via PUT /api/config
    let flags_payload = serde_json::json!({
        "advertise": true,
        "reconnect": true,
    });
    let (code_flags, body_flags) = tetherly_node::loopback_post(
        ui_port,
        "/api/config",
        &serde_json::to_vec(&flags_payload).unwrap(),
    )
    .await
    .expect("flags update succeeds");
    assert_eq!(code_flags, 200);
    let flags_rep: tetherly_node::NodeConfigReport = serde_json::from_slice(&body_flags).unwrap();
    assert!(flags_rep.advertise);
    assert!(flags_rep.reconnect);
    assert_eq!(flags_rep.name, "workstation-pro");
    assert!(node.is_advertise_enabled());
    assert!(node.is_reconnect_enabled());

    // 7. Verify SSE stream receives updated flags
    let sse_event2 = tokio::time::timeout(Duration::from_secs(10), sse.next_ui_event())
        .await
        .expect("timeout waiting for sse flags event")
        .expect("read sse event")
        .expect("some ui event");
    match sse_event2 {
        tetherly_node::UiEvent::ConfigChanged {
            name,
            advertise,
            reconnect,
        } => {
            assert_eq!(name, "workstation-pro");
            assert!(advertise);
            assert!(reconnect);
        }
        other => panic!("expected ConfigChanged, got: {other:?}"),
    }

    // 8. Partial update (toggle reconnect off only)
    let rec_payload = serde_json::json!({
        "reconnect": false,
    });
    let (code_rec, body_rec) = tetherly_node::loopback_post(
        ui_port,
        "/api/config",
        &serde_json::to_vec(&rec_payload).unwrap(),
    )
    .await
    .expect("reconnect update succeeds");
    assert_eq!(code_rec, 200);
    let rec_rep: tetherly_node::NodeConfigReport = serde_json::from_slice(&body_rec).unwrap();
    assert!(!rec_rep.reconnect);
    assert!(rec_rep.advertise);
    assert_eq!(rec_rep.name, "workstation-pro");
    assert!(!node.is_reconnect_enabled());
    assert!(node.is_advertise_enabled());

    // 9. Negative error paths and validation:
    // a. Empty body / no fields to update returns 400
    let empty_payload = serde_json::json!({});
    let (code_empty, body_empty) = tetherly_node::loopback_post(
        ui_port,
        "/api/config",
        &serde_json::to_vec(&empty_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_empty, 400);
    assert!(String::from_utf8_lossy(&body_empty).contains("no configuration fields to update"));

    // b. Empty name returns 400
    let empty_name_payload = serde_json::json!({
        "name": "   ",
    });
    let (code_empty_name, _) = tetherly_node::loopback_post(
        ui_port,
        "/api/config",
        &serde_json::to_vec(&empty_name_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_empty_name, 400);

    // c. Oversized name (>64 bytes) returns 400
    let over_name_payload = serde_json::json!({
        "name": "a".repeat(65),
    });
    let (code_over, _) = tetherly_node::loopback_post(
        ui_port,
        "/api/config",
        &serde_json::to_vec(&over_name_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_over, 400);

    // d. Control characters in name return 400
    let ctrl_name_payload = serde_json::json!({
        "name": "bad\nname",
    });
    let (code_ctrl, _) = tetherly_node::loopback_post(
        ui_port,
        "/api/config",
        &serde_json::to_vec(&ctrl_name_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_ctrl, 400);

    // e. Malformed JSON returns 400
    let (code_bad_json, _) =
        tetherly_node::loopback_post(ui_port, "/api/config", b"this is not json")
            .await
            .unwrap();
    assert_eq!(code_bad_json, 400);

    // 10. Clean shutdown
    node.shutdown();
}

/// ADR-039: Route table governance, physical addressing persistence, and static injection via loopback API.
#[tokio::test(flavor = "multi_thread")]
async fn route_table_governance_and_static_injection_via_loopback_api() {
    let (host, peer, g1, _g2) = pair_phone_desktop().await;
    let host_ui = host.spawn_ui().await.expect("host UI spawns");
    let peer_id = peer.identity().device_id().clone();

    // 1. Initial routes inspection via GET /api/routes:
    // Because pair_phone_desktop() established a session, peer's control addr was remembered.
    let (code_init, body_init) = tetherly_node::loopback_get(host_ui, "/api/routes")
        .await
        .expect("initial routes query succeeds");
    assert_eq!(code_init, 200);
    let init_rep: tetherly_node::RoutesReport = serde_json::from_slice(&body_init).unwrap();
    assert!(init_rep.ok);
    assert_eq!(init_rep.count, 1);
    assert_eq!(init_rep.routes[0].device_id, peer_id.as_str());
    assert!(init_rep.routes[0].online);
    assert!(init_rep.routes[0].trusted);

    // 2. Connect to loopback SSE event stream
    let mut sse = tetherly_node::loopback_events_connect(host_ui)
        .await
        .expect("connect loopback sse");

    // 3. Inject a new static route for a non-live device via POST /api/routes
    let static_device_id = "tdev_aaaaaaaaaaaaaaaa";
    let static_addr = "192.168.1.120:45717";
    let add_payload = serde_json::json!({
        "peer": static_device_id,
        "addr": static_addr,
    });
    let (code_add, body_add) = tetherly_node::loopback_post(
        host_ui,
        "/api/routes",
        &serde_json::to_vec(&add_payload).unwrap(),
    )
    .await
    .expect("add route post succeeds");
    assert_eq!(code_add, 200);
    let add_rep: tetherly_node::RouteMutateReport = serde_json::from_slice(&body_add).unwrap();
    assert!(add_rep.ok);
    assert_eq!(add_rep.device_id, static_device_id);
    assert_eq!(add_rep.addr.as_deref(), Some(static_addr));
    assert_eq!(add_rep.action, "added");

    // 4. Verify SSE stream receives UiEvent::RouteChanged { device_id, addr, action: "added" }
    let sse_event = tokio::time::timeout(Duration::from_secs(10), sse.next_ui_event())
        .await
        .expect("timeout waiting for sse route added event")
        .expect("read sse event")
        .expect("some ui event");
    match sse_event {
        tetherly_node::UiEvent::RouteChanged {
            device_id,
            addr,
            action,
        } => {
            assert_eq!(device_id, static_device_id);
            assert_eq!(addr, static_addr);
            assert_eq!(action, "added");
        }
        other => panic!("expected RouteChanged added, got: {other:?}"),
    }

    // 5. Verify persistence on disk and via GET /api/routes
    let disk_routes = tetherly_node::store::load_routes(g1.path());
    let found_disk = disk_routes
        .routes
        .iter()
        .find(|r| r.device_id == static_device_id);
    assert!(found_disk.is_some());
    assert_eq!(found_disk.unwrap().addr, static_addr);

    let (code_list, body_list) = tetherly_node::loopback_get(host_ui, "/api/routes")
        .await
        .expect("routes list succeeds");
    assert_eq!(code_list, 200);
    let list_rep: tetherly_node::RoutesReport = serde_json::from_slice(&body_list).unwrap();
    assert_eq!(list_rep.count, 2);
    let static_entry = list_rep
        .routes
        .iter()
        .find(|r| r.device_id == static_device_id)
        .expect("found static entry");
    assert_eq!(static_entry.addr, static_addr);
    assert!(!static_entry.online);
    assert!(!static_entry.trusted);

    // 6. Mutate/Update existing route address via PUT /api/routes
    let updated_addr = "192.168.1.121:45717";
    let update_payload = serde_json::json!({
        "peer": static_device_id,
        "addr": updated_addr,
    });
    let (code_up, body_up) = tetherly_node::loopback_post(
        host_ui,
        "/api/routes",
        &serde_json::to_vec(&update_payload).unwrap(),
    )
    .await
    .expect("update route succeeds");
    assert_eq!(code_up, 200);
    let up_rep: tetherly_node::RouteMutateReport = serde_json::from_slice(&body_up).unwrap();
    assert!(up_rep.ok);
    assert_eq!(up_rep.action, "updated");
    assert_eq!(up_rep.addr.as_deref(), Some(updated_addr));

    // 7. Verify SSE stream receives UiEvent::RouteChanged { action: "updated" }
    let mut saw_route_updated = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while tokio::time::Instant::now() < deadline {
        if let Ok(Ok(Some(tetherly_node::UiEvent::RouteChanged {
            device_id,
            addr,
            action,
        }))) = tokio::time::timeout(Duration::from_millis(500), sse.next_ui_event()).await
        {
            if device_id == static_device_id && action == "updated" {
                assert_eq!(addr, updated_addr);
                saw_route_updated = true;
                break;
            }
        }
    }
    assert!(saw_route_updated, "expected RouteChanged updated");

    // 8. Smart Peer Resolution: update route using peer's advertised name or alias
    let smart_addr = "10.144.144.99:45717";
    let smart_payload = serde_json::json!({
        "peer": "desk", // advertised name of desk peer in pair_phone_desktop
        "addr": smart_addr,
    });
    let (code_smart, body_smart) = tetherly_node::loopback_post(
        host_ui,
        "/api/routes",
        &serde_json::to_vec(&smart_payload).unwrap(),
    )
    .await
    .expect("smart route update succeeds");
    assert_eq!(code_smart, 200);
    let smart_rep: tetherly_node::RouteMutateReport = serde_json::from_slice(&body_smart).unwrap();
    assert_eq!(smart_rep.device_id, peer_id.as_str());
    assert_eq!(smart_rep.addr.as_deref(), Some(smart_addr));

    // Consume SSE event for smart route update
    let mut saw_smart_updated = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while tokio::time::Instant::now() < deadline {
        if let Ok(Ok(Some(tetherly_node::UiEvent::RouteChanged {
            device_id,
            addr,
            action,
        }))) = tokio::time::timeout(Duration::from_millis(500), sse.next_ui_event()).await
        {
            if device_id == peer_id.as_str() && action == "updated" {
                assert_eq!(addr, smart_addr);
                saw_smart_updated = true;
                break;
            }
        }
    }
    assert!(saw_smart_updated, "expected smart RouteChanged updated");

    // 9. Delete route via DELETE /api/routes?peer=... (using loopback_delete)
    let (code_del1, body_del1) =
        tetherly_node::loopback_delete(host_ui, &format!("/api/routes?peer={static_device_id}"))
            .await
            .expect("delete route query succeeds");
    assert_eq!(code_del1, 200);
    let del1_rep: tetherly_node::RouteMutateReport = serde_json::from_slice(&body_del1).unwrap();
    assert!(del1_rep.ok);
    assert_eq!(del1_rep.device_id, static_device_id);
    assert_eq!(del1_rep.action, "removed");
    assert!(del1_rep.addr.is_none());

    // Verify SSE receives removal event
    let mut saw_route_removed = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while tokio::time::Instant::now() < deadline {
        if let Ok(Ok(Some(tetherly_node::UiEvent::RouteChanged {
            device_id, action, ..
        }))) = tokio::time::timeout(Duration::from_millis(500), sse.next_ui_event()).await
        {
            if device_id == static_device_id && action == "removed" {
                saw_route_removed = true;
                break;
            }
        }
    }
    assert!(saw_route_removed, "expected RouteChanged removed");

    // 10. Delete route via POST /api/routes/delete with body and smart peer resolution
    let del2_payload = serde_json::json!({
        "peer": "desk",
    });
    let (code_del2, body_del2) = tetherly_node::loopback_post(
        host_ui,
        "/api/routes/delete",
        &serde_json::to_vec(&del2_payload).unwrap(),
    )
    .await
    .expect("delete via post succeeds");
    assert_eq!(code_del2, 200);
    let del2_rep: tetherly_node::RouteMutateReport = serde_json::from_slice(&body_del2).unwrap();
    assert!(del2_rep.ok);
    assert_eq!(del2_rep.device_id, peer_id.as_str());
    assert_eq!(del2_rep.action, "removed");

    // Verify disk table is now completely empty
    let empty_disk_routes = tetherly_node::store::load_routes(g1.path());
    assert!(empty_disk_routes.routes.is_empty());

    // 11. Negative and validation test cases:
    // a. Deleting non-existent route returns 404
    let (code_404, _) =
        tetherly_node::loopback_delete(host_ui, &format!("/api/routes?peer={static_device_id}"))
            .await
            .unwrap();
    assert_eq!(code_404, 404);

    // b. Invalid socket address returns 400
    let invalid_addr_payload = serde_json::json!({
        "peer": static_device_id,
        "addr": "not_an_ip_port",
    });
    let (code_inv, _) = tetherly_node::loopback_post(
        host_ui,
        "/api/routes",
        &serde_json::to_vec(&invalid_addr_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_inv, 400);

    // c. Port 0 returns 400
    let zero_port_payload = serde_json::json!({
        "peer": static_device_id,
        "addr": "127.0.0.1:0",
    });
    let (code_zero, _) = tetherly_node::loopback_post(
        host_ui,
        "/api/routes",
        &serde_json::to_vec(&zero_port_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_zero, 400);

    // d. Empty peer returns 400
    let empty_peer_payload = serde_json::json!({
        "peer": "   ",
        "addr": "192.168.1.1:45717",
    });
    let (code_empty, _) = tetherly_node::loopback_post(
        host_ui,
        "/api/routes",
        &serde_json::to_vec(&empty_peer_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_empty, 400);

    // e. Unknown peer specification returns 400
    let unk_peer_payload = serde_json::json!({
        "peer": "unknown_ghost_peer",
        "addr": "192.168.1.1:45717",
    });
    let (code_unk, _) = tetherly_node::loopback_post(
        host_ui,
        "/api/routes",
        &serde_json::to_vec(&unk_peer_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_unk, 400);

    // f. Malformed JSON returns 400
    let (code_bad_json, _) =
        tetherly_node::loopback_post(host_ui, "/api/routes", b"malformed json")
            .await
            .unwrap();
    assert_eq!(code_bad_json, 400);

    host.shutdown();
    peer.shutdown();
}

/// ADR-040: Unified network traffic & protocol telemetry observability and runtime resets.
#[tokio::test(flavor = "multi_thread")]
async fn traffic_metrics_inspection_and_telemetry_reporting_via_loopback_api() {
    let (host, peer, _g1, g2) = pair_phone_desktop().await;
    let host_ui = host.spawn_ui().await.expect("host UI spawns");
    let peer_id = peer.identity().device_id().clone();

    // 1. Initial status check via GET /api/metrics and GET /api/traffic
    let (code_m, body_m) = tetherly_node::loopback_get(host_ui, "/api/metrics")
        .await
        .expect("metrics query succeeds");
    assert_eq!(code_m, 200);
    let rep: tetherly_node::TrafficReport = serde_json::from_slice(&body_m).unwrap();
    assert!(rep.ok);
    // Baseline: sessions were established during pair_phone_desktop()
    assert!(rep.sessions_established >= 1);
    assert_eq!(rep.handshakes_failed, 0);

    // Verify GET /api/traffic returns isomorphic response
    let (code_t, body_t) = tetherly_node::loopback_get(host_ui, "/api/traffic")
        .await
        .expect("traffic query succeeds");
    assert_eq!(code_t, 200);
    let rep_t: tetherly_node::TrafficReport = serde_json::from_slice(&body_t).unwrap();
    assert_eq!(rep.total_tx_bytes, rep_t.total_tx_bytes);

    // 2. Connect to SSE event stream
    let mut sse = tetherly_node::loopback_events_connect(host_ui)
        .await
        .expect("connect loopback sse");

    // 3. Ping round-trip to generate control traffic and ping/pong protocol messages
    let ping_rep = host.ping_peer(&peer_id).await.expect("ping succeeds");
    assert!(ping_rep.ok);
    assert!(ping_rep.rtt_ms <= 5000);

    // Also trigger notify message
    let notify_payload = serde_json::json!({
        "app_name": "Slack",
        "title": "Ping",
        "body": "Test",
    });
    let (code_n, _) = tetherly_node::loopback_post(
        host_ui,
        "/api/notify",
        &serde_json::to_vec(&notify_payload).unwrap(),
    )
    .await
    .expect("post notify succeeds");
    assert_eq!(code_n, 200);

    // Give a brief moment for async send
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Check that control telemetry reflects ping/pong & notify
    let (code_m2, body_m2) = tetherly_node::loopback_get(host_ui, "/api/metrics")
        .await
        .expect("metrics query 2 succeeds");
    assert_eq!(code_m2, 200);
    let rep2: tetherly_node::TrafficReport = serde_json::from_slice(&body_m2).unwrap();
    assert!(rep2.messages.pings_sent >= 1);
    assert!(rep2.messages.notifies_sent >= 1);
    assert!(rep2.control.tx_bytes > 0);
    assert!(rep2.control.tx_frames > 0);
    assert!(rep2.total_tx_bytes >= rep2.control.tx_bytes);

    // 4. File transfer interaction: send a streaming file and check file telemetry
    let payload = b"Hello telemetry data block from integration test!";
    let (send_dir, _g_send) = temp_dir("telemetry-send");
    let file_path = send_dir.join("telemetry_src.dat");
    std::fs::write(&file_path, payload).unwrap();

    let send_payload = serde_json::json!({
        "path": file_path.to_str().unwrap(),
        "name": "telemetry_src.dat",
        "peer": peer_id.as_str(),
    });
    let (code_f, body_f) = tetherly_node::loopback_post(
        host_ui,
        "/api/file/send",
        &serde_json::to_vec(&send_payload).unwrap(),
    )
    .await
    .expect("send file post succeeds");
    assert_eq!(code_f, 200);
    let f_resp: serde_json::Value = serde_json::from_slice(&body_f).unwrap();
    let transfer_id = f_resp["transfer_id"].as_str().unwrap();

    // Wait for file arrival on peer and accept
    wait_offered(&peer, Duration::from_secs(5)).await;
    peer.user_accept_file(transfer_id).await.unwrap();

    let received = wait_inbox(g2.path(), "telemetry_src.dat", Duration::from_secs(10)).await;
    assert_eq!(received, payload);

    // Check that file telemetry recorded TX
    let (code_m3, body_m3) = tetherly_node::loopback_get(host_ui, "/api/metrics")
        .await
        .expect("metrics query 3 succeeds");
    assert_eq!(code_m3, 200);
    let rep3: tetherly_node::TrafficReport = serde_json::from_slice(&body_m3).unwrap();
    assert!(rep3.file.tx_bytes >= payload.len() as u64);
    assert!(rep3.messages.file_offers_sent >= 1);

    // 5. Reset metrics via POST /api/metrics/reset
    let (code_rst, body_rst) = tetherly_node::loopback_post(host_ui, "/api/metrics/reset", b"{}")
        .await
        .expect("reset metrics succeeds");
    assert_eq!(code_rst, 200);
    let rst_json: serde_json::Value = serde_json::from_slice(&body_rst).unwrap();
    assert_eq!(rst_json["ok"], true);
    assert_eq!(rst_json["reset"], true);

    // 6. Verify SSE event stream receives UiEvent::MetricsReset
    let mut found_reset = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_secs(2), sse.next_ui_event()).await {
            Ok(Ok(Some(tetherly_node::UiEvent::MetricsReset { timestamp_ms: _ }))) => {
                found_reset = true;
                break;
            }
            Ok(Ok(Some(_other))) => {}
            Ok(Ok(None)) => break,
            Ok(Err(_)) => break,
            Err(_) => {}
        }
    }
    assert!(
        found_reset,
        "expected to receive UiEvent::MetricsReset on SSE"
    );

    // 7. Verify metrics reset took effect via GET /api/metrics
    let (code_m4, body_m4) = tetherly_node::loopback_get(host_ui, "/api/metrics")
        .await
        .expect("metrics query 4 succeeds");
    assert_eq!(code_m4, 200);
    let rep4: tetherly_node::TrafficReport = serde_json::from_slice(&body_m4).unwrap();
    assert_eq!(rep4.total_tx_bytes, 0);
    assert_eq!(rep4.total_rx_bytes, 0);
    assert_eq!(rep4.control.tx_bytes, 0);
    assert_eq!(rep4.file.tx_bytes, 0);
    assert_eq!(rep4.messages.pings_sent, 0);
    assert_eq!(rep4.messages.notifies_sent, 0);
    assert_eq!(rep4.messages.file_offers_sent, 0);
    assert_eq!(rep4.sessions_established, 0);

    // Also test POST /api/traffic/reset endpoint
    let (code_rst2, body_rst2) = tetherly_node::loopback_post(host_ui, "/api/traffic/reset", b"")
        .await
        .expect("reset traffic succeeds");
    assert_eq!(code_rst2, 200);
    let rst_json2: serde_json::Value = serde_json::from_slice(&body_rst2).unwrap();
    assert_eq!(rst_json2["ok"], true);

    host.shutdown();
    peer.shutdown();
}

/// ADR-041: Peer capabilities introspection, multi-dimensional feature negotiation, and dynamic capability governance.
#[tokio::test(flavor = "multi_thread")]
async fn peer_capabilities_negotiation_and_introspection_via_loopback_api() {
    let (host, peer, _g1, _g2) = pair_phone_desktop().await;
    let host_ui = host.spawn_ui().await.expect("host UI spawns");
    let peer_id = peer.identity().device_id().clone();
    let host_id = host.identity().device_id().clone();

    // 1. Initial local capabilities inspection via GET /api/caps
    let (code_local, body_local) = tetherly_node::loopback_get(host_ui, "/api/caps")
        .await
        .expect("local caps query succeeds");
    assert_eq!(code_local, 200);
    let local_rep: tetherly_node::LocalCapsReport = serde_json::from_slice(&body_local).unwrap();
    assert!(local_rep.ok);
    assert_eq!(local_rep.device_id, host_id.as_str());
    for feat in ["notify", "clip", "file", "input", "screen"] {
        assert!(
            local_rep.features.contains(&feat.to_string()),
            "missing feature: {feat}"
        );
    }
    assert!(local_rep.caps.iter().any(|c| c.starts_with("fileport=")));
    assert!(local_rep.caps.iter().any(|c| c.starts_with("inputport=")));
    assert!(local_rep.caps.iter().any(|c| c.starts_with("screenport=")));

    // 2. Connect to host loopback SSE stream to verify UiEvent::PeerCapsChanged
    let mut sse = tetherly_node::loopback_events_connect(host_ui)
        .await
        .expect("connect loopback sse");

    // 3. Inspect peer capabilities via GET /api/peers/caps
    let (code_peers_caps, body_peers_caps) =
        tetherly_node::loopback_get(host_ui, "/api/peers/caps")
            .await
            .expect("peers caps query succeeds");
    assert_eq!(code_peers_caps, 200);
    let peers_rep: tetherly_node::PeersCapsReport =
        serde_json::from_slice(&body_peers_caps).unwrap();
    assert!(peers_rep.ok);
    assert_eq!(peers_rep.count, 1);
    let peer_caps = &peers_rep.peers[0];
    assert_eq!(peer_caps.device_id, peer_id.as_str());
    assert!(peer_caps.online);
    for feat in ["notify", "clip", "file", "input", "screen"] {
        assert!(
            peer_caps.features.contains(&feat.to_string()),
            "peer missing feature: {feat}"
        );
    }
    assert!(peer_caps.file_port.is_some());
    assert!(peer_caps.input_port.is_some());
    assert!(peer_caps.screen_port.is_some());

    // 4. Smart Peer Resolution on /api/peers/caps:
    // a. Query by exact device ID
    let (code_p1, body_p1) =
        tetherly_node::loopback_get(host_ui, &format!("/api/peers/caps?peer={peer_id}"))
            .await
            .expect("query peer caps by id succeeds");
    assert_eq!(code_p1, 200);
    let p1_rep: tetherly_node::PeersCapsReport = serde_json::from_slice(&body_p1).unwrap();
    assert_eq!(p1_rep.count, 1);
    assert_eq!(p1_rep.peers[0].device_id, peer_id.as_str());

    // b. Query by friendly name / alias ("desk")
    let (code_p2, body_p2) = tetherly_node::loopback_get(host_ui, "/api/peers/caps?peer=desk")
        .await
        .expect("query peer caps by name succeeds");
    assert_eq!(code_p2, 200);
    let p2_rep: tetherly_node::PeersCapsReport = serde_json::from_slice(&body_p2).unwrap();
    assert_eq!(p2_rep.count, 1);
    assert_eq!(p2_rep.peers[0].device_id, peer_id.as_str());

    // c. Query by non-existent peer returns 404
    let (code_p404, _) = tetherly_node::loopback_get(host_ui, "/api/peers/caps?peer=ghost_device")
        .await
        .expect("query ghost peer returns 404");
    assert_eq!(code_p404, 404);

    // 5. Verify GET /api/peers exposes caps & features on live peers
    let (code_peers, body_peers) = tetherly_node::loopback_get(host_ui, "/api/peers")
        .await
        .expect("get peers succeeds");
    assert_eq!(code_peers, 200);
    let peers_val: serde_json::Value = serde_json::from_slice(&body_peers).unwrap();
    let live_peer_val = &peers_val["live"][0];
    assert!(live_peer_val["caps"].is_array());
    assert!(live_peer_val["features"].is_array());
    let feats_arr: Vec<String> = serde_json::from_value(live_peer_val["features"].clone()).unwrap();
    assert!(feats_arr.contains(&"notify".to_string()));
    assert!(feats_arr.contains(&"screen".to_string()));

    // 6. Dynamic Local Capability Governance: disable "screen" and "input"
    let disable_payload = serde_json::json!({
        "disable": ["screen", "input"]
    });
    let (code_dis, body_dis) = tetherly_node::loopback_post(
        host_ui,
        "/api/caps",
        &serde_json::to_vec(&disable_payload).unwrap(),
    )
    .await
    .expect("disable caps post succeeds");
    assert_eq!(code_dis, 200);
    let dis_rep: tetherly_node::LocalCapsReport = serde_json::from_slice(&body_dis).unwrap();
    assert!(dis_rep.ok);
    assert!(!dis_rep.features.contains(&"screen".to_string()));
    assert!(!dis_rep.features.contains(&"input".to_string()));
    assert!(dis_rep.features.contains(&"notify".to_string()));
    assert!(dis_rep.features.contains(&"clip".to_string()));
    assert!(dis_rep.features.contains(&"file".to_string()));

    // 7. Verify host SSE receives UiEvent::PeerCapsChanged
    let mut found_caps_change = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_secs(2), sse.next_ui_event()).await {
            Ok(Ok(Some(tetherly_node::UiEvent::PeerCapsChanged {
                device_id,
                caps: _,
                features,
            }))) => {
                if device_id == host_id.as_str() {
                    assert!(!features.contains(&"screen".to_string()));
                    assert!(!features.contains(&"input".to_string()));
                    found_caps_change = true;
                    break;
                }
            }
            Ok(Ok(Some(_other))) => {}
            Ok(Ok(None)) => break,
            Ok(Err(_)) => break,
            Err(_) => {}
        }
    }
    assert!(
        found_caps_change,
        "expected to receive UiEvent::PeerCapsChanged on SSE"
    );

    // 8. Dynamic Local Capability Governance: re-enable "screen" via PUT /api/caps
    let enable_payload = serde_json::json!({
        "enable": ["screen"]
    });
    let (code_en, body_en) = tetherly_node::loopback_put(
        host_ui,
        "/api/caps",
        &serde_json::to_vec(&enable_payload).unwrap(),
    )
    .await
    .expect("enable caps put succeeds");
    assert_eq!(code_en, 200);
    let en_rep: tetherly_node::LocalCapsReport = serde_json::from_slice(&body_en).unwrap();
    assert!(en_rep.ok);
    assert!(en_rep.features.contains(&"screen".to_string()));
    assert!(!en_rep.features.contains(&"input".to_string()));

    // 9. Negative and validation test cases:
    // a. Empty body / no caps specified returns 400
    let empty_payload = serde_json::json!({});
    let (code_empty, body_empty) = tetherly_node::loopback_post(
        host_ui,
        "/api/caps",
        &serde_json::to_vec(&empty_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_empty, 400);
    assert!(String::from_utf8_lossy(&body_empty).contains("no capability changes specified"));

    // b. Invalid characters in capability token returns 400
    let invalid_token_payload = serde_json::json!({
        "enable": ["bad cap!"]
    });
    let (code_inv, _) = tetherly_node::loopback_post(
        host_ui,
        "/api/caps",
        &serde_json::to_vec(&invalid_token_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_inv, 400);

    // c. Token containing '=' returns 400
    let eq_token_payload = serde_json::json!({
        "enable": ["screenport=9999"]
    });
    let (code_eq, _) = tetherly_node::loopback_post(
        host_ui,
        "/api/caps",
        &serde_json::to_vec(&eq_token_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_eq, 400);

    // d. Oversized token (>32 bytes) returns 400
    let over_token_payload = serde_json::json!({
        "enable": ["a".repeat(33)]
    });
    let (code_over, _) = tetherly_node::loopback_post(
        host_ui,
        "/api/caps",
        &serde_json::to_vec(&over_token_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_over, 400);

    // e. Malformed JSON returns 400
    let (code_bad_json, _) = tetherly_node::loopback_post(host_ui, "/api/caps", b"not a json")
        .await
        .unwrap();
    assert_eq!(code_bad_json, 400);

    // 10. Clean shutdown
    host.shutdown();
    peer.shutdown();
}

/// ADR-042: Peer connection session governance, on-demand direct connect and graceful disconnect.
#[tokio::test(flavor = "multi_thread")]
async fn peer_active_connect_and_disconnect_lifecycle_via_loopback_api() {
    let (host, peer, _g1, _g2) = pair_phone_desktop().await;
    let host_ui = host.spawn_ui().await.expect("host UI spawns");
    let peer_id = peer.identity().device_id().clone();
    let peer_addr = peer.control_addr();

    // 1. Assign friendly alias to peer
    let alias_payload = serde_json::json!({
        "peer": peer_id.as_str(),
        "alias": "Workstation"
    });
    let (code_alias, _) = tetherly_node::loopback_post(
        host_ui,
        "/api/peers/alias",
        &serde_json::to_vec(&alias_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_alias, 200);

    // 2. Connect to SSE event stream
    let mut sse = tetherly_node::loopback_events_connect(host_ui)
        .await
        .expect("SSE connects");

    // 3. Verify initial online state:
    // a. GET /api/peers shows peer online
    let (code_peers, body_peers) = tetherly_node::loopback_get(host_ui, "/api/peers")
        .await
        .unwrap();
    assert_eq!(code_peers, 200);
    let peers_rep: serde_json::Value = serde_json::from_slice(&body_peers).unwrap();
    let live_peers = peers_rep["live"].as_array().unwrap();
    assert!(live_peers
        .iter()
        .any(|p| p["device_id"] == peer_id.as_str()));

    // b. POST /api/peers/connect when already live returns already_connected: true
    let conn_payload = serde_json::json!({ "peer": "Workstation" });
    let (code_c1, body_c1) = tetherly_node::loopback_post(
        host_ui,
        "/api/peers/connect",
        &serde_json::to_vec(&conn_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_c1, 200);
    let rep_c1: tetherly_node::PeerConnectReport = serde_json::from_slice(&body_c1).unwrap();
    assert!(rep_c1.ok);
    assert!(rep_c1.connected);
    assert!(rep_c1.already_connected);
    assert_eq!(rep_c1.device_id, peer_id.as_str());
    assert_eq!(rep_c1.alias.as_deref(), Some("Workstation"));

    // 4. Actively disconnect session:
    let disc_payload = serde_json::json!({ "peer": "Workstation" });
    let (code_d1, body_d1) = tetherly_node::loopback_post(
        host_ui,
        "/api/peers/disconnect",
        &serde_json::to_vec(&disc_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_d1, 200);
    let rep_d1: tetherly_node::PeerDisconnectReport = serde_json::from_slice(&body_d1).unwrap();
    assert!(rep_d1.ok);
    assert!(rep_d1.disconnected);
    assert!(!rep_d1.already_disconnected);
    assert_eq!(rep_d1.device_id, peer_id.as_str());

    // 5. Verify SSE emitted PeerDown:
    let mut saw_peer_down = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while tokio::time::Instant::now() < deadline {
        if let Ok(Ok(Some(UiEvent::PeerDown { device_id }))) =
            tokio::time::timeout(Duration::from_millis(500), sse.next_ui_event()).await
        {
            if device_id == peer_id.as_str() {
                saw_peer_down = true;
                break;
            }
        }
    }
    assert!(saw_peer_down, "must emit PeerDown on disconnect");

    // 6. Verify trust is maintained and status is offline:
    let (_, body_peers_off) = tetherly_node::loopback_get(host_ui, "/api/peers")
        .await
        .unwrap();
    let peers_off: serde_json::Value = serde_json::from_slice(&body_peers_off).unwrap();
    let live_arr = peers_off["live"].as_array().unwrap();
    assert!(!live_arr.iter().any(|p| p["device_id"] == peer_id.as_str()));
    let trusted_arr = peers_off["trusted"].as_array().unwrap();
    let trusted_match = trusted_arr
        .iter()
        .find(|t| t["device_id"] == peer_id.as_str())
        .expect("trusted entry must still exist");
    assert_eq!(trusted_match["online"], false);

    // 7. Disconnecting again is idempotent: returns already_disconnected: true
    let (code_d2, body_d2) = tetherly_node::loopback_post(
        host_ui,
        "/api/peers/disconnect",
        &serde_json::to_vec(&disc_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_d2, 200);
    let rep_d2: tetherly_node::PeerDisconnectReport = serde_json::from_slice(&body_d2).unwrap();
    assert!(rep_d2.ok);
    assert!(!rep_d2.disconnected);
    assert!(rep_d2.already_disconnected);

    // 8. Actively reconnect on-demand using remembered route:
    let (code_c2, body_c2) = tetherly_node::loopback_post(
        host_ui,
        "/api/peers/connect",
        &serde_json::to_vec(&conn_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_c2, 200);
    let rep_c2: tetherly_node::PeerConnectReport = serde_json::from_slice(&body_c2).unwrap();
    assert!(rep_c2.ok);
    assert!(rep_c2.connected);
    assert!(!rep_c2.already_connected);

    // Verify online again in GET /api/peers:
    tokio::time::sleep(Duration::from_millis(100)).await;
    let (_, body_peers_on) = tetherly_node::loopback_get(host_ui, "/api/peers")
        .await
        .unwrap();
    let peers_on: serde_json::Value = serde_json::from_slice(&body_peers_on).unwrap();
    let live_arr_on = peers_on["live"].as_array().unwrap();
    assert!(live_arr_on
        .iter()
        .any(|p| p["device_id"] == peer_id.as_str()));

    // 9. Disconnect and reconnect with explicit socket address:
    let (code_d3, _) = tetherly_node::loopback_post(
        host_ui,
        "/api/peers/disconnect",
        &serde_json::to_vec(&disc_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_d3, 200);

    let explicit_addr_payload = serde_json::json!({
        "peer": peer_id.as_str(),
        "addr": peer_addr.to_string(),
    });
    let (code_c3, body_c3) = tetherly_node::loopback_post(
        host_ui,
        "/api/peers/connect",
        &serde_json::to_vec(&explicit_addr_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_c3, 200);
    let rep_c3: tetherly_node::PeerConnectReport = serde_json::from_slice(&body_c3).unwrap();
    assert!(rep_c3.ok);
    assert!(rep_c3.connected);
    assert_eq!(rep_c3.addr.as_deref(), Some(peer_addr.to_string().as_str()));

    // 10. Negative validation & edge cases:
    // a. Empty peer query returns 400
    let empty_payload = serde_json::json!({});
    let (code_err1, body_err1) = tetherly_node::loopback_post(
        host_ui,
        "/api/peers/connect",
        &serde_json::to_vec(&empty_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_err1, 400);
    assert!(String::from_utf8_lossy(&body_err1).contains("peer identifier required"));

    // b. Unknown peer query returns 404
    let unknown_payload = serde_json::json!({ "peer": "tdev_9999999999999999" });
    let (code_err2, _) = tetherly_node::loopback_post(
        host_ui,
        "/api/peers/connect",
        &serde_json::to_vec(&unknown_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_err2, 404);

    let (code_err3, _) = tetherly_node::loopback_post(
        host_ui,
        "/api/peers/disconnect",
        &serde_json::to_vec(&unknown_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_err3, 404);

    // c. Invalid socket address returns 400
    let invalid_addr_payload = serde_json::json!({
        "peer": "Workstation",
        "addr": "not-a-socket-addr"
    });
    let (code_err4, _) = tetherly_node::loopback_post(
        host_ui,
        "/api/peers/connect",
        &serde_json::to_vec(&invalid_addr_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_err4, 400);

    // d. Zero port socket address returns 400
    let zero_port_payload = serde_json::json!({
        "peer": "Workstation",
        "addr": "127.0.0.1:0"
    });
    let (code_err5, _) = tetherly_node::loopback_post(
        host_ui,
        "/api/peers/connect",
        &serde_json::to_vec(&zero_port_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_err5, 400);

    // 11. Clean shutdown
    host.shutdown();
    peer.shutdown();
}

/// ADR-043: Peer blacklist & access control governance via loopback API.
#[tokio::test(flavor = "multi_thread")]
async fn peer_blacklist_and_access_control_governance_via_loopback_api() {
    let (host, peer, g1, _g2) = pair_phone_desktop().await;
    let host_ui = host.spawn_ui().await.expect("host UI spawns");
    let peer_ui = peer.spawn_ui().await.expect("peer UI spawns");
    let peer_id = peer.identity().device_id().clone();
    let host_id = host.identity().device_id().clone();
    let peer_addr = peer.control_addr();
    let host_addr = host.control_addr();

    // 1. Connect to SSE event stream on host
    let mut sse = tetherly_node::loopback_events_connect(host_ui)
        .await
        .expect("SSE connects");

    // 2. Query initial blacklist: must be empty
    let (code_b0, body_b0) = tetherly_node::loopback_get(host_ui, "/api/peers/blocked")
        .await
        .unwrap();
    assert_eq!(code_b0, 200);
    let b0: tetherly_node::BlockedPeersReport = serde_json::from_slice(&body_b0).unwrap();
    assert!(b0.ok);
    assert_eq!(b0.count, 0);
    assert!(b0.peers.is_empty());

    // Verify initial live peer status: peer is connected
    let (code_live0, body_live0) = tetherly_node::loopback_get(host_ui, "/api/peers")
        .await
        .unwrap();
    assert_eq!(code_live0, 200);
    let live_val0: serde_json::Value = serde_json::from_slice(&body_live0).unwrap();
    let live_arr0 = live_val0["live"].as_array().unwrap();
    assert!(live_arr0.iter().any(|p| p["device_id"] == peer_id.as_str()));

    // 3. Block peer:
    let block_payload = serde_json::json!({
        "peer": peer_id.as_str(),
        "reason": "malicious port scanning",
    });
    let (code_blk, body_blk) = tetherly_node::loopback_post(
        host_ui,
        "/api/peers/block",
        &serde_json::to_vec(&block_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_blk, 200);
    let blk_rep: tetherly_node::PeerBlockReport = serde_json::from_slice(&body_blk).unwrap();
    assert!(blk_rep.ok);
    assert!(blk_rep.blocked);
    assert!(!blk_rep.already_blocked);
    assert!(blk_rep.disconnected_active);
    assert_eq!(blk_rep.device_id, peer_id.as_str());
    assert_eq!(blk_rep.reason.as_deref(), Some("malicious port scanning"));

    // 4. Verify live connection on host is instantly terminated
    let (code_live1, body_live1) = tetherly_node::loopback_get(host_ui, "/api/peers")
        .await
        .unwrap();
    assert_eq!(code_live1, 200);
    let live_val1: serde_json::Value = serde_json::from_slice(&body_live1).unwrap();
    let live_arr1 = live_val1["live"].as_array().unwrap();
    assert!(!live_arr1.iter().any(|p| p["device_id"] == peer_id.as_str()));

    // 5. Verify SSE received peer_blocked and peer_down
    let mut saw_peer_blocked = false;
    let mut saw_peer_down = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while tokio::time::Instant::now() < deadline {
        if let Ok(Ok(Some(raw))) =
            tokio::time::timeout(Duration::from_millis(200), sse.next_event()).await
        {
            if let Ok(ev) = serde_json::from_str::<serde_json::Value>(&raw) {
                if ev["kind"] == "peer_blocked" && ev["device_id"] == peer_id.as_str() {
                    saw_peer_blocked = true;
                }
                if ev["kind"] == "peer_down" && ev["device_id"] == peer_id.as_str() {
                    saw_peer_down = true;
                }
                if saw_peer_blocked && saw_peer_down {
                    break;
                }
            }
        }
    }
    assert!(saw_peer_blocked, "must emit peer_blocked event");
    assert!(
        saw_peer_down,
        "must emit peer_down event on active session teardown"
    );

    // 6. Verify persistence in blocked.json
    let blocked_file = g1.path().join(tetherly_node::BLOCKED_FILE);
    assert!(blocked_file.exists());
    let file_content = std::fs::read_to_string(&blocked_file).unwrap();
    assert!(file_content.contains(peer_id.as_str()));
    assert!(file_content.contains("malicious port scanning"));

    // 7. Verify outbound dialing is blocked on host
    let dial_payload = serde_json::json!({
        "peer": peer_id.as_str(),
        "addr": peer_addr.to_string(),
    });
    let (code_out, body_out) = tetherly_node::loopback_post(
        host_ui,
        "/api/peers/connect",
        &serde_json::to_vec(&dial_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_out, 400);
    let out_err = String::from_utf8_lossy(&body_out);
    assert!(
        out_err.contains("blocked"),
        "error message must indicate peer is blocked: {out_err}"
    );

    // 8. Verify inbound connection from blocked peer is rejected post-handshake
    let inbound_dial = serde_json::json!({
        "peer": host_id.as_str(),
        "addr": host_addr.to_string(),
    });
    let _ = tetherly_node::loopback_post(
        peer_ui,
        "/api/peers/connect",
        &serde_json::to_vec(&inbound_dial).unwrap(),
    )
    .await;

    // Small delay to let any attempted handshake land and be rejected by host
    tokio::time::sleep(Duration::from_millis(400)).await;

    // Verify host still has NO live peer with peer_id
    let (_, body_live2) = tetherly_node::loopback_get(host_ui, "/api/peers")
        .await
        .unwrap();
    let live_val2: serde_json::Value = serde_json::from_slice(&body_live2).unwrap();
    let live_arr2 = live_val2["live"].as_array().unwrap();
    assert!(!live_arr2.iter().any(|p| p["device_id"] == peer_id.as_str()));

    // 9. Test idempotency of blocking
    let (code_blk2, body_blk2) = tetherly_node::loopback_post(
        host_ui,
        "/api/peers/block",
        &serde_json::to_vec(&block_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_blk2, 200);
    let blk_rep2: tetherly_node::PeerBlockReport = serde_json::from_slice(&body_blk2).unwrap();
    assert!(blk_rep2.ok);
    assert!(blk_rep2.already_blocked);
    assert!(!blk_rep2.disconnected_active);

    // 10. Test unblocking
    let unblock_payload = serde_json::json!({ "peer": peer_id.as_str() });
    let (code_unblk, body_unblk) = tetherly_node::loopback_post(
        host_ui,
        "/api/peers/unblock",
        &serde_json::to_vec(&unblock_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_unblk, 200);
    let unblk_rep: tetherly_node::PeerUnblockReport = serde_json::from_slice(&body_unblk).unwrap();
    assert!(unblk_rep.ok);
    assert!(unblk_rep.unblocked);
    assert!(unblk_rep.was_blocked);
    assert_eq!(unblk_rep.device_id, peer_id.as_str());

    // Verify SSE emitted peer_unblocked
    let mut saw_peer_unblocked = false;
    let deadline_unblk = tokio::time::Instant::now() + Duration::from_secs(3);
    while tokio::time::Instant::now() < deadline_unblk {
        if let Ok(Ok(Some(raw))) =
            tokio::time::timeout(Duration::from_millis(200), sse.next_event()).await
        {
            if let Ok(ev) = serde_json::from_str::<serde_json::Value>(&raw) {
                if ev["kind"] == "peer_unblocked" && ev["device_id"] == peer_id.as_str() {
                    saw_peer_unblocked = true;
                    break;
                }
            }
        }
    }
    assert!(saw_peer_unblocked, "must emit peer_unblocked event");

    // GET /api/peers/blocked is empty again
    let (_, body_b_after) = tetherly_node::loopback_get(host_ui, "/api/peers/blocked")
        .await
        .unwrap();
    let b_after: tetherly_node::BlockedPeersReport = serde_json::from_slice(&body_b_after).unwrap();
    assert_eq!(b_after.count, 0);

    // Unblocking again returns unblocked: false, was_blocked: false
    let (_, body_unblk2) = tetherly_node::loopback_post(
        host_ui,
        "/api/peers/unblock",
        &serde_json::to_vec(&unblock_payload).unwrap(),
    )
    .await
    .unwrap();
    let unblk_rep2: tetherly_node::PeerUnblockReport =
        serde_json::from_slice(&body_unblk2).unwrap();
    assert!(!unblk_rep2.unblocked);
    assert!(!unblk_rep2.was_blocked);

    // 11. Verify connection restoration after unblocking
    let (code_rec, body_rec) = tetherly_node::loopback_post(
        host_ui,
        "/api/peers/connect",
        &serde_json::to_vec(&dial_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_rec, 200);
    let rec_rep: tetherly_node::PeerConnectReport = serde_json::from_slice(&body_rec).unwrap();
    assert!(rec_rep.ok);
    assert!(rec_rep.connected);

    let (_, body_live3) = tetherly_node::loopback_get(host_ui, "/api/peers")
        .await
        .unwrap();
    let live_val3: serde_json::Value = serde_json::from_slice(&body_live3).unwrap();
    let live_arr3 = live_val3["live"].as_array().unwrap();
    assert!(live_arr3.iter().any(|p| p["device_id"] == peer_id.as_str()));

    // 12. Negative tests & security boundaries:
    // a. Blocking local device ID returns 400
    let block_self = serde_json::json!({ "peer": host_id.as_str() });
    let (code_self, body_self) = tetherly_node::loopback_post(
        host_ui,
        "/api/peers/block",
        &serde_json::to_vec(&block_self).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_self, 400);
    assert!(String::from_utf8_lossy(&body_self).contains("cannot block local device"));

    // b. Empty peer query returns 400
    let empty_payload = serde_json::json!({});
    let (code_emp, _) = tetherly_node::loopback_post(
        host_ui,
        "/api/peers/block",
        &serde_json::to_vec(&empty_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_emp, 400);

    // 13. Clean shutdown
    host.shutdown();
    peer.shutdown();
}

/// Tests end-to-end notification lifecycle governance, multi-dimensional candidate
/// query filtering, bidirectional dismissal synchronization across active Noise sessions,
/// batch purging, loopback RESTful endpoints, and UiEvent::CandidateDismissed broadcasting.
#[tokio::test(flavor = "multi_thread")]
async fn notification_bidirectional_dismissal_and_lifecycle_governance_via_loopback_api() {
    let (host, peer, _g1, _g2) = pair_phone_desktop().await;
    let host_ui = host.spawn_ui().await.unwrap();
    let _peer_ui = peer.spawn_ui().await.unwrap();
    let host_id = host.identity().device_id().clone();
    let peer_id = peer.identity().device_id().clone();

    let mut host_rx = host.subscribe();

    // 1. Initial state: host has no active notification candidates
    let (code, body) = tetherly_node::loopback_get(host_ui, "/api/notify/candidates")
        .await
        .unwrap();
    assert_eq!(code, 200);
    let init_resp: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(init_resp["ok"], true);
    assert!(init_resp["candidates"].as_array().unwrap().is_empty());

    // 2. Peer sends 3 notifications to Host:
    //    Notification 1: WeChat message (no OTP)
    //    Notification 2: WeChat second message (no OTP)
    //    Notification 3: Bank SMS (with OTP)
    let p1 = tetherly_core::NotifyPush {
        uid: "wx-101".into(),
        app_id: "com.tencent.mm".into(),
        app_name: "WeChat".into(),
        title: "张三".into(),
        body: "收到请回复".into(),
        ts: 1_700_000_001,
        actions: vec!["dismiss".into()],
    };
    peer.send_notify(&host_id, p1).await.unwrap();

    let p2 = tetherly_core::NotifyPush {
        uid: "wx-102".into(),
        app_id: "com.tencent.mm".into(),
        app_name: "WeChat".into(),
        title: "李四".into(),
        body: "今晚开会".into(),
        ts: 1_700_000_002,
        actions: vec!["dismiss".into()],
    };
    peer.send_notify(&host_id, p2).await.unwrap();

    let p3 = tetherly_core::NotifyPush {
        uid: "bank-201".into(),
        app_id: "com.example.bank".into(),
        app_name: "BankApp".into(),
        title: "安全验证".into(),
        body: "【某银行】验证码 654321，切勿泄露给他人。".into(),
        ts: 1_700_000_003,
        actions: vec!["copy".into(), "dismiss".into()],
    };
    peer.send_notify(&host_id, p3).await.unwrap();

    // 3. Wait until Host receives all 3 candidates
    let start = std::time::Instant::now();
    let (c1_id, c2_id, c3_id) = loop {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/notify/candidates")
            .await
            .unwrap();
        assert_eq!(c, 200);
        let v: serde_json::Value = serde_json::from_slice(&b).unwrap();
        let list = v["candidates"].as_array().unwrap();
        if list.len() >= 3 {
            let id1 = list.iter().find(|x| x["uid"] == "wx-101").unwrap()["id"]
                .as_str()
                .unwrap()
                .to_string();
            let id2 = list.iter().find(|x| x["uid"] == "wx-102").unwrap()["id"]
                .as_str()
                .unwrap()
                .to_string();
            let id3 = list.iter().find(|x| x["uid"] == "bank-201").unwrap()["id"]
                .as_str()
                .unwrap()
                .to_string();
            break (id1, id2, id3);
        }
        if start.elapsed() > Duration::from_secs(10) {
            panic!("host did not ingest 3 candidates in time");
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    };

    // 4. Test multi-dimensional candidate filtering:
    // a. Filter by app_id=com.tencent.mm (should return 2)
    let (code_mm, body_mm) =
        tetherly_node::loopback_get(host_ui, "/api/notify/candidates?app_id=com.tencent.mm")
            .await
            .unwrap();
    assert_eq!(code_mm, 200);
    let val_mm: serde_json::Value = serde_json::from_slice(&body_mm).unwrap();
    assert_eq!(val_mm["candidates"].as_array().unwrap().len(), 2);

    // b. Filter by has_otp=true (should return 1)
    let (code_otp, body_otp) =
        tetherly_node::loopback_get(host_ui, "/api/notify/candidates?has_otp=true")
            .await
            .unwrap();
    assert_eq!(code_otp, 200);
    let val_otp: serde_json::Value = serde_json::from_slice(&body_otp).unwrap();
    assert_eq!(val_otp["candidates"].as_array().unwrap().len(), 1);
    assert_eq!(val_otp["candidates"][0]["app_id"], "com.example.bank");

    // c. Filter by peer
    let (code_peer, body_peer) = tetherly_node::loopback_get(
        host_ui,
        &format!("/api/notify/candidates?peer={}", peer_id.as_str()),
    )
    .await
    .unwrap();
    assert_eq!(code_peer, 200);
    let val_peer: serde_json::Value = serde_json::from_slice(&body_peer).unwrap();
    assert_eq!(val_peer["candidates"].as_array().unwrap().len(), 3);

    // 5. Test single dismissal via POST /api/notify/dismiss with wire synchronization:
    // Host dismisses c1 (wx-101).
    let dismiss_c1 = serde_json::json!({ "id": c1_id });
    let (code_dis, body_dis) = tetherly_node::loopback_post(
        host_ui,
        "/api/notify/dismiss",
        &serde_json::to_vec(&dismiss_c1).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_dis, 200);
    let rep_dis: tetherly_node::CandidateDismissReport = serde_json::from_slice(&body_dis).unwrap();
    assert!(rep_dis.ok);
    assert_eq!(rep_dis.id, c1_id);
    assert_eq!(rep_dis.app_id, "com.tencent.mm");
    assert_eq!(rep_dis.uid, "wx-101");
    assert!(rep_dis.notified_remote); // Wire NotifyDismiss frame was sent to connected peer

    // Host emits UiEvent::CandidateDismissed
    let mut saw_c1_dismissed = false;
    let ev_start = std::time::Instant::now();
    while ev_start.elapsed() < Duration::from_secs(5) {
        if let Ok(Ok(tetherly_node::UiEvent::CandidateDismissed {
            id, app_id, uid, ..
        })) = tokio::time::timeout(Duration::from_millis(500), host_rx.recv()).await
        {
            if id == c1_id && app_id == "com.tencent.mm" && uid == "wx-101" {
                saw_c1_dismissed = true;
                break;
            }
        }
    }
    assert!(
        saw_c1_dismissed,
        "host must emit CandidateDismissed event for c1"
    );

    // 6. Test reverse wire dismissal:
    // Peer (phone) sends a NotifyDismiss frame over Noise tunnel for c2 (wx-102).
    let wire_dismiss = tetherly_core::frame::NotifyDismiss {
        uid: "wx-102".into(),
        app_id: "com.tencent.mm".into(),
    }
    .to_frame()
    .unwrap();
    peer.send_to(&host_id, wire_dismiss).await.unwrap();

    // Host receives wire frame, dismisses candidate from NotifyHub, and emits UiEvent::CandidateDismissed
    let mut saw_c2_dismissed = false;
    let ev2_start = std::time::Instant::now();
    while ev2_start.elapsed() < Duration::from_secs(5) {
        if let Ok(Ok(tetherly_node::UiEvent::CandidateDismissed {
            id, app_id, uid, ..
        })) = tokio::time::timeout(Duration::from_millis(500), host_rx.recv()).await
        {
            if id == c2_id && app_id == "com.tencent.mm" && uid == "wx-102" {
                saw_c2_dismissed = true;
                break;
            }
        }
    }
    assert!(
        saw_c2_dismissed,
        "host must emit CandidateDismissed event on receiving wire dismiss from peer"
    );

    // After dismissing c1 and c2, only c3 remains on host
    let (_, body_rem) = tetherly_node::loopback_get(host_ui, "/api/notify/candidates")
        .await
        .unwrap();
    let val_rem: serde_json::Value = serde_json::from_slice(&body_rem).unwrap();
    let rem_list = val_rem["candidates"].as_array().unwrap();
    assert_eq!(rem_list.len(), 1);
    assert_eq!(rem_list[0]["id"], c3_id.as_str());

    // 7. Test batch dismissal via DELETE /api/notify/candidates?all=true
    let (code_del, body_del) =
        tetherly_node::loopback_delete(host_ui, "/api/notify/candidates?all=true")
            .await
            .unwrap();
    assert_eq!(code_del, 200);
    let batch_rep: tetherly_node::BatchDismissReport = serde_json::from_slice(&body_del).unwrap();
    assert!(batch_rep.ok);
    assert_eq!(batch_rep.count, 1);
    assert_eq!(batch_rep.ids, vec![c3_id.clone()]);
    assert_eq!(batch_rep.notified_remote_count, 1);

    // Candidates list is now completely empty
    let (_, body_empty) = tetherly_node::loopback_get(host_ui, "/api/notify/candidates")
        .await
        .unwrap();
    let val_empty: serde_json::Value = serde_json::from_slice(&body_empty).unwrap();
    assert!(val_empty["candidates"].as_array().unwrap().is_empty());

    // 8. Test negative boundaries & idempotency:
    // a. Dismissing unknown candidate ID returns 400
    let bad_dismiss = serde_json::json!({ "id": "ntc_nonexistent_9999" });
    let (code_bad, body_bad) = tetherly_node::loopback_post(
        host_ui,
        "/api/notify/dismiss",
        &serde_json::to_vec(&bad_dismiss).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_bad, 400);
    assert!(String::from_utf8_lossy(&body_bad).contains("unknown candidate"));

    // b. Empty dismiss payload returns 400
    let empty_payload = serde_json::json!({});
    let (code_emp, _) = tetherly_node::loopback_post(
        host_ui,
        "/api/notify/dismiss",
        &serde_json::to_vec(&empty_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_emp, 400);

    // c. Legacy POST /api/dismiss also works with unknown ID returning 400
    let (code_leg, _) = tetherly_node::loopback_post(
        host_ui,
        "/api/dismiss",
        &serde_json::to_vec(&bad_dismiss).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_leg, 400);

    // 9. Clean shutdown
    host.shutdown();
    peer.shutdown();
}

/// Tests end-to-end clipboard lifecycle governance, zero-plaintext metadata telemetry,
/// bounded ring buffer history (MAX_CLIP_HISTORY = 32), inbound & outbound UiEvent::ClipboardSynced
/// emissions, loopback RESTful endpoints (/api/clip/status, /api/clip/history, /api/clip/clear),
/// and clipboard purge with UiEvent::ClipboardCleared broadcasting.
#[tokio::test(flavor = "multi_thread")]
async fn clipboard_lifecycle_history_and_observability_via_loopback_api() {
    let (host, peer, _g1, _g2) = pair_phone_desktop().await;
    let host_ui = host.spawn_ui().await.unwrap();
    let _peer_ui = peer.spawn_ui().await.unwrap();
    let host_id = host.identity().device_id().clone();
    let peer_id = peer.identity().device_id().clone();

    let mut host_rx = host.subscribe();
    let mut peer_rx = peer.subscribe();

    // 1. Initial State: Host clipboard is empty, zero counts, empty history
    let (code_st, body_st) = tetherly_node::loopback_get(host_ui, "/api/clip/status")
        .await
        .unwrap();
    assert_eq!(code_st, 200);
    let st_init: tetherly_node::ClipStatusReport = serde_json::from_slice(&body_st).unwrap();
    assert!(st_init.ok);
    assert!(!st_init.has_text);
    assert_eq!(st_init.bytes, 0);
    assert_eq!(st_init.hash, None);
    assert_eq!(st_init.last_seq, 0);
    assert_eq!(st_init.inbound_count, 0);
    assert_eq!(st_init.outbound_count, 0);
    assert_eq!(st_init.history_count, 0);

    let (code_hist, body_hist) = tetherly_node::loopback_get(host_ui, "/api/clip/history")
        .await
        .unwrap();
    assert_eq!(code_hist, 200);
    let hist_init: tetherly_node::ClipHistoryReport = serde_json::from_slice(&body_hist).unwrap();
    assert!(hist_init.ok);
    assert_eq!(hist_init.total, 0);
    assert!(hist_init.entries.is_empty());

    // 2. Outbound sync: Host sends text to Peer via POST /api/clip/sync
    let test_msg1 = "tetherly-v1.52-audit-clip-1";
    let clip1_hash = hex_sha256(test_msg1.as_bytes());
    let payload1 = serde_json::json!({
        "text": test_msg1,
        "peer": peer_id.as_str(),
    });
    let (code_sync1, body_sync1) = tetherly_node::loopback_post(
        host_ui,
        "/api/clip/sync",
        &serde_json::to_vec(&payload1).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_sync1, 200);
    let sync1_val: serde_json::Value = serde_json::from_slice(&body_sync1).unwrap();
    assert_eq!(sync1_val["ok"], true);
    assert_eq!(sync1_val["sent_peers"], 1);
    assert_eq!(sync1_val["bytes"], test_msg1.len());
    assert_eq!(sync1_val["hash"], clip1_hash.as_str());

    // Host should broadcast UiEvent::ClipboardSynced (outbound)
    let start = std::time::Instant::now();
    let mut host_outbound_found = false;
    while start.elapsed() < Duration::from_secs(5) {
        if let Ok(Ok(tetherly_node::UiEvent::ClipboardSynced {
            seq,
            source,
            direction,
            bytes,
            hash,
            sent_peers,
            ..
        })) = tokio::time::timeout(Duration::from_millis(500), host_rx.recv()).await
        {
            if direction == "outbound" && hash == clip1_hash {
                assert_eq!(seq, 1);
                assert_eq!(source, "local");
                assert_eq!(bytes, test_msg1.len());
                assert_eq!(sent_peers, 1);
                host_outbound_found = true;
                break;
            }
        }
    }
    assert!(
        host_outbound_found,
        "host did not emit outbound ClipboardSynced event"
    );

    // Peer should receive clip, update OS clipboard, and emit UiEvent::ClipboardSynced (inbound)
    let start = std::time::Instant::now();
    let mut peer_inbound_found = false;
    while start.elapsed() < Duration::from_secs(5) {
        if let Ok(Ok(tetherly_node::UiEvent::ClipboardSynced {
            seq,
            source,
            direction,
            bytes,
            hash,
            sent_peers,
            ..
        })) = tokio::time::timeout(Duration::from_millis(500), peer_rx.recv()).await
        {
            if direction == "inbound" && hash == clip1_hash {
                assert_eq!(seq, 1);
                assert_eq!(source, host_id.to_string());
                assert_eq!(bytes, test_msg1.len());
                assert_eq!(sent_peers, 0);
                peer_inbound_found = true;
                break;
            }
        }
    }
    assert!(
        peer_inbound_found,
        "peer did not emit inbound ClipboardSynced event"
    );
    assert_eq!(peer.clip().get_text().unwrap().as_deref(), Some(test_msg1));

    // 3. Inspect Host status and history
    let (code_st2, body_st2) = tetherly_node::loopback_get(host_ui, "/api/clip/status")
        .await
        .unwrap();
    assert_eq!(code_st2, 200);
    let st2: tetherly_node::ClipStatusReport = serde_json::from_slice(&body_st2).unwrap();
    assert!(st2.has_text);
    assert_eq!(st2.bytes, test_msg1.len());
    assert_eq!(st2.hash, Some(clip1_hash.clone()));
    assert_eq!(st2.last_seq, 1);
    assert_eq!(st2.outbound_count, 1);
    assert_eq!(st2.inbound_count, 0);
    assert_eq!(st2.history_count, 1);

    let (code_hist2, body_hist2) = tetherly_node::loopback_get(host_ui, "/api/clip/history")
        .await
        .unwrap();
    assert_eq!(code_hist2, 200);
    let hist2: tetherly_node::ClipHistoryReport = serde_json::from_slice(&body_hist2).unwrap();
    assert_eq!(hist2.total, 1);
    assert_eq!(hist2.entries.len(), 1);
    assert_eq!(hist2.entries[0].seq, 1);
    assert_eq!(hist2.entries[0].direction, "outbound");
    assert_eq!(hist2.entries[0].bytes, test_msg1.len());
    assert_eq!(hist2.entries[0].hash, clip1_hash);

    // 4. Inbound sync on Host: Peer sends text back to Host
    let test_msg2 = "tetherly-v1.52-response-clip-2";
    let clip2_hash = hex_sha256(test_msg2.as_bytes());
    peer.send_clipboard_text(test_msg2, Some(&host_id))
        .await
        .unwrap();

    // Host should receive and emit UiEvent::ClipboardSynced (inbound)
    let start = std::time::Instant::now();
    let mut host_inbound_found = false;
    while start.elapsed() < Duration::from_secs(5) {
        if let Ok(Ok(tetherly_node::UiEvent::ClipboardSynced {
            seq,
            source,
            direction,
            bytes,
            hash,
            ..
        })) = tokio::time::timeout(Duration::from_millis(500), host_rx.recv()).await
        {
            if direction == "inbound" && hash == clip2_hash {
                assert_eq!(seq, 2);
                assert_eq!(source, peer_id.to_string());
                assert_eq!(bytes, test_msg2.len());
                host_inbound_found = true;
                break;
            }
        }
    }
    assert!(
        host_inbound_found,
        "host did not emit inbound ClipboardSynced event"
    );
    assert_eq!(host.clip().get_text().unwrap().as_deref(), Some(test_msg2));

    // Host status should now reflect both outbound and inbound counts
    let (_, body_st3) = tetherly_node::loopback_get(host_ui, "/api/clip/status")
        .await
        .unwrap();
    let st3: tetherly_node::ClipStatusReport = serde_json::from_slice(&body_st3).unwrap();
    assert_eq!(st3.outbound_count, 1);
    assert_eq!(st3.inbound_count, 1);
    assert_eq!(st3.history_count, 2);
    assert_eq!(st3.last_seq, 2);

    // Test history query with ?limit=1
    let (_, body_lim) = tetherly_node::loopback_get(host_ui, "/api/clip/history?limit=1")
        .await
        .unwrap();
    let hist_lim: tetherly_node::ClipHistoryReport = serde_json::from_slice(&body_lim).unwrap();
    assert_eq!(hist_lim.total, 1);
    assert_eq!(hist_lim.entries.len(), 1);
    assert_eq!(hist_lim.entries[0].seq, 2);
    assert_eq!(hist_lim.entries[0].direction, "inbound");

    // 5. Test Ring Buffer Bounded Capacity (32 entries max)
    for i in 3..=40 {
        let dummy = format!("clip-msg-{i}");
        let _ = host.send_clipboard_text(&dummy, Some(&peer_id)).await;
    }
    let (_, body_bounded) = tetherly_node::loopback_get(host_ui, "/api/clip/history")
        .await
        .unwrap();
    let hist_bounded: tetherly_node::ClipHistoryReport =
        serde_json::from_slice(&body_bounded).unwrap();
    assert_eq!(hist_bounded.total, tetherly_core::MAX_CLIP_HISTORY);
    assert_eq!(hist_bounded.entries.len(), 32);
    // history() returns newest-first: index 0 is newest (40), index 31 is oldest surviving (40 - 32 + 1 = 9)
    assert_eq!(hist_bounded.entries[0].seq, 40);
    assert_eq!(hist_bounded.entries[31].seq, 9);

    // 6. Test Clear & Purge via POST /api/clip/clear
    let (code_clear, body_clear) = tetherly_node::loopback_post(host_ui, "/api/clip/clear", b"{}")
        .await
        .unwrap();
    assert_eq!(code_clear, 200);
    let clear_rep: tetherly_node::ClipClearReport = serde_json::from_slice(&body_clear).unwrap();
    assert!(clear_rep.ok);
    assert_eq!(clear_rep.cleared_history_count, 32);
    assert!(clear_rep.timestamp_ms > 0);

    // Host should emit UiEvent::ClipboardCleared
    let start = std::time::Instant::now();
    let mut cleared_ev_found = false;
    while start.elapsed() < Duration::from_secs(5) {
        if let Ok(Ok(tetherly_node::UiEvent::ClipboardCleared { timestamp_ms })) =
            tokio::time::timeout(Duration::from_millis(500), host_rx.recv()).await
        {
            assert_eq!(timestamp_ms, clear_rep.timestamp_ms);
            cleared_ev_found = true;
            break;
        }
    }
    assert!(cleared_ev_found, "host did not emit ClipboardCleared event");

    // Status and History are fully cleared
    let (_, body_st_cleared) = tetherly_node::loopback_get(host_ui, "/api/clip/status")
        .await
        .unwrap();
    let st_cleared: tetherly_node::ClipStatusReport =
        serde_json::from_slice(&body_st_cleared).unwrap();
    assert!(!st_cleared.has_text);
    assert_eq!(st_cleared.bytes, 0);
    assert_eq!(st_cleared.hash, None);
    assert_eq!(st_cleared.history_count, 0);

    let (_, body_hist_cleared) = tetherly_node::loopback_get(host_ui, "/api/clip/history")
        .await
        .unwrap();
    let hist_cleared: tetherly_node::ClipHistoryReport =
        serde_json::from_slice(&body_hist_cleared).unwrap();
    assert_eq!(hist_cleared.total, 0);
    assert!(hist_cleared.entries.is_empty());

    // System clipboard on host is cleared
    assert_eq!(host.clip().get_text().unwrap().as_deref(), Some(""));

    // 7. Test DELETE /api/clip alias for clear
    host.clip().set_text("test-delete-verb").unwrap();
    let (code_del, body_del) = tetherly_node::loopback_delete(host_ui, "/api/clip")
        .await
        .unwrap();
    assert_eq!(code_del, 200);
    let del_rep: tetherly_node::ClipClearReport = serde_json::from_slice(&body_del).unwrap();
    assert!(del_rep.ok);
    assert_eq!(host.clip().get_text().unwrap().as_deref(), Some(""));

    // 8. Clean shutdown
    host.shutdown();
    peer.shutdown();
}

/// ADR-046: Cross-Device File Offer Lifecycle Governance, Lightweight Offer Introspection,
/// Batch Decision Control, Auto-Accept Trusted Peers Policy, and Loopback RESTful Closed-Loop.
#[tokio::test(flavor = "multi_thread")]
async fn file_offer_lifecycle_and_terminal_governance_via_loopback_api() {
    let (host, peer, _g1, g2) = pair_phone_desktop().await;
    let host_ui = host.spawn_ui().await.unwrap();
    let peer_ui = peer.spawn_ui().await.unwrap();
    let host_id = host.identity().device_id().clone();
    let peer_id = peer.identity().device_id().clone();
    let peer_str = peer_id.to_string();

    let start_caps = std::time::Instant::now();
    loop {
        if host.live_peers().iter().any(|p| p.file_port.is_some())
            && peer.live_peers().iter().any(|p| p.file_port.is_some())
        {
            break;
        }
        if start_caps.elapsed() > Duration::from_secs(5) {
            panic!("fileport caps.update never arrived");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let mut host_rx = host.subscribe();

    // 1. Initial State: No pending offers, auto-accept disabled, counter is 0
    let (code_init, body_init) = tetherly_node::loopback_get(host_ui, "/api/file/offers")
        .await
        .unwrap();
    assert_eq!(code_init, 200);
    let init_offers: tetherly_node::FileOffersReport = serde_json::from_slice(&body_init).unwrap();
    assert!(init_offers.ok);
    assert_eq!(init_offers.count, 0);
    assert!(init_offers.offers.is_empty());
    assert!(!init_offers.auto_accept_trusted);

    let (code_auto_init, body_auto_init) =
        tetherly_node::loopback_get(host_ui, "/api/file/auto-accept")
            .await
            .unwrap();
    assert_eq!(code_auto_init, 200);
    let auto_init: tetherly_node::AutoAcceptReport =
        serde_json::from_slice(&body_auto_init).unwrap();
    assert!(auto_init.ok);
    assert!(!auto_init.enabled);
    assert_eq!(auto_init.auto_accepted_count, 0);

    // 2. Peer offers a file to Host
    let (send_dir, _g_send) = temp_dir("offer-send");
    let file1_path = send_dir.join("offer_doc1.txt");
    let file1_data = b"Tetherly v1.53 Offer Ingestion and Lifecycle Test File 1";
    std::fs::write(&file1_path, file1_data).unwrap();

    let payload1 = serde_json::json!({
        "path": file1_path.to_str().unwrap(),
        "name": "offer_doc1.txt",
        "peer": host_id,
    });
    let (code_s1, body_s1) = tetherly_node::loopback_post(
        peer_ui,
        "/api/file/send",
        &serde_json::to_vec(&payload1).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_s1, 200);
    let s1_resp: serde_json::Value = serde_json::from_slice(&body_s1).unwrap();
    let transfer_id1 = s1_resp["transfer_id"].as_str().unwrap().to_string();

    wait_offered(&host, Duration::from_secs(10)).await;

    // 3. Introspect offers on Host via GET /api/file/offers
    let (_, body_offers1) = tetherly_node::loopback_get(host_ui, "/api/file/offers")
        .await
        .unwrap();
    let rep_offers1: tetherly_node::FileOffersReport =
        serde_json::from_slice(&body_offers1).unwrap();
    assert_eq!(rep_offers1.count, 1);
    assert_eq!(rep_offers1.offers[0].transfer_id, transfer_id1);
    assert_eq!(rep_offers1.offers[0].filename, "offer_doc1.txt");
    assert_eq!(rep_offers1.offers[0].total_bytes, file1_data.len() as u64);
    assert_eq!(rep_offers1.offers[0].resume_from, 0);
    assert_eq!(rep_offers1.offers[0].source, peer_str);

    // Test query by specific ID
    let (code_by_id, body_by_id) =
        tetherly_node::loopback_get(host_ui, &format!("/api/file/offers?id={transfer_id1}"))
            .await
            .unwrap();
    assert_eq!(code_by_id, 200);
    let single_offer: tetherly_node::FileOfferDto = serde_json::from_slice(&body_by_id).unwrap();
    assert_eq!(single_offer.transfer_id, transfer_id1);
    assert_eq!(single_offer.filename, "offer_doc1.txt");

    // Test query by peer filter
    let (_, body_peer_filter) =
        tetherly_node::loopback_get(host_ui, &format!("/api/file/offers?peer={peer_id}"))
            .await
            .unwrap();
    let peer_offers: tetherly_node::FileOffersReport =
        serde_json::from_slice(&body_peer_filter).unwrap();
    assert_eq!(peer_offers.count, 1);

    // Non-matching peer filter returns 0 offers
    let (_, body_empty_filter) =
        tetherly_node::loopback_get(host_ui, "/api/file/offers?peer=tdev_nonexistent")
            .await
            .unwrap();
    let empty_filter_rep: tetherly_node::FileOffersReport =
        serde_json::from_slice(&body_empty_filter).unwrap();
    assert_eq!(empty_filter_rep.count, 0);

    // 404 for unknown transfer ID
    let (code_404, _) = tetherly_node::loopback_get(host_ui, "/api/file/offers?id=tfl_nonexistent")
        .await
        .unwrap();
    assert_eq!(code_404, 404);

    // 4. Reject the offer on Host via POST /api/file/reject
    let rej_payload = serde_json::json!({ "id": transfer_id1 });
    let (code_rej, body_rej) = tetherly_node::loopback_post(
        host_ui,
        "/api/file/reject",
        &serde_json::to_vec(&rej_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_rej, 200);
    let rej_rep: tetherly_node::FileDecisionReport = serde_json::from_slice(&body_rej).unwrap();
    assert!(rej_rep.ok);
    assert!(!rej_rep.accepted);
    assert_eq!(rej_rep.transfer_id, transfer_id1);
    assert_eq!(rej_rep.filename, "offer_doc1.txt");
    assert_eq!(rej_rep.peer, peer_str);

    // Verify host receives UiEvent::FileRejected
    let start = std::time::Instant::now();
    let mut file_rej_found = false;
    while start.elapsed() < Duration::from_secs(5) {
        if let Ok(Ok(tetherly_node::UiEvent::FileRejected {
            transfer_id,
            source,
            ..
        })) = tokio::time::timeout(Duration::from_millis(500), host_rx.recv()).await
        {
            if transfer_id == transfer_id1 && source == peer_str {
                file_rej_found = true;
                break;
            }
        }
    }
    assert!(file_rej_found, "host did not emit FileRejected event");

    // Pending offers is now empty
    let (_, body_after_rej) = tetherly_node::loopback_get(host_ui, "/api/file/offers")
        .await
        .unwrap();
    let after_rej_rep: tetherly_node::FileOffersReport =
        serde_json::from_slice(&body_after_rej).unwrap();
    assert_eq!(after_rej_rep.count, 0);

    // Wait until peer receives the rejection frame and marks transfer_id1 Failed
    let start_peer_rej = std::time::Instant::now();
    loop {
        let (_, b_peer_xfers) = tetherly_node::loopback_get(peer_ui, "/api/file/transfers")
            .await
            .unwrap();
        let rep_peer_xfers: tetherly_node::TransfersReport =
            serde_json::from_slice(&b_peer_xfers).unwrap();
        if let Some(t) = rep_peer_xfers
            .transfers
            .iter()
            .find(|t| t.transfer_id == transfer_id1)
        {
            if t.phase == tetherly_node::TransferPhase::Failed {
                break;
            }
        }
        if start_peer_rej.elapsed() > Duration::from_secs(10) {
            panic!("timed out waiting for peer to observe rejection of {transfer_id1}");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    // 5. Test Batch Reject: Peer offers 2 files
    let file_a_path = send_dir.join("batch_a.bin");
    let file_b_path = send_dir.join("batch_b.bin");
    std::fs::write(&file_a_path, b"Batch Content A").unwrap();
    std::fs::write(&file_b_path, b"Batch Content B").unwrap();

    let (c_a, b_a) = tetherly_node::loopback_post(
        peer_ui,
        "/api/file/send",
        &serde_json::to_vec(&serde_json::json!({
            "path": file_a_path.to_str().unwrap(),
            "name": "batch_a.bin",
            "peer": host_id,
        }))
        .unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c_a, 200);
    let id_a = serde_json::from_slice::<serde_json::Value>(&b_a).unwrap()["transfer_id"]
        .as_str()
        .unwrap()
        .to_string();

    let (c_b, b_b) = tetherly_node::loopback_post(
        peer_ui,
        "/api/file/send",
        &serde_json::to_vec(&serde_json::json!({
            "path": file_b_path.to_str().unwrap(),
            "name": "batch_b.bin",
            "peer": host_id,
        }))
        .unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c_b, 200);
    let id_b = serde_json::from_slice::<serde_json::Value>(&b_b).unwrap()["transfer_id"]
        .as_str()
        .unwrap()
        .to_string();

    // Wait until both offers appear on Host
    let start_wait = std::time::Instant::now();
    loop {
        let (_, b_chk) = tetherly_node::loopback_get(host_ui, "/api/file/offers")
            .await
            .unwrap();
        let rep_chk: tetherly_node::FileOffersReport = serde_json::from_slice(&b_chk).unwrap();
        if rep_chk.count == 2 {
            break;
        }
        if start_wait.elapsed() > Duration::from_secs(10) {
            panic!("timed out waiting for 2 offers, got {}", rep_chk.count);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    // Batch reject all offers from peer
    let batch_rej_payload = serde_json::json!({
        "all": true,
        "peer": peer_id,
    });
    let (code_batch_rej, body_batch_rej) = tetherly_node::loopback_post(
        host_ui,
        "/api/file/reject",
        &serde_json::to_vec(&batch_rej_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_batch_rej, 200);
    let batch_rej: tetherly_node::FileBatchDecisionReport =
        serde_json::from_slice(&body_batch_rej).unwrap();
    assert!(batch_rej.ok);
    assert_eq!(batch_rej.rejected_count, 2);
    assert_eq!(batch_rej.total_evaluated, 2);
    assert_eq!(batch_rej.decisions.len(), 2);
    assert!(batch_rej.decisions.iter().any(|d| d.transfer_id == id_a));
    assert!(batch_rej.decisions.iter().any(|d| d.transfer_id == id_b));

    // Verify both offers are gone
    let (_, b_after_batch) = tetherly_node::loopback_get(host_ui, "/api/file/offers")
        .await
        .unwrap();
    let after_batch: tetherly_node::FileOffersReport =
        serde_json::from_slice(&b_after_batch).unwrap();
    assert_eq!(after_batch.count, 0);

    // Wait until peer receives the rejection frames and marks both id_a and id_b Failed
    let start_peer_batch = std::time::Instant::now();
    loop {
        let (_, b_peer_xfers) = tetherly_node::loopback_get(peer_ui, "/api/file/transfers")
            .await
            .unwrap();
        let rep_peer_xfers: tetherly_node::TransfersReport =
            serde_json::from_slice(&b_peer_xfers).unwrap();
        let a_failed = rep_peer_xfers
            .transfers
            .iter()
            .find(|t| t.transfer_id == id_a)
            .map(|t| t.phase == tetherly_node::TransferPhase::Failed)
            .unwrap_or(false);
        let b_failed = rep_peer_xfers
            .transfers
            .iter()
            .find(|t| t.transfer_id == id_b)
            .map(|t| t.phase == tetherly_node::TransferPhase::Failed)
            .unwrap_or(false);
        if a_failed && b_failed {
            break;
        }
        if start_peer_batch.elapsed() > Duration::from_secs(10) {
            panic!("timed out waiting for peer to observe batch rejection of {id_a} and {id_b}");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    // 6. Test Auto-Accept Policy for Trusted Peers on Peer node
    let mut peer_rx = peer.subscribe();
    let auto_enable_payload = serde_json::json!({ "enabled": true });
    let (code_en, body_en) = tetherly_node::loopback_post(
        peer_ui,
        "/api/file/auto-accept",
        &serde_json::to_vec(&auto_enable_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_en, 200);
    let en_rep: tetherly_node::AutoAcceptReport = serde_json::from_slice(&body_en).unwrap();
    assert!(en_rep.ok);
    assert!(en_rep.enabled);

    // Verify UiEvent::FileAutoAcceptChanged emission on peer
    let start = std::time::Instant::now();
    let mut auto_ev_found = false;
    while start.elapsed() < Duration::from_secs(5) {
        if let Ok(Ok(tetherly_node::UiEvent::FileAutoAcceptChanged { enabled })) =
            tokio::time::timeout(Duration::from_millis(500), peer_rx.recv()).await
        {
            if enabled {
                auto_ev_found = true;
                break;
            }
        }
    }
    assert!(
        auto_ev_found,
        "peer did not emit FileAutoAcceptChanged event"
    );

    // Host sends a file while auto-accept is enabled on peer
    let auto_file_path = send_dir.join("auto_file.dat");
    let auto_data = b"Autonomous File Transfer Protocol Verification Data Payload v1.53";
    std::fs::write(&auto_file_path, auto_data).unwrap();
    let auto_expected_hash = hex_sha256(auto_data);

    let (c_auto_send, b_auto_send) = tetherly_node::loopback_post(
        host_ui,
        "/api/file/send",
        &serde_json::to_vec(&serde_json::json!({
            "path": auto_file_path.to_str().unwrap(),
            "name": "auto_file.dat",
            "peer": peer_id,
        }))
        .unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c_auto_send, 200);
    let auto_tid = serde_json::from_slice::<serde_json::Value>(&b_auto_send).unwrap()
        ["transfer_id"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(!auto_tid.is_empty());

    // File should be automatically accepted and received in peer inbox without manual call!
    let start_auto = std::time::Instant::now();
    loop {
        let (c_auto_st, b_auto_st) = tetherly_node::loopback_get(peer_ui, "/api/file/auto-accept")
            .await
            .unwrap();
        assert_eq!(c_auto_st, 200);
        let auto_st: tetherly_node::AutoAcceptReport = serde_json::from_slice(&b_auto_st).unwrap();
        if auto_st.auto_accepted_count >= 1 {
            assert!(auto_st.enabled);
            break;
        }
        if start_auto.elapsed() > Duration::from_secs(20) {
            let (_, body_offers) = tetherly_node::loopback_get(peer_ui, "/api/file/offers")
                .await
                .unwrap();
            let (_, body_transfers) = tetherly_node::loopback_get(peer_ui, "/api/file/transfers")
                .await
                .unwrap();
            panic!(
                "auto-accept counter did not reach 1 within 20s. auto_st: {:?}, peer offers: {}, \
                 peer transfers: {}",
                auto_st,
                String::from_utf8_lossy(&body_offers),
                String::from_utf8_lossy(&body_transfers),
            );
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    let start_inbox = std::time::Instant::now();
    let auto_dest = g2.path().join("inbox").join("auto_file.dat");
    let received_data = loop {
        if let Ok(bytes) = std::fs::read(&auto_dest) {
            if !bytes.is_empty() {
                break bytes;
            }
        }
        if start_inbox.elapsed() > Duration::from_secs(35) {
            let entries = std::fs::read_dir(g2.path().join("inbox"))
                .map(|rd| {
                    rd.filter_map(|e| e.ok().map(|x| x.file_name().to_string_lossy().to_string()))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let (_, body_host_xfers) = tetherly_node::loopback_get(host_ui, "/api/file/transfers")
                .await
                .unwrap();
            let (_, body_peer_xfers) = tetherly_node::loopback_get(peer_ui, "/api/file/transfers")
                .await
                .unwrap();
            panic!(
                "inbox file missing after 35s. entries: {:?}, host transfers: {}, \
                 peer transfers: {}",
                entries,
                String::from_utf8_lossy(&body_host_xfers),
                String::from_utf8_lossy(&body_peer_xfers),
            );
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert_eq!(hex_sha256(&received_data), auto_expected_hash);

    // 7. Disable Auto-Accept Policy
    let (code_dis, body_dis) = tetherly_node::loopback_post(
        peer_ui,
        "/api/file/auto-accept",
        &serde_json::to_vec(&serde_json::json!({ "enabled": false })).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_dis, 200);
    let dis_rep: tetherly_node::AutoAcceptReport = serde_json::from_slice(&body_dis).unwrap();
    assert!(!dis_rep.enabled);

    // 8. Error boundaries and Security Red Lines
    // a. Accept with empty body (no ID, no all) returns 400 Bad Request
    let (c_bad_acc, _) = tetherly_node::loopback_post(host_ui, "/api/file/accept", b"{}")
        .await
        .unwrap();
    assert_eq!(c_bad_acc, 400);

    // b. Reject with unknown transfer ID returns 404 Not Found
    let (c_bad_rej, _) = tetherly_node::loopback_post(
        host_ui,
        "/api/file/reject",
        &serde_json::to_vec(&serde_json::json!({ "id": "tfl_unknown_xyz" })).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c_bad_rej, 404);

    // c. Malformed JSON returns 400
    let (c_mal, _) = tetherly_node::loopback_post(host_ui, "/api/file/accept", b"{ bad json")
        .await
        .unwrap();
    assert_eq!(c_mal, 400);

    // d. Multi-file offer cumulative total_bytes verification & decision reporting
    let multi_offer_id = "tr_multifile_test";
    let multi_offer = FileOffer {
        transfer_id: multi_offer_id.into(),
        files: vec![
            FileMeta {
                name: "part1.bin".into(),
                size: 1500,
                sha256: "1".repeat(64),
            },
            FileMeta {
                name: "part2.bin".into(),
                size: 2500,
                sha256: "2".repeat(64),
            },
        ],
    };
    peer.send_to(&host_id, multi_offer.to_frame().unwrap())
        .await
        .unwrap();

    let start_multi = std::time::Instant::now();
    loop {
        let (code_multi, body_multi) =
            tetherly_node::loopback_get(host_ui, &format!("/api/file/offers?id={multi_offer_id}"))
                .await
                .unwrap();
        if code_multi == 200 {
            let multi_dto: tetherly_node::FileOfferDto =
                serde_json::from_slice(&body_multi).unwrap();
            assert_eq!(multi_dto.transfer_id, multi_offer_id);
            assert_eq!(multi_dto.filename, "part1.bin");
            assert_eq!(multi_dto.total_bytes, 4000);
            assert_eq!(multi_dto.size, 4000);
            break;
        }
        if start_multi.elapsed() > Duration::from_secs(5) {
            panic!("timed out waiting for multi-file offer on host");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    // Reject via user_reject_file_reported and verify decision report fields
    let rej_rep = host
        .user_reject_file_reported(multi_offer_id)
        .await
        .unwrap();
    assert!(!rej_rep.accepted);
    assert_eq!(rej_rep.transfer_id, multi_offer_id);
    assert_eq!(rej_rep.total_bytes, 4000);
    assert_eq!(rej_rep.filename, "part1.bin");
    assert_eq!(rej_rep.peer, peer_str);

    // 9. Clean shutdown
    host.shutdown();
    peer.shutdown();
}

/// ADR-052: Batch file offer pipeline, deep resume observability, and loopback Host 403 isolation.
#[tokio::test(flavor = "multi_thread")]
async fn batch_file_offer_pipeline_and_resume_observability_via_loopback_api() {
    let (host, peer, _g1, g2) = pair_phone_desktop().await;
    let host_ui = host.spawn_ui().await.expect("host UI spawns");
    let peer_ui = peer.spawn_ui().await.expect("peer UI spawns");
    let host_id = host.identity().device_id().clone();
    let peer_id = peer.identity().device_id().clone();

    // 1. Prepare multiple distinct files on disk for batch offer
    let (files_dir, _fg) = temp_dir("batch-send-files");
    let file1 = files_dir.join("document.pdf");
    let file2 = files_dir.join("photo.jpg");
    let file3 = files_dir.join("dataset.csv");
    std::fs::write(
        &file1,
        b"%PDF-1.4 sample document payload for tetherly batch transfer",
    )
    .unwrap();
    std::fs::write(&file2, b"JPEG image binary stream payload bytes 1234567890").unwrap();
    std::fs::write(
        &file3,
        b"id,name,value\n1,alpha,100\n2,beta,200\n3,gamma,300\n",
    )
    .unwrap();

    let total_expected_bytes = (std::fs::metadata(&file1).unwrap().len()
        + std::fs::metadata(&file2).unwrap().len()
        + std::fs::metadata(&file3).unwrap().len()) as u64;

    // 2. Submit batch send request via POST /api/file/send-batch
    let batch_req = serde_json::json!({
        "peer": peer_id.as_str(),
        "paths": [
            file1.to_str().unwrap(),
            file2.to_str().unwrap(),
            file3.to_str().unwrap(),
        ]
    });
    let (code_batch, body_batch) = tetherly_node::loopback_post(
        host_ui,
        "/api/file/send-batch",
        &serde_json::to_vec(&batch_req).unwrap(),
    )
    .await
    .expect("batch send post succeeds");
    assert_eq!(code_batch, 200);

    let batch_rep: tetherly_node::BatchSendReport =
        serde_json::from_slice(&body_batch).expect("valid BatchSendReport");
    assert!(batch_rep.ok);
    assert!(batch_rep.batch_id.starts_with("tbt_"));
    assert_eq!(batch_rep.total_files, 3);
    assert_eq!(batch_rep.successful_files, 3);
    assert_eq!(batch_rep.failed_files, 0);
    assert_eq!(batch_rep.total_bytes, total_expected_bytes);
    assert_eq!(batch_rep.items.len(), 3);
    for item in &batch_rep.items {
        assert_eq!(item.status, "sent");
        assert!(item.transfer_id.is_some());
        assert!(item.size > 0);
    }

    // 3. Verify Peer receives the 3 incoming file offers via /api/file/offers
    let start_offers = std::time::Instant::now();
    let offers_list = loop {
        let (code_off, body_off) = tetherly_node::loopback_get(peer_ui, "/api/file/offers")
            .await
            .expect("offers query succeeds");
        assert_eq!(code_off, 200);
        let rep: tetherly_node::FileOffersReport = serde_json::from_slice(&body_off).unwrap();
        if rep.count >= 3 {
            break rep.offers;
        }
        if start_offers.elapsed() > Duration::from_secs(10) {
            panic!("peer did not receive 3 offers in time, got {}", rep.count);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert_eq!(offers_list.len(), 3);
    assert!(offers_list.iter().any(|o| o.filename == "document.pdf"));
    assert!(offers_list.iter().any(|o| o.filename == "photo.jpg"));
    assert!(offers_list.iter().any(|o| o.filename == "dataset.csv"));

    // 4. Test Deep Resume Observability via GET /api/file/resume and /api/storage
    let inbox = g2.path().join("inbox");
    std::fs::create_dir_all(&inbox).unwrap();
    let fake_tid = "tfl_resumetest_fake_001";
    let part_file = inbox.join(format!("{fake_tid}.part"));
    let resume_file = inbox.join(format!(".{fake_tid}.resume.json"));
    let partial_data = vec![0x42u8; 131072]; // 128 KiB chunk-aligned
    std::fs::write(&part_file, &partial_data).unwrap();

    let marker_content = serde_json::json!({
        "transfer_id": fake_tid,
        "source": host_id.as_str(),
        "offset": 131072,
        "total_bytes": 262144,
        "last_active": 1700000000,
    });
    std::fs::write(&resume_file, serde_json::to_vec(&marker_content).unwrap()).unwrap();

    // Query GET /api/file/resume
    let (code_res, body_res) = tetherly_node::loopback_get(peer_ui, "/api/file/resume")
        .await
        .expect("resume markers query succeeds");
    assert_eq!(code_res, 200);
    let res_rep: tetherly_node::ResumeMarkersReport =
        serde_json::from_slice(&body_res).expect("valid ResumeMarkersReport");
    assert!(res_rep.ok);
    assert!(res_rep.count >= 1);
    assert!(res_rep.total_partial_bytes >= 131072);

    let marker = res_rep
        .markers
        .iter()
        .find(|m| m.transfer_id == fake_tid)
        .expect("fake resume marker found");
    assert_eq!(marker.source, host_id.as_str());
    assert_eq!(marker.offset_bytes, 131072);
    assert_eq!(marker.total_bytes, 262144);
    assert!((marker.progress_percent - 50.0).abs() < 0.1);
    assert!(marker.chunk_aligned);
    assert!(marker.part_file_exists);
    assert_eq!(marker.part_file_bytes, 131072);

    // Verify GET /api/storage also reflects resume markers count
    let (code_st, body_st) = tetherly_node::loopback_get(peer_ui, "/api/storage")
        .await
        .expect("storage query succeeds");
    assert_eq!(code_st, 200);
    let st_rep: tetherly_node::StorageReport = serde_json::from_slice(&body_st).unwrap();
    assert!(st_rep.resume_markers_count >= 1);

    // 5. Test Loopback Isolation & Host 403 Guard
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut evil_stream = tokio::net::TcpStream::connect(format!("127.0.0.1:{host_ui}"))
        .await
        .expect("connect to host ui port");
    let evil_req = "POST /api/file/send-batch HTTP/1.1\r\nHost: evil.attacker.com\r\nContent-Length: 2\r\n\r\n{}".to_string();
    evil_stream.write_all(evil_req.as_bytes()).await.unwrap();
    evil_stream.flush().await.unwrap();
    let mut evil_buf = vec![0u8; 1024];
    let n = evil_stream.read(&mut evil_buf).await.unwrap();
    let evil_resp = String::from_utf8_lossy(&evil_buf[..n]);
    assert!(
        evil_resp.starts_with("HTTP/1.1 403 Forbidden"),
        "mismatched Host header must be rejected with 403 Forbidden, got: {evil_resp}"
    );

    // 6. Test Error Boundaries and Validation
    eprintln!("[TEST] step 6: testing validation...");
    // a. Empty items & empty paths returns 400 Bad Request
    let empty_payload = serde_json::json!({
        "peer": peer_id.as_str(),
        "paths": [],
        "items": []
    });
    let (code_empty, body_empty) = tetherly_node::loopback_post(
        host_ui,
        "/api/file/send-batch",
        &serde_json::to_vec(&empty_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_empty, 400);
    assert!(String::from_utf8_lossy(&body_empty).contains("no files specified"));

    // b. Disconnected peer returns 400 Bad Request
    let disconnected_peer_id = tetherly_core::DeviceId::from_id_pk(&[0x99u8; 32]).to_string();
    let fake_peer_payload = serde_json::json!({
        "peer": disconnected_peer_id,
        "paths": [file1.to_str().unwrap()]
    });
    let (code_fake, body_fake) = tetherly_node::loopback_post(
        host_ui,
        "/api/file/send-batch",
        &serde_json::to_vec(&fake_peer_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_fake, 400);
    assert!(String::from_utf8_lossy(&body_fake).contains("peer is not connected"));

    // c. Unresolvable peer identifier returns 400 Bad Request
    let unknown_peer_payload = serde_json::json!({
        "peer": "nonexistent_alias_9999",
        "paths": [file1.to_str().unwrap()]
    });
    let (code_unknown, body_unknown) = tetherly_node::loopback_post(
        host_ui,
        "/api/file/send-batch",
        &serde_json::to_vec(&unknown_peer_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_unknown, 400);
    assert!(String::from_utf8_lossy(&body_unknown).contains("bad peer id"));

    // 7. Clean shutdown
    host.shutdown();
    peer.shutdown();
}

/// ADR-053: Inbox file lifecycle governance, multi-dimensional query, safe single deletion, and batch purge.
#[tokio::test(flavor = "multi_thread")]
async fn inbox_file_lifecycle_query_deletion_and_safe_purge_via_loopback_api() {
    let (data_dir, _guard) = temp_dir("inbox-lifecycle-test");
    let node = Node::start(cfg(data_dir.clone(), "inbox-node", "macos")).unwrap();
    let ui_port = node.spawn_ui().await.expect("spawn UI succeeds");

    // Connect to loopback SSE event stream
    let mut sse = tetherly_node::loopback_events_connect(ui_port)
        .await
        .expect("connect loopback sse");

    let inbox_dir = data_dir.join("inbox");
    std::fs::create_dir_all(&inbox_dir).unwrap();

    // 1. Initial empty inbox check via GET /api/file/inbox and /api/inbox alias
    let (c_init, b_init) = tetherly_node::loopback_get(ui_port, "/api/file/inbox")
        .await
        .unwrap();
    assert_eq!(c_init, 200);
    let init_rep: tetherly_node::InboxReport = serde_json::from_slice(&b_init).unwrap();
    assert!(init_rep.ok);
    assert_eq!(init_rep.count, 0);
    assert_eq!(init_rep.total_bytes, 0);
    assert!(init_rep.files.is_empty());

    let (c_alias, b_alias) = tetherly_node::loopback_get(ui_port, "/api/inbox")
        .await
        .unwrap();
    assert_eq!(c_alias, 200);
    let alias_rep: tetherly_node::InboxReport = serde_json::from_slice(&b_alias).unwrap();
    assert_eq!(alias_rep.count, 0);

    // 2. Populate inbox with realistic test files and hidden/sidecar files
    let file1_name = "report_alpha.pdf";
    let file2_name = "photo_vacation.jpg";
    let file3_name = "archive_data.tar.gz";
    let file4_name = "alpha_notes.txt";
    let hidden_part = ".tfl_123.part";
    let hidden_resume = ".tfl_123.resume.json";

    std::fs::write(inbox_dir.join(file1_name), vec![0x01; 1024]).unwrap(); // 1 KiB
    tokio::time::sleep(Duration::from_millis(50)).await;
    std::fs::write(inbox_dir.join(file2_name), vec![0x02; 2048]).unwrap(); // 2 KiB
    tokio::time::sleep(Duration::from_millis(50)).await;
    std::fs::write(inbox_dir.join(file3_name), vec![0x03; 4096]).unwrap(); // 4 KiB
    tokio::time::sleep(Duration::from_millis(50)).await;
    std::fs::write(inbox_dir.join(file4_name), vec![0x04; 512]).unwrap(); // 512 B

    // Hidden sidecars should NOT be listed as inbox user files
    std::fs::write(inbox_dir.join(hidden_part), vec![0xFF; 8192]).unwrap();
    std::fs::write(inbox_dir.join(hidden_resume), b"{}").unwrap();

    let total_user_bytes = 1024 + 2048 + 4096 + 512;

    // 3. Inspect full inbox list
    let (c_full, b_full) = tetherly_node::loopback_get(ui_port, "/api/file/inbox")
        .await
        .unwrap();
    assert_eq!(c_full, 200);
    let full_rep: tetherly_node::InboxReport = serde_json::from_slice(&b_full).unwrap();
    assert_eq!(full_rep.count, 4);
    assert_eq!(full_rep.total_bytes, total_user_bytes);
    assert_eq!(full_rep.files.len(), 4);
    // Verify files sorted descending by modification time (most recent first: file4, file3, file2, file1)
    assert_eq!(full_rep.files[0].name, file4_name);
    assert_eq!(full_rep.files[0].extension.as_deref(), Some("txt"));
    assert!(full_rep.files[0].is_safe);
    assert_eq!(full_rep.files[0].size, 512);

    assert_eq!(full_rep.files[1].name, file3_name);
    assert_eq!(full_rep.files[1].extension.as_deref(), Some("gz"));

    // 4. Multi-dimensional query: substring search and limit
    // Search for "alpha" (case-insensitive substring: should match file4_name and file1_name)
    let (c_search, b_search) = tetherly_node::loopback_get(ui_port, "/api/file/inbox?search=ALPHA")
        .await
        .unwrap();
    assert_eq!(c_search, 200);
    let search_rep: tetherly_node::InboxReport = serde_json::from_slice(&b_search).unwrap();
    assert_eq!(search_rep.count, 2);
    assert_eq!(search_rep.total_bytes, 1024 + 512);
    assert_eq!(search_rep.files.len(), 2);
    assert!(search_rep.files.iter().any(|f| f.name == file1_name));
    assert!(search_rep.files.iter().any(|f| f.name == file4_name));

    // Limit parameter
    let (c_lim, b_lim) = tetherly_node::loopback_get(ui_port, "/api/file/inbox?limit=2")
        .await
        .unwrap();
    assert_eq!(c_lim, 200);
    let lim_rep: tetherly_node::InboxReport = serde_json::from_slice(&b_lim).unwrap();
    assert_eq!(lim_rep.count, 4); // total matched count remains 4
    assert_eq!(lim_rep.files.len(), 2); // truncated to 2

    // 5. Security Red Line: Path traversal guard on delete
    // Malicious filenames with .., /, \, or hidden prefix must return 400 Bad Request
    let evil_payloads = [
        "../outside.txt",
        "../../etc/passwd",
        "/absolute/path/file.txt",
        r"..\windows\path.txt",
        ".hidden_file",
        "",
        "..",
        "folder/file.txt",
    ];
    for evil in evil_payloads {
        let payload = serde_json::json!({ "name": evil });
        let (c_evil, b_evil) = tetherly_node::loopback_post(
            ui_port,
            "/api/file/inbox/delete",
            &serde_json::to_vec(&payload).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(c_evil, 400, "evil path '{evil}' must be rejected with 400");
        let err_msg = String::from_utf8_lossy(&b_evil);
        assert!(
            err_msg.contains("unsafe file name") || err_msg.contains("missing required 'name'"),
            "error response must reject evil path '{evil}', got: {err_msg}"
        );
    }

    // 6. Delete a single file safely via POST /api/file/inbox/delete
    let del_payload = serde_json::json!({ "name": file2_name });
    let (c_del, b_del) = tetherly_node::loopback_post(
        ui_port,
        "/api/file/inbox/delete",
        &serde_json::to_vec(&del_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c_del, 200);
    let del_rep: tetherly_node::InboxDeleteReport = serde_json::from_slice(&b_del).unwrap();
    assert!(del_rep.ok);
    assert_eq!(del_rep.name, file2_name);
    assert_eq!(del_rep.freed_bytes, 2048);

    // Verify physical file is deleted on disk
    assert!(!inbox_dir.join(file2_name).exists());

    // Verify SSE receives UiEvent::InboxFileDeleted
    let sse_ev1 = tokio::time::timeout(Duration::from_secs(5), sse.next_ui_event())
        .await
        .expect("timeout waiting for sse inbox file deleted event")
        .expect("read sse event")
        .expect("some ui event");
    match sse_ev1 {
        tetherly_node::UiEvent::InboxFileDeleted {
            name, freed_bytes, ..
        } => {
            assert_eq!(name, file2_name);
            assert_eq!(freed_bytes, 2048);
        }
        other => panic!("expected InboxFileDeleted, got: {other:?}"),
    }

    // Deleting a non-existent file returns 404
    let (c_404, _) = tetherly_node::loopback_post(
        ui_port,
        "/api/file/inbox/delete",
        &serde_json::to_vec(&del_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c_404, 404);

    // 7. Clear all remaining files via POST /api/file/inbox/clear
    let (c_clr, b_clr) = tetherly_node::loopback_post(ui_port, "/api/file/inbox/clear", b"{}")
        .await
        .unwrap();
    assert_eq!(c_clr, 200);
    let clr_rep: tetherly_node::InboxClearReport = serde_json::from_slice(&b_clr).unwrap();
    assert!(clr_rep.ok);
    assert_eq!(clr_rep.deleted_count, 3); // file1, file3, file4
    assert_eq!(clr_rep.freed_bytes, 1024 + 4096 + 512);

    // Verify all 3 user files deleted from disk, but hidden files (.part, .resume.json) untouched
    assert!(!inbox_dir.join(file1_name).exists());
    assert!(!inbox_dir.join(file3_name).exists());
    assert!(!inbox_dir.join(file4_name).exists());
    assert!(inbox_dir.join(hidden_part).exists());
    assert!(inbox_dir.join(hidden_resume).exists());

    // Verify SSE receives UiEvent::InboxCleared
    let mut saw_inbox_cleared = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline {
        if let Ok(Ok(Some(tetherly_node::UiEvent::InboxCleared {
            deleted_count,
            freed_bytes,
            ..
        }))) = tokio::time::timeout(Duration::from_millis(500), sse.next_ui_event()).await
        {
            assert_eq!(deleted_count, 3);
            assert_eq!(freed_bytes, 1024 + 4096 + 512);
            saw_inbox_cleared = true;
            break;
        }
    }
    assert!(saw_inbox_cleared, "expected InboxCleared");

    // 8. Loopback Isolation: Host 403 Forbidden check
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut evil_stream = tokio::net::TcpStream::connect(format!("127.0.0.1:{ui_port}"))
        .await
        .expect("connect to ui port");
    let evil_req = "GET /api/file/inbox HTTP/1.1\r\nHost: evil.attacker.com\r\n\r\n";
    evil_stream.write_all(evil_req.as_bytes()).await.unwrap();
    evil_stream.flush().await.unwrap();
    let mut evil_buf = vec![0u8; 1024];
    let n = evil_stream.read(&mut evil_buf).await.unwrap();
    let evil_resp = String::from_utf8_lossy(&evil_buf[..n]);
    assert!(
        evil_resp.starts_with("HTTP/1.1 403 Forbidden"),
        "mismatched Host header must be rejected with 403 Forbidden, got: {evil_resp}"
    );

    node.shutdown();
}

/// ADR-054: LAN device discovery, beacon governance, smart dial resolution and loopback isolation.
#[tokio::test(flavor = "multi_thread")]
async fn lan_discovery_and_beacon_governance_via_loopback_api() {
    let (data_dir, _guard) = temp_dir("discovery-test");
    let node = Node::start(cfg(data_dir.clone(), "discovery-node", "macos")).unwrap();
    let ui_port = node.spawn_ui().await.expect("spawn UI succeeds");

    // Connect to loopback SSE event stream
    let mut sse = tetherly_node::loopback_events_connect(ui_port)
        .await
        .expect("connect loopback sse");

    // 1. Initial discovery report check via GET /api/discovery and /api/peers/discovered alias
    let (c_init, b_init) = tetherly_node::loopback_get(ui_port, "/api/discovery")
        .await
        .unwrap();
    assert_eq!(c_init, 200);
    let init_rep: tetherly_node::DiscoveryReport = serde_json::from_slice(&b_init).unwrap();
    assert!(init_rep.ok);
    assert_eq!(init_rep.count, 0);
    assert!(init_rep.peers.is_empty());

    let (c_alias, b_alias) = tetherly_node::loopback_get(ui_port, "/api/peers/discovered")
        .await
        .unwrap();
    assert_eq!(c_alias, 200);
    let alias_rep: tetherly_node::DiscoveryReport = serde_json::from_slice(&b_alias).unwrap();
    assert!(alias_rep.ok);
    assert_eq!(alias_rep.count, 0);

    // 2. Trigger active rescan via POST /api/discovery/rescan and /api/peers/discover alias
    let (c_rescan, b_rescan) =
        tetherly_node::loopback_post(ui_port, "/api/discovery/rescan", b"{}")
            .await
            .unwrap();
    assert_eq!(c_rescan, 200);
    let rescan_rep: tetherly_node::DiscoveryRescanReport =
        serde_json::from_slice(&b_rescan).unwrap();
    assert!(rescan_rep.ok);
    assert!(rescan_rep.message.contains("rescan initiated"));

    let (c_rescan_alias, b_rescan_alias) =
        tetherly_node::loopback_post(ui_port, "/api/peers/discover", b"{}")
            .await
            .unwrap();
    assert_eq!(c_rescan_alias, 200);
    let rescan_alias_rep: tetherly_node::DiscoveryRescanReport =
        serde_json::from_slice(&b_rescan_alias).unwrap();
    assert!(rescan_alias_rep.ok);

    // 3. Simulate discovered peers and verify Zero-Plaintext SSE events
    // Self device should be filtered out
    let self_id = node.identity().device_id().to_string();
    node.record_discovered_peer(
        &self_id,
        "self-node",
        &["127.0.0.1"],
        45717,
        None,
        &["file"],
    );
    assert_eq!(node.discovered_peers().count, 0);

    // Peer 1: MacBook Pro
    let dev1_id = "tdev_macbook_pro_16";
    let dev1_name = "MacBook Pro M3";
    let dev1_addrs = ["127.0.0.1"];
    let dev1_port = 45999;
    let dev1_fp = "fp_mbp16_abc";
    let dev1_caps = ["file", "clipboard", "screen"];
    node.record_discovered_peer(
        dev1_id,
        dev1_name,
        &dev1_addrs,
        dev1_port,
        Some(dev1_fp),
        &dev1_caps,
    );

    // Verify SSE receives PeerDiscovered event
    let sse_ev1 = tokio::time::timeout(Duration::from_secs(5), sse.next_ui_event())
        .await
        .expect("timeout waiting for sse peer discovered event")
        .expect("read sse event")
        .expect("some ui event");
    match sse_ev1 {
        tetherly_node::UiEvent::PeerDiscovered {
            device_id,
            name,
            addr,
            caps,
        } => {
            assert_eq!(device_id, dev1_id);
            assert_eq!(name, dev1_name);
            assert_eq!(addr, "127.0.0.1:45999");
            assert_eq!(caps, vec!["file", "clipboard", "screen"]);
        }
        other => panic!("expected PeerDiscovered, got: {other:?}"),
    }

    // Peer 2: iPhone
    let dev2_id = "tdev_iphone_15_pro";
    let dev2_name = "iPhone 15 Pro";
    let dev2_addrs = ["127.0.0.1"];
    let dev2_port = 45998;
    let dev2_caps = ["ancs", "notify"];
    node.record_discovered_peer(dev2_id, dev2_name, &dev2_addrs, dev2_port, None, &dev2_caps);

    let sse_ev2 = tokio::time::timeout(Duration::from_secs(5), sse.next_ui_event())
        .await
        .expect("timeout waiting for sse peer discovered event 2")
        .expect("read sse event")
        .expect("some ui event");
    match sse_ev2 {
        tetherly_node::UiEvent::PeerDiscovered {
            device_id,
            name,
            addr,
            caps,
        } => {
            assert_eq!(device_id, dev2_id);
            assert_eq!(name, dev2_name);
            assert_eq!(addr, "127.0.0.1:45998");
            assert_eq!(caps, vec!["ancs", "notify"]);
        }
        other => panic!("expected PeerDiscovered, got: {other:?}"),
    }

    // 4. Query GET /api/discovery and verify payload structure
    let (c_list, b_list) = tetherly_node::loopback_get(ui_port, "/api/discovery")
        .await
        .unwrap();
    assert_eq!(c_list, 200);
    let list_rep: tetherly_node::DiscoveryReport = serde_json::from_slice(&b_list).unwrap();
    assert!(list_rep.ok);
    assert_eq!(list_rep.count, 2);
    assert_eq!(list_rep.peers.len(), 2);

    let p1 = list_rep
        .peers
        .iter()
        .find(|p| p.device_id == dev1_id)
        .expect("find p1");
    assert_eq!(p1.name, dev1_name);
    assert_eq!(p1.port, dev1_port);
    assert_eq!(p1.fp.as_deref(), Some(dev1_fp));
    assert_eq!(p1.addrs, vec!["127.0.0.1"]);
    assert_eq!(p1.caps, vec!["file", "clipboard", "screen"]);
    assert!(!p1.is_trusted);
    assert!(!p1.is_online);

    // 5. Smart dial resolution testing via POST /api/dial
    // 5a. Bad json / missing both addr and peer -> 400
    let (c_empty, b_empty) = tetherly_node::loopback_post(ui_port, "/api/dial", b"{}")
        .await
        .unwrap();
    assert_eq!(c_empty, 400);
    assert!(String::from_utf8_lossy(&b_empty).contains("missing addr or peer"));

    // 5b. Unresolvable target peer name -> 400
    let dial_bad = serde_json::json!({ "peer": "non_existent_device" });
    let (c_bad, b_bad) = tetherly_node::loopback_post(
        ui_port,
        "/api/dial",
        &serde_json::to_vec(&dial_bad).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c_bad, 400);
    let err_msg = String::from_utf8_lossy(&b_bad);
    assert!(
        err_msg.contains("unable to resolve or reach peer target 'non_existent_device'"),
        "unexpected error message: {err_msg}"
    );

    // 5c. Resolvable target by device_id or name (attempts dial to candidate IPs)
    let dial_dev = serde_json::json!({ "peer": dev1_id });
    let (c_dial_dev, b_dial_dev) = tetherly_node::loopback_post(
        ui_port,
        "/api/dial",
        &serde_json::to_vec(&dial_dev).unwrap(),
    )
    .await
    .unwrap();
    // Since 192.168.1.105 is not listening in test environment, dial will return error from dial attempt
    assert_eq!(c_dial_dev, 400);
    let dial_err = String::from_utf8_lossy(&b_dial_dev);
    assert!(
        dial_err.contains("unable to resolve or reach peer target")
            || dial_err.contains("connection")
            || dial_err.contains("refused")
            || dial_err.contains("unreachable"),
        "expected reachability failure, got: {dial_err}"
    );

    // 6. Test beacon expiration and SSE event
    let expired = node.expire_discovered_peer(dev2_id);
    assert!(expired);

    let sse_exp = tokio::time::timeout(Duration::from_secs(5), sse.next_ui_event())
        .await
        .expect("timeout waiting for sse peer expired event")
        .expect("read sse event")
        .expect("some ui event");
    match sse_exp {
        tetherly_node::UiEvent::PeerExpired { device_id } => {
            assert_eq!(device_id, dev2_id);
        }
        other => panic!("expected PeerExpired, got: {other:?}"),
    }

    // Verify list count drops to 1
    assert_eq!(node.discovered_peers().count, 1);

    // 7. Security Red Line: Loopback Isolation Host 403 Forbidden check
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut evil_stream = tokio::net::TcpStream::connect(format!("127.0.0.1:{ui_port}"))
        .await
        .expect("connect to ui port");
    let evil_req = "GET /api/discovery HTTP/1.1\r\nHost: evil.attacker.com\r\n\r\n";
    evil_stream.write_all(evil_req.as_bytes()).await.unwrap();
    evil_stream.flush().await.unwrap();
    let mut evil_buf = vec![0u8; 1024];
    let n = evil_stream.read(&mut evil_buf).await.unwrap();
    let evil_resp = String::from_utf8_lossy(&evil_buf[..n]);
    assert!(
        evil_resp.starts_with("HTTP/1.1 403 Forbidden"),
        "mismatched Host header must be rejected with 403 Forbidden, got: {evil_resp}"
    );

    node.shutdown();
}

/// ADR-055: Inbox content extraction, binary-safe download, media raw preview, native open & loopback isolation.
#[tokio::test(flavor = "multi_thread")]
async fn inbox_file_download_raw_preview_and_native_open_via_loopback_api() {
    let (data_dir, _guard) = temp_dir("inbox-download-test");
    let node = Node::start(cfg(data_dir.clone(), "inbox-download-node", "macos")).unwrap();
    let ui_port = node.spawn_ui().await.expect("spawn UI succeeds");

    // Inject MemoryOpener to intercept system open calls without side effects
    let opener = tetherly_node::MemoryOpener::new();
    node.set_opener(Arc::new(opener.clone()));

    // Connect to loopback SSE event stream
    let mut sse = tetherly_node::loopback_events_connect(ui_port)
        .await
        .expect("connect loopback sse");

    // 1. Prepare sample files in <data_dir>/inbox
    let inbox_path = data_dir.join("inbox");
    std::fs::create_dir_all(&inbox_path).unwrap();

    let txt_name = "report.txt";
    let txt_bytes = b"Hello Tetherly Inbox Content Delivery!";
    std::fs::write(inbox_path.join(txt_name), txt_bytes).unwrap();

    let png_name = "screenshot.png";
    // Binary PNG bytes containing non-UTF-8 sequences to verify binary fidelity
    let png_bytes = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\x00\x00\x00\x01\x00\x00\x00\x01\x08\x06\x00\x00\x00\x1f\x15\xc4\x89\xaa\xbb\xcc\xdd\xee\xff";
    std::fs::write(inbox_path.join(png_name), png_bytes).unwrap();

    let json_name = "data.json";
    let json_bytes = br#"{"system": "tetherly", "version": "1.62", "safe": true}"#;
    std::fs::write(inbox_path.join(json_name), json_bytes).unwrap();

    // 2. Binary-Safe Streaming Download: GET /api/file/inbox/download
    // 2a. Download text file and verify body
    let (c_txt, b_txt) = tetherly_node::loopback_get(
        ui_port,
        &format!("/api/file/inbox/download?name={txt_name}"),
    )
    .await
    .unwrap();
    assert_eq!(c_txt, 200);
    assert_eq!(b_txt, txt_bytes);

    // 2b. Download binary PNG via alias /api/inbox/download and verify byte-for-byte fidelity
    let (c_png, b_png) =
        tetherly_node::loopback_get(ui_port, &format!("/api/inbox/download?name={png_name}"))
            .await
            .unwrap();
    assert_eq!(c_png, 200);
    assert_eq!(
        b_png, png_bytes,
        "binary download must maintain 100% byte fidelity"
    );

    // 2c. Direct TCP request to verify Content-Disposition: attachment and MIME headers
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut tcp_stream = tokio::net::TcpStream::connect(format!("127.0.0.1:{ui_port}"))
        .await
        .expect("connect to ui port");
    let req = format!("GET /api/file/inbox/download?name={png_name} HTTP/1.1\r\nHost: 127.0.0.1:{ui_port}\r\n\r\n");
    tcp_stream.write_all(req.as_bytes()).await.unwrap();
    tcp_stream.flush().await.unwrap();
    let mut resp_buf = vec![0u8; 4096];
    let n = tcp_stream.read(&mut resp_buf).await.unwrap();
    let resp_str = String::from_utf8_lossy(&resp_buf[..n]);
    assert!(resp_str.starts_with("HTTP/1.1 200 OK"));
    assert!(resp_str.contains("Content-Type: image/png"));
    assert!(resp_str.contains(&format!(
        "Content-Disposition: attachment; filename=\"{png_name}\""
    )));

    // 3. Raw Media / Content Preview: GET /api/file/inbox/raw
    // 3a. Read raw JSON
    let (c_raw_json, b_raw_json) =
        tetherly_node::loopback_get(ui_port, &format!("/api/file/inbox/raw?name={json_name}"))
            .await
            .unwrap();
    assert_eq!(c_raw_json, 200);
    assert_eq!(b_raw_json, json_bytes);

    // 3b. Direct TCP request to verify Content-Disposition: inline
    let mut tcp_stream2 = tokio::net::TcpStream::connect(format!("127.0.0.1:{ui_port}"))
        .await
        .expect("connect to ui port");
    let req2 = format!(
        "GET /api/file/inbox/raw?name={json_name} HTTP/1.1\r\nHost: 127.0.0.1:{ui_port}\r\n\r\n"
    );
    tcp_stream2.write_all(req2.as_bytes()).await.unwrap();
    tcp_stream2.flush().await.unwrap();
    let mut resp_buf2 = vec![0u8; 4096];
    let n2 = tcp_stream2.read(&mut resp_buf2).await.unwrap();
    let resp_str2 = String::from_utf8_lossy(&resp_buf2[..n2]);
    assert!(resp_str2.starts_with("HTTP/1.1 200 OK"));
    assert!(resp_str2.contains("Content-Type: application/json"));
    assert!(resp_str2.contains(&format!(
        "Content-Disposition: inline; filename=\"{json_name}\""
    )));

    // 4. Native Open via POST /api/file/inbox/open
    // 4a. Open specific file
    let open_file_payload = serde_json::json!({ "name": txt_name });
    let (c_open_file, b_open_file) = tetherly_node::loopback_post(
        ui_port,
        "/api/file/inbox/open",
        &serde_json::to_vec(&open_file_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c_open_file, 200);
    let open_file_rep: tetherly_node::InboxOpenReport =
        serde_json::from_slice(&b_open_file).unwrap();
    assert!(open_file_rep.ok);
    assert_eq!(open_file_rep.name.as_deref(), Some(txt_name));
    assert!(open_file_rep.path.ends_with(txt_name));

    // Verify MemoryOpener recorded opened path
    let opened_urls = opener.opened();
    assert_eq!(opened_urls.len(), 1);
    assert!(opened_urls[0].ends_with(txt_name));

    // Verify SSE receives UiEvent::InboxFileOpened with Zero-Plaintext sanitized metadata
    let sse_ev1 = tokio::time::timeout(Duration::from_secs(5), sse.next_ui_event())
        .await
        .expect("timeout waiting for sse inbox file opened event")
        .expect("read sse event")
        .expect("some ui event");
    match sse_ev1 {
        tetherly_node::UiEvent::InboxFileOpened { name, timestamp_ms } => {
            assert_eq!(name.as_deref(), Some(txt_name));
            assert!(timestamp_ms > 0);
        }
        other => panic!("expected InboxFileOpened, got: {other:?}"),
    }

    // 4b. Open inbox directory itself (name: None) via alias POST /api/inbox/open
    let (c_open_dir, b_open_dir) = tetherly_node::loopback_post(ui_port, "/api/inbox/open", b"{}")
        .await
        .unwrap();
    assert_eq!(c_open_dir, 200);
    let open_dir_rep: tetherly_node::InboxOpenReport = serde_json::from_slice(&b_open_dir).unwrap();
    assert!(open_dir_rep.ok);
    assert_eq!(open_dir_rep.name, None);
    assert!(open_dir_rep.path.ends_with("inbox"));

    // Verify MemoryOpener recorded opened directory
    let opened_urls2 = opener.opened();
    assert_eq!(opened_urls2.len(), 2);
    assert!(opened_urls2[1].ends_with("inbox"));

    // Verify SSE receives UiEvent::InboxFileOpened with name: None
    let mut saw_open_dir = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline {
        if let Ok(Ok(Some(tetherly_node::UiEvent::InboxFileOpened { name, timestamp_ms }))) =
            tokio::time::timeout(Duration::from_millis(500), sse.next_ui_event()).await
        {
            if name.is_none() {
                assert!(timestamp_ms > 0);
                saw_open_dir = true;
                break;
            }
        }
    }
    assert!(saw_open_dir, "expected InboxFileOpened for dir");

    // 5. Security Red Line: Path traversal guard on download, raw and open
    let evil_payloads = [
        "../outside.txt",
        "../../etc/passwd",
        "/absolute/file.txt",
        r"..\windows\path.txt",
        ".hidden_file",
        "..",
        "sub/folder.txt",
    ];
    for evil in evil_payloads {
        // Download traversal check -> 400 Bad Request
        let encoded_evil = tetherly_cli_test_url_encode(evil);
        let (c_dl_evil, _) = tetherly_node::loopback_get(
            ui_port,
            &format!("/api/file/inbox/download?name={encoded_evil}"),
        )
        .await
        .unwrap();
        assert_eq!(
            c_dl_evil, 400,
            "download with evil path '{evil}' must return 400"
        );

        // Raw traversal check -> 400 Bad Request
        let (c_raw_evil, _) = tetherly_node::loopback_get(
            ui_port,
            &format!("/api/file/inbox/raw?name={encoded_evil}"),
        )
        .await
        .unwrap();
        assert_eq!(
            c_raw_evil, 400,
            "raw preview with evil path '{evil}' must return 400"
        );

        // Open traversal check -> 400 Bad Request
        let evil_open_req = serde_json::json!({ "name": evil });
        let (c_open_evil, _) = tetherly_node::loopback_post(
            ui_port,
            "/api/file/inbox/open",
            &serde_json::to_vec(&evil_open_req).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(
            c_open_evil, 400,
            "open with evil path '{evil}' must return 400"
        );
    }

    // 6. Non-existent file error handling -> 404 Not Found
    let (c_dl_404, _) = tetherly_node::loopback_get(
        ui_port,
        "/api/file/inbox/download?name=non_existent_file.pdf",
    )
    .await
    .unwrap();
    assert_eq!(c_dl_404, 404);

    let (c_raw_404, _) =
        tetherly_node::loopback_get(ui_port, "/api/file/inbox/raw?name=non_existent_file.pdf")
            .await
            .unwrap();
    assert_eq!(c_raw_404, 404);

    let not_found_req = serde_json::json!({ "name": "non_existent_file.pdf" });
    let (c_open_404, _) = tetherly_node::loopback_post(
        ui_port,
        "/api/file/inbox/open",
        &serde_json::to_vec(&not_found_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c_open_404, 404);

    // 7. Security Red Line: Loopback Isolation Host 403 Forbidden check
    let mut evil_stream = tokio::net::TcpStream::connect(format!("127.0.0.1:{ui_port}"))
        .await
        .expect("connect to ui port");
    let evil_req =
        "GET /api/file/inbox/download?name=report.txt HTTP/1.1\r\nHost: evil.attacker.com\r\n\r\n";
    evil_stream.write_all(evil_req.as_bytes()).await.unwrap();
    evil_stream.flush().await.unwrap();
    let mut evil_buf = vec![0u8; 1024];
    let n = evil_stream.read(&mut evil_buf).await.unwrap();
    let evil_resp = String::from_utf8_lossy(&evil_buf[..n]);
    assert!(
        evil_resp.starts_with("HTTP/1.1 403 Forbidden"),
        "mismatched Host header must be rejected with 403 Forbidden, got: {evil_resp}"
    );

    node.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn node_system_telemetry_probe_injection_and_metrics_via_loopback_api() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let (data_dir, _guard) = temp_dir("sys-telemetry");
    let node = Node::start(cfg(data_dir.clone(), "sys-node", "macos")).unwrap();
    let ui_port = node.spawn_ui().await.unwrap();
    assert_ne!(ui_port, 0, "test node must bind loopback UI");

    // 1. Direct PlatformSysProbe report test
    let rep_native = node.system_report().expect("native system_report");
    assert!(rep_native.ok);
    assert_eq!(rep_native.node_name, "sys-node");
    assert_eq!(rep_native.process.pid, std::process::id());
    assert!(rep_native.process.thread_count >= 1);
    assert!(rep_native.host.num_cpus >= 1);
    assert_eq!(rep_native.host.os, std::env::consts::OS);
    assert_eq!(rep_native.host.arch, std::env::consts::ARCH);

    // 2. Deterministic test mocking via MemorySysProbe
    let mock_probe = tetherly_node::MemorySysProbe::default();
    mock_probe.set_process(tetherly_node::ProcessSample {
        rss_bytes: 42 * 1024 * 1024,
        virt_bytes: 128 * 1024 * 1024,
        thread_count: 12,
        open_fds: Some(36),
        cpu_time_ms: 54321,
    });
    mock_probe.set_host(tetherly_node::HostSample {
        total_memory_bytes: Some(32 * 1024 * 1024 * 1024),
    });
    node.set_sys_probe(Arc::new(mock_probe));

    let mock_rep = node.system_report().expect("mock system_report");
    assert_eq!(mock_rep.process.rss_bytes, 42 * 1024 * 1024);
    assert_eq!(mock_rep.process.virt_bytes, 128 * 1024 * 1024);
    assert_eq!(mock_rep.process.thread_count, 12);
    assert_eq!(mock_rep.process.open_fds, Some(36));
    assert_eq!(mock_rep.process.cpu_time_ms, 54321);
    assert_eq!(mock_rep.process.memory_rss_formatted, "42.00 MiB");
    assert_eq!(mock_rep.process.memory_virt_formatted, "128.00 MiB");
    assert_eq!(
        mock_rep.host.total_memory_bytes,
        Some(32 * 1024 * 1024 * 1024)
    );
    assert_eq!(
        mock_rep.host.total_memory_formatted,
        Some("32.00 GiB".to_string())
    );

    // 3. Connect to SSE stream
    let mut sse = tetherly_node::loopback_events_connect(ui_port)
        .await
        .expect("connect loopback sse");

    // 4. Query /api/sys via loopback HTTP
    let (code, resp) = tetherly_node::loopback_get(ui_port, "/api/sys")
        .await
        .expect("get /api/sys");
    assert_eq!(code, 200);
    let json_val: serde_json::Value =
        serde_json::from_slice(&resp).expect("parse /api/sys response");
    assert_eq!(json_val["ok"], true);
    assert_eq!(json_val["node_name"], "sys-node");
    assert_eq!(json_val["process"]["rss_bytes"], 42 * 1024 * 1024);
    assert_eq!(json_val["process"]["virt_bytes"], 128 * 1024 * 1024);
    assert_eq!(json_val["process"]["thread_count"], 12);
    assert_eq!(json_val["process"]["open_fds"], 36);
    assert_eq!(json_val["process"]["cpu_time_ms"], 54321);
    assert_eq!(
        json_val["host"]["total_memory_bytes"],
        32 * 1024 * 1024 * 1024u64
    );

    // 5. Query /api/system and /api/process aliases
    let (c_sys, _) = tetherly_node::loopback_get(ui_port, "/api/system")
        .await
        .expect("get /api/system");
    assert_eq!(c_sys, 200);

    let (c_proc, _) = tetherly_node::loopback_get(ui_port, "/api/process")
        .await
        .expect("get /api/process");
    assert_eq!(c_proc, 200);

    // 6. Verify SSE event broadcast (UiEvent::SystemMetricsRefreshed)
    let ev = tokio::time::timeout(Duration::from_secs(5), sse.next_ui_event())
        .await
        .expect("timeout waiting for sse")
        .expect("read event")
        .expect("some event");
    match ev {
        tetherly_node::UiEvent::SystemMetricsRefreshed {
            pid,
            rss_bytes,
            virt_bytes,
            thread_count,
            open_fds,
            cpu_time_ms,
            num_cpus,
            total_memory_bytes,
            ..
        } => {
            assert_eq!(pid, std::process::id());
            assert_eq!(rss_bytes, 42 * 1024 * 1024);
            assert_eq!(virt_bytes, 128 * 1024 * 1024);
            assert_eq!(thread_count, 12);
            assert_eq!(open_fds, Some(36));
            assert_eq!(cpu_time_ms, 54321);
            assert!(num_cpus >= 1);
            assert_eq!(total_memory_bytes, Some(32 * 1024 * 1024 * 1024));
        }
        other => panic!("expected SystemMetricsRefreshed, got: {other:?}"),
    }

    // 7. Security Red Line: Loopback Isolation Host 403 Forbidden check
    let mut evil_stream = tokio::net::TcpStream::connect(format!("127.0.0.1:{ui_port}"))
        .await
        .expect("connect to ui port");
    let evil_req = "GET /api/sys HTTP/1.1\r\nHost: evil.attacker.com\r\n\r\n";
    evil_stream.write_all(evil_req.as_bytes()).await.unwrap();
    evil_stream.flush().await.unwrap();
    let mut evil_buf = vec![0u8; 1024];
    let n = evil_stream.read(&mut evil_buf).await.unwrap();
    let evil_resp = String::from_utf8_lossy(&evil_buf[..n]);
    assert!(
        evil_resp.starts_with("HTTP/1.1 403 Forbidden"),
        "mismatched Host header must be rejected with 403 Forbidden, got: {evil_resp}"
    );

    node.shutdown();
}

/// ADR-058: Peer connection session keep-alive heartbeat governance, dynamic ping interval tuning,
/// multi-dimensional link quality profiling (min/max/avg RTT, jitter, loss rate) and active burst probing.
#[tokio::test(flavor = "multi_thread")]
async fn peer_link_quality_profiling_keepalive_tuning_and_burst_probe_via_loopback_api() {
    let (host, peer, _g1, _g2) = pair_phone_desktop().await;
    let host_ui = host.spawn_ui().await.expect("host UI spawns");
    let peer_id = peer.identity().device_id().clone();

    // 1. Connect to SSE stream on host UI
    let mut sse = tetherly_node::loopback_events_connect(host_ui)
        .await
        .expect("SSE connects");

    // 2. Query default heartbeat config via GET /api/peers/heartbeat and alias /api/heartbeat
    let (code_hb, body_hb) = tetherly_node::loopback_get(host_ui, "/api/peers/heartbeat")
        .await
        .unwrap();
    assert_eq!(code_hb, 200);
    let hb_cfg: tetherly_node::HeartbeatConfig = serde_json::from_slice(&body_hb).unwrap();
    assert_eq!(hb_cfg.interval_secs, 15);
    assert_eq!(hb_cfg.deadline_secs, 45);

    let (code_hb_alias, _) = tetherly_node::loopback_get(host_ui, "/api/heartbeat")
        .await
        .unwrap();
    assert_eq!(code_hb_alias, 200);

    // 3. Update heartbeat config via POST /api/peers/heartbeat
    let update_hb = serde_json::json!({
        "interval_secs": 20,
        "deadline_secs": 60,
    });
    let (code_hb_post, body_hb_post) = tetherly_node::loopback_post(
        host_ui,
        "/api/peers/heartbeat",
        &serde_json::to_vec(&update_hb).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_hb_post, 200);
    let updated_cfg: tetherly_node::HeartbeatConfig =
        serde_json::from_slice(&body_hb_post).unwrap();
    assert_eq!(updated_cfg.interval_secs, 20);
    assert_eq!(updated_cfg.deadline_secs, 60);

    // Verify SSE receives HeartbeatConfigChanged
    let mut saw_hb_event = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while tokio::time::Instant::now() < deadline {
        if let Ok(Ok(Some(UiEvent::HeartbeatConfigChanged {
            interval_secs,
            deadline_secs,
            ..
        }))) = tokio::time::timeout(Duration::from_millis(500), sse.next_ui_event()).await
        {
            if interval_secs == 20 && deadline_secs == 60 {
                saw_hb_event = true;
                break;
            }
        }
    }
    assert!(saw_hb_event, "must emit HeartbeatConfigChanged over SSE");

    // 4. Negative validation on heartbeat config:
    // a. deadline <= interval returns 400
    let bad_hb_1 = serde_json::json!({
        "interval_secs": 30,
        "deadline_secs": 30,
    });
    let (code_bad_1, _) = tetherly_node::loopback_post(
        host_ui,
        "/api/peers/heartbeat",
        &serde_json::to_vec(&bad_hb_1).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_bad_1, 400);

    // b. interval == 0 returns 400
    let bad_hb_2 = serde_json::json!({
        "interval_secs": 0,
        "deadline_secs": 30,
    });
    let (code_bad_2, _) = tetherly_node::loopback_post(
        host_ui,
        "/api/peers/heartbeat",
        &serde_json::to_vec(&bad_hb_2).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_bad_2, 400);

    // 5. Query initial link quality report via GET /api/peers/link
    let (code_link, body_link) = tetherly_node::loopback_get(host_ui, "/api/peers/link")
        .await
        .unwrap();
    assert_eq!(code_link, 200);
    let link_rep: tetherly_node::LinkQualityReport = serde_json::from_slice(&body_link).unwrap();
    assert_eq!(link_rep.links.len(), 1);
    assert_eq!(link_rep.links[0].device_id, peer_id.as_str());
    assert!(link_rep.links[0].is_online);

    // 6. Run on-demand multi-packet burst probe via POST /api/peers/link/probe
    let probe_payload = serde_json::json!({
        "peer": peer_id.as_str(),
        "count": 4,
        "interval_ms": 25,
    });
    let (code_probe, body_probe) = tetherly_node::loopback_post(
        host_ui,
        "/api/peers/link/probe",
        &serde_json::to_vec(&probe_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_probe, 200);
    let probe_rep: tetherly_node::LinkProbeReport = serde_json::from_slice(&body_probe).unwrap();
    assert!(probe_rep.ok);
    assert_eq!(probe_rep.peer, peer_id.as_str());
    assert_eq!(probe_rep.sent, 4);
    assert_eq!(probe_rep.received, 4);
    assert_eq!(probe_rep.packet_loss_rate, 0.0);
    assert!(probe_rep.avg_rtt_ms.is_some());
    assert_eq!(probe_rep.grade, tetherly_node::LinkGrade::Excellent);

    // 7. Verify SSE receives PeerLinkQualityRefreshed
    let mut saw_link_event = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while tokio::time::Instant::now() < deadline {
        if let Ok(Ok(Some(UiEvent::PeerLinkQualityRefreshed {
            device_id: ev_peer,
            grade,
            packet_loss_rate,
            avg_rtt_ms,
            ..
        }))) = tokio::time::timeout(Duration::from_millis(500), sse.next_ui_event()).await
        {
            if ev_peer == peer_id.as_str() {
                assert_eq!(grade, "excellent");
                assert_eq!(packet_loss_rate, 0.0);
                assert!(avg_rtt_ms.is_some());
                saw_link_event = true;
                break;
            }
        }
    }
    assert!(
        saw_link_event,
        "must emit PeerLinkQualityRefreshed over SSE"
    );

    // 8. Query updated link quality report to verify sliding-window metrics
    let (code_link_after, body_link_after) =
        tetherly_node::loopback_get(host_ui, "/api/peers/link")
            .await
            .unwrap();
    assert_eq!(code_link_after, 200);
    let link_after: tetherly_node::LinkQualityReport =
        serde_json::from_slice(&body_link_after).unwrap();
    let peer_stat = &link_after.links[0];
    assert!(peer_stat.pings_sent >= 4);
    assert!(peer_stat.pongs_recv >= 4);
    assert_eq!(peer_stat.grade, tetherly_node::LinkGrade::Excellent);
    assert!(peer_stat.avg_rtt_ms.is_some());
    assert!(peer_stat.min_rtt_ms.is_some());
    assert!(peer_stat.max_rtt_ms.is_some());

    // 9. Test probe with URL query parameters
    let (code_qprobe, body_qprobe) = tetherly_node::loopback_post(
        host_ui,
        &format!("/api/peers/link/probe?peer={peer_id}&count=2&interval_ms=20"),
        b"{}",
    )
    .await
    .unwrap();
    assert_eq!(code_qprobe, 200);
    let qrep: tetherly_node::LinkProbeReport = serde_json::from_slice(&body_qprobe).unwrap();
    assert_eq!(qrep.sent, 2);
    assert_eq!(qrep.received, 2);

    // 10. Negative probe validation: non-existent peer returns 400
    let non_peer = serde_json::json!({
        "peer": "tdev_nonexistent99999999",
        "count": 3,
    });
    let (code_probe_err, _) = tetherly_node::loopback_post(
        host_ui,
        "/api/peers/link/probe",
        &serde_json::to_vec(&non_peer).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_probe_err, 400);

    // 11. Security Red Line: Loopback Isolation Host 403 Forbidden check
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut evil_stream = tokio::net::TcpStream::connect(format!("127.0.0.1:{host_ui}"))
        .await
        .expect("connect to ui port");
    let evil_req = "GET /api/peers/link HTTP/1.1\r\nHost: evil.attacker.com\r\n\r\n";
    evil_stream.write_all(evil_req.as_bytes()).await.unwrap();
    evil_stream.flush().await.unwrap();
    let mut evil_buf = vec![0u8; 1024];
    let n = evil_stream.read(&mut evil_buf).await.unwrap();
    let evil_resp = String::from_utf8_lossy(&evil_buf[..n]);
    assert!(
        evil_resp.starts_with("HTTP/1.1 403 Forbidden"),
        "mismatched Host header must be rejected with 403 Forbidden, got: {evil_resp}"
    );

    // 12. Clean shutdown
    host.shutdown();
    peer.shutdown();
}

/// ADR-059: Node security audit trail, structured circular audit log, multi-dimensional filtering,
/// clearing governance, SSE real-time broadcasting, and zero-plaintext compliance.
#[tokio::test(flavor = "multi_thread")]
async fn node_security_audit_trail_filtering_purging_and_zero_plaintext_via_loopback_api() {
    let (host, peer, _g1, _g2) = pair_phone_desktop().await;
    let host_ui = host.spawn_ui().await.expect("host UI spawns");
    let peer_id = peer.identity().device_id().clone();

    // 1. Connect to SSE stream on host UI
    let mut sse = tetherly_node::loopback_events_connect(host_ui)
        .await
        .expect("SSE connects");

    // 2. Query initial audit report via GET /api/audit and aliases
    let (code_audit, body_audit) = tetherly_node::loopback_get(host_ui, "/api/audit")
        .await
        .unwrap();
    assert_eq!(code_audit, 200);
    let audit_rep: tetherly_node::AuditLogReport = serde_json::from_slice(&body_audit).unwrap();
    assert!(audit_rep.ok);
    assert!(audit_rep.capacity >= 500);
    // At least the pairing action was recorded
    assert!(
        audit_rep.total >= 1,
        "initial pairing must be logged in audit trail"
    );

    // Verify alias endpoints
    let (code_alias1, _) = tetherly_node::loopback_get(host_ui, "/api/security/audit")
        .await
        .unwrap();
    assert_eq!(code_alias1, 200);
    let (code_alias2, _) = tetherly_node::loopback_get(host_ui, "/api/audit/logs")
        .await
        .unwrap();
    assert_eq!(code_alias2, 200);

    // Verify snapshot reflects audit count
    let (code_state, body_state) = tetherly_node::loopback_get(host_ui, "/api/state")
        .await
        .unwrap();
    assert_eq!(code_state, 200);
    let state_val: serde_json::Value = serde_json::from_slice(&body_state).unwrap();
    assert_eq!(
        state_val["audit_count"].as_u64().unwrap_or(0),
        audit_rep.total as u64
    );

    // 3. Trigger diverse actions to populate audit engine:
    // a. Peer block and unblock (Security category)
    let block_payload = serde_json::json!({
        "peer": peer_id.as_str(),
        "reason": "suspicious probe activity"
    });
    let (code_block, _) = tetherly_node::loopback_post(
        host_ui,
        "/api/peers/block",
        &serde_json::to_vec(&block_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_block, 200);

    let unblock_payload = serde_json::json!({
        "peer": peer_id.as_str()
    });
    let (code_unblock, _) = tetherly_node::loopback_post(
        host_ui,
        "/api/peers/unblock",
        &serde_json::to_vec(&unblock_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_unblock, 200);

    // b. Heartbeat tuning (Config category)
    let hb_payload = serde_json::json!({
        "interval_secs": 25,
        "deadline_secs": 75,
    });
    let (code_hb, _) = tetherly_node::loopback_post(
        host_ui,
        "/api/peers/heartbeat",
        &serde_json::to_vec(&hb_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_hb, 200);

    // c. Route addition and removal (Route category)
    let route_add_payload = serde_json::json!({
        "peer": peer_id.as_str(),
        "addr": "192.168.1.188:45715"
    });
    let (code_rt_add, _) = tetherly_node::loopback_post(
        host_ui,
        "/api/routes",
        &serde_json::to_vec(&route_add_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_rt_add, 200);

    let route_rm_payload = serde_json::json!({
        "peer": peer_id.as_str(),
        "addr": "192.168.1.188:45715"
    });
    let (code_rt_rm, _) = tetherly_node::loopback_post(
        host_ui,
        "/api/routes/remove",
        &serde_json::to_vec(&route_rm_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_rt_rm, 200);

    // d. Directly record custom security events with different severities
    let sec_alert = host.record_audit(
        tetherly_node::AuditCategory::Security,
        tetherly_node::AuditSeverity::Alert,
        "auth_failure",
        Some("tdev_untrusted_node"),
        "Excessive failed handshake attempts detected",
        false,
    );
    assert_eq!(sec_alert.category, tetherly_node::AuditCategory::Security);
    assert_eq!(sec_alert.severity, tetherly_node::AuditSeverity::Alert);
    assert!(!sec_alert.success);

    let file_warn = host.record_audit(
        tetherly_node::AuditCategory::File,
        tetherly_node::AuditSeverity::Warning,
        "inbox_quota_warning",
        Some("large_archive.zip"),
        "Inbox capacity utilization exceeded 90%",
        true,
    );
    assert_eq!(file_warn.category, tetherly_node::AuditCategory::File);
    assert_eq!(file_warn.severity, tetherly_node::AuditSeverity::Warning);

    // 4. Verify SSE receives AuditEventRecorded
    let mut saw_audit_event = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while tokio::time::Instant::now() < deadline {
        if let Ok(Ok(Some(UiEvent::AuditEventRecorded {
            action,
            category,
            severity,
            target,
            summary,
            success,
            ..
        }))) = tokio::time::timeout(Duration::from_millis(500), sse.next_ui_event()).await
        {
            if action == "auth_failure" {
                assert_eq!(category, "security");
                assert_eq!(severity, "alert");
                assert_eq!(target.as_deref(), Some("tdev_untrusted_node"));
                assert!(!success);
                // Zero-plaintext assertion: verify no sensitive token leaks in summary
                assert!(!summary.contains("private_key"));
                assert!(!summary.contains("password"));
                saw_audit_event = true;
                break;
            }
        }
    }
    assert!(
        saw_audit_event,
        "must emit AuditEventRecorded over SSE for high severity audit action"
    );

    // 5. Multi-dimensional filtering validation via query params:
    // a. Filter by category=security
    let (code_cat, body_cat) = tetherly_node::loopback_get(host_ui, "/api/audit?category=security")
        .await
        .unwrap();
    assert_eq!(code_cat, 200);
    let cat_rep: tetherly_node::AuditLogReport = serde_json::from_slice(&body_cat).unwrap();
    assert!(!cat_rep.entries.is_empty());
    for entry in &cat_rep.entries {
        assert_eq!(
            entry.category,
            tetherly_node::AuditCategory::Security,
            "all returned entries must belong to Security category"
        );
    }

    // b. Filter by category=route
    let (code_rt, body_rt) = tetherly_node::loopback_get(host_ui, "/api/audit?category=route")
        .await
        .unwrap();
    assert_eq!(code_rt, 200);
    let rt_rep: tetherly_node::AuditLogReport = serde_json::from_slice(&body_rt).unwrap();
    assert!(!rt_rep.entries.is_empty());
    for entry in &rt_rep.entries {
        assert_eq!(entry.category, tetherly_node::AuditCategory::Route);
    }

    // c. Filter by severity=warning (matches Warning and Alert, excludes Info and Notice)
    let (code_sev, body_sev) = tetherly_node::loopback_get(host_ui, "/api/audit?severity=warning")
        .await
        .unwrap();
    assert_eq!(code_sev, 200);
    let sev_rep: tetherly_node::AuditLogReport = serde_json::from_slice(&body_sev).unwrap();
    assert!(!sev_rep.entries.is_empty());
    for entry in &sev_rep.entries {
        assert!(
            entry.severity >= tetherly_node::AuditSeverity::Warning,
            "severity must be >= Warning"
        );
    }

    // d. Filter by severity=alert (only Alert)
    let (code_alert, body_alert) =
        tetherly_node::loopback_get(host_ui, "/api/audit?severity=alert")
            .await
            .unwrap();
    assert_eq!(code_alert, 200);
    let alert_rep: tetherly_node::AuditLogReport = serde_json::from_slice(&body_alert).unwrap();
    assert!(!alert_rep.entries.is_empty());
    for entry in &alert_rep.entries {
        assert_eq!(entry.severity, tetherly_node::AuditSeverity::Alert);
    }

    // e. Limit constraint
    let (code_lim, body_lim) = tetherly_node::loopback_get(host_ui, "/api/audit?limit=2")
        .await
        .unwrap();
    assert_eq!(code_lim, 200);
    let lim_rep: tetherly_node::AuditLogReport = serde_json::from_slice(&body_lim).unwrap();
    assert_eq!(lim_rep.entries.len(), 2);

    // 6. Multi-dimensional filtering via POST /api/audit/query
    let query_post = serde_json::json!({
        "category": "security",
        "severity": "warning",
        "limit": 5,
    });
    let (code_qpost, body_qpost) = tetherly_node::loopback_post(
        host_ui,
        "/api/audit/query",
        &serde_json::to_vec(&query_post).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_qpost, 200);
    let qpost_rep: tetherly_node::AuditLogReport = serde_json::from_slice(&body_qpost).unwrap();
    assert!(!qpost_rep.entries.is_empty());
    assert!(qpost_rep.entries.len() <= 5);
    for entry in &qpost_rep.entries {
        assert_eq!(entry.category, tetherly_node::AuditCategory::Security);
        assert!(entry.severity >= tetherly_node::AuditSeverity::Warning);
    }

    // 7. Negative parameter validation:
    // a. Invalid category returns 400
    let (code_bad_cat, _) =
        tetherly_node::loopback_get(host_ui, "/api/audit?category=unknown_nonsense_category")
            .await
            .unwrap();
    assert_eq!(code_bad_cat, 400);

    // b. Invalid severity returns 400
    let (code_bad_sev, _) =
        tetherly_node::loopback_get(host_ui, "/api/audit?severity=super_ultra_urgent")
            .await
            .unwrap();
    assert_eq!(code_bad_sev, 400);

    // 8. Audit Log Purging / Clearing Governance:
    // Call POST /api/audit/clear (or DELETE /api/audit)
    let (code_clear, body_clear) = tetherly_node::loopback_post(host_ui, "/api/audit/clear", b"{}")
        .await
        .unwrap();
    assert_eq!(code_clear, 200);
    let clear_rep: tetherly_node::AuditClearReport = serde_json::from_slice(&body_clear).unwrap();
    assert!(clear_rep.ok);
    assert!(clear_rep.cleared_count >= 5);

    // Query audit report after clear:
    // The clear action itself leaves a single record documenting that the log was cleared!
    let (code_after, body_after) = tetherly_node::loopback_get(host_ui, "/api/audit")
        .await
        .unwrap();
    assert_eq!(code_after, 200);
    let after_rep: tetherly_node::AuditLogReport = serde_json::from_slice(&body_after).unwrap();
    assert_eq!(after_rep.total, 1);
    assert_eq!(after_rep.entries.len(), 1);
    assert_eq!(after_rep.entries[0].action, "audit_log_cleared");
    assert_eq!(
        after_rep.entries[0].category,
        tetherly_node::AuditCategory::Security
    );

    // 9. Security Red Line: Loopback Isolation Host 403 Forbidden check
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut evil_stream = tokio::net::TcpStream::connect(format!("127.0.0.1:{host_ui}"))
        .await
        .expect("connect to ui port");
    let evil_req = "GET /api/audit HTTP/1.1\r\nHost: evil.attacker.com\r\n\r\n";
    evil_stream.write_all(evil_req.as_bytes()).await.unwrap();
    evil_stream.flush().await.unwrap();
    let mut evil_buf = vec![0u8; 1024];
    let n = evil_stream.read(&mut evil_buf).await.unwrap();
    let evil_resp = String::from_utf8_lossy(&evil_buf[..n]);
    assert!(
        evil_resp.starts_with("HTTP/1.1 403 Forbidden"),
        "mismatched Host header must be rejected with 403 Forbidden, got: {evil_resp}"
    );

    // 10. Clean shutdown
    host.shutdown();
    peer.shutdown();
}

#[tokio::test]
async fn node_direct_secure_messaging_history_governance_and_zero_plaintext_via_loopback_api() {
    let (host, peer, _g1, _g2) = pair_phone_desktop().await;
    let host_ui = host.spawn_ui().await.expect("host UI spawns");
    let peer_ui = peer.spawn_ui().await.expect("peer UI spawns");
    let host_id = host.identity().device_id().clone();
    let peer_id = peer.identity().device_id().clone();

    // 1. Connect to SSE stream on host UI and peer UI
    let mut sse_host = tetherly_node::loopback_events_connect(host_ui)
        .await
        .expect("host SSE connects");
    let mut sse_peer = tetherly_node::loopback_events_connect(peer_ui)
        .await
        .expect("peer SSE connects");

    // 2. Query initial message history on both host and peer
    let (code_init, body_init) = tetherly_node::loopback_get(host_ui, "/api/message/history")
        .await
        .unwrap();
    assert_eq!(code_init, 200);
    let init_rep: tetherly_node::PeerMessageHistoryReport =
        serde_json::from_slice(&body_init).unwrap();
    assert!(init_rep.ok);
    assert_eq!(init_rep.total, 0);
    assert_eq!(init_rep.unread, 0);
    assert_eq!(init_rep.returned, 0);
    assert!(init_rep.messages.is_empty());

    // Verify snapshot initial unread count
    let (code_state, body_state) = tetherly_node::loopback_get(host_ui, "/api/state")
        .await
        .unwrap();
    assert_eq!(code_state, 200);
    let state_val: serde_json::Value = serde_json::from_slice(&body_state).unwrap();
    assert_eq!(
        state_val["unread_messages_count"].as_u64().unwrap_or(999),
        0
    );

    // 3. Send message from host to peer via POST /api/message/send
    let test_msg_text = "Hello Peer! This is a secure end-to-end direct note.";
    let send_payload = serde_json::json!({
        "peer": peer_id.as_str(),
        "text": test_msg_text,
    });
    let (code_send, body_send) = tetherly_node::loopback_post(
        host_ui,
        "/api/message/send",
        &serde_json::to_vec(&send_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_send, 200);
    let send_rep: tetherly_node::PeerMessageSendReport =
        serde_json::from_slice(&body_send).unwrap();
    assert!(send_rep.ok);
    assert_eq!(send_rep.message.text, test_msg_text);
    assert_eq!(
        send_rep.message.direction,
        tetherly_node::MessageDirection::Outbound
    );
    assert!(send_rep.message.read);
    assert_eq!(send_rep.message.peer, peer_id.to_string());

    // 4. Verify SSE MessageSent event on host
    let mut saw_sent_event = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while tokio::time::Instant::now() < deadline {
        if let Ok(Ok(Some(UiEvent::MessageSent { id, peer, len, .. }))) =
            tokio::time::timeout(Duration::from_millis(500), sse_host.next_ui_event()).await
        {
            if peer == peer_id.to_string() && len == test_msg_text.len() {
                assert_eq!(id, send_rep.message.id);
                saw_sent_event = true;
                break;
            }
        }
    }
    assert!(
        saw_sent_event,
        "host must emit MessageSent event over SSE with length and peer metadata"
    );

    // 5. Verify peer receives the message over Noise transport and ingests into peer engine
    // Peer should emit MessageReceived event over SSE
    let mut saw_recv_event = false;
    let deadline_peer = tokio::time::Instant::now() + Duration::from_secs(4);
    while tokio::time::Instant::now() < deadline_peer {
        if let Ok(Ok(Some(UiEvent::MessageReceived { id, peer, len, .. }))) =
            tokio::time::timeout(Duration::from_millis(500), sse_peer.next_ui_event()).await
        {
            if peer == host_id.to_string() && len == test_msg_text.len() {
                assert_eq!(id, send_rep.message.id);
                saw_recv_event = true;
                break;
            }
        }
    }
    assert!(
        saw_recv_event,
        "peer must receive TextMessage frame over Noise and emit MessageReceived over SSE"
    );

    // 6. Query peer message history:
    // Unread-only query without marking read
    let (code_peer_hist, body_peer_hist) = tetherly_node::loopback_get(
        peer_ui,
        "/api/message/history?unread_only=true&mark_read=false",
    )
    .await
    .unwrap();
    assert_eq!(code_peer_hist, 200);
    let peer_rep: tetherly_node::PeerMessageHistoryReport =
        serde_json::from_slice(&body_peer_hist).unwrap();
    assert_eq!(peer_rep.total, 1);
    assert_eq!(peer_rep.unread, 1);
    assert_eq!(peer_rep.returned, 1);
    let peer_msg = &peer_rep.messages[0];
    assert_eq!(peer_msg.text, test_msg_text);
    assert_eq!(peer_msg.direction, tetherly_node::MessageDirection::Inbound);
    assert!(!peer_msg.read);
    assert_eq!(peer_msg.peer, host_id.to_string());

    // Peer snapshot reflects unread_messages_count == 1
    let (code_peer_state, body_peer_state) = tetherly_node::loopback_get(peer_ui, "/api/state")
        .await
        .unwrap();
    assert_eq!(code_peer_state, 200);
    let peer_state_val: serde_json::Value = serde_json::from_slice(&body_peer_state).unwrap();
    assert_eq!(
        peer_state_val["unread_messages_count"]
            .as_u64()
            .unwrap_or(999),
        1
    );

    // Now query with mark_read=true
    let (code_peer_mark, body_peer_mark) =
        tetherly_node::loopback_get(peer_ui, "/api/message/history?mark_read=true")
            .await
            .unwrap();
    assert_eq!(code_peer_mark, 200);
    let mark_rep: tetherly_node::PeerMessageHistoryReport =
        serde_json::from_slice(&body_peer_mark).unwrap();
    assert_eq!(mark_rep.total, 1);
    assert_eq!(mark_rep.unread, 0);

    // Peer snapshot reflects unread_messages_count == 0 after read
    let (_, body_peer_state2) = tetherly_node::loopback_get(peer_ui, "/api/state")
        .await
        .unwrap();
    let peer_state_val2: serde_json::Value = serde_json::from_slice(&body_peer_state2).unwrap();
    assert_eq!(
        peer_state_val2["unread_messages_count"]
            .as_u64()
            .unwrap_or(999),
        0
    );

    // 7. Two-way communication: reply from peer to host
    let reply_text = "Received loud and clear! Tetherly direct messaging confirmed.";
    let reply_payload = serde_json::json!({
        "peer": host_id.as_str(),
        "text": reply_text,
    });
    let (code_reply, body_reply) = tetherly_node::loopback_post(
        peer_ui,
        "/api/message/send",
        &serde_json::to_vec(&reply_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_reply, 200);
    let reply_rep: tetherly_node::PeerMessageSendReport =
        serde_json::from_slice(&body_reply).unwrap();
    assert!(reply_rep.ok);

    // Wait briefly for host to receive inbound reply
    let deadline_host_recv = tokio::time::Instant::now() + Duration::from_secs(4);
    let mut saw_host_recv = false;
    while tokio::time::Instant::now() < deadline_host_recv {
        if let Ok(Ok(Some(UiEvent::MessageReceived { id, peer, len, .. }))) =
            tokio::time::timeout(Duration::from_millis(500), sse_host.next_ui_event()).await
        {
            if peer == peer_id.to_string() && len == reply_text.len() {
                assert_eq!(id, reply_rep.message.id);
                saw_host_recv = true;
                break;
            }
        }
    }
    assert!(
        saw_host_recv,
        "host must receive reply TextMessage from peer over Noise"
    );

    // Host should now have 2 messages in history (1 outbound, 1 inbound)
    let (code_host_hist, body_host_hist) =
        tetherly_node::loopback_get(host_ui, "/api/message/history?mark_read=true")
            .await
            .unwrap();
    assert_eq!(code_host_hist, 200);
    let host_rep: tetherly_node::PeerMessageHistoryReport =
        serde_json::from_slice(&body_host_hist).unwrap();
    assert_eq!(host_rep.total, 2);
    assert_eq!(host_rep.returned, 2);

    // 8. Multi-dimensional filtering and query:
    // a. Filter by peer
    let (code_peer_filter, body_peer_filter) =
        tetherly_node::loopback_get(host_ui, &format!("/api/message/history?peer={peer_id}"))
            .await
            .unwrap();
    assert_eq!(code_peer_filter, 200);
    let peer_filt_rep: tetherly_node::PeerMessageHistoryReport =
        serde_json::from_slice(&body_peer_filter).unwrap();
    assert_eq!(peer_filt_rep.returned, 2);

    // b. Filter by limit=1
    let (code_lim, body_lim) = tetherly_node::loopback_get(host_ui, "/api/message/history?limit=1")
        .await
        .unwrap();
    assert_eq!(code_lim, 200);
    let lim_rep: tetherly_node::PeerMessageHistoryReport =
        serde_json::from_slice(&body_lim).unwrap();
    assert_eq!(lim_rep.returned, 1);
    assert_eq!(lim_rep.total, 2);

    // c. Query via POST /api/message/query
    let q_post_payload = serde_json::json!({
        "peer": peer_id.as_str(),
        "limit": 1,
        "unread_only": false,
        "mark_read": false,
    });
    let (code_qpost, body_qpost) = tetherly_node::loopback_post(
        host_ui,
        "/api/message/query",
        &serde_json::to_vec(&q_post_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_qpost, 200);
    let qpost_rep: tetherly_node::PeerMessageHistoryReport =
        serde_json::from_slice(&body_qpost).unwrap();
    assert_eq!(qpost_rep.returned, 1);

    // 9. Negative validation:
    // a. Empty message text returns 400
    let bad_empty_payload = serde_json::json!({
        "peer": peer_id.as_str(),
        "text": "   ",
    });
    let (code_empty, _) = tetherly_node::loopback_post(
        host_ui,
        "/api/message/send",
        &serde_json::to_vec(&bad_empty_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_empty, 400);

    // b. Oversized message text (> 32 KiB) returns 400
    let oversized_text = "A".repeat(33 * 1024);
    let bad_oversized_payload = serde_json::json!({
        "peer": peer_id.as_str(),
        "text": oversized_text,
    });
    let (code_oversized, _) = tetherly_node::loopback_post(
        host_ui,
        "/api/message/send",
        &serde_json::to_vec(&bad_oversized_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_oversized, 400);

    // c. Unknown non-existent peer returns 400
    let bad_peer_payload = serde_json::json!({
        "peer": "tdev_non_existent_fake_device_id",
        "text": "hello",
    });
    let (code_bad_peer, _) = tetherly_node::loopback_post(
        host_ui,
        "/api/message/send",
        &serde_json::to_vec(&bad_peer_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_bad_peer, 400);

    // 10. Message clearing governance:
    let (code_clear, body_clear) =
        tetherly_node::loopback_post(host_ui, "/api/message/clear", b"{}")
            .await
            .unwrap();
    assert_eq!(code_clear, 200);
    let clear_rep: tetherly_node::PeerMessageClearReport =
        serde_json::from_slice(&body_clear).unwrap();
    assert!(clear_rep.ok);
    assert_eq!(clear_rep.cleared_count, 2);
    assert_eq!(clear_rep.target, "all");

    // History should now be empty
    let (_, body_empty_hist) = tetherly_node::loopback_get(host_ui, "/api/message/history")
        .await
        .unwrap();
    let empty_hist: tetherly_node::PeerMessageHistoryReport =
        serde_json::from_slice(&body_empty_hist).unwrap();
    assert_eq!(empty_hist.total, 0);
    assert!(empty_hist.messages.is_empty());

    // 11. Security Red Line: Loopback Isolation Host 403 Forbidden check
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut evil_stream = tokio::net::TcpStream::connect(format!("127.0.0.1:{host_ui}"))
        .await
        .expect("connect to ui port");
    let evil_req = "GET /api/message/history HTTP/1.1\r\nHost: evil.attacker.com\r\n\r\n";
    evil_stream.write_all(evil_req.as_bytes()).await.unwrap();
    evil_stream.flush().await.unwrap();
    let mut evil_buf = vec![0u8; 1024];
    let n = evil_stream.read(&mut evil_buf).await.unwrap();
    let evil_resp = String::from_utf8_lossy(&evil_buf[..n]);
    assert!(
        evil_resp.starts_with("HTTP/1.1 403 Forbidden"),
        "mismatched Host header must be rejected with 403 Forbidden, got: {evil_resp}"
    );

    // 12. Clean shutdown
    host.shutdown();
    peer.shutdown();
}

#[tokio::test]
async fn node_outbox_file_lifecycle_query_clearing_resend_and_zero_plaintext_via_loopback_api() {
    let (host, peer, _g1, _g2) = pair_phone_desktop().await;
    let host_ui = host.spawn_ui().await.expect("host UI spawns");
    let _peer_ui = peer.spawn_ui().await.expect("peer UI spawns");
    let _host_id = host.identity().device_id().clone();
    let peer_id = peer.identity().device_id().clone();

    // 1. Connect to SSE stream on host UI
    let mut sse_host = tetherly_node::loopback_events_connect(host_ui)
        .await
        .expect("host SSE connects");

    // 2. Query initial outbox history via GET /api/file/outbox
    let (code_init, body_init) = tetherly_node::loopback_get(host_ui, "/api/file/outbox")
        .await
        .unwrap();
    assert_eq!(code_init, 200);
    let init_rep: tetherly_node::OutboxReport = serde_json::from_slice(&body_init).unwrap();
    assert!(init_rep.ok);
    assert_eq!(init_rep.total, 0);
    assert_eq!(init_rep.returned, 0);
    assert!(init_rep.entries.is_empty());

    // 3. Create a temporary local file to send
    let (temp_dir, _guard) = temp_dir("outbox");
    let file_path = temp_dir.join("outbox_spec_sample.bin");
    let sample_payload = vec![0x42u8; 128 * 1024];
    std::fs::write(&file_path, &sample_payload).unwrap();

    // 4. Offer and send file from host to peer
    let tr_id = host
        .offer_and_send_file(&peer_id, &file_path, None)
        .await
        .expect("offer_and_send_file succeeds");

    // Verify immediate SSE event UiEvent::OutboxUpdated with InFlight status
    let deadline = tokio::time::Instant::now() + Duration::from_secs(4);
    let mut saw_inflight_ev = false;
    while tokio::time::Instant::now() < deadline {
        if let Ok(Ok(Some(UiEvent::OutboxUpdated {
            transfer_id,
            peer,
            filename,
            size,
            status,
            ..
        }))) = tokio::time::timeout(Duration::from_millis(500), sse_host.next_ui_event()).await
        {
            if transfer_id == tr_id && status == tetherly_node::OutboxStatus::InFlight.as_str() {
                assert_eq!(peer, peer_id.to_string());
                assert_eq!(filename, "outbox_spec_sample.bin");
                assert_eq!(size, sample_payload.len() as u64);
                saw_inflight_ev = true;
                break;
            }
        }
    }
    assert!(
        saw_inflight_ev,
        "host must emit OutboxUpdated InFlight event"
    );

    // Check outbox query: 1 entry in flight
    let (code_inflight, body_inflight) = tetherly_node::loopback_get(host_ui, "/api/file/outbox")
        .await
        .unwrap();
    assert_eq!(code_inflight, 200);
    let inflight_rep: tetherly_node::OutboxReport = serde_json::from_slice(&body_inflight).unwrap();
    assert_eq!(inflight_rep.total, 1);
    assert_eq!(
        inflight_rep.entries[0].status,
        tetherly_node::OutboxStatus::InFlight
    );
    assert_eq!(
        inflight_rep.entries[0].source_path.as_deref(),
        Some(file_path.to_str().unwrap())
    );

    // 5. Peer accepts file and transfer completes
    wait_offered(&peer, Duration::from_secs(3)).await;
    peer.user_accept_file(&tr_id).await.unwrap();

    // Verify SSE event UiEvent::OutboxUpdated with Completed status
    let deadline_comp = tokio::time::Instant::now() + Duration::from_secs(6);
    let mut saw_completed_ev = false;
    while tokio::time::Instant::now() < deadline_comp {
        if let Ok(Ok(Some(UiEvent::OutboxUpdated {
            transfer_id,
            status,
            ..
        }))) = tokio::time::timeout(Duration::from_millis(500), sse_host.next_ui_event()).await
        {
            if transfer_id == tr_id && status == tetherly_node::OutboxStatus::Completed.as_str() {
                saw_completed_ev = true;
                break;
            }
        }
    }
    assert!(
        saw_completed_ev,
        "host must emit OutboxUpdated Completed event"
    );

    // Verify outbox entry is now Completed
    let (code_comp, body_comp) = tetherly_node::loopback_get(host_ui, "/api/file/outbox")
        .await
        .unwrap();
    assert_eq!(code_comp, 200);
    let comp_rep: tetherly_node::OutboxReport = serde_json::from_slice(&body_comp).unwrap();
    assert_eq!(comp_rep.total, 1);
    let entry1 = &comp_rep.entries[0];
    assert_eq!(entry1.status, tetherly_node::OutboxStatus::Completed);
    assert!(entry1.completed_ms.is_some());

    // 6. Test Rejection flow: offer and reject
    let reject_payload = b"declined payload data";
    let tr_reject_id = host
        .offer_and_send_bytes(&peer_id, "sensitive_decline.txt", reject_payload, false)
        .await
        .unwrap();
    wait_offered(&peer, Duration::from_secs(3)).await;
    peer.user_reject_file(&tr_reject_id).await.unwrap();

    // Verify SSE event UiEvent::OutboxUpdated with Rejected status
    let deadline_rej = tokio::time::Instant::now() + Duration::from_secs(6);
    let mut saw_rejected_ev = false;
    while tokio::time::Instant::now() < deadline_rej {
        if let Ok(Ok(Some(UiEvent::OutboxUpdated {
            transfer_id,
            status,
            ..
        }))) = tokio::time::timeout(Duration::from_millis(500), sse_host.next_ui_event()).await
        {
            if transfer_id == tr_reject_id
                && status == tetherly_node::OutboxStatus::Rejected.as_str()
            {
                saw_rejected_ev = true;
                break;
            }
        }
    }
    assert!(
        saw_rejected_ev,
        "host must emit OutboxUpdated Rejected event"
    );

    // Verify outbox has 2 entries (1 completed, 1 rejected)
    let (code_both, body_both) = tetherly_node::loopback_get(host_ui, "/api/file/outbox")
        .await
        .unwrap();
    assert_eq!(code_both, 200);
    let both_rep: tetherly_node::OutboxReport = serde_json::from_slice(&body_both).unwrap();
    assert_eq!(both_rep.total, 2);

    // 7. Multi-dimensional filtering and query:
    // a. Query by status=completed
    let (code_filt_comp, body_filt_comp) =
        tetherly_node::loopback_get(host_ui, "/api/file/outbox?status=completed")
            .await
            .unwrap();
    assert_eq!(code_filt_comp, 200);
    let filt_comp_rep: tetherly_node::OutboxReport =
        serde_json::from_slice(&body_filt_comp).unwrap();
    assert_eq!(filt_comp_rep.returned, 1);
    assert_eq!(
        filt_comp_rep.entries[0].status,
        tetherly_node::OutboxStatus::Completed
    );

    // b. Query by status=rejected
    let (code_filt_rej, body_filt_rej) =
        tetherly_node::loopback_get(host_ui, "/api/file/outbox?status=rejected")
            .await
            .unwrap();
    assert_eq!(code_filt_rej, 200);
    let filt_rej_rep: tetherly_node::OutboxReport = serde_json::from_slice(&body_filt_rej).unwrap();
    assert_eq!(filt_rej_rep.returned, 1);
    assert_eq!(
        filt_rej_rep.entries[0].status,
        tetherly_node::OutboxStatus::Rejected
    );

    // c. Query by limit=1
    let (code_lim, body_lim) = tetherly_node::loopback_get(host_ui, "/api/file/outbox?limit=1")
        .await
        .unwrap();
    assert_eq!(code_lim, 200);
    let lim_rep: tetherly_node::OutboxReport = serde_json::from_slice(&body_lim).unwrap();
    assert_eq!(lim_rep.total, 2);
    assert_eq!(lim_rep.returned, 1);

    // d. Advanced query via POST /api/file/outbox/query
    let query_payload = serde_json::json!({
        "peer": peer_id.as_str(),
        "status": "completed",
        "limit": 5,
    });
    let (code_qpost, body_qpost) = tetherly_node::loopback_post(
        host_ui,
        "/api/file/outbox/query",
        &serde_json::to_vec(&query_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_qpost, 200);
    let qpost_rep: tetherly_node::OutboxReport = serde_json::from_slice(&body_qpost).unwrap();
    assert_eq!(qpost_rep.returned, 1);

    // 8. One-Key Resend Pipeline via POST /api/file/outbox/resend
    let resend_payload = serde_json::json!({
        "id": entry1.id,
    });
    let (code_resend, body_resend) = tetherly_node::loopback_post(
        host_ui,
        "/api/file/outbox/resend",
        &serde_json::to_vec(&resend_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_resend, 200);
    let resend_rep: tetherly_node::OutboxResendReport =
        serde_json::from_slice(&body_resend).unwrap();
    assert!(resend_rep.ok);
    assert_eq!(resend_rep.original_id, entry1.id);
    assert!(resend_rep.new_transfer_id.starts_with("tr_"));

    // Host outbox now has 3 entries
    let (code_after_resend, body_after_resend) =
        tetherly_node::loopback_get(host_ui, "/api/file/outbox")
            .await
            .unwrap();
    assert_eq!(code_after_resend, 200);
    let after_resend_rep: tetherly_node::OutboxReport =
        serde_json::from_slice(&body_after_resend).unwrap();
    assert_eq!(after_resend_rep.total, 3);

    // 9. Negative validation:
    // a. Resend non-existent ID -> 404
    let bad_resend_payload = serde_json::json!({
        "id": "out_tr_non_existent_fake_id",
    });
    let (code_bad_resend, _) = tetherly_node::loopback_post(
        host_ui,
        "/api/file/outbox/resend",
        &serde_json::to_vec(&bad_resend_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_bad_resend, 404);

    // 10. Outbox clearing governance:
    // a. Partial clear: clear only rejected
    let clear_rej_payload = serde_json::json!({
        "status": "rejected",
    });
    let (code_clear_rej, body_clear_rej) = tetherly_node::loopback_post(
        host_ui,
        "/api/file/outbox/clear",
        &serde_json::to_vec(&clear_rej_payload).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_clear_rej, 200);
    let clear_rej_rep: tetherly_node::OutboxClearReport =
        serde_json::from_slice(&body_clear_rej).unwrap();
    assert!(clear_rej_rep.ok);
    assert_eq!(clear_rej_rep.cleared_count, 1);

    // Check SSE event UiEvent::OutboxCleared
    let deadline_clr = tokio::time::Instant::now() + Duration::from_secs(3);
    let mut saw_cleared_ev = false;
    while tokio::time::Instant::now() < deadline_clr {
        if let Ok(Ok(Some(UiEvent::OutboxCleared { count, status, .. }))) =
            tokio::time::timeout(Duration::from_millis(500), sse_host.next_ui_event()).await
        {
            if count == 1
                && status.as_deref() == Some(tetherly_node::OutboxStatus::Rejected.as_str())
            {
                saw_cleared_ev = true;
                break;
            }
        }
    }
    assert!(saw_cleared_ev, "host must emit OutboxCleared event");

    // b. Full clear: clear everything remaining
    let (code_clear_all, body_clear_all) =
        tetherly_node::loopback_post(host_ui, "/api/file/outbox/clear", b"{}")
            .await
            .unwrap();
    assert_eq!(code_clear_all, 200);
    let clear_all_rep: tetherly_node::OutboxClearReport =
        serde_json::from_slice(&body_clear_all).unwrap();
    assert!(clear_all_rep.ok);
    assert_eq!(clear_all_rep.cleared_count, 2);

    // Outbox history must now be empty
    let (_, body_final_empty) = tetherly_node::loopback_get(host_ui, "/api/file/outbox")
        .await
        .unwrap();
    let final_empty_rep: tetherly_node::OutboxReport =
        serde_json::from_slice(&body_final_empty).unwrap();
    assert_eq!(final_empty_rep.total, 0);
    assert!(final_empty_rep.entries.is_empty());

    // 11. Security Red Line: Loopback Isolation Host 403 Forbidden check
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut evil_stream = tokio::net::TcpStream::connect(format!("127.0.0.1:{host_ui}"))
        .await
        .expect("connect to ui port");
    let evil_req = "GET /api/file/outbox HTTP/1.1\r\nHost: malicious.domain.com\r\n\r\n";
    evil_stream.write_all(evil_req.as_bytes()).await.unwrap();
    evil_stream.flush().await.unwrap();
    let mut evil_buf = vec![0u8; 1024];
    let n = evil_stream.read(&mut evil_buf).await.unwrap();
    let evil_resp = String::from_utf8_lossy(&evil_buf[..n]);
    assert!(
        evil_resp.starts_with("HTTP/1.1 403 Forbidden"),
        "mismatched Host header must be rejected with 403 Forbidden, got: {evil_resp}"
    );

    // 12. Clean shutdown
    host.shutdown();
    peer.shutdown();
}

/// ADR-062: Peer device profiling, tag & group governance, multi-dimensional scoping,
/// and Zero-Plaintext SSE broadcasting via loopback API.
#[tokio::test(flavor = "multi_thread")]
async fn peer_device_profiling_tag_and_group_governance_via_loopback_api() {
    let (host, peer, g1, _g2) = pair_phone_desktop().await;
    let host_ui = host.spawn_ui().await.expect("host UI spawns");
    let _host_id = host.identity().device_id().clone();
    let peer_id = peer.identity().device_id().clone();

    // 1. Connect to SSE event stream on host UI
    let mut sse_host = tetherly_node::loopback_events_connect(host_ui)
        .await
        .expect("host SSE connects");

    // 2. Query initial profiles report: initially empty table
    let (code_init, body_init) = tetherly_node::loopback_get(host_ui, "/api/peers/profiles")
        .await
        .unwrap();
    assert_eq!(code_init, 200);
    let init_rep: tetherly_node::PeerProfilesReport = serde_json::from_slice(&body_init).unwrap();
    assert!(init_rep.ok);
    assert_eq!(init_rep.total, 0);
    assert!(init_rep.profiles.is_empty());

    // Verify alias /api/profiles
    let (code_alias, body_alias) = tetherly_node::loopback_get(host_ui, "/api/profiles")
        .await
        .unwrap();
    assert_eq!(code_alias, 200);
    assert_eq!(body_init, body_alias);

    // 3. Query profile of peer: returns default synthesized profile
    let (code_get_peer, body_get_peer) = tetherly_node::loopback_get(
        host_ui,
        &format!("/api/peers/profile?peer={}", peer_id.as_str()),
    )
    .await
    .unwrap();
    assert_eq!(code_get_peer, 200);
    let peer_prof_default: tetherly_node::PeerProfile =
        serde_json::from_slice(&body_get_peer).unwrap();
    assert_eq!(peer_prof_default.device_id, peer_id.as_str());
    assert_eq!(
        peer_prof_default.device_type,
        tetherly_node::DeviceType::Other
    ); // default unclassified
    assert!(peer_prof_default.tags.is_empty());
    assert_eq!(peer_prof_default.group, None);
    assert_eq!(peer_prof_default.notes, None);

    // 4. Update peer profile via POST /api/peers/profile:
    // Set device_type = "laptop", group = "lab-cluster", notes = "Confidential test note", tags = ["work", "dev"]
    let private_note = "Confidential test note - must never leak in SSE or logs";
    let update_req = serde_json::json!({
        "peer": peer_id.as_str(),
        "device_type": "laptop",
        "group": "lab-cluster",
        "notes": private_note,
        "tags": ["work", "dev"],
    });
    let (code_up, body_up) = tetherly_node::loopback_post(
        host_ui,
        "/api/peers/profile",
        &serde_json::to_vec(&update_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_up, 200);
    let up_rep: tetherly_node::PeerProfileUpdateReport = serde_json::from_slice(&body_up).unwrap();
    assert!(up_rep.ok);
    assert_eq!(up_rep.device_id, peer_id.as_str());
    assert_eq!(
        up_rep.profile.device_type,
        tetherly_node::DeviceType::Laptop
    );
    assert_eq!(up_rep.profile.group.as_deref(), Some("lab-cluster"));
    assert_eq!(up_rep.profile.notes.as_deref(), Some(private_note));
    assert_eq!(up_rep.profile.tags, vec!["work", "dev"]);

    // 5. Zero-Plaintext Invariant & SSE Event verification:
    // Listen for UiEvent::PeerProfileUpdated on host SSE
    let mut saw_profile_sse = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while tokio::time::Instant::now() < deadline {
        if let Ok(Ok(Some(raw_str))) =
            tokio::time::timeout(Duration::from_millis(500), sse_host.next_event()).await
        {
            if let Ok(ev) = serde_json::from_str::<serde_json::Value>(&raw_str) {
                if ev["kind"] == "peer_profile_updated" && ev["device_id"] == peer_id.as_str() {
                    assert_eq!(ev["device_type"], "laptop");
                    assert_eq!(ev["group"], "lab-cluster");
                    let sse_tags = ev["tags"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|v| v.as_str().unwrap())
                        .collect::<Vec<_>>();
                    assert_eq!(sse_tags, vec!["work", "dev"]);
                    assert!(ev["timestamp_ms"].as_u64().unwrap_or(0) > 0);

                    // ZERO-PLAINTEXT CHECK: Verify 'notes' is strictly absent from SSE event payload!
                    assert!(
                        ev.get("notes").is_none(),
                        "Zero-Plaintext Invariant: sensitive profile notes must NOT be emitted in SSE!"
                    );
                    assert!(
                        !raw_str.contains("Confidential"),
                        "Zero-Plaintext Invariant: raw event text must not contain note plaintext!"
                    );
                    saw_profile_sse = true;
                    break;
                }
            }
        }
    }
    assert!(saw_profile_sse, "must emit peer_profile_updated SSE event");

    // 6. Test Tag governance endpoints:
    // a. Add tag "nas" via POST /api/peers/tags/add
    let tag_add_req = serde_json::json!({
        "peer": peer_id.as_str(),
        "tag": "nas",
    });
    let (code_tag_add, body_tag_add) = tetherly_node::loopback_post(
        host_ui,
        "/api/peers/tags/add",
        &serde_json::to_vec(&tag_add_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_tag_add, 200);
    let tag_add_rep: tetherly_node::PeerTagsReport = serde_json::from_slice(&body_tag_add).unwrap();
    assert!(tag_add_rep.ok);
    assert_eq!(tag_add_rep.tags, vec!["work", "dev", "nas"]);

    // b. Idempotent tag add: adding "nas" again doesn't duplicate
    let (code_tag_dup, body_tag_dup) = tetherly_node::loopback_post(
        host_ui,
        "/api/peers/tags/add",
        &serde_json::to_vec(&tag_add_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_tag_dup, 200);
    let tag_dup_rep: tetherly_node::PeerTagsReport = serde_json::from_slice(&body_tag_dup).unwrap();
    assert_eq!(tag_dup_rep.tags, vec!["work", "dev", "nas"]);

    // c. Remove tag "dev" via POST /api/peers/tags/remove
    let tag_rm_req = serde_json::json!({
        "peer": peer_id.as_str(),
        "tag": "dev",
    });
    let (code_tag_rm, body_tag_rm) = tetherly_node::loopback_post(
        host_ui,
        "/api/peers/tags/remove",
        &serde_json::to_vec(&tag_rm_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_tag_rm, 200);
    let tag_rm_rep: tetherly_node::PeerTagsReport = serde_json::from_slice(&body_tag_rm).unwrap();
    assert_eq!(tag_rm_rep.tags, vec!["work", "nas"]);

    // 7. Test Group governance endpoints:
    // a. Update group to "production" via POST /api/peers/group
    let grp_set_req = serde_json::json!({
        "peer": peer_id.as_str(),
        "group": "production",
    });
    let (code_grp_set, body_grp_set) = tetherly_node::loopback_post(
        host_ui,
        "/api/peers/group",
        &serde_json::to_vec(&grp_set_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_grp_set, 200);
    let grp_set_rep: tetherly_node::PeerGroupReport =
        serde_json::from_slice(&body_grp_set).unwrap();
    assert!(grp_set_rep.ok);
    assert_eq!(grp_set_rep.group.as_deref(), Some("production"));

    // b. Clear group via setting group: null
    let grp_clr_req = serde_json::json!({
        "peer": peer_id.as_str(),
        "group": serde_json::Value::Null,
    });
    let (code_grp_clr, body_grp_clr) = tetherly_node::loopback_post(
        host_ui,
        "/api/peers/group",
        &serde_json::to_vec(&grp_clr_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_grp_clr, 200);
    let grp_clr_rep: tetherly_node::PeerGroupReport =
        serde_json::from_slice(&body_grp_clr).unwrap();
    assert!(grp_clr_rep.ok);
    assert_eq!(grp_clr_rep.group, None);

    // Set group back to "production" for multi-dimensional filtering tests
    let _ = tetherly_node::loopback_post(
        host_ui,
        "/api/peers/group",
        &serde_json::to_vec(&grp_set_req).unwrap(),
    )
    .await
    .unwrap();

    // 8. Multi-dimensional scoping & query filtering:
    // a. Filter by tag=work
    let (c1, b1) = tetherly_node::loopback_get(host_ui, "/api/peers/profiles?tag=work")
        .await
        .unwrap();
    assert_eq!(c1, 200);
    let r1: tetherly_node::PeerProfilesReport = serde_json::from_slice(&b1).unwrap();
    assert_eq!(r1.total, 1);
    assert_eq!(r1.profiles[0].device_id, peer_id.as_str());

    // b. Filter by non-existent tag
    let (c2, b2) = tetherly_node::loopback_get(host_ui, "/api/peers/profiles?tag=nonexistent")
        .await
        .unwrap();
    assert_eq!(c2, 200);
    let r2: tetherly_node::PeerProfilesReport = serde_json::from_slice(&b2).unwrap();
    assert_eq!(r2.total, 0);

    // c. Filter by group=production
    let (c3, b3) = tetherly_node::loopback_get(host_ui, "/api/peers/profiles?group=production")
        .await
        .unwrap();
    assert_eq!(c3, 200);
    let r3: tetherly_node::PeerProfilesReport = serde_json::from_slice(&b3).unwrap();
    assert_eq!(r3.total, 1);

    // d. Filter by type=laptop
    let (c4, b4) = tetherly_node::loopback_get(host_ui, "/api/peers/profiles?type=laptop")
        .await
        .unwrap();
    assert_eq!(c4, 200);
    let r4: tetherly_node::PeerProfilesReport = serde_json::from_slice(&b4).unwrap();
    assert_eq!(r4.total, 1);

    // e. Filter by type=server (no match)
    let (c5, b5) = tetherly_node::loopback_get(host_ui, "/api/peers/profiles?type=server")
        .await
        .unwrap();
    assert_eq!(c5, 200);
    let r5: tetherly_node::PeerProfilesReport = serde_json::from_slice(&b5).unwrap();
    assert_eq!(r5.total, 0);

    // 9. PeerView & State enrichment check:
    // GET /api/peers and GET /api/state must automatically fuse peer profile metadata
    let (c_peers, b_peers) = tetherly_node::loopback_get(host_ui, "/api/peers")
        .await
        .unwrap();
    assert_eq!(c_peers, 200);
    let peers_val: serde_json::Value = serde_json::from_slice(&b_peers).unwrap();
    let live_peers = peers_val["live"].as_array().unwrap();
    let peer_in_live = live_peers
        .iter()
        .find(|p| p["device_id"] == peer_id.as_str())
        .expect("peer in live list");
    assert_eq!(peer_in_live["device_type"], "laptop");
    assert_eq!(peer_in_live["group"], "production");
    let live_tags = peer_in_live["tags"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(live_tags, vec!["work", "nas"]);

    let trusted_peers = peers_val["trusted"].as_array().unwrap();
    let peer_in_trusted = trusted_peers
        .iter()
        .find(|p| p["device_id"] == peer_id.as_str())
        .expect("peer in trusted list");
    assert_eq!(peer_in_trusted["device_type"], "laptop");
    assert_eq!(peer_in_trusted["group"], "production");

    // 10. Crash consistency & atomic disk persistence verification:
    // Verify profiles.json was written to disk with POSIX 0o600 permissions
    let profiles_path = g1.path().join("profiles.json");
    assert!(profiles_path.exists(), "profiles.json must exist on disk");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::metadata(&profiles_path).unwrap().permissions();
        assert_eq!(
            perms.mode() & 0o777,
            0o600,
            "profiles.json permissions must be 0600"
        );
    }
    // Read and verify disk content contains updated profile
    let disk_table = tetherly_node::load_profiles(g1.path());
    let disk_prof = disk_table
        .profiles
        .iter()
        .find(|p| p.device_id == peer_id.as_str())
        .expect("persisted profile in table");
    assert_eq!(disk_prof.device_type, tetherly_node::DeviceType::Laptop);
    assert_eq!(disk_prof.group.as_deref(), Some("production"));
    assert_eq!(disk_prof.tags, vec!["work", "nas"]);

    // 11. Negative testing & validation boundaries:
    // a. Invalid tag with spaces -> 400
    let invalid_tag_req = serde_json::json!({
        "peer": peer_id.as_str(),
        "tag": "invalid tag with spaces",
    });
    let (code_inv_tag, _) = tetherly_node::loopback_post(
        host_ui,
        "/api/peers/tags/add",
        &serde_json::to_vec(&invalid_tag_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_inv_tag, 400);

    // b. Invalid device type string -> 400
    let invalid_type_req = serde_json::json!({
        "peer": peer_id.as_str(),
        "device_type": "alien_spaceship",
    });
    let (code_inv_type, _) = tetherly_node::loopback_post(
        host_ui,
        "/api/peers/profile",
        &serde_json::to_vec(&invalid_type_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_inv_type, 400);

    // c. Empty peer ID in profile request -> 400
    let empty_peer_req = serde_json::json!({
        "device_type": "laptop",
    });
    let (code_no_peer, _) = tetherly_node::loopback_post(
        host_ui,
        "/api/peers/profile",
        &serde_json::to_vec(&empty_peer_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_no_peer, 400);

    // 12. Security Red Line: Loopback Isolation Host 403 Forbidden check
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut evil_stream = tokio::net::TcpStream::connect(format!("127.0.0.1:{host_ui}"))
        .await
        .expect("connect to ui port");
    let evil_req = "GET /api/peers/profiles HTTP/1.1\r\nHost: malicious.domain.com\r\n\r\n";
    evil_stream.write_all(evil_req.as_bytes()).await.unwrap();
    evil_stream.flush().await.unwrap();
    let mut evil_buf = vec![0u8; 1024];
    let n = evil_stream.read(&mut evil_buf).await.unwrap();
    let evil_resp = String::from_utf8_lossy(&evil_buf[..n]);
    assert!(
        evil_resp.starts_with("HTTP/1.1 403 Forbidden"),
        "mismatched Host header must be rejected with 403 Forbidden, got: {evil_resp}"
    );

    // 13. Clean shutdown
    host.shutdown();
    peer.shutdown();
}

/// ADR-063: Policy-based tag & group targeted dispatch, multi-peer fan-out,
/// single-pass hash streaming, granular auto-accept policy, SSE Zero-Plaintext, and Host 403 isolation.
#[tokio::test(flavor = "multi_thread")]
async fn policy_based_tag_and_group_targeted_multi_dispatch_via_loopback_api() {
    let (h_dir, _gh) = temp_dir("host");
    let (p1_dir, _gp1) = temp_dir("peer1");
    let (p2_dir, _gp2) = temp_dir("peer2");
    let host = Node::start(cfg(h_dir, "host", "linux")).unwrap();
    let peer1 = Node::start(cfg(p1_dir.clone(), "peer1", "linux")).unwrap();
    let peer2 = Node::start(cfg(p2_dir, "peer2", "linux")).unwrap();
    host.spawn_listeners().await.unwrap();
    peer1.spawn_listeners().await.unwrap();
    peer2.spawn_listeners().await.unwrap();

    let pin = tetherly_core::pin::parse_pin("25170394").unwrap();
    host.set_pin(pin);
    peer1.set_pin(pin);
    peer2.set_pin(pin);

    host.dial(peer1.control_addr()).await.unwrap();
    host.dial(peer2.control_addr()).await.unwrap();

    let start = std::time::Instant::now();
    loop {
        let peers = host.live_peers();
        if peers.len() >= 2 && peers.iter().all(|p| p.file_port.is_some()) {
            break;
        }
        if start.elapsed() > Duration::from_secs(45) {
            panic!("host did not see 2 live peers with file_port");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    wait_live(&peer1, Duration::from_secs(45)).await;
    wait_live(&peer2, Duration::from_secs(45)).await;

    let host_ui = host.spawn_ui().await.expect("host UI spawns");
    let peer1_ui = peer1.spawn_ui().await.expect("peer1 UI spawns");
    let _peer2_ui = peer2.spawn_ui().await.expect("peer2 UI spawns");

    let host_id = host.identity().device_id().clone();
    let peer1_id = peer1.identity().device_id().clone();
    let peer2_id = peer2.identity().device_id().clone();

    // 1. Connect to SSE event streams on host UI and peer1 UI
    let mut sse_host = tetherly_node::loopback_events_connect(host_ui)
        .await
        .expect("host SSE connects");
    let mut sse_peer1 = tetherly_node::loopback_events_connect(peer1_ui)
        .await
        .expect("peer1 SSE connects");

    // 2. Configure peer profiles on Host:
    // Peer1: device_type="laptop", group="dev", tags=["lab", "frontend"]
    let p1_prof_req = serde_json::json!({
        "peer": peer1_id.as_str(),
        "device_type": "laptop",
        "group": "dev",
        "tags": ["lab", "frontend"],
    });
    let (code_p1, body_p1) = tetherly_node::loopback_post(
        host_ui,
        "/api/peers/profile",
        &serde_json::to_vec(&p1_prof_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(
        code_p1,
        200,
        "peer1 profile update failed: {}",
        String::from_utf8_lossy(&body_p1)
    );

    // Peer2: device_type="server", group="ops", tags=["lab", "backend"]
    let p2_prof_req = serde_json::json!({
        "peer": peer2_id.as_str(),
        "device_type": "server",
        "group": "ops",
        "tags": ["lab", "backend"],
    });
    let (code_p2, body_p2) = tetherly_node::loopback_post(
        host_ui,
        "/api/peers/profile",
        &serde_json::to_vec(&p2_prof_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(
        code_p2,
        200,
        "peer2 profile update failed: {}",
        String::from_utf8_lossy(&body_p2)
    );

    // 3. Test Multi-Target Message Dispatch via POST /api/message/send-multi:
    // A. Dispatch by Tag: tag="lab" (matches both peer1 and peer2)
    let secret_msg = "Confidential cluster deployment instructions";
    let msg_lab_req = serde_json::json!({
        "tag": "lab",
        "text": secret_msg,
    });
    let (code_msg_lab, body_msg_lab) = tetherly_node::loopback_post(
        host_ui,
        "/api/message/send-multi",
        &serde_json::to_vec(&msg_lab_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_msg_lab, 200);
    let lab_msg_rep: tetherly_node::MultiMessageSendReport =
        serde_json::from_slice(&body_msg_lab).unwrap();
    assert!(lab_msg_rep.ok);
    assert_eq!(
        lab_msg_rep.selector,
        tetherly_node::TargetSelector::Tag("lab".to_string())
    );
    assert_eq!(lab_msg_rep.target_count, 2);
    assert_eq!(lab_msg_rep.success_count, 2);
    assert_eq!(lab_msg_rep.failed_count, 0);
    assert_eq!(lab_msg_rep.dispatches.len(), 2);
    assert!(lab_msg_rep
        .dispatches
        .iter()
        .any(|d| d.peer == peer1_id.as_str() && d.success));
    assert!(lab_msg_rep
        .dispatches
        .iter()
        .any(|d| d.peer == peer2_id.as_str() && d.success));

    // SSE Verification & Zero-Plaintext Invariant on Host:
    // Listen for UiEvent::MultiDispatchCompleted
    let mut saw_msg_dispatch_sse = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while tokio::time::Instant::now() < deadline {
        if let Ok(Ok(Some(raw_str))) =
            tokio::time::timeout(Duration::from_millis(500), sse_host.next_event()).await
        {
            if let Ok(ev) = serde_json::from_str::<serde_json::Value>(&raw_str) {
                if ev["kind"] == "multi_dispatch_completed" && ev["dispatch_kind"] == "message" {
                    assert_eq!(ev["selector"], "tag:lab");
                    assert_eq!(ev["target_count"], 2);
                    assert_eq!(ev["success_count"], 2);
                    assert_eq!(ev["failed_count"], 0);
                    assert!(ev["timestamp_ms"].as_u64().unwrap_or(0) > 0);

                    // ZERO-PLAINTEXT CHECK: Raw SSE event text must NOT leak message plaintext!
                    assert!(
                        !raw_str.contains("Confidential"),
                        "Zero-Plaintext Invariant: raw SSE must NOT contain secret message text!"
                    );
                    assert!(
                        ev.get("text").is_none() && ev.get("message").is_none(),
                        "Zero-Plaintext Invariant: event payload must omit text bodies!"
                    );
                    saw_msg_dispatch_sse = true;
                    break;
                }
            }
        }
    }
    assert!(
        saw_msg_dispatch_sse,
        "must emit multi_dispatch_completed for message"
    );

    // B. Dispatch by Group: group="ops" (matches only peer2) via alias /api/msg/send-multi
    let msg_ops_req = serde_json::json!({
        "group": "ops",
        "text": "Ops emergency alert",
    });
    let (code_msg_ops, body_msg_ops) = tetherly_node::loopback_post(
        host_ui,
        "/api/msg/send-multi",
        &serde_json::to_vec(&msg_ops_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_msg_ops, 200);
    let ops_msg_rep: tetherly_node::MultiMessageSendReport =
        serde_json::from_slice(&body_msg_ops).unwrap();
    assert!(ops_msg_rep.ok);
    assert_eq!(
        ops_msg_rep.selector,
        tetherly_node::TargetSelector::Group("ops".to_string())
    );
    assert_eq!(ops_msg_rep.target_count, 1);
    assert_eq!(ops_msg_rep.success_count, 1);
    assert_eq!(ops_msg_rep.dispatches[0].peer, peer2_id.as_str());

    // C. Dispatch by Type: type="laptop" via single-peer /api/message/send fallback
    let msg_type_req = serde_json::json!({
        "type": "laptop",
        "text": "Laptop maintenance announcement",
    });
    let (code_msg_type, body_msg_type) = tetherly_node::loopback_post(
        host_ui,
        "/api/message/send",
        &serde_json::to_vec(&msg_type_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_msg_type, 200);
    let type_msg_rep: serde_json::Value = serde_json::from_slice(&body_msg_type).unwrap();
    assert_eq!(type_msg_rep["ok"], true);
    assert_eq!(type_msg_rep["target_count"], 1);
    assert_eq!(type_msg_rep["success_count"], 1);

    // 4. Test Multi-Target File Transfer Dispatch via POST /api/file/send-multi:
    // A. Single-target group dispatch (group="dev", matches only peer1)
    use base64::Engine;
    let b64_dev_data =
        base64::engine::general_purpose::STANDARD.encode(b"dev_config_payload_12345");
    let file_dev_req = serde_json::json!({
        "group": "dev",
        "filename": "dev_config.json",
        "data": b64_dev_data,
    });
    let (code_file_dev, body_file_dev) = tetherly_node::loopback_post(
        host_ui,
        "/api/file/send-multi",
        &serde_json::to_vec(&file_dev_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_file_dev, 200);
    let dev_file_rep: tetherly_node::MultiFileSendReport =
        serde_json::from_slice(&body_file_dev).unwrap();
    assert!(dev_file_rep.ok);
    assert_eq!(dev_file_rep.filename, "dev_config.json");
    assert_eq!(
        dev_file_rep.selector,
        tetherly_node::TargetSelector::Group("dev".to_string())
    );
    assert_eq!(dev_file_rep.target_count, 1);
    assert_eq!(dev_file_rep.success_count, 1);
    assert_eq!(dev_file_rep.failed_count, 0);
    assert_eq!(dev_file_rep.dispatches[0].peer, peer1_id.as_str());
    assert!(dev_file_rep.dispatches[0].transfer_id.is_some());
    assert!(dev_file_rep.dispatches[0].outbox_id.is_some());

    // Verify Outbox on Host contains dev_config.json
    let (code_outbox, body_outbox) = tetherly_node::loopback_get(host_ui, "/api/file/outbox")
        .await
        .unwrap();
    assert_eq!(code_outbox, 200);
    let outbox_rep: tetherly_node::OutboxReport = serde_json::from_slice(&body_outbox).unwrap();
    assert!(outbox_rep
        .entries
        .iter()
        .any(|e| e.filename == "dev_config.json" && e.peer == peer1_id.as_str()));

    // B. Multi-target tag fan-out with single-pass hash (tag="lab", matches peer1 and peer2)
    let payload_bytes = b"shared_stream_optimized_asset_data_bytes_987654321";
    let b64_lab_data = base64::engine::general_purpose::STANDARD.encode(payload_bytes);
    let file_lab_req = serde_json::json!({
        "tag": "lab",
        "filename": "shared_asset.bin",
        "data": b64_lab_data,
    });
    let (code_file_lab, body_file_lab) = tetherly_node::loopback_post(
        host_ui,
        "/api/file/send-multi",
        &serde_json::to_vec(&file_lab_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_file_lab, 200);
    let lab_file_rep: tetherly_node::MultiFileSendReport =
        serde_json::from_slice(&body_file_lab).unwrap();
    assert!(lab_file_rep.ok);
    assert_eq!(lab_file_rep.filename, "shared_asset.bin");
    assert_eq!(
        lab_file_rep.selector,
        tetherly_node::TargetSelector::Tag("lab".to_string())
    );
    assert_eq!(lab_file_rep.target_count, 2);
    assert_eq!(lab_file_rep.success_count, 2);
    assert_eq!(lab_file_rep.failed_count, 0);
    assert_eq!(lab_file_rep.dispatches.len(), 2);

    // Verify peer unique transfer IDs were generated to prevent collisions
    let tid1 = lab_file_rep
        .dispatches
        .iter()
        .find(|d| d.peer == peer1_id.as_str())
        .unwrap()
        .transfer_id
        .as_ref()
        .unwrap();
    let tid2 = lab_file_rep
        .dispatches
        .iter()
        .find(|d| d.peer == peer2_id.as_str())
        .unwrap()
        .transfer_id
        .as_ref()
        .unwrap();
    assert_ne!(tid1, tid2, "transfer IDs across peers must be unique");

    // SSE Verification on Host for MultiDispatchCompleted (file)
    let mut saw_file_dispatch_sse = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while tokio::time::Instant::now() < deadline {
        if let Ok(Ok(Some(raw_str))) =
            tokio::time::timeout(Duration::from_millis(500), sse_host.next_event()).await
        {
            if let Ok(ev) = serde_json::from_str::<serde_json::Value>(&raw_str) {
                if ev["kind"] == "multi_dispatch_completed"
                    && ev["dispatch_kind"] == "file"
                    && ev["selector"] == "tag:lab"
                {
                    assert_eq!(ev["target_count"], 2);
                    assert_eq!(ev["success_count"], 2);
                    assert_eq!(ev["failed_count"], 0);
                    assert!(ev["timestamp_ms"].as_u64().unwrap_or(0) > 0);
                    saw_file_dispatch_sse = true;
                    break;
                }
            }
        }
    }
    assert!(
        saw_file_dispatch_sse,
        "must emit multi_dispatch_completed for file"
    );

    // 5. Test Granular Auto-Accept Policy on Peer1 (/api/file/auto-accept/policy & alias /api/auto-accept):
    // A. Query initial policy: default disabled
    let (code_pol_init, body_pol_init) =
        tetherly_node::loopback_get(peer1_ui, "/api/file/auto-accept/policy")
            .await
            .unwrap();
    assert_eq!(code_pol_init, 200);
    let init_pol_rep: tetherly_node::AutoAcceptPolicyReport =
        serde_json::from_slice(&body_pol_init).unwrap();
    assert!(init_pol_rep.ok);
    assert!(!init_pol_rep.policy.enabled);

    // B. Peer1 tags Host as "trusted-ci"
    let host_tag_req = serde_json::json!({
        "peer": host_id.as_str(),
        "tags": ["trusted-ci"],
    });
    let (code_tag_host, _) = tetherly_node::loopback_post(
        peer1_ui,
        "/api/peers/profile",
        &serde_json::to_vec(&host_tag_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_tag_host, 200);

    // C. Update AutoAcceptPolicy on Peer1 to allow tag "trusted-ci" up to 10 MiB
    let update_pol_req = serde_json::json!({
        "enabled": true,
        "allowed_tags": ["trusted-ci"],
        "max_file_size": 10_485_760,
    });
    let (code_up_pol, body_up_pol) = tetherly_node::loopback_post(
        peer1_ui,
        "/api/file/auto-accept/policy",
        &serde_json::to_vec(&update_pol_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_up_pol, 200);
    let up_pol_rep: tetherly_node::AutoAcceptPolicyReport =
        serde_json::from_slice(&body_up_pol).unwrap();
    assert!(up_pol_rep.ok);
    assert!(up_pol_rep.policy.enabled);
    assert_eq!(up_pol_rep.policy.allowed_tags, vec!["trusted-ci"]);
    assert_eq!(up_pol_rep.policy.max_file_size, Some(10_485_760));

    // SSE Verification on Peer1: AutoAcceptPolicyChanged
    let mut saw_policy_sse = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while tokio::time::Instant::now() < deadline {
        if let Ok(Ok(Some(raw_str))) =
            tokio::time::timeout(Duration::from_millis(500), sse_peer1.next_event()).await
        {
            if let Ok(ev) = serde_json::from_str::<serde_json::Value>(&raw_str) {
                if ev["kind"] == "auto_accept_policy_changed" {
                    assert_eq!(ev["enabled"], true);
                    let tags = ev["allowed_tags"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|v| v.as_str().unwrap())
                        .collect::<Vec<_>>();
                    assert_eq!(tags, vec!["trusted-ci"]);
                    assert!(ev["timestamp_ms"].as_u64().unwrap_or(0) > 0);
                    saw_policy_sse = true;
                    break;
                }
            }
        }
    }
    assert!(
        saw_policy_sse,
        "must emit auto_accept_policy_changed SSE event"
    );

    // Verify atomic disk persistence of auto_accept.json
    let auto_accept_path = p1_dir.join(tetherly_node::AUTO_ACCEPT_FILE);
    assert!(
        auto_accept_path.exists(),
        "auto_accept.json must be persisted to disk"
    );

    // D. Test Policy-Triggered Auto-Accept:
    // Host sends a file "ci_artifact.tar" directly to Peer1
    let ci_data = base64::engine::general_purpose::STANDARD.encode(b"automated_ci_build_payload");
    let send_ci_req = serde_json::json!({
        "peer": peer1_id.as_str(),
        "filename": "ci_artifact.tar",
        "data": ci_data,
    });
    let (code_send_ci, body_send_ci) = tetherly_node::loopback_post(
        host_ui,
        "/api/file/send",
        &serde_json::to_vec(&send_ci_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(
        code_send_ci,
        200,
        "file send failed: {}",
        String::from_utf8_lossy(&body_send_ci)
    );

    // Wait briefly and verify Peer1 auto-accepted the transfer
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let mut auto_accepted = false;
    while tokio::time::Instant::now() < deadline {
        let (c, b) = tetherly_node::loopback_get(peer1_ui, "/api/file/auto-accept/policy")
            .await
            .unwrap();
        if c == 200 {
            if let Ok(rep) = serde_json::from_slice::<tetherly_node::AutoAcceptPolicyReport>(&b) {
                if rep.auto_accepted_count >= 1 {
                    auto_accepted = true;
                    break;
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        auto_accepted,
        "peer1 must automatically accept file matching policy"
    );

    // 6. Security Red Line: Loopback Isolation Host 403 Forbidden check on multi endpoints
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut evil_stream = tokio::net::TcpStream::connect(format!("127.0.0.1:{host_ui}"))
        .await
        .expect("connect to host ui port");
    let evil_req = "POST /api/message/send-multi HTTP/1.1\r\nHost: evil.attacker.domain\r\nContent-Length: 2\r\n\r\n{}";
    evil_stream.write_all(evil_req.as_bytes()).await.unwrap();
    evil_stream.flush().await.unwrap();
    let mut evil_buf = vec![0u8; 1024];
    let n = evil_stream.read(&mut evil_buf).await.unwrap();
    let evil_resp = String::from_utf8_lossy(&evil_buf[..n]);
    assert!(
        evil_resp.starts_with("HTTP/1.1 403 Forbidden"),
        "mismatched Host header must be rejected with 403 Forbidden, got: {evil_resp}"
    );

    // 7. Clean shutdown
    host.shutdown();
    peer1.shutdown();
    peer2.shutdown();
}

/// ADR-064: Policy-based clipboard auto-sync, fail-closed sensitive content leak guard,
/// multi-dimensional scope governance, Zero-Plaintext observability, and Host 403 isolation.
#[tokio::test(flavor = "multi_thread")]
async fn policy_based_clipboard_auto_sync_and_sensitive_leak_guard_via_loopback_api() {
    let (host, peer, _g1, _g2) = pair_phone_desktop().await;
    let host_ui = host.spawn_ui().await.expect("host UI succeeds");
    let peer_ui = peer.spawn_ui().await.expect("peer UI succeeds");

    // Connect to host loopback SSE stream
    let mut host_sse = tetherly_node::loopback_events_connect(host_ui)
        .await
        .expect("connect host sse");

    // 1. Initial Default Policy Check via GET /api/clip/policy
    let (code_h, body_h) = tetherly_node::loopback_get(host_ui, "/api/clip/policy")
        .await
        .unwrap();
    assert_eq!(code_h, 200);
    let h_rep: tetherly_node::ClipboardPolicyReport = serde_json::from_slice(&body_h).unwrap();
    assert!(h_rep.ok);
    assert_eq!(h_rep.policy.mode, tetherly_core::ClipSyncMode::Manual);
    assert!(h_rep.policy.sensitive_guard);
    assert_eq!(h_rep.policy.max_payload_bytes, 65536);
    assert!(h_rep.policy.allowed_groups.is_empty());
    assert!(h_rep.policy.allowed_tags.is_empty());
    assert!(h_rep.policy.allowed_types.is_empty());
    assert_eq!(h_rep.auto_synced_out_count, 0);
    assert_eq!(h_rep.auto_synced_in_count, 0);
    assert_eq!(h_rep.sensitive_blocked_count, 0);

    // 2. Configure Peer and Host:
    // a. Peer: set mode to bidirectional_auto, allow only peers with tag "workstation"
    let peer_policy_update = serde_json::json!({
        "mode": "bidirectional_auto",
        "sensitive_guard": true,
        "allowed_tags": ["workstation"]
    });
    let (code_p_set, body_p_set) = tetherly_node::loopback_post(
        peer_ui,
        "/api/clip/policy",
        &serde_json::to_vec(&peer_policy_update).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_p_set, 200);
    let p_set_rep: tetherly_node::ClipboardPolicyReport =
        serde_json::from_slice(&body_p_set).unwrap();
    assert_eq!(
        p_set_rep.policy.mode,
        tetherly_core::ClipSyncMode::BidirectionalAuto
    );
    assert_eq!(p_set_rep.policy.allowed_tags, vec!["workstation"]);

    // b. On Peer, label Host as having tag "workstation"
    let host_dev = host.identity().device_id().to_string();
    let add_tag_body = serde_json::json!({
        "peer": host_dev,
        "tag": "workstation"
    });
    let (code_tag, _) = tetherly_node::loopback_post(
        peer_ui,
        "/api/peers/tags/add",
        &serde_json::to_vec(&add_tag_body).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_tag, 200);

    // c. Host: set mode to bidirectional_auto with sensitive guard enabled
    let host_policy_update = serde_json::json!({
        "mode": "bidirectional_auto",
        "sensitive_guard": true
    });
    let (code_h_set, body_h_set) = tetherly_node::loopback_post(
        host_ui,
        "/api/clip/policy",
        &serde_json::to_vec(&host_policy_update).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_h_set, 200);
    let h_set_rep: tetherly_node::ClipboardPolicyReport =
        serde_json::from_slice(&body_h_set).unwrap();
    assert_eq!(
        h_set_rep.policy.mode,
        tetherly_core::ClipSyncMode::BidirectionalAuto
    );
    assert!(h_set_rep.policy.sensitive_guard);

    // d. Check SSE receives ClipboardPolicyChanged
    let mut got_policy_ev = false;
    let t_deadline = std::time::Instant::now() + Duration::from_secs(5);
    while std::time::Instant::now() < t_deadline {
        if let Ok(Ok(Some(raw))) =
            tokio::time::timeout(Duration::from_millis(200), host_sse.next_event()).await
        {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
                if v.get("kind").and_then(|k| k.as_str()) == Some("clipboard_policy_changed") {
                    assert_eq!(
                        v.get("mode").and_then(|m| m.as_str()),
                        Some("bidirectional_auto")
                    );
                    assert_eq!(
                        v.get("sensitive_guard").and_then(|b| b.as_bool()),
                        Some(true)
                    );
                    got_policy_ev = true;
                    break;
                }
            }
        }
    }
    assert!(
        got_policy_ev,
        "host SSE must receive clipboard_policy_changed"
    );

    // 3. Sensitive Content Guard Interception (Fail-Closed) & Zero-Plaintext Invariant
    // a. OpenSSH Private Key
    let ssh_key = "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAABlwAAAAdzc2gtcn\n-----END OPENSSH PRIVATE KEY-----";
    host.clip().set_text(ssh_key).expect("set host clipboard");

    let (code_poll1, body_poll1) =
        tetherly_node::loopback_post(host_ui, "/api/clip/auto-sync/poll", b"{}")
            .await
            .unwrap();
    assert_eq!(code_poll1, 200);
    let poll1_rep: serde_json::Value = serde_json::from_slice(&body_poll1).unwrap();
    assert_eq!(poll1_rep["dispatched_peers"].as_u64().unwrap_or(999), 0);
    assert_eq!(
        poll1_rep["stats"]["sensitive_blocked_count"]
            .as_u64()
            .unwrap_or(0),
        1
    );

    // Verify SSE received clipboard_sync_blocked with ZERO PLAINTEXT
    let mut got_block_ev = false;
    let t_deadline = std::time::Instant::now() + Duration::from_secs(5);
    while std::time::Instant::now() < t_deadline {
        if let Ok(Ok(Some(raw))) =
            tokio::time::timeout(Duration::from_millis(200), host_sse.next_event()).await
        {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
                if v.get("kind").and_then(|k| k.as_str()) == Some("clipboard_sync_blocked") {
                    assert_eq!(
                        v.get("reason").and_then(|r| r.as_str()),
                        Some("private_key")
                    );
                    assert_eq!(
                        v.get("hash").and_then(|h| h.as_str()),
                        Some(tetherly_core::hex_sha256(ssh_key.as_bytes()).as_str())
                    );
                    // ZERO-PLAINTEXT INVARIANT: raw json must NEVER contain any segment of the secret
                    assert!(!raw.contains("OPENSSH PRIVATE KEY"));
                    assert!(!raw.contains("b3BlbnNzaC1rZXk"));
                    got_block_ev = true;
                    break;
                }
            }
        }
    }
    assert!(
        got_block_ev,
        "host SSE must receive clipboard_sync_blocked for private_key"
    );

    // Peer clipboard must remain unaffected
    let (code_p_clip, body_p_clip) = tetherly_node::loopback_get(peer_ui, "/api/clip")
        .await
        .unwrap();
    assert_eq!(code_p_clip, 200);
    let p_clip_rep: serde_json::Value = serde_json::from_slice(&body_p_clip).unwrap();
    assert!(p_clip_rep["text"].is_null());

    // b. API Token: AWS Access Key
    let aws_token = "AKIAIOSFODNN7EXAMPLE";
    host.clip().set_text(aws_token).expect("set host clipboard");
    let (code_poll2, body_poll2) =
        tetherly_node::loopback_post(host_ui, "/api/clip/auto-sync/poll", b"{}")
            .await
            .unwrap();
    assert_eq!(code_poll2, 200);
    let poll2_rep: serde_json::Value = serde_json::from_slice(&body_poll2).unwrap();
    assert_eq!(poll2_rep["dispatched_peers"].as_u64().unwrap_or(999), 0);
    assert_eq!(
        poll2_rep["stats"]["sensitive_blocked_count"]
            .as_u64()
            .unwrap_or(0),
        2
    );

    // c. Bearer JWT
    let jwt_token = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIiwibmFtZSI6IkpvaG4gRG9lIiwiaWF0IjoxNTE2MjM5MDIyfQ.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c";
    host.clip().set_text(jwt_token).expect("set host clipboard");
    let (code_poll3, body_poll3) =
        tetherly_node::loopback_post(host_ui, "/api/clip/auto-sync/poll", b"{}")
            .await
            .unwrap();
    assert_eq!(code_poll3, 200);
    let poll3_rep: serde_json::Value = serde_json::from_slice(&body_poll3).unwrap();
    assert_eq!(poll3_rep["dispatched_peers"].as_u64().unwrap_or(999), 0);
    assert_eq!(
        poll3_rep["stats"]["sensitive_blocked_count"]
            .as_u64()
            .unwrap_or(0),
        3
    );

    // 4. Compliant Everyday Text Auto-Sync
    let clean_text = "Tetherly LAN mesh automated cross-device clipboard sync v1.71";
    host.clip()
        .set_text(clean_text)
        .expect("set host clipboard");

    let (code_poll4, body_poll4) =
        tetherly_node::loopback_post(host_ui, "/api/clip/auto-sync/poll", b"{}")
            .await
            .unwrap();
    assert_eq!(code_poll4, 200);
    let poll4_rep: serde_json::Value = serde_json::from_slice(&body_poll4).unwrap();
    assert_eq!(poll4_rep["dispatched_peers"].as_u64().unwrap_or(0), 1);
    assert_eq!(
        poll4_rep["stats"]["auto_synced_out_count"]
            .as_u64()
            .unwrap_or(0),
        1
    );

    // Wait for Peer to receive and apply clipboard frame
    let t_deadline = std::time::Instant::now() + Duration::from_secs(5);
    let mut peer_received = false;
    while std::time::Instant::now() < t_deadline {
        let (c, b) = tetherly_node::loopback_get(peer_ui, "/api/clip")
            .await
            .unwrap();
        if c == 200 {
            if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&b) {
                if v.get("text").and_then(|t| t.as_str()) == Some(clean_text) {
                    peer_received = true;
                    break;
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        peer_received,
        "peer must receive compliant auto-synced clipboard text"
    );

    // Check Peer stats: auto_synced_in_count == 1
    let (code_p_stat, body_p_stat) = tetherly_node::loopback_get(peer_ui, "/api/clip/policy")
        .await
        .unwrap();
    assert_eq!(code_p_stat, 200);
    let p_stat_rep: tetherly_node::ClipboardPolicyReport =
        serde_json::from_slice(&body_p_stat).unwrap();
    assert_eq!(p_stat_rep.auto_synced_in_count, 1);

    // 5. Max Payload Size Ceiling Enforcement
    let ceiling_policy = serde_json::json!({
        "max_payload_bytes": 30
    });
    let (code_ceil, _) = tetherly_node::loopback_post(
        host_ui,
        "/api/clip/policy",
        &serde_json::to_vec(&ceiling_policy).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_ceil, 200);

    let oversized_text = "This string definitely exceeds thirty bytes of length ceiling";
    host.clip()
        .set_text(oversized_text)
        .expect("set oversized clipboard");

    let (code_poll5, body_poll5) =
        tetherly_node::loopback_post(host_ui, "/api/clip/auto-sync/poll", b"{}")
            .await
            .unwrap();
    assert_eq!(code_poll5, 200);
    let poll5_rep: serde_json::Value = serde_json::from_slice(&body_poll5).unwrap();
    assert_eq!(poll5_rep["dispatched_peers"].as_u64().unwrap_or(999), 0);

    // 6. Inbound Scope Governance Filtering:
    // Update peer policy to allow only peers with tag "restricted_tag_only"
    let scope_policy = serde_json::json!({
        "mode": "bidirectional_auto",
        "allowed_tags": ["restricted_tag_only"]
    });
    let (code_scope, _) = tetherly_node::loopback_post(
        peer_ui,
        "/api/clip/policy",
        &serde_json::to_vec(&scope_policy).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_scope, 200);

    // Reset host policy max_payload_bytes
    let reset_host_pol = serde_json::json!({
        "max_payload_bytes": 65536
    });
    let (code_reset, _) = tetherly_node::loopback_post(
        host_ui,
        "/api/clip/policy",
        &serde_json::to_vec(&reset_host_pol).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_reset, 200);

    // Host sends a new text; since Host is not tagged with "restricted_tag_only" on peer,
    // peer should reject it in apply_clip!
    let scoped_text = "New text that should be rejected by peer scope policy";
    host.clip()
        .set_text(scoped_text)
        .expect("set host clipboard");
    let (code_poll6, _) = tetherly_node::loopback_post(host_ui, "/api/clip/auto-sync/poll", b"{}")
        .await
        .unwrap();
    assert_eq!(code_poll6, 200);

    // Wait and verify peer clipboard was NOT updated to scoped_text (remains clean_text)
    tokio::time::sleep(Duration::from_millis(200)).await;
    let (_, b_clip_check) = tetherly_node::loopback_get(peer_ui, "/api/clip")
        .await
        .unwrap();
    let clip_check: serde_json::Value = serde_json::from_slice(&b_clip_check).unwrap();
    assert_eq!(
        clip_check.get("text").and_then(|t| t.as_str()),
        Some(clean_text),
        "peer must not accept clipboard text from peer not matching allowed_tags"
    );

    // 7. Atomic Persistence Permissions Verification (POSIX 0o600)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let host_pol_file = _g1.path().join("clipboard_policy.json");
        assert!(host_pol_file.exists());
        let meta = std::fs::metadata(&host_pol_file).unwrap();
        let mode = meta.permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o600,
            "clipboard_policy.json must be persisted with 0o600 permissions"
        );
    }

    // 8. Security Red Line: Host 403 Forbidden Isolation Check
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut evil_stream = tokio::net::TcpStream::connect(format!("127.0.0.1:{host_ui}"))
        .await
        .expect("connect to host ui port");
    let evil_req = "GET /api/clip/policy HTTP/1.1\r\nHost: malicious.hacker.internal\r\n\r\n";
    evil_stream.write_all(evil_req.as_bytes()).await.unwrap();
    evil_stream.flush().await.unwrap();
    let mut evil_buf = vec![0u8; 1024];
    let n = evil_stream.read(&mut evil_buf).await.unwrap();
    let evil_resp = String::from_utf8_lossy(&evil_buf[..n]);
    assert!(
        evil_resp.starts_with("HTTP/1.1 403 Forbidden"),
        "mismatched Host header on /api/clip/policy must be rejected with 403 Forbidden, got: {evil_resp}"
    );

    // 9. Clean shutdown
    host.shutdown();
    peer.shutdown();
}

/// ADR-065: Cross-Device Secure Resource & URL Handoff Pipeline, Controlled Open Governance,
/// Ingress Policy Scoping, Handoff Inbox Lifecycle, Zero-Plaintext SSE, and Host 403 Isolation.
#[tokio::test(flavor = "multi_thread")]
async fn resource_and_url_handoff_pipeline_and_governance_via_loopback_api() {
    let (host, peer, _g1, g2) = pair_phone_desktop().await;
    let host_ui = host.spawn_ui().await.expect("host UI spawns");
    let peer_ui = peer.spawn_ui().await.expect("peer UI spawns");
    let host_id = host.identity().device_id().clone();
    let peer_id = peer.identity().device_id().clone();

    // 1. Connect SSE event streams on both nodes
    let mut sse_host = tetherly_node::loopback_events_connect(host_ui)
        .await
        .expect("host SSE connects");
    let mut sse_peer = tetherly_node::loopback_events_connect(peer_ui)
        .await
        .expect("peer SSE connects");

    // 2. Query initial handoff policy on Peer: default is prompt_only, empty filters, max 100 entries
    let (code_pol, body_pol) = tetherly_node::loopback_get(peer_ui, "/api/handoff/policy")
        .await
        .unwrap();
    assert_eq!(code_pol, 200);
    let pol_rep: serde_json::Value = serde_json::from_slice(&body_pol).unwrap();
    assert!(pol_rep["ok"].as_bool().unwrap_or(false));
    assert_eq!(pol_rep["policy"]["action"], "prompt_only");
    assert_eq!(pol_rep["policy"]["max_inbox_entries"], 100);

    // Initial inbox on Peer must be empty
    let (code_inbox, body_inbox) = tetherly_node::loopback_get(peer_ui, "/api/handoff/inbox")
        .await
        .unwrap();
    assert_eq!(code_inbox, 200);
    let inbox_rep: serde_json::Value = serde_json::from_slice(&body_inbox).unwrap();
    assert!(inbox_rep["ok"].as_bool().unwrap_or(false));
    assert_eq!(inbox_rep["total"], 0);
    assert_eq!(inbox_rep["entries"].as_array().unwrap().len(), 0);

    // 3. Test Compliant URL Handoff Dispatch under default prompt_only policy:
    // Host sends URL to Peer
    let handoff_req = serde_json::json!({
        "peer": peer_id.as_str(),
        "url": "https://docs.tetherly.mesh/v1.72/handoff-guide",
        "title": "Tetherly Handoff Guide",
        "auto_open": true
    });
    let (code_send, body_send) = tetherly_node::loopback_post(
        host_ui,
        "/api/handoff/send",
        &serde_json::to_vec(&handoff_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_send, 200);
    let send_rep: serde_json::Value = serde_json::from_slice(&body_send).unwrap();
    assert!(send_rep["ok"].as_bool().unwrap_or(false));
    let handoff_id = send_rep["id"].as_str().unwrap().to_string();
    assert!(handoff_id.starts_with("ho_"));
    assert_eq!(send_rep["peer"], peer_id.as_str());
    assert_eq!(
        send_rep["url"],
        "https://docs.tetherly.mesh/v1.72/handoff-guide"
    );
    assert_eq!(send_rep["title"], "Tetherly Handoff Guide");

    // SSE Verification on Host: resource_handoff_dispatched
    let mut saw_dispatched_sse = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while tokio::time::Instant::now() < deadline {
        if let Ok(Ok(Some(raw_str))) =
            tokio::time::timeout(Duration::from_millis(500), sse_host.next_event()).await
        {
            if let Ok(ev) = serde_json::from_str::<serde_json::Value>(&raw_str) {
                if ev["kind"] == "resource_handoff_dispatched" {
                    assert_eq!(ev["host"], "docs.tetherly.mesh");
                    assert_eq!(ev["target_count"], 1);
                    assert_eq!(ev["success_count"], 1);
                    assert!(ev["timestamp_ms"].as_u64().unwrap_or(0) > 0);
                    saw_dispatched_sse = true;
                    break;
                }
            }
        }
    }
    assert!(
        saw_dispatched_sse,
        "host must emit resource_handoff_dispatched SSE event"
    );

    // SSE Verification on Peer: resource_handoff_received with Zero-Plaintext check
    let mut saw_received_sse = false;
    let deadline_peer = tokio::time::Instant::now() + Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline_peer {
        if let Ok(Ok(Some(raw_str))) =
            tokio::time::timeout(Duration::from_millis(500), sse_peer.next_event()).await
        {
            if let Ok(ev) = serde_json::from_str::<serde_json::Value>(&raw_str) {
                if ev["kind"] == "resource_handoff_received" {
                    assert_eq!(ev["id"], handoff_id);
                    assert_eq!(ev["host"], "docs.tetherly.mesh");
                    assert_eq!(ev["title"], "Tetherly Handoff Guide");
                    assert_eq!(ev["status"], "pending");
                    assert_eq!(ev["auto_opened"], false);

                    // ZERO-PLAINTEXT INVARIANT: Raw SSE event must only expose host and title,
                    // not arbitrary URL paths/queries that could contain credentials or confidential tokens!
                    assert!(
                        !raw_str.contains("/v1.72/handoff-guide"),
                        "Zero-Plaintext Invariant: raw SSE must NOT expose full URL path or query params"
                    );
                    assert!(
                        ev.get("url").is_none(),
                        "Zero-Plaintext Invariant: event payload must omit raw URL"
                    );
                    saw_received_sse = true;
                    break;
                }
            }
        }
    }
    assert!(
        saw_received_sse,
        "peer must emit resource_handoff_received SSE event"
    );

    // 4. Verify Entry Stored in Peer Inbox:
    let (code_inbox1, body_inbox1) = tetherly_node::loopback_get(peer_ui, "/api/handoff/inbox")
        .await
        .unwrap();
    assert_eq!(code_inbox1, 200);
    let inbox1: serde_json::Value = serde_json::from_slice(&body_inbox1).unwrap();
    assert_eq!(inbox1["total"], 1);
    let entry1 = &inbox1["entries"][0];
    assert_eq!(entry1["id"], handoff_id);
    assert_eq!(
        entry1["url"],
        "https://docs.tetherly.mesh/v1.72/handoff-guide"
    );
    assert_eq!(entry1["title"], "Tetherly Handoff Guide");
    assert_eq!(entry1["status"], "pending");

    // 5. Test Controlled Open via POST /api/handoff/inbox/open:
    let open_req = serde_json::json!({
        "id": handoff_id
    });
    let (code_open, body_open) = tetherly_node::loopback_post(
        peer_ui,
        "/api/handoff/inbox/open",
        &serde_json::to_vec(&open_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_open, 200);
    let open_rep: serde_json::Value = serde_json::from_slice(&body_open).unwrap();
    assert!(open_rep["ok"].as_bool().unwrap_or(false));
    assert_eq!(open_rep["id"], handoff_id);
    assert_eq!(open_rep["status"], "opened");
    assert!(open_rep["opened_ms"].as_u64().unwrap_or(0) > 0);

    // Verify inbox status updated to "opened"
    let (_, body_inbox_op) =
        tetherly_node::loopback_get(peer_ui, "/api/handoff/inbox?status=opened")
            .await
            .unwrap();
    let inbox_op: serde_json::Value = serde_json::from_slice(&body_inbox_op).unwrap();
    assert_eq!(inbox_op["total"], 1);
    assert_eq!(inbox_op["entries"][0]["status"], "opened");

    // 6. Security Firewall: ResourceHandoffValidator Negative Injections Testing
    // a. Local file traversal & sensitive access: file:///etc/passwd
    let bad_file_req = serde_json::json!({
        "peer": peer_id.as_str(),
        "url": "file:///etc/passwd"
    });
    let (c_bf, b_bf) = tetherly_node::loopback_post(
        host_ui,
        "/api/handoff/send",
        &serde_json::to_vec(&bad_file_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c_bf, 400);
    assert!(String::from_utf8_lossy(&b_bf).contains("forbidden dangerous url scheme: 'file'"));

    // b. Script injection: javascript:alert(document.cookie)
    let bad_js_req = serde_json::json!({
        "peer": peer_id.as_str(),
        "url": "javascript:alert(document.cookie)"
    });
    let (c_js, b_js) = tetherly_node::loopback_post(
        host_ui,
        "/api/handoff/send",
        &serde_json::to_vec(&bad_js_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c_js, 400);
    assert!(String::from_utf8_lossy(&b_js).contains("forbidden dangerous url scheme: 'javascript'"));

    // c. Credential injection: http://admin:secret@evil.com/admin
    let bad_cred_req = serde_json::json!({
        "peer": peer_id.as_str(),
        "url": "http://admin:secret@evil.com/admin"
    });
    let (c_cr, b_cr) = tetherly_node::loopback_post(
        host_ui,
        "/api/handoff/send",
        &serde_json::to_vec(&bad_cred_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c_cr, 400);
    assert!(String::from_utf8_lossy(&b_cr)
        .contains("userinfo credentials not permitted in handoff url"));

    // d. Cloud metadata probe: http://169.254.169.254/latest/meta-data/
    let bad_meta_req = serde_json::json!({
        "peer": peer_id.as_str(),
        "url": "http://169.254.169.254/latest/meta-data/"
    });
    let (c_mt, b_mt) = tetherly_node::loopback_post(
        host_ui,
        "/api/handoff/send",
        &serde_json::to_vec(&bad_meta_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c_mt, 400);
    assert!(String::from_utf8_lossy(&b_mt).contains("forbidden access to cloud metadata address"));

    // e. CRLF / Control char injection: https://example.com/test\r\nX-Injected:evil
    let bad_crlf_req = serde_json::json!({
        "peer": peer_id.as_str(),
        "url": "https://example.com/test\r\nX-Injected:evil"
    });
    let (c_cl, b_cl) = tetherly_node::loopback_post(
        host_ui,
        "/api/handoff/send",
        &serde_json::to_vec(&bad_crlf_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c_cl, 400);
    assert!(String::from_utf8_lossy(&b_cl)
        .contains("handoff url contains invalid control or whitespace characters"));

    // 7. Policy Scoping & AutoOpen Ingress Mode:
    // Peer configures policy: action = "auto_open", allowed_groups = ["dev-ops"]
    let pol_update_req = serde_json::json!({
        "action": "auto_open",
        "allowed_groups": ["dev-ops"]
    });
    let (c_p_up, b_p_up) = tetherly_node::loopback_post(
        peer_ui,
        "/api/handoff/policy",
        &serde_json::to_vec(&pol_update_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c_p_up, 200);
    let p_up_rep: serde_json::Value = serde_json::from_slice(&b_p_up).unwrap();
    assert_eq!(p_up_rep["policy"]["action"], "auto_open");
    assert_eq!(
        p_up_rep["policy"]["allowed_groups"],
        serde_json::json!(["dev-ops"])
    );

    // First attempt: Host is not in "dev-ops" group on Peer.
    // Host sends URL with auto_open: true.
    let auto_req1 = serde_json::json!({
        "peer": peer_id.as_str(),
        "url": "https://crates.io/crates/tokio",
        "title": "Tokio Crate",
        "auto_open": true
    });
    let (c_ar1, b_ar1) = tetherly_node::loopback_post(
        host_ui,
        "/api/handoff/send",
        &serde_json::to_vec(&auto_req1).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c_ar1, 200);
    let ar1_rep: serde_json::Value = serde_json::from_slice(&b_ar1).unwrap();
    let ho_id_unscoped = ar1_rep["id"].as_str().unwrap().to_string();

    // Wait and verify Peer stored it as pending (NOT auto-opened because Host is not in dev-ops group)
    let t_check = tokio::time::Instant::now() + Duration::from_secs(3);
    let mut saw_unscoped = false;
    while tokio::time::Instant::now() < t_check {
        let (c, b) = tetherly_node::loopback_get(peer_ui, "/api/handoff/inbox?status=pending")
            .await
            .unwrap();
        if c == 200 {
            if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&b) {
                if val["entries"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|e| e["id"] == ho_id_unscoped)
                {
                    saw_unscoped = true;
                    break;
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        saw_unscoped,
        "unscoped peer handoff must be saved as pending without auto-open"
    );

    // Second attempt: Group Host under "dev-ops" on Peer
    let set_host_grp = serde_json::json!({
        "peer": host_id.as_str(),
        "group": "dev-ops"
    });
    let (c_grp, _) = tetherly_node::loopback_post(
        peer_ui,
        "/api/peers/group",
        &serde_json::to_vec(&set_host_grp).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c_grp, 200);

    // Host sends URL again with auto_open: true. Now Host IS in "dev-ops" group!
    let auto_req2 = serde_json::json!({
        "peer": peer_id.as_str(),
        "url": "https://rust-lang.org",
        "title": "Rust Language Official",
        "auto_open": true
    });
    let (c_ar2, b_ar2) = tetherly_node::loopback_post(
        host_ui,
        "/api/handoff/send",
        &serde_json::to_vec(&auto_req2).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c_ar2, 200);
    let ar2_rep: serde_json::Value = serde_json::from_slice(&b_ar2).unwrap();
    let ho_id_scoped = ar2_rep["id"].as_str().unwrap().to_string();

    // Verify Peer receives and automatically opens it!
    let t_scoped = tokio::time::Instant::now() + Duration::from_secs(4);
    let mut saw_auto_opened = false;
    while tokio::time::Instant::now() < t_scoped {
        let (c, b) = tetherly_node::loopback_get(peer_ui, "/api/handoff/inbox?status=opened")
            .await
            .unwrap();
        if c == 200 {
            if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&b) {
                if val["entries"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|e| e["id"] == ho_id_scoped)
                {
                    saw_auto_opened = true;
                    break;
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        saw_auto_opened,
        "scoped peer matching group must be automatically opened"
    );

    // 8. Multi-cast Handoff Dispatch via POST /api/handoff/send with all: true
    let multi_req = serde_json::json!({
        "all": true,
        "url": "https://github.com/tetherly/tetherly",
        "title": "Tetherly Repo",
        "auto_open": false
    });
    let (c_multi, b_multi) = tetherly_node::loopback_post(
        host_ui,
        "/api/handoff/send",
        &serde_json::to_vec(&multi_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c_multi, 200);
    let multi_rep: serde_json::Value = serde_json::from_slice(&b_multi).unwrap();
    assert!(multi_rep["ok"].as_bool().unwrap_or(false));
    assert!(multi_rep["target_count"].as_u64().unwrap_or(0) >= 1);
    assert!(multi_rep["success_count"].as_u64().unwrap_or(0) >= 1);

    // 9. Inbox Dismiss & Clear Lifecycle:
    // Dismiss unscoped entry
    let dis_req = serde_json::json!({
        "id": ho_id_unscoped
    });
    let (c_dis, b_dis) = tetherly_node::loopback_post(
        peer_ui,
        "/api/handoff/inbox/dismiss",
        &serde_json::to_vec(&dis_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c_dis, 200);
    let dis_rep: serde_json::Value = serde_json::from_slice(&b_dis).unwrap();
    assert_eq!(dis_rep["status"], "dismissed");

    // Query dismissed entries
    let (c_inbox_dis, b_inbox_dis) =
        tetherly_node::loopback_get(peer_ui, "/api/handoff/inbox?status=dismissed")
            .await
            .unwrap();
    assert_eq!(c_inbox_dis, 200);
    let inbox_dis: serde_json::Value = serde_json::from_slice(&b_inbox_dis).unwrap();
    assert!(inbox_dis["entries"]
        .as_array()
        .unwrap()
        .iter()
        .any(|e| e["id"] == ho_id_unscoped));

    // Clear inbox
    let (c_clr, b_clr) = tetherly_node::loopback_post(peer_ui, "/api/handoff/inbox/clear", b"{}")
        .await
        .unwrap();
    assert_eq!(c_clr, 200);
    let clr_rep: serde_json::Value = serde_json::from_slice(&b_clr).unwrap();
    assert!(clr_rep["ok"].as_bool().unwrap_or(false));
    assert!(clr_rep["cleared"].as_u64().unwrap_or(0) >= 1);

    // After clear, total entries in inbox is 0
    let (_, b_empty) = tetherly_node::loopback_get(peer_ui, "/api/handoff/inbox")
        .await
        .unwrap();
    let empty_rep: serde_json::Value = serde_json::from_slice(&b_empty).unwrap();
    assert_eq!(empty_rep["total"], 0);
    assert!(empty_rep["entries"].as_array().unwrap().is_empty());

    // 10. Atomic Persistence & POSIX 0o600 Permissions Check
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let peer_pol_file = g2.path().join("handoff_policy.json");
        assert!(
            peer_pol_file.exists(),
            "handoff_policy.json must exist on disk"
        );
        let meta_pol = std::fs::metadata(&peer_pol_file).unwrap();
        assert_eq!(
            meta_pol.permissions().mode() & 0o777,
            0o600,
            "handoff_policy.json must have 0o600 permissions"
        );

        let peer_inbox_file = g2.path().join("handoff_inbox.json");
        assert!(
            peer_inbox_file.exists(),
            "handoff_inbox.json must exist on disk"
        );
        let meta_inbox = std::fs::metadata(&peer_inbox_file).unwrap();
        assert_eq!(
            meta_inbox.permissions().mode() & 0o777,
            0o600,
            "handoff_inbox.json must have 0o600 permissions"
        );
    }

    // 11. Security Red Line: Host 403 Forbidden Isolation Check
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut evil_stream = tokio::net::TcpStream::connect(format!("127.0.0.1:{peer_ui}"))
        .await
        .expect("connect to peer ui port");
    let evil_req = "GET /api/handoff/inbox HTTP/1.1\r\nHost: evil.attacker.domain\r\n\r\n";
    evil_stream.write_all(evil_req.as_bytes()).await.unwrap();
    evil_stream.flush().await.unwrap();
    let mut evil_buf = vec![0u8; 1024];
    let n = evil_stream.read(&mut evil_buf).await.unwrap();
    let evil_resp = String::from_utf8_lossy(&evil_buf[..n]);
    assert!(
        evil_resp.starts_with("HTTP/1.1 403 Forbidden"),
        "mismatched Host header on /api/handoff/inbox must be rejected with 403 Forbidden, got: {evil_resp}"
    );

    // 12. Clean shutdown
    host.shutdown();
    peer.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn media_playback_control_pipeline_and_state_sync_via_loopback_api() {
    let (host, peer, _g1, g2) = pair_phone_desktop().await;
    let host_ui = host.spawn_ui().await.expect("host UI spawns");
    let peer_ui = peer.spawn_ui().await.expect("peer UI spawns");
    let host_id = host.identity().device_id().clone();
    let peer_id = peer.identity().device_id().clone();

    // 3. Connect to Peer SSE Event Stream:
    let mut peer_sse = tetherly_node::loopback_events_connect(peer_ui)
        .await
        .expect("connect to peer sse stream");

    // 4. Initial Policy & State Check:
    let (code_pol, body_pol) = tetherly_node::loopback_get(peer_ui, "/api/media/policy")
        .await
        .unwrap();
    assert_eq!(code_pol, 200);
    let pol_init: serde_json::Value = serde_json::from_slice(&body_pol).unwrap();
    assert_eq!(pol_init["policy"]["mode"], "allow");
    assert_eq!(pol_init["policy"]["max_volume_limit"], 100);
    assert_eq!(pol_init["policy"]["allow_seek"], true);

    // 5. Peer Updates Local Player Playback State:
    let state_update_req = serde_json::json!({
        "title": "Rustacean Symphony No. 9",
        "artist": "Ferris & The Crabs",
        "album": "Memory Safety",
        "state": "Playing",
        "volume": 75,
        "position_ms": 30000,
        "duration_ms": 240000,
        "can_play": false,
        "can_pause": true
    });
    let (code_st, body_st) = tetherly_node::loopback_post(
        peer_ui,
        "/api/media/state",
        &serde_json::to_vec(&state_update_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_st, 200);
    let st_rep: serde_json::Value = serde_json::from_slice(&body_st).unwrap();
    assert!(st_rep["ok"].as_bool().unwrap_or(false));
    assert_eq!(st_rep["state"]["title"], "Rustacean Symphony No. 9");
    assert_eq!(st_rep["state"]["artist"], "Ferris & The Crabs");
    assert_eq!(st_rep["state"]["state"], "playing");
    assert_eq!(st_rep["state"]["volume_percent"], 75);

    // Host queries Peer's remote playback state:
    let t_remote_st = tokio::time::Instant::now() + Duration::from_secs(4);
    let mut remote_state_synced = false;
    while tokio::time::Instant::now() < t_remote_st {
        let (c, b) =
            tetherly_node::loopback_get(host_ui, &format!("/api/media/state?peer={}", peer_id))
                .await
                .unwrap();
        if c == 200 {
            if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&b) {
                if let Some(st) = val.get("state").filter(|s| !s.is_null()) {
                    if st["title"] == "Rustacean Symphony No. 9" && st["state"] == "playing" {
                        remote_state_synced = true;
                        break;
                    }
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        remote_state_synced,
        "host must receive peer's published media state via TYPE_MEDIA_STATE wire broadcast"
    );

    // 6. Host Sends Remote 'pause' Command:
    let pause_cmd_req = serde_json::json!({
        "peer": peer_id.as_str(),
        "command": "pause"
    });
    let (code_cmd1, body_cmd1) = tetherly_node::loopback_post(
        host_ui,
        "/api/media/command",
        &serde_json::to_vec(&pause_cmd_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_cmd1, 200);
    let cmd1_rep: serde_json::Value = serde_json::from_slice(&body_cmd1).unwrap();
    assert!(cmd1_rep["ok"].as_bool().unwrap_or(false));
    assert_eq!(cmd1_rep["command"], "pause");
    assert_eq!(cmd1_rep["peer"], peer_id.as_str());

    // Verify Peer receives command and updates player state to Paused:
    let t_pause = tokio::time::Instant::now() + Duration::from_secs(4);
    let mut pause_verified = false;
    while tokio::time::Instant::now() < t_pause {
        let (c, b) = tetherly_node::loopback_get(peer_ui, "/api/media/state")
            .await
            .unwrap();
        if c == 200 {
            if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&b) {
                if let Some(st) = val.get("state").filter(|s| !s.is_null()) {
                    if st["state"] == "paused" && st["can_play"] == true && st["can_pause"] == false
                    {
                        pause_verified = true;
                        break;
                    }
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        pause_verified,
        "peer player state machine must transition to Paused upon receiving Pause command"
    );

    // 7. Host Sends 'seek' Command (offset_ms: 15000):
    let seek_cmd_req = serde_json::json!({
        "peer": peer_id.as_str(),
        "command": "seek",
        "offset_ms": 15000
    });
    let (code_seek, _) = tetherly_node::loopback_post(
        host_ui,
        "/api/media/command",
        &serde_json::to_vec(&seek_cmd_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_seek, 200);

    // Verify position advanced from 30000 to 45000 ms:
    let t_seek = tokio::time::Instant::now() + Duration::from_secs(4);
    let mut seek_verified = false;
    while tokio::time::Instant::now() < t_seek {
        let (c, b) = tetherly_node::loopback_get(peer_ui, "/api/media/state")
            .await
            .unwrap();
        if c == 200 {
            if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&b) {
                if let Some(st) = val.get("state").filter(|s| !s.is_null()) {
                    if st["position_ms"] == 45000 {
                        seek_verified = true;
                        break;
                    }
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        seek_verified,
        "peer player position must advance by 15000ms upon receiving Seek command"
    );

    // 8. Volume Safety Clamping & Policy Ceiling:
    // Peer updates policy with max_volume_limit = 60:
    let pol_update_req = serde_json::json!({
        "max_volume_limit": 60
    });
    let (c_pup, b_pup) = tetherly_node::loopback_post(
        peer_ui,
        "/api/media/policy",
        &serde_json::to_vec(&pol_update_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c_pup, 200);
    let pup_rep: serde_json::Value = serde_json::from_slice(&b_pup).unwrap();
    assert_eq!(pup_rep["policy"]["max_volume_limit"], 60);

    // Host attempts to set volume to 95%:
    let vol_cmd_req = serde_json::json!({
        "peer": peer_id.as_str(),
        "command": "volume",
        "volume": 95
    });
    let (code_vol, _) = tetherly_node::loopback_post(
        host_ui,
        "/api/media/command",
        &serde_json::to_vec(&vol_cmd_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_vol, 200);

    // Verify Peer player volume was clamped to 60%:
    let t_vol = tokio::time::Instant::now() + Duration::from_secs(4);
    let mut vol_clamped = false;
    while tokio::time::Instant::now() < t_vol {
        let (c, b) = tetherly_node::loopback_get(peer_ui, "/api/media/state")
            .await
            .unwrap();
        if c == 200 {
            if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&b) {
                if let Some(st) = val.get("state").filter(|s| !s.is_null()) {
                    if st["volume_percent"] == 60 {
                        vol_clamped = true;
                        break;
                    }
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        vol_clamped,
        "peer must clamp requested 95% volume down to max_volume_limit of 60%"
    );

    // 9. Profile Scoping: Group-based Ingress Access Control:
    // Peer configures policy to only allow members of group "living-room":
    let group_pol_req = serde_json::json!({
        "allowed_groups": ["living-room"]
    });
    let (c_gpol, _) = tetherly_node::loopback_post(
        peer_ui,
        "/api/media/policy",
        &serde_json::to_vec(&group_pol_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c_gpol, 200);

    // Host is currently NOT in "living-room" group on Peer.
    // Host sends 'toggle' command:
    let toggle_cmd_req = serde_json::json!({
        "peer": peer_id.as_str(),
        "command": "toggle"
    });
    let (c_tog1, _) = tetherly_node::loopback_post(
        host_ui,
        "/api/media/command",
        &serde_json::to_vec(&toggle_cmd_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c_tog1, 200);

    // Sleep briefly to ensure packet arrived and was processed:
    tokio::time::sleep(Duration::from_millis(300)).await;

    // Verify Peer state is still Paused (command was blocked by policy):
    let (_, b_unauth) = tetherly_node::loopback_get(peer_ui, "/api/media/state")
        .await
        .unwrap();
    let unauth_rep: serde_json::Value = serde_json::from_slice(&b_unauth).unwrap();
    assert_eq!(
        unauth_rep["state"]["state"], "paused",
        "command from unauthorized group must be blocked, playback remaining Paused"
    );

    // Now group Host under "living-room" on Peer:
    let set_grp_req = serde_json::json!({
        "peer": host_id.as_str(),
        "group": "living-room"
    });
    let (c_setgrp, _) = tetherly_node::loopback_post(
        peer_ui,
        "/api/peers/group",
        &serde_json::to_vec(&set_grp_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c_setgrp, 200);

    // Host sends 'toggle' command again. Now Host IS authorized!
    let (c_tog2, _) = tetherly_node::loopback_post(
        host_ui,
        "/api/media/command",
        &serde_json::to_vec(&toggle_cmd_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c_tog2, 200);

    // Verify Peer state transitioned to Playing:
    let t_tog = tokio::time::Instant::now() + Duration::from_secs(4);
    let mut toggle_verified = false;
    while tokio::time::Instant::now() < t_tog {
        let (c, b) = tetherly_node::loopback_get(peer_ui, "/api/media/state")
            .await
            .unwrap();
        if c == 200 {
            if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&b) {
                if let Some(st) = val.get("state").filter(|s| !s.is_null()) {
                    if st["state"] == "playing" {
                        toggle_verified = true;
                        break;
                    }
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        toggle_verified,
        "peer player state must transition to Playing when authorized group peer sends Toggle"
    );

    // 10. Multi-cast Command Dispatch (all: true):
    let multi_cmd_req = serde_json::json!({
        "all": true,
        "command": "pause"
    });
    let (c_multi, b_multi) = tetherly_node::loopback_post(
        host_ui,
        "/api/media/command",
        &serde_json::to_vec(&multi_cmd_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(c_multi, 200);
    let multi_rep: serde_json::Value = serde_json::from_slice(&b_multi).unwrap();
    assert!(multi_rep["ok"].as_bool().unwrap_or(false));
    assert!(multi_rep["target_count"].as_u64().unwrap_or(0) >= 1);
    assert!(multi_rep["success_count"].as_u64().unwrap_or(0) >= 1);

    // 11. Atomic Persistence & POSIX 0o600 Permissions:
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let peer_policy_file = g2.path().join("media_policy.json");
        assert!(
            peer_policy_file.exists(),
            "media_policy.json must exist on disk"
        );
        let meta_pol = std::fs::metadata(&peer_policy_file).unwrap();
        assert_eq!(
            meta_pol.permissions().mode() & 0o777,
            0o600,
            "media_policy.json must have 0o600 permissions"
        );
    }

    // 12. Host 403 Forbidden Isolation Verification:
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut evil_stream = tokio::net::TcpStream::connect(format!("127.0.0.1:{peer_ui}"))
        .await
        .expect("connect to peer ui port");
    let evil_req = "GET /api/media/state HTTP/1.1\r\nHost: evil.attacker.domain\r\n\r\n";
    evil_stream.write_all(evil_req.as_bytes()).await.unwrap();
    evil_stream.flush().await.unwrap();
    let mut evil_buf = vec![0u8; 1024];
    let n = evil_stream.read(&mut evil_buf).await.unwrap();
    let evil_resp = String::from_utf8_lossy(&evil_buf[..n]);
    assert!(
        evil_resp.starts_with("HTTP/1.1 403 Forbidden"),
        "mismatched Host header on /api/media/state must be rejected with 403 Forbidden, got: {evil_resp}"
    );

    // 13. Verify Peer SSE Events Received:
    let sse_deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    let mut saw_media_cmd_sse = false;
    let mut saw_media_state_sse = false;
    while tokio::time::Instant::now() < sse_deadline {
        if let Ok(Ok(Some(raw))) =
            tokio::time::timeout(Duration::from_millis(200), peer_sse.next_event()).await
        {
            if raw.contains("media_command_received") {
                saw_media_cmd_sse = true;
            }
            if raw.contains("media_state_changed") {
                saw_media_state_sse = true;
            }
            if saw_media_cmd_sse && saw_media_state_sse {
                break;
            }
        }
    }
    assert!(
        saw_media_cmd_sse,
        "peer must emit media_command_received SSE event"
    );
    assert!(
        saw_media_state_sse,
        "peer must emit media_state_changed SSE event"
    );

    // 14. Clean Shutdown:
    host.shutdown();
    peer.shutdown();
}

#[tokio::test]
async fn battery_power_state_sync_and_low_battery_alert_via_loopback_api() {
    let (host, peer, g1, _g2) = pair_phone_desktop().await;
    let host_ui = host.spawn_ui().await.expect("host UI spawns");
    let peer_ui = peer.spawn_ui().await.expect("peer UI spawns");
    let _host_id = host.identity().device_id().clone();
    let peer_id = peer.identity().device_id().clone();

    // 1. Connect to Host SSE Event Stream to monitor real-time alerts
    let mut host_sse = tetherly_node::loopback_events_connect(host_ui)
        .await
        .expect("connect to host sse stream");

    // 2. Initial Battery Policy Check on Host
    let (code_pol, body_pol) = tetherly_node::loopback_get(host_ui, "/api/battery/policy")
        .await
        .unwrap();
    assert_eq!(code_pol, 200);
    let pol_init: serde_json::Value = serde_json::from_slice(&body_pol).unwrap();
    assert_eq!(pol_init["policy"]["low_battery_threshold"], 20);
    assert_eq!(pol_init["policy"]["critical_battery_threshold"], 10);
    assert_eq!(pol_init["policy"]["alert_on_low_battery"], true);
    assert_eq!(pol_init["policy"]["auto_throttle_on_battery_saver"], false);

    // 3. Peer Updates Local Healthy Battery State (85%, charging, AC)
    let state_healthy = serde_json::json!({
        "battery_level": 85,
        "is_charging": true,
        "power_source": "ac",
        "battery_saver": false,
        "health_percent": 96
    });
    let (code_st, body_st) = tetherly_node::loopback_post(
        peer_ui,
        "/api/battery/state",
        &serde_json::to_vec(&state_healthy).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_st, 200);
    let st_rep: serde_json::Value = serde_json::from_slice(&body_st).unwrap();
    assert!(st_rep["ok"].as_bool().unwrap_or(false));
    assert_eq!(st_rep["state"]["battery_level"], 85);
    assert_eq!(st_rep["state"]["is_charging"], true);
    assert_eq!(st_rep["state"]["power_source"], "ac");
    assert_eq!(st_rep["state"]["battery_saver"], false);
    assert_eq!(st_rep["state"]["health_percent"], 96);
    assert!(st_rep["broadcast_count"].as_u64().unwrap_or(0) >= 1);

    // 4. Host Queries Peer's Synchronized Battery State
    let t_deadline = tokio::time::Instant::now() + Duration::from_secs(4);
    let mut remote_state_synced = false;
    while tokio::time::Instant::now() < t_deadline {
        let (c, b) =
            tetherly_node::loopback_get(host_ui, &format!("/api/battery/state?peer={}", peer_id))
                .await
                .unwrap();
        if c == 200 {
            if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&b) {
                if let Some(st) = val.get("state").filter(|s| !s.is_null()) {
                    if st["battery_level"] == 85
                        && st["is_charging"] == true
                        && st["power_source"] == "ac"
                    {
                        remote_state_synced = true;
                        break;
                    }
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        remote_state_synced,
        "host must receive peer's published battery state via TYPE_BATTERY_STATE frame"
    );

    // 5. Peer Updates Battery State to Low (16%, discharging, Battery)
    let state_low = serde_json::json!({
        "battery_level": 16,
        "is_charging": false,
        "power_source": "battery",
        "battery_saver": true,
        "health_percent": 96
    });
    let (code_low, _) = tetherly_node::loopback_post(
        peer_ui,
        "/api/battery/state",
        &serde_json::to_vec(&state_low).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_low, 200);

    // 6. Verify Host Triggers Low Battery Alert (is_critical: false)
    let sse_deadline = tokio::time::Instant::now() + Duration::from_secs(4);
    let mut saw_low_alert = false;
    let mut saw_state_changed = false;
    while tokio::time::Instant::now() < sse_deadline {
        if let Ok(Ok(Some(raw))) =
            tokio::time::timeout(Duration::from_millis(200), host_sse.next_event()).await
        {
            if raw.contains("battery_state_changed") {
                saw_state_changed = true;
            }
            if raw.contains("battery_low_alert") {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&raw) {
                    if val["battery_level"] == 16 && val["is_critical"] == false {
                        saw_low_alert = true;
                    }
                }
            }
            if saw_state_changed && saw_low_alert {
                break;
            }
        }
    }
    assert!(
        saw_state_changed,
        "host must receive battery_state_changed event"
    );
    assert!(
        saw_low_alert,
        "host must receive battery_low_alert event with is_critical: false"
    );

    // 7. Peer Updates Battery State to Critical (8%, discharging)
    let state_crit = serde_json::json!({
        "battery_level": 8,
        "is_charging": false,
        "power_source": "battery",
        "battery_saver": true,
        "health_percent": 96
    });
    let (code_crit, _) = tetherly_node::loopback_post(
        peer_ui,
        "/api/battery/state",
        &serde_json::to_vec(&state_crit).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_crit, 200);

    // 8. Verify Host Triggers Critical Alert (is_critical: true)
    let crit_deadline = tokio::time::Instant::now() + Duration::from_secs(4);
    let mut saw_crit_alert = false;
    while tokio::time::Instant::now() < crit_deadline {
        if let Ok(Ok(Some(raw))) =
            tokio::time::timeout(Duration::from_millis(200), host_sse.next_event()).await
        {
            if raw.contains("battery_low_alert") {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&raw) {
                    if val["battery_level"] == 8 && val["is_critical"] == true {
                        saw_crit_alert = true;
                        break;
                    }
                }
            }
        }
    }
    assert!(
        saw_crit_alert,
        "host must receive battery_low_alert event with is_critical: true"
    );

    // 9. Host Policy Mutation & Scoped Filtering
    let update_policy = serde_json::json!({
        "low_battery_threshold": 30,
        "critical_battery_threshold": 15,
        "alert_on_low_battery": true,
        "auto_throttle_on_battery_saver": true,
        "allowed_groups": ["trusted-mesh"],
        "allowed_tags": ["mobile"]
    });
    let (code_pol_up, body_pol_up) = tetherly_node::loopback_post(
        host_ui,
        "/api/battery/policy",
        &serde_json::to_vec(&update_policy).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_pol_up, 200);
    let pol_updated: serde_json::Value = serde_json::from_slice(&body_pol_up).unwrap();
    assert_eq!(pol_updated["policy"]["low_battery_threshold"], 30);
    assert_eq!(pol_updated["policy"]["critical_battery_threshold"], 15);
    assert_eq!(
        pol_updated["policy"]["auto_throttle_on_battery_saver"],
        true
    );
    assert_eq!(
        pol_updated["policy"]["allowed_groups"],
        serde_json::json!(["trusted-mesh"])
    );
    assert_eq!(
        pol_updated["policy"]["allowed_tags"],
        serde_json::json!(["mobile"])
    );

    // 10. File persistence check on host data_dir
    let policy_path = g1.path().join("battery_policy.json");
    assert!(
        policy_path.exists(),
        "battery_policy.json must be persisted"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::metadata(&policy_path).unwrap().permissions();
        assert_eq!(
            perms.mode() & 0o777,
            0o600,
            "battery_policy.json must have 0o600 permissions"
        );
    }

    // 11. Security Guard: Host header 403 Forbidden attack defense
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], host_ui));
    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let evil_req =
        "GET /api/battery/state HTTP/1.1\r\nHost: malicious-attacker.com\r\nConnection: close\r\n\r\n";
    tokio::io::AsyncWriteExt::write_all(&mut stream, evil_req.as_bytes())
        .await
        .unwrap();
    tokio::io::AsyncWriteExt::flush(&mut stream).await.unwrap();
    let mut raw_resp = Vec::new();
    tokio::io::AsyncReadExt::read_to_end(&mut stream, &mut raw_resp)
        .await
        .unwrap();
    let resp_str = String::from_utf8_lossy(&raw_resp);
    assert!(
        resp_str.starts_with("HTTP/1.1 403") || resp_str.starts_with("HTTP/1.0 403"),
        "Host header attack must be rejected with 403 Forbidden"
    );

    // 12. Clean Shutdown
    host.shutdown();
    peer.shutdown();
}

#[tokio::test]
async fn locate_find_my_device_pipeline_acoustic_ring_and_anti_harassment_via_loopback_api() {
    let (host, peer, _g1, g2) = pair_phone_desktop().await;
    let host_ui = host.spawn_ui().await.expect("host UI spawns");
    let peer_ui = peer.spawn_ui().await.expect("peer UI spawns");
    let host_id = host.identity().device_id().clone();
    let peer_id = peer.identity().device_id().clone();

    // 1. Connect to Peer SSE Event Stream to monitor real-time ringing events
    let mut peer_sse = tetherly_node::loopback_events_connect(peer_ui)
        .await
        .expect("connect to peer sse stream");

    // 2. Initial Locator Policy Check on Peer
    let (code_pol, body_pol) = tetherly_node::loopback_get(peer_ui, "/api/locate/policy")
        .await
        .unwrap();
    assert_eq!(code_pol, 200);
    let pol_init: serde_json::Value = serde_json::from_slice(&body_pol).unwrap();
    assert_eq!(pol_init["policy"]["enabled"], true);
    assert_eq!(pol_init["policy"]["allow_override_silent"], true);
    assert_eq!(pol_init["policy"]["max_duration_secs"], 60);

    // 3. Initial Status Check on Peer (must not be ringing)
    let (code_st, body_st) = tetherly_node::loopback_get(peer_ui, "/api/locate/status")
        .await
        .unwrap();
    assert_eq!(code_st, 200);
    let st_init: serde_json::Value = serde_json::from_slice(&body_st).unwrap();
    assert_eq!(st_init["is_ringing"], false);
    assert!(st_init["active_ring"].is_null());

    // 4. Host sends locate ring request to Peer
    let ring_req = serde_json::json!({
        "peer": peer_id.to_string(),
        "duration_secs": 15,
        "pattern": "siren",
        "override_silent": true,
        "message": "Keys or phone misplaced in living room"
    });
    let (code_ring, body_ring) = tetherly_node::loopback_post(
        host_ui,
        "/api/locate/ring",
        &serde_json::to_vec(&ring_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_ring, 200);
    let ring_rep: serde_json::Value = serde_json::from_slice(&body_ring).unwrap();
    assert_eq!(ring_rep["ok"], true);
    let req_id = ring_rep["request_id"].as_str().unwrap().to_string();
    assert!(req_id.starts_with("loc_"));
    assert_eq!(ring_rep["duration_secs"], 15);
    assert_eq!(ring_rep["pattern"], "siren");

    // 5. Verify Peer is now ringing
    let t_deadline = tokio::time::Instant::now() + Duration::from_secs(4);
    let mut peer_started_ringing = false;
    while tokio::time::Instant::now() < t_deadline {
        let (c, b) = tetherly_node::loopback_get(peer_ui, "/api/locate/status")
            .await
            .unwrap();
        if c == 200 {
            if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&b) {
                if val["is_ringing"] == true {
                    if let Some(ar) = val.get("active_ring").filter(|r| !r.is_null()) {
                        if ar["request_id"] == req_id
                            && ar["pattern"] == "siren"
                            && ar["initiator_peer"] == host_id.to_string()
                        {
                            peer_started_ringing = true;
                            break;
                        }
                    }
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        peer_started_ringing,
        "peer must transition to active ringing upon receiving TYPE_LOCATE_REQUEST frame"
    );

    // 6. Verify Peer receives SSE event locate_ring_triggered
    let sse_deadline = tokio::time::Instant::now() + Duration::from_secs(4);
    let mut saw_ring_triggered = false;
    while tokio::time::Instant::now() < sse_deadline {
        if let Ok(Ok(Some(raw))) =
            tokio::time::timeout(Duration::from_millis(200), peer_sse.next_event()).await
        {
            if raw.contains("locate_ring_triggered") {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&raw) {
                    if val["request_id"] == req_id && val["pattern"] == "siren" {
                        saw_ring_triggered = true;
                        break;
                    }
                }
            }
        }
    }
    assert!(
        saw_ring_triggered,
        "peer must emit locate_ring_triggered SSE event"
    );

    // 7. Test Anti-Harassment Rate Limiting: Host immediately attempts second ring to Peer
    let (code_rate, body_rate) = tetherly_node::loopback_post(
        host_ui,
        "/api/locate/ring",
        &serde_json::to_vec(&ring_req).unwrap(),
    )
    .await
    .unwrap();
    // Host should reject with 400 Bad Request due to 10s cooldown
    assert_eq!(code_rate, 400);
    let rate_err = String::from_utf8_lossy(&body_rate);
    assert!(
        rate_err.contains("anti-harassment") || rate_err.contains("rate limit"),
        "subsequent locate request within 10s must be throttled: {}",
        rate_err
    );

    // 8. Host stops Peer's ringing via /api/locate/stop
    let stop_req = serde_json::json!({
        "peer": peer_id.to_string(),
        "reason": "device_found"
    });
    let (code_stop, body_stop) = tetherly_node::loopback_post(
        host_ui,
        "/api/locate/stop",
        &serde_json::to_vec(&stop_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_stop, 200);
    let stop_rep: serde_json::Value = serde_json::from_slice(&body_stop).unwrap();
    assert_eq!(stop_rep["ok"], true);

    // 9. Verify Peer ceases ringing
    let stop_deadline = tokio::time::Instant::now() + Duration::from_secs(4);
    let mut peer_stopped_ringing = false;
    while tokio::time::Instant::now() < stop_deadline {
        let (c, b) = tetherly_node::loopback_get(peer_ui, "/api/locate/status")
            .await
            .unwrap();
        if c == 200 {
            if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&b) {
                if val["is_ringing"] == false && val["active_ring"].is_null() {
                    peer_stopped_ringing = true;
                    break;
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        peer_stopped_ringing,
        "peer must stop ringing upon receiving TYPE_LOCATE_STOP frame"
    );

    // 10. Verify Peer receives SSE event locate_ring_stopped
    let sse_stop_deadline = tokio::time::Instant::now() + Duration::from_secs(4);
    let mut saw_ring_stopped = false;
    while tokio::time::Instant::now() < sse_stop_deadline {
        if let Ok(Ok(Some(raw))) =
            tokio::time::timeout(Duration::from_millis(200), peer_sse.next_event()).await
        {
            if raw.contains("locate_ring_stopped") {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&raw) {
                    if val["reason"] == "device_found" {
                        saw_ring_stopped = true;
                        break;
                    }
                }
            }
        }
    }
    assert!(
        saw_ring_stopped,
        "peer must emit locate_ring_stopped SSE event"
    );

    // 11. Peer updates Locator Policy
    let update_pol = serde_json::json!({
        "enabled": true,
        "allow_override_silent": false,
        "max_duration_secs": 25,
        "allowed_groups": ["trusted_family"],
        "allowed_tags": ["mobile"]
    });
    let (code_pol_up, body_pol_up) = tetherly_node::loopback_post(
        peer_ui,
        "/api/locate/policy",
        &serde_json::to_vec(&update_pol).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_pol_up, 200);
    let pol_updated: serde_json::Value = serde_json::from_slice(&body_pol_up).unwrap();
    assert_eq!(pol_updated["policy"]["allow_override_silent"], false);
    assert_eq!(pol_updated["policy"]["max_duration_secs"], 25);
    assert_eq!(
        pol_updated["policy"]["allowed_groups"],
        serde_json::json!(["trusted_family"])
    );
    assert_eq!(
        pol_updated["policy"]["allowed_tags"],
        serde_json::json!(["mobile"])
    );

    // 12. File persistence check on peer data_dir (0o600 permissions)
    let policy_path = g2.path().join("locator_policy.json");
    assert!(
        policy_path.exists(),
        "locator_policy.json must be persisted"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::metadata(&policy_path).unwrap().permissions();
        assert_eq!(
            perms.mode() & 0o777,
            0o600,
            "locator_policy.json must have 0o600 permissions"
        );
    }

    // 13. Security Guard: Host header 403 Forbidden attack defense
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], peer_ui));
    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let evil_req =
        "GET /api/locate/status HTTP/1.1\r\nHost: malicious-attacker.com\r\nConnection: close\r\n\r\n";
    tokio::io::AsyncWriteExt::write_all(&mut stream, evil_req.as_bytes())
        .await
        .unwrap();
    tokio::io::AsyncWriteExt::flush(&mut stream).await.unwrap();
    let mut raw_resp = Vec::new();
    tokio::io::AsyncReadExt::read_to_end(&mut stream, &mut raw_resp)
        .await
        .unwrap();
    let resp_str = String::from_utf8_lossy(&raw_resp);
    assert!(
        resp_str.starts_with("HTTP/1.1 403") || resp_str.starts_with("HTTP/1.0 403"),
        "Host header attack must be rejected with 403 Forbidden"
    );

    // 14. Clean Shutdown
    host.shutdown();
    peer.shutdown();
}

#[tokio::test]
async fn action_predefined_quick_actions_local_execution_and_governance_via_loopback_api() {
    let (host, peer, _g1, g2) = pair_phone_desktop().await;
    let host_ui = host.spawn_ui().await.expect("host UI spawns");
    let peer_ui = peer.spawn_ui().await.expect("peer UI spawns");
    let _host_id = host.identity().device_id().clone();
    let peer_id = peer.identity().device_id().clone();

    // 1. Connect to Host SSE Event Stream to monitor real-time action results
    let mut host_sse = tetherly_node::loopback_events_connect(host_ui)
        .await
        .expect("connect to host sse stream");

    // 2. Initial Action Policy Check on Peer
    let (code_pol, body_pol) = tetherly_node::loopback_get(peer_ui, "/api/action/policy")
        .await
        .unwrap();
    assert_eq!(code_pol, 200);
    let pol_init: serde_json::Value = serde_json::from_slice(&body_pol).unwrap();
    assert_eq!(pol_init["policy"]["mode"], "allow");
    assert_eq!(pol_init["policy"]["max_timeout_secs"], 10);
    assert_eq!(pol_init["policy"]["expose_action_list"], true);

    // 3. Initial Local Actions Check on Peer (must be empty)
    let (code_loc, body_loc) = tetherly_node::loopback_get(peer_ui, "/api/action/local")
        .await
        .unwrap();
    assert_eq!(code_loc, 200);
    let loc_init: serde_json::Value = serde_json::from_slice(&body_loc).unwrap();
    assert_eq!(loc_init["count"], 0);
    assert!(loc_init["actions"].as_array().unwrap().is_empty());

    // 4. Peer registers a predefined local action (safe echo test)
    let add_req = serde_json::json!({
        "id": "ping_action",
        "name": "Ping Test Action",
        "command": "echo",
        "args": ["pong_payload_123"],
        "description": "Echoes pong payload",
        "icon": "🏓",
        "timeout_secs": 5,
        "enabled": true
    });
    let (code_add, body_add) = tetherly_node::loopback_post(
        peer_ui,
        "/api/action/local",
        &serde_json::to_vec(&add_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_add, 200);
    let add_rep: serde_json::Value = serde_json::from_slice(&body_add).unwrap();
    assert_eq!(add_rep["ok"], true);
    assert_eq!(add_rep["count"], 1);
    assert_eq!(add_rep["actions"][0]["id"], "ping_action");
    assert_eq!(add_rep["actions"][0]["name"], "Ping Test Action");

    // Verify Peer local actions now returns 1 item
    let (code_loc2, body_loc2) = tetherly_node::loopback_get(peer_ui, "/api/action/local")
        .await
        .unwrap();
    assert_eq!(code_loc2, 200);
    let loc_list: serde_json::Value = serde_json::from_slice(&body_loc2).unwrap();
    assert_eq!(loc_list["count"], 1);
    assert_eq!(loc_list["actions"][0]["id"], "ping_action");

    // 5. Host requests Peer's advertised actions
    let refresh_path = format!(
        "/api/action/remote/refresh?peer={}",
        tetherly_cli_test_url_encode(&peer_id.to_string())
    );
    let (code_ref, _) = tetherly_node::loopback_post(host_ui, &refresh_path, &[])
        .await
        .unwrap();
    assert_eq!(code_ref, 200);

    // 6. Wait for Host to receive and cache Peer's advertised actions
    let remote_path = format!(
        "/api/action/remote?peer={}",
        tetherly_cli_test_url_encode(&peer_id.to_string())
    );
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let mut found_advertised_action = false;
    let mut raw_remote_body = Vec::new();
    while tokio::time::Instant::now() < deadline {
        let (c, b) = tetherly_node::loopback_get(host_ui, &remote_path)
            .await
            .unwrap();
        if c == 200 {
            if let Ok(rep) = serde_json::from_slice::<serde_json::Value>(&b) {
                if let Some(actions) = rep.get("actions").and_then(|a| a.as_array()) {
                    if actions.iter().any(|act| act["id"] == "ping_action") {
                        found_advertised_action = true;
                        raw_remote_body = b;
                        break;
                    }
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        found_advertised_action,
        "host must receive peer's advertised action 'ping_action' over mesh"
    );

    // ADR-008: Security Guardrail: Publicly advertised metadata MUST NOT leak command or args!
    let remote_json_str = String::from_utf8_lossy(&raw_remote_body);
    assert!(
        !remote_json_str.contains("pong_payload_123"),
        "advertised action metadata must never leak underlying arguments to network"
    );
    assert!(
        !remote_json_str.contains("\"command\""),
        "advertised action metadata must never contain command field"
    );

    // 7. Host triggers Peer's predefined 'ping_action'
    let trig_req = serde_json::json!({
        "peer": peer_id.to_string(),
        "action_id": "ping_action"
    });
    let (code_trig, body_trig) = tetherly_node::loopback_post(
        host_ui,
        "/api/action/trigger",
        &serde_json::to_vec(&trig_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_trig, 200);
    let trig_rep: serde_json::Value = serde_json::from_slice(&body_trig).unwrap();
    let trig_id = trig_rep["trigger_id"].as_str().unwrap().to_string();
    assert!(trig_id.starts_with("trig_"));
    assert_eq!(trig_rep["status"], "triggered");

    // 8. Wait for Peer to execute and return ActionResultFrame to Host
    let exec_deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let mut action_executed_successfully = false;
    while tokio::time::Instant::now() < exec_deadline {
        let (c, b) = tetherly_node::loopback_get(host_ui, "/api/action/history")
            .await
            .unwrap();
        if c == 200 {
            if let Ok(rep) = serde_json::from_slice::<serde_json::Value>(&b) {
                let history = if let Some(arr) = rep.get("results").and_then(|r| r.as_array()) {
                    arr.clone()
                } else if let Some(arr) = rep.as_array() {
                    arr.clone()
                } else {
                    Vec::new()
                };
                if let Some(entry) = history.iter().find(|h| h["trigger_id"] == trig_id) {
                    if entry["success"] == true && entry["exit_code"] == 0 {
                        let out = entry["output_summary"].as_str().unwrap_or("");
                        if out.contains("pong_payload_123") {
                            action_executed_successfully = true;
                            break;
                        }
                    }
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        action_executed_successfully,
        "host must receive successful action result containing command output"
    );

    // 9. Verify Host receives SSE event action_result_received
    let sse_deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    let mut saw_result_event = false;
    while tokio::time::Instant::now() < sse_deadline {
        if let Ok(Ok(Some(raw))) =
            tokio::time::timeout(Duration::from_millis(200), host_sse.next_event()).await
        {
            if raw.contains("action_result_received") && raw.contains(&trig_id) {
                saw_result_event = true;
                break;
            }
        }
    }
    assert!(
        saw_result_event,
        "host must receive action_result_received SSE event"
    );

    // 10. Test Anti-Harassment Rate Limiting (5-second cooldown)
    let (code_rate, body_rate) = tetherly_node::loopback_post(
        host_ui,
        "/api/action/trigger",
        &serde_json::to_vec(&trig_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_rate, 400);
    let rate_err = String::from_utf8_lossy(&body_rate);
    assert!(
        rate_err.contains("rate limit") || rate_err.contains("cooldown"),
        "subsequent trigger within 5s must be throttled: {}",
        rate_err
    );

    // 11. Peer updates Action Policy to Disabled
    let update_pol = serde_json::json!({
        "mode": "Disabled",
        "max_timeout_secs": 15,
        "expose_action_list": false
    });
    let (code_pol_up, body_pol_up) = tetherly_node::loopback_post(
        peer_ui,
        "/api/action/policy",
        &serde_json::to_vec(&update_pol).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_pol_up, 200);
    let pol_updated: serde_json::Value = serde_json::from_slice(&body_pol_up).unwrap();
    assert_eq!(pol_updated["policy"]["mode"], "disabled");
    assert_eq!(pol_updated["policy"]["max_timeout_secs"], 15);
    assert_eq!(pol_updated["policy"]["expose_action_list"], false);

    // 12. File persistence check on peer data_dir (0o600 permissions)
    let actions_path = g2.path().join("actions.json");
    let policy_path = g2.path().join("action_policy.json");
    assert!(actions_path.exists(), "actions.json must be persisted");
    assert!(policy_path.exists(), "action_policy.json must be persisted");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms1 = std::fs::metadata(&actions_path).unwrap().permissions();
        assert_eq!(
            perms1.mode() & 0o777,
            0o600,
            "actions.json must have 0o600 permissions"
        );
        let perms2 = std::fs::metadata(&policy_path).unwrap().permissions();
        assert_eq!(
            perms2.mode() & 0o777,
            0o600,
            "action_policy.json must have 0o600 permissions"
        );
    }

    // 13. Security Guard: Host header 403 Forbidden attack defense
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], peer_ui));
    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let evil_req =
        "GET /api/action/local HTTP/1.1\r\nHost: malicious-attacker.com\r\nConnection: close\r\n\r\n";
    tokio::io::AsyncWriteExt::write_all(&mut stream, evil_req.as_bytes())
        .await
        .unwrap();
    tokio::io::AsyncWriteExt::flush(&mut stream).await.unwrap();
    let mut raw_resp = Vec::new();
    tokio::io::AsyncReadExt::read_to_end(&mut stream, &mut raw_resp)
        .await
        .unwrap();
    let resp_str = String::from_utf8_lossy(&raw_resp);
    assert!(
        resp_str.starts_with("HTTP/1.1 403") || resp_str.starts_with("HTTP/1.0 403"),
        "Host header attack must be rejected with 403 Forbidden"
    );

    // 14. Clean Shutdown
    host.shutdown();
    peer.shutdown();
}

/// ADR-070: Cross-device focus mode, do-not-disturb synchronization,
/// notification suppression, OTP breakthrough, and quiet hours governance via loopback API.
#[tokio::test]
async fn focus_mode_cross_device_synchronization_quiet_hours_and_governance_via_loopback_api() {
    let (host, peer, g1, _g2) = pair_phone_desktop().await;
    let host_ui = host.spawn_ui().await.expect("host UI spawns");
    let peer_ui = peer.spawn_ui().await.expect("peer UI spawns");
    let host_id = host.identity().device_id().clone();
    let _peer_id = peer.identity().device_id().clone();

    // 1. Connect to Host and Peer SSE streams
    let mut host_sse = tetherly_node::loopback_events_connect(host_ui)
        .await
        .expect("connect to host sse stream");
    let mut peer_sse = tetherly_node::loopback_events_connect(peer_ui)
        .await
        .expect("connect to peer sse stream");

    // 2. Initial Focus State Check on Host (must be inactive)
    let (code_st, body_st) = tetherly_node::loopback_get(host_ui, "/api/focus/state")
        .await
        .unwrap();
    assert_eq!(code_st, 200);
    let st_init: serde_json::Value = serde_json::from_slice(&body_st).unwrap();
    assert_eq!(st_init["ok"], true);
    assert_eq!(st_init["enabled"], false);
    assert_eq!(st_init["active"], false);
    assert_eq!(st_init["mode"], "dnd");

    // 3. Initial Focus Policy Check on Host
    let (code_pol, body_pol) = tetherly_node::loopback_get(host_ui, "/api/focus/policy")
        .await
        .unwrap();
    assert_eq!(code_pol, 200);
    let pol_init: serde_json::Value = serde_json::from_slice(&body_pol).unwrap();
    assert_eq!(pol_init["ok"], true);
    assert_eq!(pol_init["policy"]["enabled"], true);
    assert_eq!(pol_init["policy"]["default_mode"], "dnd");
    assert_eq!(pol_init["policy"]["allow_emergency_breakthrough"], true);

    // 4. Host activates focus mode with Meeting mode, duration 30m, reason
    let set_req = serde_json::json!({
        "enabled": true,
        "mode": "meeting",
        "duration": 30,
        "reason": "Sprint Planning",
        "broadcast": true
    });
    let (code_on, body_on) = tetherly_node::loopback_post(
        host_ui,
        "/api/focus/state",
        &serde_json::to_vec(&set_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_on, 200);
    let on_rep: serde_json::Value = serde_json::from_slice(&body_on).unwrap();
    assert_eq!(on_rep["ok"], true);
    assert_eq!(on_rep["active"], true);
    assert_eq!(on_rep["mode"], "meeting");
    assert_eq!(on_rep["reason"], "Sprint Planning");
    assert!(on_rep["expires_ms"].as_u64().is_some());
    assert!(on_rep["broadcast_count"].as_u64().unwrap() >= 1);

    // 5. Host GET /api/focus/state reflects the active state
    let (code_st2, body_st2) = tetherly_node::loopback_get(host_ui, "/api/focus/state")
        .await
        .unwrap();
    assert_eq!(code_st2, 200);
    let st2: serde_json::Value = serde_json::from_slice(&body_st2).unwrap();
    assert_eq!(st2["active"], true);
    assert_eq!(st2["enabled"], true);
    assert_eq!(st2["mode"], "meeting");
    assert_eq!(st2["reason"], "Sprint Planning");
    assert!(st2["remaining_secs"].as_u64().unwrap() > 0);

    // 6. Host SSE receives UiEvent::FocusStateChanged
    let sse_deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    let mut saw_host_focus_ev = false;
    while tokio::time::Instant::now() < sse_deadline {
        if let Ok(Ok(Some(raw))) =
            tokio::time::timeout(Duration::from_millis(200), host_sse.next_event()).await
        {
            if raw.contains("focus_state_changed")
                && raw.contains("meeting")
                && raw.contains("Sprint Planning")
            {
                saw_host_focus_ev = true;
                break;
            }
        }
    }
    assert!(
        saw_host_focus_ev,
        "host SSE must emit focus_state_changed event"
    );

    // 7. Peer receives FocusStateFrame over mesh and stores in peer focus states
    let peer_poll_deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let mut peer_recorded_host = false;
    while tokio::time::Instant::now() < peer_poll_deadline {
        let (c, b) = tetherly_node::loopback_get(peer_ui, "/api/focus/peers")
            .await
            .unwrap();
        if c == 200 {
            if let Ok(rep) = serde_json::from_slice::<serde_json::Value>(&b) {
                if let Some(peers) = rep.get("peers").and_then(|p| p.as_array()) {
                    if let Some(h) = peers.iter().find(|p| p["peer"] == host_id.as_str()) {
                        if h["active"] == true && h["state"]["mode"] == "meeting" {
                            peer_recorded_host = true;
                            break;
                        }
                    }
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        peer_recorded_host,
        "peer must record host's advertised focus state via mesh"
    );

    // 8. Peer SSE receives UiEvent::PeerFocusStateChanged
    let peer_sse_deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    let mut saw_peer_focus_ev = false;
    while tokio::time::Instant::now() < peer_sse_deadline {
        if let Ok(Ok(Some(raw))) =
            tokio::time::timeout(Duration::from_millis(200), peer_sse.next_event()).await
        {
            if raw.contains("peer_focus_state_changed") && raw.contains("meeting") {
                saw_peer_focus_ev = true;
                break;
            }
        }
    }
    assert!(
        saw_peer_focus_ev,
        "peer SSE must emit peer_focus_state_changed event"
    );

    // 9. Test Notification Suppression on Host:
    // Regular notification banner is suppressed
    let normal_notify = host.dispatch_notification_banner(
        "Team Chat",
        "Hey how is the meeting going?",
        None,
        false,
    );
    assert!(
        !normal_notify.unwrap(),
        "normal notification must be suppressed in focus mode"
    );

    // Critical OTP notification banner breaks through suppression
    let otp_notify =
        host.dispatch_notification_banner("Verification", "Your OTP code is 987654", None, true);
    assert!(
        otp_notify.unwrap(),
        "critical/OTP notification must breakthrough in focus mode"
    );

    // test_notification_banner API report also reflects suppression
    let test_notify_rep = host
        .test_notification_banner(Some("Test"), Some("Testing focus suppression"))
        .unwrap();
    assert!(!test_notify_rep.sent);
    assert!(test_notify_rep.suppressed);
    assert_eq!(
        test_notify_rep.suppression_reason.as_deref(),
        Some("focus_mode_meeting")
    );

    // 10. Peer initiates manual focus sync request via POST /api/focus/sync
    let (code_sync, body_sync) = tetherly_node::loopback_post(peer_ui, "/api/focus/sync", b"{}")
        .await
        .unwrap();
    assert_eq!(code_sync, 200);
    let sync_rep: serde_json::Value = serde_json::from_slice(&body_sync).unwrap();
    assert_eq!(sync_rep["ok"], true);
    assert!(sync_rep["sync_requests_sent"].as_u64().unwrap() >= 1);

    // 11. Host updates focus policy (enables quiet hours, mirrors, breakthrough)
    let pol_update = serde_json::json!({
        "quiet_hours": true,
        "start": "23:00",
        "end": "07:00",
        "mirror": true,
        "breakthrough": true
    });
    let (code_pol_up, body_pol_up) = tetherly_node::loopback_post(
        host_ui,
        "/api/focus/policy",
        &serde_json::to_vec(&pol_update).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_pol_up, 200);
    let pol_updated: serde_json::Value = serde_json::from_slice(&body_pol_up).unwrap();
    assert_eq!(pol_updated["ok"], true);
    assert_eq!(pol_updated["policy"]["quiet_hours_enabled"], true);
    assert_eq!(pol_updated["policy"]["quiet_hours_start_minute"], 1380); // 23 * 60
    assert_eq!(pol_updated["policy"]["quiet_hours_end_minute"], 420); // 7 * 60
    assert_eq!(pol_updated["policy"]["mirror_inbound_focus"], true);

    // Verify policy file persistence on disk with 0o600 permissions
    let policy_path = g1.path().join("focus_policy.json");
    assert!(policy_path.exists(), "focus_policy.json must be persisted");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::metadata(&policy_path).unwrap().permissions();
        assert_eq!(
            perms.mode() & 0o777,
            0o600,
            "focus_policy.json must have 0o600 permissions"
        );
    }

    // 12. Host turns off focus mode and quiet hours
    let (code_reset_pol, _) = tetherly_node::loopback_post(
        host_ui,
        "/api/focus/policy",
        &serde_json::to_vec(&serde_json::json!({
            "quiet_hours_enabled": false
        }))
        .unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_reset_pol, 200);

    let off_req = serde_json::json!({
        "enabled": false,
        "broadcast": true
    });
    let (code_off, body_off) = tetherly_node::loopback_post(
        host_ui,
        "/api/focus/state",
        &serde_json::to_vec(&off_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_off, 200);
    let off_rep: serde_json::Value = serde_json::from_slice(&body_off).unwrap();
    assert_eq!(off_rep["active"], false);
    assert_eq!(off_rep["enabled"], false);

    // Notifications now pass through normally
    let restored_notify =
        host.dispatch_notification_banner("Team Chat", "Meeting concluded", None, false);
    assert!(
        restored_notify.unwrap(),
        "notifications must be delivered normally when focus is off"
    );

    // 13. Security Red Line: Host header 403 Forbidden attack defense
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], host_ui));
    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let evil_req =
        "GET /api/focus/state HTTP/1.1\r\nHost: evil-attacker-phishing.com\r\nConnection: close\r\n\r\n";
    tokio::io::AsyncWriteExt::write_all(&mut stream, evil_req.as_bytes())
        .await
        .unwrap();
    tokio::io::AsyncWriteExt::flush(&mut stream).await.unwrap();
    let mut raw_resp = Vec::new();
    tokio::io::AsyncReadExt::read_to_end(&mut stream, &mut raw_resp)
        .await
        .unwrap();
    let resp_str = String::from_utf8_lossy(&raw_resp);
    assert!(
        resp_str.starts_with("HTTP/1.1 403") || resp_str.starts_with("HTTP/1.0 403"),
        "Host header attack must be rejected with 403 Forbidden: {resp_str}"
    );

    // 14. Clean Shutdown
    host.shutdown();
    peer.shutdown();
}

/// ADR-071 End-to-End Integration Test:
/// Multi-Display Virtual Spatial Topology, Borderless Edge Cursor Crossing &
/// Spatial Layout Governance Pipeline via Loopback HTTP API & Wire Frame Protocol.
#[tokio::test]
async fn topology_multi_display_spatial_layout_borderless_cursor_and_governance_via_loopback_api() {
    let (host, peer, g1, _g2) = pair_phone_desktop().await;
    let host_ui = host.spawn_ui().await.expect("host UI spawns");
    let peer_ui = peer.spawn_ui().await.expect("peer UI spawns");
    let host_id = host.identity().device_id().clone();
    let peer_id = peer.identity().device_id().clone();

    // 1. Connect to Host and Peer SSE streams
    let mut host_sse = tetherly_node::loopback_events_connect(host_ui)
        .await
        .expect("connect to host sse stream");
    let mut _peer_sse = tetherly_node::loopback_events_connect(peer_ui)
        .await
        .expect("connect to peer sse stream");

    // 2. Initial Topology Layout Check on Host (default 1920x1080, empty neighbors)
    let (code_lay, body_lay) = tetherly_node::loopback_get(host_ui, "/api/topology/layout")
        .await
        .unwrap();
    assert_eq!(code_lay, 200);
    let lay_init: serde_json::Value = serde_json::from_slice(&body_lay).unwrap();
    assert_eq!(lay_init["ok"], true);
    assert_eq!(lay_init["display"]["width"], 1920);
    assert_eq!(lay_init["display"]["height"], 1080);
    assert_eq!(lay_init["config"]["resistance_px"], 15);
    assert_eq!(lay_init["config"]["delay_ms"], 100);
    assert_eq!(lay_init["config"]["corner_lock_size_px"], 30);

    // 3. Initial Topology Policy Check on Host
    let (code_pol, body_pol) = tetherly_node::loopback_get(host_ui, "/api/topology/policy")
        .await
        .unwrap();
    assert_eq!(code_pol, 200);
    let pol_init: serde_json::Value = serde_json::from_slice(&body_pol).unwrap();
    assert_eq!(pol_init["ok"], true);
    assert_eq!(pol_init["policy"]["enabled"], true);
    assert_eq!(pol_init["policy"]["allow_inbound_control"], true);
    assert_eq!(pol_init["policy"]["auto_link_paired_peers"], true);

    // 4. Host updates display configuration and crossing parameters via POST /api/topology/config
    let cfg_req = serde_json::json!({
        "width": 2560,
        "height": 1440,
        "scale_factor": 1.25,
        "resistance_px": 25,
        "delay_ms": 120,
        "corner_lock_size_px": 30,
        "wrap_around": true
    });
    let (code_cfg, body_cfg) = tetherly_node::loopback_post(
        host_ui,
        "/api/topology/config",
        &serde_json::to_vec(&cfg_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_cfg, 200);
    let cfg_rep: serde_json::Value = serde_json::from_slice(&body_cfg).unwrap();
    assert_eq!(cfg_rep["ok"], true);
    assert_eq!(cfg_rep["display"]["width"], 2560);
    assert_eq!(cfg_rep["display"]["height"], 1440);
    assert_eq!(cfg_rep["config"]["resistance_px"], 25);
    assert_eq!(cfg_rep["config"]["delay_ms"], 120);
    assert_eq!(cfg_rep["config"]["corner_lock_size_px"], 30);
    assert_eq!(cfg_rep["config"]["wrap_around"], true);

    // 5. Host links Peer to its Right edge via POST /api/topology/link
    let link_req = serde_json::json!({
        "edge": "right",
        "peer": peer_id.to_string(),
        "peer_edge": "left",
        "offset": 100
    });
    let (code_link, body_link) = tetherly_node::loopback_post(
        host_ui,
        "/api/topology/link",
        &serde_json::to_vec(&link_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_link, 200);
    let link_rep: serde_json::Value = serde_json::from_slice(&body_link).unwrap();
    assert_eq!(link_rep["ok"], true);
    assert_eq!(link_rep["edge"], "right");
    assert_eq!(link_rep["peer_id"], peer_id.to_string());
    assert_eq!(link_rep["peer_edge"], "left");
    assert_eq!(link_rep["offset"], 100);

    // Verify GET /api/topology/layout reflects the newly linked neighbor
    let (code_lay2, body_lay2) = tetherly_node::loopback_get(host_ui, "/api/topology/layout")
        .await
        .unwrap();
    assert_eq!(code_lay2, 200);
    let lay2: serde_json::Value = serde_json::from_slice(&body_lay2).unwrap();
    assert_eq!(lay2["neighbors"]["right"]["peer_id"], peer_id.to_string());
    assert_eq!(lay2["neighbors"]["right"]["peer_edge"], "left");
    assert_eq!(lay2["neighbors"]["right"]["offset"], 100);

    // 6. Verify SSE event on Host
    let mut saw_topology_changed = false;
    let sse_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(3);
    while tokio::time::Instant::now() < sse_deadline {
        if let Ok(Ok(Some(ev_raw))) =
            tokio::time::timeout(std::time::Duration::from_millis(400), host_sse.next_event()).await
        {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&ev_raw) {
                if v["kind"] == "topology_changed" && v["neighbors_count"] == 1 {
                    saw_topology_changed = true;
                    break;
                }
            }
        }
    }
    assert!(
        saw_topology_changed,
        "Host must emit topology_changed SSE event with 1 neighbor"
    );

    // 7. Trigger network topology sync via POST /api/topology/sync
    let (code_sync, body_sync) = tetherly_node::loopback_post(host_ui, "/api/topology/sync", b"{}")
        .await
        .unwrap();
    assert_eq!(code_sync, 200);
    let sync_rep: serde_json::Value = serde_json::from_slice(&body_sync).unwrap();
    assert_eq!(sync_rep["ok"], true);
    assert!(sync_rep["broadcast_count"].as_u64().unwrap() >= 1);

    // Peer should receive Host's topology over Wire protocol
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let (code_peer_peers, body_peer_peers) =
        tetherly_node::loopback_get(peer_ui, "/api/topology/peers")
            .await
            .unwrap();
    assert_eq!(code_peer_peers, 200);
    let peer_peers_rep: serde_json::Value = serde_json::from_slice(&body_peer_peers).unwrap();
    assert_eq!(peer_peers_rep["ok"], true);
    let peers_arr = peer_peers_rep["peers"].as_array().expect("peers array");
    let host_found = peers_arr
        .iter()
        .find(|p| p["peer_id"] == host_id.to_string());
    assert!(
        host_found.is_some(),
        "Peer must have cached Host's topology state"
    );
    let h_entry = host_found.unwrap();
    assert_eq!(h_entry["display"]["width"], 2560);
    assert_eq!(h_entry["display"]["height"], 1440);

    // 8. Test Edge Crossing Engine & Anti-Misclick Corner Lock Guardrails
    // Corner lock zone test: cursor movement in corner (e.g., x=2550, y=10, corner_lock=30)
    let corner_cross = host.evaluate_cursor_crossing(2550, 10);
    assert!(
        !corner_cross.crossed,
        "Cursor motion in corner lock zone must not cross boundary"
    );
    assert_eq!(
        corner_cross.blocked_reason.as_deref(),
        Some("corner anti-misclick exclusion zone")
    );

    // Position cursor near right edge inside bounds to prime last_pos
    let _ = host.evaluate_cursor_crossing(2550, 720);

    // Insufficient resistance test: push distance = 2559 - 2550 = 9px < 25px
    let weak_cross = host.evaluate_cursor_crossing(2559, 720);
    assert!(
        !weak_cross.crossed,
        "Cursor motion below resistance threshold must not cross boundary"
    );

    // Host tunes config with delay_ms = 0 to test pure resistance crossing
    let cfg_fast = serde_json::json!({
        "resistance_px": 10,
        "delay_ms": 0
    });
    let (code_cfg2, _) = tetherly_node::loopback_post(
        host_ui,
        "/api/topology/config",
        &serde_json::to_vec(&cfg_fast).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_cfg2, 200);

    // Prime last pos again
    let _ = host.evaluate_cursor_crossing(2540, 720);
    // Push past right edge: 2559 - 2540 = 19px >= 10px, delay = 0
    let valid_cross = host.evaluate_cursor_crossing(2559, 720);
    assert!(
        valid_cross.crossed,
        "Cursor pushing past resistance must trigger crossing: {:?}",
        valid_cross.blocked_reason
    );
    assert_eq!(
        valid_cross.edge,
        Some(tetherly_core::topology::DisplayEdge::Right)
    );
    assert_eq!(valid_cross.target_peer, Some(peer_id.to_string()));
    assert!(valid_cross.entry_x.is_some());
    assert!(valid_cross.entry_y.is_some());

    // 9. Host unlinks right edge via POST /api/topology/unlink
    let unlink_req = serde_json::json!({
        "edge": "right"
    });
    let (code_unlink, body_unlink) = tetherly_node::loopback_post(
        host_ui,
        "/api/topology/unlink",
        &serde_json::to_vec(&unlink_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_unlink, 200);
    let unlink_rep: serde_json::Value = serde_json::from_slice(&body_unlink).unwrap();
    assert_eq!(unlink_rep["ok"], true);

    // Verify neighbor is removed
    let (code_lay3, body_lay3) = tetherly_node::loopback_get(host_ui, "/api/topology/layout")
        .await
        .unwrap();
    assert_eq!(code_lay3, 200);
    let lay3: serde_json::Value = serde_json::from_slice(&body_lay3).unwrap();
    assert!(lay3["neighbors"]["right"].is_null());

    // 10. Host updates topology governance policy via POST /api/topology/policy
    let pol_update = serde_json::json!({
        "enabled": true,
        "allow_inbound": false,
        "auto_link": false,
        "group": ["workstations"],
        "tag": ["display-hub"],
        "device_type": ["desktop"]
    });
    let (code_pol_up, body_pol_up) = tetherly_node::loopback_post(
        host_ui,
        "/api/topology/policy",
        &serde_json::to_vec(&pol_update).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_pol_up, 200);
    let pol_updated: serde_json::Value = serde_json::from_slice(&body_pol_up).unwrap();
    assert_eq!(pol_updated["ok"], true);
    assert_eq!(pol_updated["policy"]["allow_inbound_control"], false);
    assert_eq!(pol_updated["policy"]["auto_link_paired_peers"], false);
    assert_eq!(
        pol_updated["policy"]["allowed_groups"],
        serde_json::json!(["workstations"])
    );

    // Verify layout and policy files persisted with 0o600 permissions
    let layout_path = g1.path().join("topology_layout.json");
    let policy_path = g1.path().join("topology_policy.json");
    assert!(layout_path.exists(), "topology_layout.json must exist");
    assert!(policy_path.exists(), "topology_policy.json must exist");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms_l = std::fs::metadata(&layout_path).unwrap().permissions();
        assert_eq!(
            perms_l.mode() & 0o777,
            0o600,
            "topology_layout.json must have 0o600 perms"
        );
        let perms_p = std::fs::metadata(&policy_path).unwrap().permissions();
        assert_eq!(
            perms_p.mode() & 0o777,
            0o600,
            "topology_policy.json must have 0o600 perms"
        );
    }

    // 11. Security Red Line: Host header 403 Forbidden attack defense
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], host_ui));
    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let evil_req =
        "GET /api/topology/layout HTTP/1.1\r\nHost: evil-attacker-phishing.com\r\nConnection: close\r\n\r\n";
    tokio::io::AsyncWriteExt::write_all(&mut stream, evil_req.as_bytes())
        .await
        .unwrap();
    tokio::io::AsyncWriteExt::flush(&mut stream).await.unwrap();
    let mut raw_resp = Vec::new();
    tokio::io::AsyncReadExt::read_to_end(&mut stream, &mut raw_resp)
        .await
        .unwrap();
    let resp_str = String::from_utf8_lossy(&raw_resp);
    assert!(
        resp_str.starts_with("HTTP/1.1 403") || resp_str.starts_with("HTTP/1.0 403"),
        "Host header attack must be rejected with 403 Forbidden: {resp_str}"
    );

    // 12. Clean Shutdown
    host.shutdown();
    peer.shutdown();
}

/// ADR-072 End-to-End Integration Test:
/// Virtual Peripheral Sharing, Cross-Device Input Control Tunneling &
/// Lifecycle Governance Pipeline via Loopback HTTP API & Wire Frame Protocol.
#[tokio::test]
async fn input_tunnel_virtual_peripheral_sharing_lifecycle_and_governance_via_loopback_api() {
    let (host, peer, g1, _g2) = pair_phone_desktop().await;
    let host_ui = host.spawn_ui().await.expect("host UI spawns");
    let peer_ui = peer.spawn_ui().await.expect("peer UI spawns");
    let host_id = host.identity().device_id().clone();
    let peer_id = peer.identity().device_id().clone();

    // 1. Connect to Host and Peer SSE streams
    let mut host_sse = tetherly_node::loopback_events_connect(host_ui)
        .await
        .expect("connect to host sse stream");
    let mut _peer_sse = tetherly_node::loopback_events_connect(peer_ui)
        .await
        .expect("connect to peer sse stream");

    // 2. Initial Input Tunnel Status Check on Host (inactive, default policy)
    let (code_tun, body_tun) = tetherly_node::loopback_get(host_ui, "/api/input/tunnel")
        .await
        .unwrap();
    assert_eq!(code_tun, 200);
    let tun_init: serde_json::Value = serde_json::from_slice(&body_tun).unwrap();
    assert_eq!(tun_init["ok"], true);
    assert_eq!(tun_init["active"], false);
    assert_eq!(tun_init["policy"]["enabled"], true);
    assert_eq!(tun_init["policy"]["allow_inbound_control"], true);
    assert_eq!(tun_init["policy"]["allow_keyboard"], true);
    assert_eq!(tun_init["policy"]["allow_mouse"], true);
    assert_eq!(tun_init["policy"]["hotkey_release_enabled"], true);
    assert_eq!(tun_init["policy"]["idle_timeout_secs"], 300);

    // 3. Initial Policy Check via GET /api/input/policy
    let (code_pol, body_pol) = tetherly_node::loopback_get(host_ui, "/api/input/policy")
        .await
        .unwrap();
    assert_eq!(code_pol, 200);
    let pol_init: serde_json::Value = serde_json::from_slice(&body_pol).unwrap();
    assert_eq!(pol_init["ok"], true);
    assert_eq!(pol_init["policy"]["enabled"], true);

    // 4. Host updates input governance policy via POST /api/input/policy
    let pol_req = serde_json::json!({
        "enabled": true,
        "allow_inbound_control": true,
        "allow_keyboard": true,
        "allow_mouse": true,
        "hotkey_release_enabled": false,
        "idle_timeout_secs": 180,
        "allowed_groups": ["workstations"]
    });
    let (code_set, body_set) = tetherly_node::loopback_post(
        host_ui,
        "/api/input/policy",
        &serde_json::to_vec(&pol_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_set, 200);
    let pol_set: serde_json::Value = serde_json::from_slice(&body_set).unwrap();
    assert_eq!(pol_set["ok"], true);
    assert_eq!(pol_set["policy"]["hotkey_release_enabled"], false);
    assert_eq!(pol_set["policy"]["idle_timeout_secs"], 180);

    // Verify policy file persisted with 0o600 permissions
    let policy_path = g1.path().join("input_policy.json");
    assert!(policy_path.exists(), "input_policy.json must exist");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::metadata(&policy_path).unwrap().permissions();
        assert_eq!(
            perms.mode() & 0o777,
            0o600,
            "input_policy.json must have 0o600 perms"
        );
    }

    // 5. Check SSE event on Host for policy change
    let mut saw_policy_changed = false;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(3);
    while tokio::time::Instant::now() < deadline {
        if let Ok(Ok(Some(raw))) =
            tokio::time::timeout(std::time::Duration::from_millis(300), host_sse.next_event()).await
        {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
                if v["kind"] == "input_tunnel_policy_changed" {
                    saw_policy_changed = true;
                    break;
                }
            }
        }
    }
    assert!(
        saw_policy_changed,
        "Host must emit input_tunnel_policy_changed SSE event"
    );

    // 6. Clear filters so peer is eligible for input control tunnel
    let (code_clear, _body_clear) = tetherly_node::loopback_post(
        host_ui,
        "/api/input/policy",
        &serde_json::to_vec(&serde_json::json!({
            "clear_filters": true,
            "hotkey_release_enabled": true,
            "idle_timeout_secs": 300
        }))
        .unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_clear, 200);

    // 7. Host attaches input tunnel to Peer via POST /api/input/tunnel/attach
    let attach_req = serde_json::json!({
        "peer": peer_id.to_string(),
        "edge": "right",
        "x": 100,
        "y": 200
    });
    let (code_att, body_att) = tetherly_node::loopback_post(
        host_ui,
        "/api/input/tunnel/attach",
        &serde_json::to_vec(&attach_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_att, 200);
    let att_rep: serde_json::Value = serde_json::from_slice(&body_att).unwrap();
    assert_eq!(att_rep["ok"], true);
    assert_eq!(att_rep["action"], "attach");
    assert_eq!(att_rep["role"], "controller");
    assert_eq!(att_rep["peer_id"], peer_id.to_string());
    let session_id = att_rep["session_id"].as_str().unwrap().to_string();

    // 8. Verify active session on Host via GET /api/input/tunnel
    let (code_host_act, body_host_act) = tetherly_node::loopback_get(host_ui, "/api/input/tunnel")
        .await
        .unwrap();
    assert_eq!(code_host_act, 200);
    let host_act: serde_json::Value = serde_json::from_slice(&body_host_act).unwrap();
    assert_eq!(host_act["active"], true);
    assert_eq!(host_act["session"]["session_id"], session_id);
    assert_eq!(host_act["session"]["role"], "controller");
    assert_eq!(host_act["session"]["peer_id"], peer_id.to_string());

    // 9. Verify Peer received handshake frame and activated input tunnel session
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(4);
    let mut peer_active = false;
    while tokio::time::Instant::now() < deadline {
        let (code_p, body_p) = tetherly_node::loopback_get(peer_ui, "/api/input/tunnel")
            .await
            .unwrap();
        if code_p == 200 {
            if let Ok(rep) = serde_json::from_slice::<serde_json::Value>(&body_p) {
                if rep["active"] == true && rep["session"]["session_id"] == session_id {
                    assert_eq!(rep["session"]["role"], "target");
                    assert_eq!(rep["session"]["peer_id"], host_id.to_string());
                    peer_active = true;
                    break;
                }
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(
        peer_active,
        "Peer must activate input tunnel session as target node"
    );

    // 10. Release input tunnel via POST /api/input/tunnel/release on Host
    let rel_req = serde_json::json!({
        "reason": "user_returned_to_desktop"
    });
    let (code_rel, body_rel) = tetherly_node::loopback_post(
        host_ui,
        "/api/input/tunnel/release",
        &serde_json::to_vec(&rel_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_rel, 200);
    let rel_rep: serde_json::Value = serde_json::from_slice(&body_rel).unwrap();
    assert_eq!(rel_rep["ok"], true);
    assert_eq!(rel_rep["action"], "release");
    assert_eq!(rel_rep["session_id"], session_id);

    // Verify Host is inactive
    let (code_host_inact, body_host_inact) =
        tetherly_node::loopback_get(host_ui, "/api/input/tunnel")
            .await
            .unwrap();
    assert_eq!(code_host_inact, 200);
    let host_inact: serde_json::Value = serde_json::from_slice(&body_host_inact).unwrap();
    assert_eq!(host_inact["active"], false);

    // Verify Peer receives release frame and becomes inactive
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(4);
    let mut peer_inactive = false;
    while tokio::time::Instant::now() < deadline {
        let (code_p, body_p) = tetherly_node::loopback_get(peer_ui, "/api/input/tunnel")
            .await
            .unwrap();
        if code_p == 200 {
            if let Ok(rep) = serde_json::from_slice::<serde_json::Value>(&body_p) {
                if rep["active"] == false {
                    peer_inactive = true;
                    break;
                }
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(
        peer_inactive,
        "Peer must transition to inactive after input tunnel release"
    );

    // 11. Security Red Line: Host header 403 Forbidden attack defense
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], host_ui));
    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let evil_req =
        "GET /api/input/tunnel HTTP/1.1\r\nHost: evil-attacker-phishing.com\r\nConnection: close\r\n\r\n";
    tokio::io::AsyncWriteExt::write_all(&mut stream, evil_req.as_bytes())
        .await
        .unwrap();
    tokio::io::AsyncWriteExt::flush(&mut stream).await.unwrap();
    let mut raw_resp = Vec::new();
    tokio::io::AsyncReadExt::read_to_end(&mut stream, &mut raw_resp)
        .await
        .unwrap();
    let resp_str = String::from_utf8_lossy(&raw_resp);
    assert!(
        resp_str.starts_with("HTTP/1.1 403") || resp_str.starts_with("HTTP/1.0 403"),
        "Host header attack must be rejected with 403 Forbidden: {resp_str}"
    );

    // 12. Clean Shutdown
    host.shutdown();
    peer.shutdown();
}

#[tokio::test]
async fn virtual_display_secondary_screen_lifecycle_and_governance_via_loopback_api() {
    let (host, peer, g1, _g2) = pair_phone_desktop().await;
    let host_ui = host.spawn_ui().await.expect("host UI spawns");
    let peer_ui = peer.spawn_ui().await.expect("peer UI spawns");
    let host_id = host.identity().device_id().clone();
    let peer_id = peer.identity().device_id().clone();

    // 1. Connect to Host SSE stream
    let mut host_sse = tetherly_node::loopback_events_connect(host_ui)
        .await
        .expect("connect to host sse stream");

    // 2. Initial Virtual Display Status Check on Host (inactive, default policy)
    let (code_disp, body_disp) = tetherly_node::loopback_get(host_ui, "/api/display/virtual")
        .await
        .unwrap();
    assert_eq!(code_disp, 200);
    let disp_init: serde_json::Value = serde_json::from_slice(&body_disp).unwrap();
    assert_eq!(disp_init["ok"], true);
    assert_eq!(disp_init["active"], false);
    assert_eq!(disp_init["policy"]["enabled"], true);
    assert_eq!(disp_init["policy"]["allow_hosting"], true);
    assert_eq!(disp_init["policy"]["allow_remote_extend"], true);
    assert_eq!(disp_init["policy"]["max_resolution_width"], 3840);
    assert_eq!(disp_init["policy"]["max_resolution_height"], 2160);
    assert_eq!(disp_init["policy"]["max_fps"], 60);
    assert_eq!(disp_init["policy"]["idle_timeout_secs"], 300);

    // 3. Initial Policy Check via GET /api/display/virtual/policy
    let (code_pol, body_pol) = tetherly_node::loopback_get(host_ui, "/api/display/virtual/policy")
        .await
        .unwrap();
    assert_eq!(code_pol, 200);
    let pol_init: serde_json::Value = serde_json::from_slice(&body_pol).unwrap();
    assert_eq!(pol_init["ok"], true);
    assert_eq!(pol_init["policy"]["enabled"], true);

    // 4. Host updates virtual display governance policy via POST /api/display/virtual/policy
    let pol_req = serde_json::json!({
        "enabled": true,
        "allow_hosting": true,
        "allow_remote_extend": true,
        "max_resolution_width": 2560,
        "max_resolution_height": 1440,
        "max_fps": 30,
        "idle_timeout_secs": 180,
        "allowed_groups": ["tablets"]
    });
    let (code_set, body_set) = tetherly_node::loopback_post(
        host_ui,
        "/api/display/virtual/policy",
        &serde_json::to_vec(&pol_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_set, 200);
    let pol_set: serde_json::Value = serde_json::from_slice(&body_set).unwrap();
    assert_eq!(pol_set["ok"], true);
    assert_eq!(pol_set["policy"]["max_fps"], 30);
    assert_eq!(pol_set["policy"]["idle_timeout_secs"], 180);

    // Verify policy file persisted with 0o600 permissions
    let policy_path = g1.path().join("virtual_display_policy.json");
    assert!(
        policy_path.exists(),
        "virtual_display_policy.json must exist"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::metadata(&policy_path).unwrap().permissions();
        assert_eq!(
            perms.mode() & 0o777,
            0o600,
            "virtual_display_policy.json must have 0o600 perms"
        );
    }

    // 5. Check SSE event on Host for policy change
    let mut saw_policy_changed = false;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(3);
    while tokio::time::Instant::now() < deadline {
        if let Ok(Ok(Some(raw))) =
            tokio::time::timeout(std::time::Duration::from_millis(300), host_sse.next_event()).await
        {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
                if v["kind"] == "virtual_display_policy_changed" {
                    saw_policy_changed = true;
                    break;
                }
            }
        }
    }
    assert!(
        saw_policy_changed,
        "Host must emit virtual_display_policy_changed SSE event"
    );

    // 6. Clear filters so peer is eligible for virtual display attachment
    let (code_clear, _body_clear) = tetherly_node::loopback_post(
        host_ui,
        "/api/display/virtual/policy",
        &serde_json::to_vec(&serde_json::json!({
            "clear_filters": true,
            "max_fps": 60,
            "idle_timeout_secs": 300
        }))
        .unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_clear, 200);

    // 7. Host attaches peer as virtual secondary screen via POST /api/display/virtual/attach
    let attach_req = serde_json::json!({
        "peer": peer_id.to_string(),
        "edge": "right",
        "width": 1920,
        "height": 1080,
        "fps": 60
    });
    let (code_att, body_att) = tetherly_node::loopback_post(
        host_ui,
        "/api/display/virtual/attach",
        &serde_json::to_vec(&attach_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_att, 200);
    let att_rep: serde_json::Value = serde_json::from_slice(&body_att).unwrap();
    assert_eq!(att_rep["ok"], true);
    assert_eq!(att_rep["action"], "attach");
    assert_eq!(att_rep["role"], "host");
    assert_eq!(att_rep["peer_id"], peer_id.to_string());
    let session_id = att_rep["session_id"].as_str().unwrap().to_string();

    // 8. Verify active session on Host via GET /api/display/virtual
    let (code_host_act, body_host_act) =
        tetherly_node::loopback_get(host_ui, "/api/display/virtual")
            .await
            .unwrap();
    assert_eq!(code_host_act, 200);
    let host_act: serde_json::Value = serde_json::from_slice(&body_host_act).unwrap();
    assert_eq!(host_act["active"], true);
    assert_eq!(host_act["session"]["session_id"], session_id);
    assert_eq!(host_act["session"]["role"], "host");
    assert_eq!(host_act["session"]["peer_id"], peer_id.to_string());
    assert_eq!(host_act["session"]["assigned_edge"], "right");
    assert_eq!(host_act["session"]["mode"]["width"], 1920);
    assert_eq!(host_act["session"]["mode"]["height"], 1080);

    // 9. Verify Peer received attach frame and activated as sink node
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(4);
    let mut peer_active = false;
    while tokio::time::Instant::now() < deadline {
        let (code_p, body_p) = tetherly_node::loopback_get(peer_ui, "/api/display/virtual")
            .await
            .unwrap();
        if code_p == 200 {
            if let Ok(rep) = serde_json::from_slice::<serde_json::Value>(&body_p) {
                if rep["active"] == true && rep["session"]["session_id"] == session_id {
                    assert_eq!(rep["session"]["role"], "sink");
                    assert_eq!(rep["session"]["peer_id"], host_id.to_string());
                    peer_active = true;
                    break;
                }
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(
        peer_active,
        "Peer must activate virtual display session as sink node"
    );

    // 10. Detach virtual display via POST /api/display/virtual/detach on Host
    let det_req = serde_json::json!({
        "reason": "presentation_finished"
    });
    let (code_det, body_det) = tetherly_node::loopback_post(
        host_ui,
        "/api/display/virtual/detach",
        &serde_json::to_vec(&det_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_det, 200);
    let det_rep: serde_json::Value = serde_json::from_slice(&body_det).unwrap();
    assert_eq!(det_rep["ok"], true);
    assert_eq!(det_rep["action"], "detach");
    assert_eq!(det_rep["session_id"], session_id);

    // Verify Host is inactive
    let (code_host_inact, body_host_inact) =
        tetherly_node::loopback_get(host_ui, "/api/display/virtual")
            .await
            .unwrap();
    assert_eq!(code_host_inact, 200);
    let host_inact: serde_json::Value = serde_json::from_slice(&body_host_inact).unwrap();
    assert_eq!(host_inact["active"], false);

    // Verify Peer receives detach frame and becomes inactive
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(4);
    let mut peer_inactive = false;
    while tokio::time::Instant::now() < deadline {
        let (code_p, body_p) = tetherly_node::loopback_get(peer_ui, "/api/display/virtual")
            .await
            .unwrap();
        if code_p == 200 {
            if let Ok(rep) = serde_json::from_slice::<serde_json::Value>(&body_p) {
                if rep["active"] == false {
                    peer_inactive = true;
                    break;
                }
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(
        peer_inactive,
        "Peer must transition to inactive after virtual display detach"
    );

    // 11. Security Red Line: Host header 403 Forbidden attack defense
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], host_ui));
    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let evil_req =
        "GET /api/display/virtual HTTP/1.1\r\nHost: evil-attacker-virtual-display.com\r\nConnection: close\r\n\r\n";
    tokio::io::AsyncWriteExt::write_all(&mut stream, evil_req.as_bytes())
        .await
        .unwrap();
    tokio::io::AsyncWriteExt::flush(&mut stream).await.unwrap();
    let mut raw_resp = Vec::new();
    tokio::io::AsyncReadExt::read_to_end(&mut stream, &mut raw_resp)
        .await
        .unwrap();
    let resp_str = String::from_utf8_lossy(&raw_resp);
    assert!(
        resp_str.starts_with("HTTP/1.1 403") || resp_str.starts_with("HTTP/1.0 403"),
        "Host header attack must be rejected with 403 Forbidden: {resp_str}"
    );

    // 12. Clean Shutdown
    host.shutdown();
    peer.shutdown();
}

#[tokio::test]
async fn spatial_audio_routing_lifecycle_and_governance_via_loopback_api() {
    let (host, peer, g1, _g2) = pair_phone_desktop().await;
    let host_ui = host.spawn_ui().await.expect("host UI spawns");
    let peer_ui = peer.spawn_ui().await.expect("peer UI spawns");
    let host_id = host.identity().device_id().clone();
    let peer_id = peer.identity().device_id().clone();

    // 1. Connect to Host SSE stream
    let mut host_sse = tetherly_node::loopback_events_connect(host_ui)
        .await
        .expect("connect to host sse stream");

    // 2. Initial Audio Routing Status Check on Host (inactive, default policy)
    let (code_snd, body_snd) = tetherly_node::loopback_get(host_ui, "/api/audio/routing")
        .await
        .unwrap();
    assert_eq!(code_snd, 200);
    let snd_init: serde_json::Value = serde_json::from_slice(&body_snd).unwrap();
    assert_eq!(snd_init["ok"], true);
    assert_eq!(snd_init["active"], false);
    assert_eq!(snd_init["policy"]["enabled"], true);
    assert_eq!(snd_init["policy"]["allow_inbound_sink"], true);
    assert_eq!(snd_init["policy"]["allow_outbound_routing"], true);
    assert_eq!(snd_init["policy"]["max_volume_limit"], 100);
    assert_eq!(snd_init["policy"]["allow_spatial_rendering"], true);
    assert_eq!(snd_init["policy"]["idle_timeout_secs"], 300);

    // 3. Initial Policy Check via GET /api/audio/routing/policy
    let (code_pol, body_pol) = tetherly_node::loopback_get(host_ui, "/api/audio/routing/policy")
        .await
        .unwrap();
    assert_eq!(code_pol, 200);
    let pol_init: serde_json::Value = serde_json::from_slice(&body_pol).unwrap();
    assert_eq!(pol_init["ok"], true);
    assert_eq!(pol_init["policy"]["enabled"], true);

    // 4. Host updates audio routing governance policy via POST /api/audio/routing/policy
    let pol_req = serde_json::json!({
        "enabled": true,
        "allow_inbound_sink": true,
        "allow_outbound_routing": true,
        "max_volume_limit": 80,
        "allow_spatial_rendering": true,
        "idle_timeout_secs": 180,
        "allowed_groups": ["speakers"]
    });
    let (code_set, body_set) = tetherly_node::loopback_post(
        host_ui,
        "/api/audio/routing/policy",
        &serde_json::to_vec(&pol_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_set, 200);
    let pol_set: serde_json::Value = serde_json::from_slice(&body_set).unwrap();
    assert_eq!(pol_set["ok"], true);
    assert_eq!(pol_set["policy"]["max_volume_limit"], 80);
    assert_eq!(pol_set["policy"]["idle_timeout_secs"], 180);

    // Verify policy file persisted with 0o600 permissions
    let policy_path = g1.path().join("audio_routing_policy.json");
    assert!(policy_path.exists(), "audio_routing_policy.json must exist");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::metadata(&policy_path).unwrap().permissions();
        assert_eq!(
            perms.mode() & 0o777,
            0o600,
            "audio_routing_policy.json must have 0o600 perms"
        );
    }

    // 5. Check SSE event on Host for audio_routing_policy_changed
    let mut saw_policy_changed = false;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(3);
    while tokio::time::Instant::now() < deadline {
        if let Ok(Ok(Some(raw))) =
            tokio::time::timeout(std::time::Duration::from_millis(300), host_sse.next_event()).await
        {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
                if v["kind"] == "audio_routing_policy_changed" {
                    saw_policy_changed = true;
                    break;
                }
            }
        }
    }
    assert!(
        saw_policy_changed,
        "Host must emit audio_routing_policy_changed SSE event"
    );

    // 6. Clear filters so peer is eligible for spatial audio attachment
    let (code_clear, _body_clear) = tetherly_node::loopback_post(
        host_ui,
        "/api/audio/routing/policy",
        &serde_json::to_vec(&serde_json::json!({
            "clear_filters": true,
            "max_volume_limit": 100,
            "idle_timeout_secs": 300
        }))
        .unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_clear, 200);

    // 7. Host attaches peer for spatial audio routing via POST /api/audio/routing/attach
    let attach_req = serde_json::json!({
        "peer": peer_id.to_string(),
        "channel_layout": "surround5_1",
        "sample_rate": 48000,
        "volume_gain": 1.2,
        "spatial_position": {
            "x": 1.5,
            "y": -0.5,
            "z": 0.0
        }
    });
    let (code_att, body_att) = tetherly_node::loopback_post(
        host_ui,
        "/api/audio/routing/attach",
        &serde_json::to_vec(&attach_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_att, 200);
    let att_rep: serde_json::Value = serde_json::from_slice(&body_att).unwrap();
    assert_eq!(att_rep["ok"], true);
    assert_eq!(att_rep["action"], "attach");
    assert_eq!(att_rep["role"], "source");
    assert_eq!(att_rep["peer_id"], peer_id.to_string());
    let session_id = att_rep["session_id"].as_str().unwrap().to_string();

    // 8. Verify active session on Host via GET /api/audio/routing
    let (code_host_act, body_host_act) = tetherly_node::loopback_get(host_ui, "/api/audio/routing")
        .await
        .unwrap();
    assert_eq!(code_host_act, 200);
    let host_act: serde_json::Value = serde_json::from_slice(&body_host_act).unwrap();
    assert_eq!(host_act["active"], true);
    assert_eq!(host_act["session"]["session_id"], session_id);
    assert_eq!(host_act["session"]["role"], "source");
    assert_eq!(host_act["session"]["peer_id"], peer_id.to_string());
    assert_eq!(host_act["session"]["layout"], "surround5_1");
    assert_eq!(host_act["session"]["sample_rate"], 48000);

    // 9. Verify Peer received attach frame and activated as sink node
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(4);
    let mut peer_active = false;
    while tokio::time::Instant::now() < deadline {
        let (code_p, body_p) = tetherly_node::loopback_get(peer_ui, "/api/audio/routing")
            .await
            .unwrap();
        if code_p == 200 {
            if let Ok(rep) = serde_json::from_slice::<serde_json::Value>(&body_p) {
                if rep["active"] == true && rep["session"]["session_id"] == session_id {
                    assert_eq!(rep["session"]["role"], "sink");
                    assert_eq!(rep["session"]["peer_id"], host_id.to_string());
                    peer_active = true;
                    break;
                }
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(
        peer_active,
        "Peer must activate spatial audio routing session as sink node"
    );

    // 10. Detach audio routing session via POST /api/audio/routing/detach on Host
    let det_req = serde_json::json!({
        "reason": "audio_stream_complete"
    });
    let (code_det, body_det) = tetherly_node::loopback_post(
        host_ui,
        "/api/audio/routing/detach",
        &serde_json::to_vec(&det_req).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(code_det, 200);
    let det_rep: serde_json::Value = serde_json::from_slice(&body_det).unwrap();
    assert_eq!(det_rep["ok"], true);
    assert_eq!(det_rep["action"], "detach");
    assert_eq!(det_rep["session_id"], session_id);

    // Verify Host is inactive
    let (code_host_inact, body_host_inact) =
        tetherly_node::loopback_get(host_ui, "/api/audio/routing")
            .await
            .unwrap();
    assert_eq!(code_host_inact, 200);
    let host_inact: serde_json::Value = serde_json::from_slice(&body_host_inact).unwrap();
    assert_eq!(host_inact["active"], false);

    // Verify Peer receives detach frame and becomes inactive
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(4);
    let mut peer_inactive = false;
    while tokio::time::Instant::now() < deadline {
        let (code_p, body_p) = tetherly_node::loopback_get(peer_ui, "/api/audio/routing")
            .await
            .unwrap();
        if code_p == 200 {
            if let Ok(rep) = serde_json::from_slice::<serde_json::Value>(&body_p) {
                if rep["active"] == false {
                    peer_inactive = true;
                    break;
                }
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(
        peer_inactive,
        "Peer must transition to inactive after audio routing detach"
    );

    // 11. Security Red Line: Host header 403 Forbidden attack defense
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], host_ui));
    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let evil_req =
        "GET /api/audio/routing HTTP/1.1\r\nHost: evil-attacker-spatial-audio.com\r\nConnection: close\r\n\r\n";
    tokio::io::AsyncWriteExt::write_all(&mut stream, evil_req.as_bytes())
        .await
        .unwrap();
    tokio::io::AsyncWriteExt::flush(&mut stream).await.unwrap();
    let mut raw_resp = Vec::new();
    tokio::io::AsyncReadExt::read_to_end(&mut stream, &mut raw_resp)
        .await
        .unwrap();
    let resp_str = String::from_utf8_lossy(&raw_resp);
    assert!(
        resp_str.starts_with("HTTP/1.1 403") || resp_str.starts_with("HTTP/1.0 403"),
        "Host header attack must be rejected with 403 Forbidden: {resp_str}"
    );

    // 12. Clean Shutdown
    host.shutdown();
    peer.shutdown();
}

fn tetherly_cli_test_url_encode(s: &str) -> String {
    let mut res = String::new();
    for b in s.as_bytes() {
        match *b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                res.push(*b as char);
            }
            byte => {
                res.push_str(&format!("%{byte:02X}"));
            }
        }
    }
    res
}

mod tracing_capture {
    use std::io::{self, Write};
    use std::sync::{Arc, Mutex};
    use tracing_subscriber::fmt::MakeWriter;

    #[derive(Clone)]
    pub struct Capture(Arc<Mutex<Vec<u8>>>);

    impl Capture {
        /// Install with an explicit max level. Redaction tests use `trace` so
        /// a leak at any level is captured rather than filtered away.
        pub fn install_with(level: &str) -> Self {
            let cap = Capture(Arc::new(Mutex::new(Vec::new())));
            let _ = tracing_subscriber::fmt()
                .with_max_level(level.parse().unwrap_or(tracing::Level::TRACE))
                .with_writer(cap.clone())
                .with_ansi(false)
                .try_init();
            cap
        }

        pub fn dump(&self) -> String {
            String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
        }
    }

    impl<'a> MakeWriter<'a> for Capture {
        type Writer = GuardWriter;

        fn make_writer(&'a self) -> Self::Writer {
            GuardWriter(self.0.clone())
        }
    }

    pub struct GuardWriter(Arc<Mutex<Vec<u8>>>);

    impl Write for GuardWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
}
