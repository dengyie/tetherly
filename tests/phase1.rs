// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Phase 1 LAN loopback: notify/OTP, clip, persist reconnect, file confirm.
//! Physical Android p95 / 8h soak remain Manual-required.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tetherly_core::{hex_sha256, FileMeta, FileOffer, ManualClock, NotifyPush, CANDIDATE_TTL_MS};
use tetherly_node::{MemoryInsertor, Node, NodeConfig, OverlayConfig, UiEvent};

const OTP_BODY: &str = "【测试】验证码：524681，您正在登录";
const OTP: &str = "524681";

struct DirGuard(PathBuf);

impl DirGuard {}

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
        screen_codec_h264: false,
        screen_bitrate_kbps: 4000,
        screen_bitrate_auto: true,
        screen_fps: 30,
        screen_fps_auto: true,
        screen_scale_pct: 100,
        screen_scale_auto: true,
        screen_profile: tetherly_core::ScreenContentProfile::Auto,
        screen_profile_auto: true,
        screen_qp_min: tetherly_core::SCREEN_QP_MIN_DEFAULT,
        screen_qp_max: tetherly_core::SCREEN_QP_MAX_DEFAULT,
        screen_matrix_mode: tetherly_core::ScreenMatrixMode::Auto,
        screen_playout_mode: tetherly_core::ScreenPlayoutMode::Adaptive,
        screen_cursor_mode: tetherly_core::ScreenCursorMode::Decoupled,
        screen_squelch_mode: tetherly_core::ScreenSquelchMode::Auto,
        screen_fec_mode: tetherly_core::ScreenFecMode::Auto,
        screen_pacer_mode: tetherly_core::ScreenPacerMode::Auto,
        screen_trendline_mode: tetherly_core::ScreenTrendlineMode::Auto,
        screen_harq_mode: tetherly_core::ScreenHarqMode::Auto,
        screen_twcc_mode: tetherly_core::ScreenTwccMode::Auto,
        screen_probe_mode: tetherly_core::ScreenProbeMode::Auto,
        screen_sync_mode: tetherly_core::ScreenAudioSyncMode::Auto,
        screen_latency_mode: tetherly_core::ScreenProfilerMode::Auto,
        screen_tuning_mode: tetherly_core::ScreenTunerMode::Standard,
        screen_display_id: 0,
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
    // Record opens instead of launching the user's real browser.
    let opener = tetherly_node::MemoryOpener::new();
    phone.set_opener(Arc::new(opener.clone()));
    desk.set_opener(Arc::new(opener.clone()));
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

/// v0.3.3 — a folder travels as one bundled transfer and lands as a real
/// folder (nested files, empty dirs, no Finder litter, no leftover tar).
#[tokio::test(flavor = "multi_thread")]
async fn a_folder_arrives_as_a_folder() {
    let (a_dir, _g1) = temp_dir("dir-a");
    let (b_dir, _g2) = temp_dir("dir-b");
    let a = Node::start(cfg(a_dir.clone(), "a", "macos")).unwrap();
    let b = Node::start(cfg(b_dir.clone(), "b", "windows")).unwrap();
    a.spawn_listeners().await.unwrap();
    b.spawn_listeners().await.unwrap();
    let pin = tetherly_core::pin::parse_pin("25170394").unwrap();
    a.set_pin(pin);
    b.set_pin(pin);
    a.dial(b.control_addr()).await.unwrap();
    wait_live(&a, Duration::from_secs(45)).await;
    wait_live(&b, Duration::from_secs(45)).await;

    let src = std::path::Path::new(&a_dir).join("src").join("muse ext");
    std::fs::create_dir_all(src.join("icons/16")).unwrap();
    std::fs::create_dir_all(src.join("empty")).unwrap();
    std::fs::write(src.join("manifest.json"), b"{}").unwrap();
    std::fs::write(src.join("icons/16/a.png"), vec![7u8; 300_000]).unwrap();
    std::fs::write(src.join(".DS_Store"), b"x").unwrap();
    let peer_b = b.identity().device_id().clone();
    let id = a.offer_and_send_folder(&peer_b, &src).await.unwrap();
    wait_offered(&b, Duration::from_secs(3)).await;
    b.user_accept_file(&id).await.unwrap();
    let got = wait_inbox(&b_dir, "muse ext/icons/16/a.png", Duration::from_secs(20)).await;
    assert_eq!(got, vec![7u8; 300_000]);
    let inbox = std::path::Path::new(&b_dir).join("inbox");
    assert_eq!(
        std::fs::read(inbox.join("muse ext/manifest.json")).unwrap(),
        b"{}"
    );
    assert!(inbox.join("muse ext/empty").is_dir());
    assert!(!inbox.join("muse ext/.DS_Store").exists());
    assert!(!inbox.join("muse ext.tar").exists());
    let leftovers: Vec<_> = std::fs::read_dir(&inbox)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".part") || n.ends_with(".unpack"))
        .collect();
    assert!(leftovers.is_empty(), "leftovers: {leftovers:?}");
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
            bundle: None,
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
