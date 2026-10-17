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
