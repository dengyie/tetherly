// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Phase 1 LAN loopback: notify/OTP, clip, persist reconnect, file confirm.
//! Physical Android p95 / 8h soak remain Manual-required.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tetherly_core::{hex_sha256, ManualClock, NotifyPush, CANDIDATE_TTL_MS};
use tetherly_node::{MemoryInsertor, Node, NodeConfig, OverlayConfig, UiEvent};

const OTP_BODY: &str = "【测试】验证码：524681，您正在登录";
const OTP: &str = "524681";

struct DirGuard(PathBuf);

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
    wait_live(&phone, Duration::from_secs(8)).await;
    wait_live(&desk, Duration::from_secs(8)).await;
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
    wait_live(&a, Duration::from_secs(8)).await;
    wait_live(&b, Duration::from_secs(8)).await;

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
    wait_live(&phone2, Duration::from_secs(8)).await;
    wait_live(&desk, Duration::from_secs(8)).await;
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
    let buf = tracing_capture::Capture::install();
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
    wait_live(&a, Duration::from_secs(8)).await;
    wait_live(&b, Duration::from_secs(8)).await;
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

/// Minimal tracing subscriber that records formatted events for M1.6.
mod tracing_capture {
    use std::io::{self, Write};
    use std::sync::{Arc, Mutex};
    use tracing_subscriber::fmt::MakeWriter;

    #[derive(Clone)]
    pub struct Capture(Arc<Mutex<Vec<u8>>>);

    impl Capture {
        pub fn install() -> Self {
            let cap = Capture(Arc::new(Mutex::new(Vec::new())));
            let _ = tracing_subscriber::fmt()
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
