// SPDX-License-Identifier: Apache-2.0 OR MIT
//! On-device probe. Built for `aarch64-linux-android`, pushed with adb, run
//! from `adb shell` to prove the real pairing + notify path on a physical
//! phone (no APK/gradle needed). Also runs on the host for a quick smoke test.
//!
//! Modes:
//!   android_probe <host:port> <pin> [data_dir] [title] [body]      # pair + notify
//!   android_probe bench <host:port> <pin> [data_dir] [count]       # M1.1 RTT p95
//!
//! `bench` measures the phone→desktop→phone round-trip over the real Noise
//! session (ping that the desktop answers with pong) and prints p50/p95/p99 in
//! microseconds. That is the transport half of the M1.1 budget; the phone's
//! listener→enqueue and the desktop's ingest→candidate are local microseconds.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tetherly_android::{percentile, rtt_bench, AndroidClient};
use tetherly_core::NotifyPush;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() >= 2 && args[1] == "bench" {
        bench(&args);
        return;
    }
    probe(&args);
}

/// Pair (or resume) and deliver one notification — the M1.1 smoke proof.
fn probe(args: &[String]) {
    if args.len() < 3 {
        eprintln!("usage: android_probe <host:port> <pin> [data_dir] [title] [body]");
        eprintln!("       android_probe bench <host:port> <pin> [data_dir] [count]");
        std::process::exit(2);
    }
    let addr: SocketAddr = args[1].parse().expect("host:port");
    let pin = tetherly_core::pin::parse_pin(&args[2]).expect("8-digit pin");
    let dir = args
        .get(3)
        .cloned()
        .unwrap_or_else(|| "/data/local/tmp/tetherly-probe".to_string());
    let title = args
        .get(4)
        .cloned()
        .unwrap_or_else(|| "Tetherly Android probe".to_string());
    let body = args
        .get(5)
        .cloned()
        .unwrap_or_else(|| "【Tetherly】验证码:246810".to_string());

    let client = AndroidClient::load_or_create(dir, "android-probe").expect("client");
    println!("local_device_id={}", client.device_id());
    let handle = client.start(addr, Some(pin), false);

    let mut waited = 0;
    while handle.connected_peer().is_none() && waited < 150 {
        std::thread::sleep(Duration::from_millis(100));
        waited += 1;
    }
    let peer = match handle.connected_peer() {
        Some(p) => p,
        None => {
            eprintln!("not connected: {:?}", handle.last_error());
            handle.stop();
            std::process::exit(1);
        }
    };
    println!("connected_peer={peer}");

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let push = NotifyPush {
        uid: format!("probe-{now}"),
        app_id: "app.tetherly.probe".to_string(),
        app_name: "Tetherly Probe".to_string(),
        title,
        body,
        ts: now,
        actions: vec!["copy".to_string(), "dismiss".to_string()],
    };
    println!("notify_queued={}", handle.notify(push));
    std::thread::sleep(Duration::from_millis(1500));
    handle.stop();
    println!("done");
}

/// M1.1 latency harness: N ping/pong round-trips, report percentiles in µs.
fn bench(args: &[String]) {
    if args.len() < 4 {
        eprintln!("usage: android_probe bench <host:port> <pin> [data_dir] [count]");
        std::process::exit(2);
    }
    let addr: SocketAddr = args[2].parse().expect("host:port");
    let pin = tetherly_core::pin::parse_pin(&args[3]).expect("8-digit pin");
    let dir = PathBuf::from(
        args.get(4)
            .cloned()
            .unwrap_or_else(|| "/data/local/tmp/tetherly-probe".to_string()),
    );
    let count: usize = args.get(5).and_then(|s| s.parse().ok()).unwrap_or(100);

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let samples = match rt.block_on(rtt_bench(
        addr,
        dir,
        "android-probe",
        Some(pin),
        count,
        Duration::from_millis(10),
    )) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("bench failed: {e}");
            std::process::exit(1);
        }
    };
    let n = samples.len();
    let min = samples.iter().copied().min().unwrap_or(0);
    let max = samples.iter().copied().max().unwrap_or(0);
    let mean = samples.iter().sum::<u64>() / n.max(1) as u64;
    println!("samples={n}");
    println!("rtt_min_us={min}");
    println!("rtt_mean_us={mean}");
    println!("rtt_p50_us={}", percentile(&samples, 0.50).unwrap_or(0));
    println!("rtt_p95_us={}", percentile(&samples, 0.95).unwrap_or(0));
    println!("rtt_p99_us={}", percentile(&samples, 0.99).unwrap_or(0));
    println!("rtt_max_us={max}");
}
