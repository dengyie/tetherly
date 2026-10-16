// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Phase 1 LAN loopback: notify/OTP, clip, persist reconnect, file confirm.
//! Physical Android p95 / 8h soak remain Manual-required.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tetherly_core::{hex_sha256, FileMeta, FileOffer, ManualClock, NotifyPush, CANDIDATE_TTL_MS};
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
    while node.live_peers().is_empty() {
        if start.elapsed() > timeout {
            panic!("peer did not come up");
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
    wait_live(&phone, Duration::from_secs(25)).await;
    wait_live(&desk, Duration::from_secs(25)).await;
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
    wait_live(&a, Duration::from_secs(25)).await;
    wait_live(&b, Duration::from_secs(25)).await;

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
    wait_live(&phone2, Duration::from_secs(25)).await;
    wait_live(&desk, Duration::from_secs(25)).await;
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
    wait_live(&a, Duration::from_secs(25)).await;
    wait_live(&b, Duration::from_secs(25)).await;
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
    wait_live(&a, Duration::from_secs(25)).await;
    wait_live(&b, Duration::from_secs(25)).await;
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
    wait_live(&a, Duration::from_secs(25)).await;
    wait_live(&b, Duration::from_secs(25)).await;
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
    wait_live(&host, Duration::from_secs(25)).await;
    wait_live(&peer, Duration::from_secs(25)).await;
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
    wait_offered(&peer, Duration::from_secs(5)).await;
    peer.user_accept_file(transfer_id).await.unwrap();

    // 4. Peer receives and saves the file in inbox
    let received = wait_inbox(g2.path(), "report.pdf", Duration::from_secs(10)).await;
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

    wait_offered(&peer, Duration::from_secs(5)).await;
    peer.user_accept_file(transfer_id_b64).await.unwrap();
    let received_b64 = wait_inbox(g2.path(), "b64_doc.txt", Duration::from_secs(10)).await;
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

    wait_offered(&peer, Duration::from_secs(5)).await;
    peer.user_accept_file(transfer_id).await.unwrap();

    let received = wait_inbox(g2.path(), "large_archive.bin", Duration::from_secs(10)).await;
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

    wait_offered(&peer, Duration::from_secs(5)).await;
    peer.user_accept_file(transfer_id).await.unwrap();

    let received = wait_inbox(g2.path(), "large_progress.bin", Duration::from_secs(10)).await;
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
