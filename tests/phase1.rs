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
    let sse_event2 = tokio::time::timeout(Duration::from_secs(10), sse.next_ui_event())
        .await
        .expect("timeout waiting for sse pin cleared event")
        .expect("read sse event")
        .expect("some ui event");
    match sse_event2 {
        tetherly_node::UiEvent::PinStateChanged { active } => {
            assert!(!active);
        }
        other => panic!("expected PinStateChanged active=false, got: {other:?}"),
    }

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
    let sse_event2 = tokio::time::timeout(Duration::from_secs(10), sse.next_ui_event())
        .await
        .expect("timeout waiting for sse route updated event")
        .expect("read sse event")
        .expect("some ui event");
    match sse_event2 {
        tetherly_node::UiEvent::RouteChanged {
            device_id,
            addr,
            action,
        } => {
            assert_eq!(device_id, static_device_id);
            assert_eq!(addr, updated_addr);
            assert_eq!(action, "updated");
        }
        other => panic!("expected RouteChanged updated, got: {other:?}"),
    }

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
    let sse_smart = tokio::time::timeout(Duration::from_secs(10), sse.next_ui_event())
        .await
        .expect("timeout waiting for sse smart route event")
        .expect("read sse event")
        .expect("some ui event");
    match sse_smart {
        tetherly_node::UiEvent::RouteChanged {
            device_id,
            addr,
            action,
        } => {
            assert_eq!(device_id, peer_id.as_str());
            assert_eq!(addr, smart_addr);
            assert_eq!(action, "updated");
        }
        other => panic!("expected RouteChanged updated, got: {other:?}"),
    }

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
    let sse_event3 = tokio::time::timeout(Duration::from_secs(10), sse.next_ui_event())
        .await
        .expect("timeout waiting for sse route removed event")
        .expect("read sse event")
        .expect("some ui event");
    match sse_event3 {
        tetherly_node::UiEvent::RouteChanged {
            device_id, action, ..
        } => {
            assert_eq!(device_id, static_device_id);
            assert_eq!(action, "removed");
        }
        other => panic!("expected RouteChanged removed, got: {other:?}"),
    }

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
        let (code_multi, body_multi) = tetherly_node::loopback_get(
            host_ui,
            &format!("/api/file/offers?id={multi_offer_id}"),
        )
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
