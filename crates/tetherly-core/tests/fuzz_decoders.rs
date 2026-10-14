// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Deterministic decoder fuzzing (spec §13 `fuzz` layer) on the stable
//! toolchain.
//!
//! libFuzzer / `cargo-fuzz` need nightly and a sanitizer runtime, which is not
//! what CI runs. This driver is the stable-toolchain equivalent: it seeds from
//! every *valid* frame the protocol can produce, then mutates each seed many
//! ways (bit flips, truncation, length-field lies, byte splices) *without*
//! touching the decoder API it is testing. The panic/over-allocation guards
//! are the whole point — a crash here is a remote DoS the unit tests missed.
//!
//! It is deterministic (fixed seed), so it never flakes and every CI run
//! replays the same corpus. Failure messages name the seed index and mutation
//! so a hit can be reproduced by hand.

use tetherly_core::frame::{
    ClipSet, FileDecision, FileDone, FileMeta, FileOffer, InnerFrame, NotifyDismiss, NotifyPush,
    NotifyReply,
};
use tetherly_core::screen::{
    ControlMsg, PixelFormat, ScreenAssembler, ScreenFrame, ScreenRect, SCREEN_PAYLOAD_MAX,
};
use tetherly_core::{
    CapsUpdate, DataAssembler, DeviceId, Hello, InputEvent, NotificationEvent, DEFAULT_MIN_SCORE,
};

/// Small xorshift so the corpus is reproducible with no dependency and no OS
/// entropy (a fixed seed means an identical run everywhere).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next() % n as u64) as usize
        }
    }
}

/// The full set of protocol-valid byte frames a decoder may be handed. Every
/// seed is something a well-behaved peer could actually send.
fn corpus() -> Vec<Vec<u8>> {
    let mut c: Vec<Vec<u8>> = Vec::new();

    // Inner frames: several types, some with MUST_UNDERSTAND.
    for ty in [0x0001u16, 0x0101, 0x0201, 0x0501, 0x0502, 0xFFFF] {
        for flags in [0u16, 0x0001] {
            c.push(
                InnerFrame {
                    ty,
                    flags,
                    msg_id: 0x1122_3344_5566_7788,
                    payload: b"hello-inner".to_vec(),
                }
                .encode()
                .unwrap(),
            );
        }
    }

    // Control JSON frames.
    c.push(
        NotifyPush {
            uid: "u".into(),
            app_id: "com.apple.MobileSMS".into(),
            app_name: "信息".into(),
            title: "验证码".into(),
            body: "你的验证码是 123456".into(),
            ts: 42,
            actions: vec!["copy".into()],
        }
        .to_frame()
        .unwrap()
        .payload,
    );
    c.push(
        NotifyDismiss {
            uid: "u".into(),
            app_id: "sms".into(),
        }
        .to_frame()
        .unwrap()
        .payload,
    );
    c.push(
        NotifyReply {
            uid: "u".into(),
            app_id: "sms".into(),
            text: "好的".into(),
        }
        .to_frame()
        .unwrap()
        .payload,
    );
    c.push(
        ClipSet::text(7, "clipboard-дані")
            .unwrap()
            .to_frame()
            .unwrap()
            .payload,
    );
    c.push(
        FileOffer {
            transfer_id: "tr_abc".into(),
            files: vec![FileMeta {
                name: "报告.pdf".into(),
                size: 12345,
                sha256: "ab".repeat(32),
            }],
        }
        .to_frame()
        .unwrap()
        .payload,
    );
    c.push(
        FileDecision {
            transfer_id: "tr_abc".into(),
            resume_from: None,
        }
        .accept_frame()
        .unwrap()
        .payload,
    );
    c.push(
        FileDecision {
            transfer_id: "tr_abc".into(),
            resume_from: Some(65536),
        }
        .accept_frame()
        .unwrap()
        .payload,
    );
    c.push(
        FileDone {
            transfer_id: "tr_abc".into(),
            sha256: "cd".repeat(32),
        }
        .to_frame()
        .unwrap()
        .payload,
    );
    c.push(
        CapsUpdate {
            caps: vec!["notify".into(), "screen".into(), "screenport=45720".into()],
        }
        .to_frame()
        .unwrap()
        .payload,
    );

    // Input events, one per kind.
    for ev in [
        InputEvent::enter(1),
        InputEvent::leave(2),
        InputEvent::move_to(3, 500, 500),
        InputEvent::button(4, 1, true),
        InputEvent::wheel(5, -3, 4),
        InputEvent::key(6, 0x1E, true, 0x02),
        InputEvent::clip_hint(7),
    ] {
        c.push(ev.to_frame().unwrap().payload);
    }

    // Screen frames: a whole raw frame and a multi-fragment frame body.
    let rect = ScreenRect::new(0, 0, 8, 8).unwrap();
    let frame = ScreenFrame::raw(1, rect, PixelFormat::Bgra8, vec![0xAB; 8 * 8 * 4]).unwrap();
    c.push(frame.encode_payload().unwrap());
    for f in frame.to_frames().unwrap() {
        c.push(f.payload);
    }
    let big = ScreenFrame::raw(
        2,
        ScreenRect::new(0, 0, 200, 200).unwrap(),
        PixelFormat::Rgba8,
        vec![0x7F; 200 * 200 * 4],
    )
    .unwrap();
    for f in big.to_frames().unwrap() {
        c.push(f.payload);
    }

    // Control messages.
    for msg in [
        ControlMsg::Start,
        ControlMsg::Stop,
        ControlMsg::Ack { seq: 9 },
    ] {
        c.push(msg.encode_payload());
    }

    // ANCS notification-source events (8 bytes) and data-source tuples.
    c.push(vec![0, 0, 4, 1, 0x2A, 0, 0, 0]);
    c.push(vec![1, 0x20, 0, 0, 0x2A, 0, 0, 0]);
    // GetNotificationAttributes response: cmd | uid(4) | attr | len(2) | value
    c.push(vec![
        0, 0x2A, 0, 0, 0, 3, 5, 0, b'h', b'e', b'l', b'l', b'o',
    ]);
    // GetAppAttributes response: cmd | app_id NUL | attr | len(2) | value
    c.push(vec![
        1, b'c', b'o', b'm', b'.', b'x', 0, 0, 2, 0, b'h', b'i',
    ]);

    // Hello (canonical JSON).
    let id_pk = [3u8; 32];
    let hello = Hello {
        proto: 1,
        device_id: DeviceId::from_id_pk(&id_pk),
        id_pk: tetherly_core::hello::encode_b64_32(&id_pk),
        n_pk: tetherly_core::hello::encode_b64_32(&[4u8; 32]),
        caps: vec!["notify".into(), "clip".into(), "file".into()],
        name: "Mango-PC".into(),
        platform: "windows".into(),
    };
    c.push(hello.to_canonical_bytes().unwrap());

    c
}

