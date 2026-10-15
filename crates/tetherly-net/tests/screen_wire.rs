// SPDX-License-Identifier: Apache-2.0 OR MIT
//! A full-size screen frame must survive the wire.
//!
//! Pins the latent oversize defect fixed alongside v1.21: `INNER_PAYLOAD_MAX`
//! used to be a flat 64 KiB, ignoring the 12-byte inner header and the 16-byte
//! AEAD tag that both eat into the transport's 65535-byte message cap. A
//! screen frame large enough to fragment therefore produced fragment payloads
//! of `24 + 65512 = 65536` bytes whose encoded plaintext (`65548`) the Noise
//! transport refuses — every such frame failed to send with
//! `Crypto(Noise("input error"))`. zstd keeps most real frames small, which is
//! why the shipped end-to-end path never hit it. This test sends the fragments
//! through a real session and reassembles, so the budget cannot silently drift
//! above the transport again.

use std::sync::{Arc, Mutex};

use tetherly_core::frame::INNER_PAYLOAD_MAX;
use tetherly_core::ports::{Clock, MemoryTrustStore, SystemClock};
use tetherly_core::screen::{PixelFormat, ScreenAssembler, ScreenFrame, ScreenRect};
use tetherly_core::session::LockoutTable;
use tetherly_core::TcpLimiter;
use tetherly_crypto::Identity;
use tetherly_net::session::{accept_session, dial_session, SessionConfig, HANDSHAKE_TIMEOUT};
use tokio::net::{TcpListener, TcpStream};

fn cfg(seed: u8, name: &str) -> SessionConfig {
    SessionConfig {
        identity: Identity::from_secrets([seed; 32], [seed.wrapping_add(1); 32]),
        name: name.into(),
        platform: "linux".into(),
        trust: Arc::new(Mutex::new(MemoryTrustStore::default())),
        lockout: Arc::new(Mutex::new(LockoutTable::default())),
        tcp_limiter: Arc::new(Mutex::new(TcpLimiter::default())),
        pin: Some(tetherly_core::pin::parse_pin("25170394").unwrap()),
        pin_created_ms: SystemClock.unix_ms(),
        clock: Arc::new(SystemClock),
        handshake_timeout: HANDSHAKE_TIMEOUT,
        hello_override: None,
        resume_only: false,
        screen_only: false,
        caps: None,
    }
}

/// A 1024x1024 BGRA frame is 4 MiB — many fragments past the inner budget, and
/// the exact shape whose fragments used to exceed the transport cap.
fn big_frame() -> ScreenFrame {
    let rect = ScreenRect::new(0, 0, 1024, 1024).unwrap();
    let bytes: Vec<u8> = (0..rect.pixel_bytes(PixelFormat::Bgra8))
        .map(|i| (i % 251) as u8)
        .collect();
    ScreenFrame::raw(42, rect, PixelFormat::Bgra8, bytes).unwrap()
}

#[tokio::test]
async fn a_fragmented_screen_frame_crosses_a_real_session_and_reassembles() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let frame = big_frame();
    let frames = frame.to_frames().unwrap();
    assert!(
        frames.len() > 2,
        "the frame must fragment, got {}",
        frames.len()
    );
    for fr in &frames {
        assert!(
            fr.payload.len() <= INNER_PAYLOAD_MAX,
            "fragment payload {} exceeds INNER_PAYLOAD_MAX {INNER_PAYLOAD_MAX}",
            fr.payload.len()
        );
    }

    let expected = frame.clone();
    let expected_count = frames.len();
    let server = tokio::spawn(async move {
        let (stream, peer) = listener.accept().await.unwrap();
        let mut sess = accept_session(stream, peer, &cfg(0x71, "server"))
            .await
            .unwrap();
        let mut asm = ScreenAssembler::new();
        let mut out = None;
        for _ in 0..expected_count {
            let fr = sess
                .recv_inner()
                .await
                .expect("a fragment must decrypt and decode");
            out = asm.push(&fr).expect("fragment must be well-formed");
        }
        out.expect("the last fragment yields the frame")
    });

    let stream = TcpStream::connect(addr).await.unwrap();
    let mut sess = dial_session(stream, addr, &cfg(0x81, "client"))
        .await
        .unwrap();
    for fr in frames {
        sess.send_bounded(fr)
            .await
            .expect("every fragment must be encryptable and sendable");
    }

    let got = server.await.unwrap();
    assert_eq!(
        got, expected,
        "the reassembled frame must equal the original"
    );
}