/// One mutation of a seed. Never constructs the decoder's own state — it only
/// rewrites the byte string, exactly like a hostile peer would.
fn mutate(rng: &mut Rng, seed: &[u8]) -> Vec<u8> {
    let mut b = seed.to_vec();
    if b.is_empty() {
        return vec![(rng.next() & 0xFF) as u8];
    }
    match rng.below(6) {
        // Flip one bit.
        0 => {
            let i = rng.below(b.len());
            b[i] ^= 1 << rng.below(8);
        }
        // Set a byte to an extreme value (length fields love 0xFF).
        1 => {
            let i = rng.below(b.len());
            b[i] = if rng.below(2) == 0 { 0xFF } else { 0x00 };
        }
        // Truncate.
        2 => {
            let n = rng.below(b.len());
            b.truncate(n);
        }
        // Extend with junk.
        3 => {
            let n = 1 + rng.below(64);
            for _ in 0..n {
                b.push((rng.next() & 0xFF) as u8);
            }
        }
        // Overwrite 1..=8 bytes with random values (hits multi-byte headers).
        4 => {
            let start = rng.below(b.len());
            let room = b.len() - start;
            let len = 1 + rng.below(8.min(room));
            for x in b[start..start + len].iter_mut() {
                *x = (rng.next() & 0xFF) as u8;
            }
        }
        // Splice a slice of the seed elsewhere (reorders/duplicates structure).
        _ => {
            let a = rng.below(b.len());
            let c = rng.below(b.len());
            let len = rng.below(b.len() - a);
            let chunk = b[a..a + len].to_vec();
            for (k, byte) in chunk.into_iter().enumerate() {
                let at = (c + k) % b.len();
                b[at] = byte;
            }
        }
    }
    b
}

/// Run one mutated payload through every byte-level decoder. None of these may
/// panic, and the stateful ones may not grow without bound.
fn drive(buf: &[u8]) {
    let _ = InnerFrame::decode(buf);
    let _ = ScreenFrame::decode_payload(buf);
    let _ = ControlMsg::decode_payload(buf);
    let _ = NotificationEvent::parse(buf);
    let _ = Hello::from_bytes(buf);

    let _ = NotifyPush::from_payload(buf);
    let _ = NotifyDismiss::from_payload(buf);
    let _ = NotifyReply::from_payload(buf);
    let _ = ClipSet::from_payload(buf);
    let _ = FileOffer::from_payload(buf);
    let _ = FileDecision::from_payload(buf);
    let _ = FileDone::from_payload(buf);
    let _ = CapsUpdate::from_payload(buf);

    // Decode as an inner frame, then push into the stateful consumers the same
    // way a live session does.
    if let Ok(f) = InnerFrame::decode(buf) {
        let _ = InputEvent::from_frame(&f);
        let mut asm = ScreenAssembler::new();
        let _ = asm.push(&f);
    }

    let mut ancs = DataAssembler::default();
    if ancs.push(buf).is_err() {
        ancs.reset();
    }
    while let Ok(Some(_)) = ancs.next_item() {}
    assert!(
        ancs.pending() <= 8 * 1024,
        "ANCS assembler exceeded its budget"
    );

    let _ = tetherly_core::sanitize_file_name(&String::from_utf8_lossy(buf));
    let _ = tetherly_core::pin::parse_pin(&String::from_utf8_lossy(buf));
    let _ = tetherly_core::device_id::DeviceId::parse(&String::from_utf8_lossy(buf));
    // OTP extraction runs on untrusted notification text in production.
    let _ = tetherly_core::otp::extract_otp_with_min_score(
        &String::from_utf8_lossy(buf),
        DEFAULT_MIN_SCORE,
    );
}

#[test]
fn mutating_the_valid_corpus_never_panics_a_decoder() {
    let seeds = corpus();
    assert!(seeds.len() >= 20, "corpus should cover every decoder");
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);

    const MUTATIONS_PER_SEED: usize = 512;
    for (i, seed) in seeds.iter().enumerate() {
        // The seed itself must be handled cleanly (it is a valid frame).
        drive(seed);
        for m in 0..MUTATIONS_PER_SEED {
            let mut bytes = mutate(&mut rng, seed);
            // Occasionally mutate a second time so mutations stack.
            if m % 4 == 0 {
                bytes = mutate(&mut rng, &bytes);
            }
            let label = format!("seed {i} mutation {m}");
            let result = std::panic::catch_unwind(|| drive(&bytes));
            assert!(result.is_ok(), "decoder panicked on {label}: {bytes:02x?}");
        }
    }
}

/// Rare-but-real shapes: empty input, a lone magic prefix, and a header that
/// claims a huge body. These are the crash-class inputs a fuzzer finds first.
#[test]
fn adversarial_edge_inputs_are_rejected_cleanly() {
    let cases: Vec<Vec<u8>> = vec![
        vec![],
        b"TMV1".to_vec(),
        b"TMF1".to_vec(),
        b"TIN1".to_vec(),
        b"TFL2".to_vec(),
        b"TFL1".to_vec(),
        {
            // A screen header declaring a 65535x65535 raw frame.
            let mut v = b"TMV1".to_vec();
            v.push(1);
            v.extend_from_slice(&0u64.to_be_bytes());
            v.extend_from_slice(&0i32.to_be_bytes());
            v.extend_from_slice(&0i32.to_be_bytes());
            v.extend_from_slice(&65535u16.to_be_bytes());
            v.extend_from_slice(&65535u16.to_be_bytes());
            v.push(0);
            v.push(0);
            v.extend_from_slice(&0u32.to_be_bytes());
            v
        },
        {
            // A fragment header whose count/idx/total disagree.
            let mut v = b"TMF1".to_vec();
            v.extend_from_slice(&16u32.to_be_bytes()); // total
            v.extend_from_slice(&7u32.to_be_bytes()); // idx past the end
            v.extend_from_slice(&1u32.to_be_bytes()); // count
            v.extend_from_slice(&5u64.to_be_bytes()); // seq
            v.extend_from_slice(&[0u8; 8]);
            v
        },
    ];
    for c in cases {
        let r = std::panic::catch_unwind(|| drive(&c));
        assert!(r.is_ok(), "decoder panicked on {c:02x?}");
    }

    // The giant-rect header must be refused, not allocated.
    let mut giant = b"TMV1".to_vec();
    giant.push(1);
    giant.extend_from_slice(&0u64.to_be_bytes());
    giant.extend_from_slice(&0i32.to_be_bytes());
    giant.extend_from_slice(&0i32.to_be_bytes());
    giant.extend_from_slice(&65535u16.to_be_bytes());
    giant.extend_from_slice(&65535u16.to_be_bytes());
    giant.push(0);
    giant.push(0);
    giant.extend_from_slice(&0u32.to_be_bytes());
    let rect = ScreenRect::new(0, 0, 65535, 65535).unwrap();
    assert!(rect.pixel_bytes(PixelFormat::Bgra8) > SCREEN_PAYLOAD_MAX);
    assert!(ScreenFrame::decode_payload(&giant).is_err());
}
