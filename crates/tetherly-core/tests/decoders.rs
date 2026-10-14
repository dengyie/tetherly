// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Property tests for the untrusted decoders (spec §13 `proptest` layer).
//!
//! Every parser a *peer* can drive is fed arbitrary and structured input. Two
//! invariants per decoder:
//!
//!   1. it never panics on arbitrary bytes — an overflow, slice, or unbounded
//!      allocation surfaces here instead of in production against a hostile
//!      peer;
//!   2. when it accepts, the value re-encodes and re-decodes to itself.
//!
//! This guards exactly the class of bug v1.15 had to fix by hand (peer
//! `ReplayGuard` overflow, peer `ScreenAssembler` underflow). OS-free: it uses
//! only the `tetherly-core` public surface, so it runs on every CI host.

use proptest::prelude::*;
use tetherly_core::frame::{
    CapsUpdate, ClipSet, FileDecision, FileDone, FileMeta, FileOffer, InnerFrame, NotifyDismiss,
    NotifyPush, NotifyReply,
};
use tetherly_core::screen::{
    ControlMsg, PixelFormat, ScreenAssembler, ScreenFrame, ScreenRect, SCREEN_PAYLOAD_MAX,
};
use tetherly_core::{DataAssembler, DeviceId, Hello, InputEvent, NotificationEvent};

// ---------------------------------------------------------------- strategies

/// A short arbitrary string (bounded so a JSON frame stays under the 64 KiB
/// inner payload cap). `char` covers the full Unicode range, so multi-byte
/// boundaries are exercised too.
fn text(max_chars: usize) -> impl Strategy<Value = String> {
    prop::collection::vec(any::<char>(), 0..max_chars).prop_map(|v| v.into_iter().collect())
}

fn bytes(max: usize) -> impl Strategy<Value = Vec<u8>> {
    prop::collection::vec(any::<u8>(), 0..max)
}

fn push_strategy() -> impl Strategy<Value = NotifyPush> {
    (
        text(24),
        text(24),
        text(24),
        text(32),
        text(32),
        any::<u64>(),
        prop::collection::vec(text(20), 0..6),
    )
        .prop_map(
            |(uid, app_id, app_name, title, body, ts, actions)| NotifyPush {
                uid,
                app_id,
                app_name,
                title,
                body,
                ts,
                actions,
            },
        )
}

fn file_meta() -> impl Strategy<Value = FileMeta> {
    (text(24), any::<u64>(), text(32)).prop_map(|(name, size, hash)| FileMeta {
        name,
        size,
        // Keep a valid (64 hex) or deliberately-invalid hash: the *frame* codec
        // does not validate, only `FileHub` does, so any string must round-trip.
        sha256: hash,
    })
}

/// An already-valid input event. `Move` coordinates stay inside `0..=1000`
/// (`SCREEN_UNITS`) so the decoder's range check accepts them.
fn input_event() -> impl Strategy<Value = InputEvent> {
    (
        any::<u64>(),
        0u8..7,
        (0..=1000i32, 0..=1000i32, any::<u8>(), any::<bool>()),
        (any::<i16>(), any::<i16>(), any::<u16>(), any::<u8>()),
    )
        .prop_map(
            |(seq, kind, (x, y, mask, down), (dx, dy, code, mods))| match kind {
                0 => InputEvent::enter(seq),
                1 => InputEvent::leave(seq),
                2 => InputEvent::move_to(seq, x, y),
                3 => InputEvent::button(seq, mask, down),
                4 => InputEvent::wheel(seq, dx, dy),
                5 => InputEvent::key(seq, code, down, mods),
                _ => InputEvent::clip_hint(seq),
            },
        )
}

fn raw_screen_frame() -> impl Strategy<Value = ScreenFrame> {
    (any::<u64>(), 1u16..=16, 1u16..=16, any::<bool>()).prop_map(|(seq, w, h, rgba)| {
        let format = if rgba {
            PixelFormat::Rgba8
        } else {
            PixelFormat::Bgra8
        };
        let rect = ScreenRect::new(0, 0, w, h).expect("non-zero rect");
        let n = w as usize * h as usize * format.bpp();
        ScreenFrame::raw(seq, rect, format, vec![0u8; n]).expect("exact raw body")
    })
}

fn control_msg() -> impl Strategy<Value = ControlMsg> {
    prop_oneof![
        Just(ControlMsg::Start),
        Just(ControlMsg::Stop),
        any::<u64>().prop_map(|seq| ControlMsg::Ack { seq }),
    ]
}

// -------------------------------------------- never-panic on arbitrary bytes

proptest! {
    #[test]
    fn inner_frame_decode_never_panics(b in bytes(512)) {
        let _ = InnerFrame::decode(&b);
    }

    #[test]
    fn screen_frame_decode_never_panics(b in bytes(2048)) {
        let _ = ScreenFrame::decode_payload(&b);
    }

    #[test]
    fn control_msg_decode_never_panics(b in bytes(64)) {
        let _ = ControlMsg::decode_payload(&b);
    }

    #[test]
    fn screen_assembler_never_panics(ty in any::<u16>(), payload in bytes(2048)) {
        let mut asm = ScreenAssembler::new();
        let f = InnerFrame { ty, flags: 0, msg_id: 0, payload };
        let _ = asm.push(&f);
    }

    #[test]
    fn input_event_decode_never_panics(ty in any::<u16>(), payload in bytes(256)) {
        let f = InnerFrame { ty, flags: 0, msg_id: 0, payload };
        let _ = InputEvent::from_frame(&f);
    }

    #[test]
    fn notification_event_parse_never_panics(b in bytes(64)) {
        let _ = NotificationEvent::parse(&b);
    }

    #[test]
    fn hello_from_bytes_never_panics(b in bytes(1024)) {
        let _ = Hello::from_bytes(&b);
    }

    #[test]
    fn device_id_parse_never_panics(s in text(64)) {
        let _ = DeviceId::parse(&s);
    }

    #[test]
    fn parse_pin_never_panics(s in text(64)) {
        let _ = tetherly_core::pin::parse_pin(&s);
    }

    /// The sanitized name must always be a pure basename: nothing that could
    /// climb out of the directory it is written into. A `..` *inside* a name
    /// (`a..b`) is fine — only a path separator, or the exact `.`/`..` names,
    /// can traverse.
    #[test]
    fn sanitize_file_name_stays_a_basename(s in text(128)) {
        if let Ok(name) = tetherly_core::sanitize_file_name(&s) {
            prop_assert!(!name.is_empty());
            prop_assert_ne!(name.as_str(), ".");
            prop_assert_ne!(name.as_str(), "..");
            prop_assert!(!name.contains('/'));
            prop_assert!(!name.contains('\0'));
        }
    }

    #[test]
    fn json_controls_decode_never_panic(b in bytes(1024)) {
        let _ = NotifyPush::from_payload(&b);
        let _ = NotifyDismiss::from_payload(&b);
        let _ = NotifyReply::from_payload(&b);
        let _ = ClipSet::from_payload(&b);
        let _ = FileOffer::from_payload(&b);
        let _ = FileDecision::from_payload(&b);
        let _ = FileDone::from_payload(&b);
        let _ = CapsUpdate::from_payload(&b);
    }

    /// Feeding arbitrary chunks, the ANCS assembler must never panic and never
    /// buffer beyond its documented budget.
    #[test]
    fn data_assembler_is_bounded(chunks in prop::collection::vec(bytes(400), 0..8)) {
        let mut asm = DataAssembler::default();
        for c in &chunks {
            if asm.push(c).is_err() {
                asm.reset();
            }
            while let Ok(Some(_)) = asm.next_item() {}
            prop_assert!(asm.pending() <= 8 * 1024);
        }
    }
}

// ------------------------------------------------- structured round-trips

proptest! {
    #[test]
    fn notify_push_roundtrips(x in push_strategy()) {
        let a = x.truncate();
        let payload = a.to_frame().unwrap().payload;
        let b = NotifyPush::from_payload(&payload).unwrap();
        prop_assert_eq!(a, b);
    }

    #[test]
    fn notify_dismiss_roundtrips(uid in text(24), app_id in text(24)) {
        let a = NotifyDismiss { uid, app_id };
        let payload = a.to_frame().unwrap().payload;
        prop_assert_eq!(NotifyDismiss::from_payload(&payload).unwrap(), a);
    }

    #[test]
    fn notify_reply_roundtrips(uid in text(24), app_id in text(24), textv in text(48)) {
        let a = NotifyReply { uid, app_id, text: textv }.truncate();
        let payload = a.to_frame().unwrap().payload;
        prop_assert_eq!(NotifyReply::from_payload(&payload).unwrap(), a);
    }

    #[test]
    fn clip_set_roundtrips(
        mime in text(16),
        textv in prop::option::of(text(48)),
        blob in prop::option::of(text(24)),
        clip_seq in any::<u64>(),
        hash in text(64),
    ) {
        let a = ClipSet { mime, text: textv, blob_ref: blob, clip_seq, hash };
        let payload = a.to_frame().unwrap().payload;
        prop_assert_eq!(ClipSet::from_payload(&payload).unwrap(), a);
    }

    #[test]
    fn file_offer_roundtrips(
        transfer_id in text(24),
        files in prop::collection::vec(file_meta(), 1..4),
    ) {
        let a = FileOffer { transfer_id, files };
        let payload = a.to_frame().unwrap().payload;
        prop_assert_eq!(FileOffer::from_payload(&payload).unwrap(), a);
    }

    #[test]
    fn file_decision_roundtrips(transfer_id in text(24), resume in prop::option::of(any::<u64>())) {
        let a = FileDecision { transfer_id, resume_from: resume };
        let payload = a.accept_frame().unwrap().payload;
        prop_assert_eq!(FileDecision::from_payload(&payload).unwrap(), a);
    }

    #[test]
    fn file_done_roundtrips(transfer_id in text(24), sha in text(64)) {
        let a = FileDone { transfer_id, sha256: sha };
        let payload = a.to_frame().unwrap().payload;
        prop_assert_eq!(FileDone::from_payload(&payload).unwrap(), a);
    }

    #[test]
    fn caps_update_roundtrips(caps in prop::collection::vec(text(16), 0..6)) {
        let a = CapsUpdate { caps };
        let payload = a.to_frame().unwrap().payload;
        prop_assert_eq!(CapsUpdate::from_payload(&payload).unwrap(), a);
    }

    #[test]
    fn input_event_roundtrips(ev in input_event()) {
        let f = ev.to_frame().unwrap();
        prop_assert_eq!(InputEvent::from_frame(&f).unwrap(), ev);
    }

    #[test]
    fn screen_frame_roundtrips(frame in raw_screen_frame()) {
        let payload = frame.encode_payload().unwrap();
        prop_assert_eq!(ScreenFrame::decode_payload(&payload).unwrap(), frame);
    }

    #[test]
    fn control_msg_roundtrips(msg in control_msg()) {
        let payload = msg.encode_payload();
        prop_assert_eq!(ControlMsg::decode_payload(&payload).unwrap(), msg);
    }

    #[test]
    fn hello_roundtrips(
        caps in prop::collection::vec(text(12), 0..5),
        name in text(24),
        platform in text(16),
    ) {
        let id_pk = [7u8; 32];
        let n_pk = [9u8; 32];
        let h = Hello {
            proto: 1,
            device_id: DeviceId::from_id_pk(&id_pk),
            id_pk: tetherly_core::hello::encode_b64_32(&id_pk),
            n_pk: tetherly_core::hello::encode_b64_32(&n_pk),
            caps,
            name,
            platform,
        };
        let bytes = h.to_canonical_bytes().unwrap();
        prop_assert_eq!(Hello::from_bytes(&bytes).unwrap(), h);
    }
}

// ------------------------------------------------ fragmentation reassembly

/// A frame large enough to need more than one `TMF1` fragment.
fn multi_fragment_frame() -> impl Strategy<Value = ScreenFrame> {
    (any::<u64>(), Just(160u16), 110u16..=250).prop_map(|(seq, w, h)| {
        let rect = ScreenRect::new(0, 0, w, h).expect("non-zero rect");
        let n = w as usize * h as usize * 4;
        ScreenFrame::raw(seq, rect, PixelFormat::Bgra8, vec![0u8; n]).expect("exact raw body")
    })
}

proptest! {
    /// Reassembly is order-independent (fragments carry an explicit index), so
    /// any permutation must still yield the original frame.
    #[test]
    fn assembler_reassembles_in_any_order(frame in multi_fragment_frame()) {
        let frames = frame.to_frames().unwrap();
        prop_assume!(frames.len() >= 2);

        let mut asm = ScreenAssembler::new();
        let mut out = None;
        for f in frames.iter().rev() {
            if let Some(got) = asm.push(f).unwrap() {
                out = Some(got);
            }
        }
        prop_assert_eq!(out, Some(frame));
    }

    /// A missing fragment must never complete: the assembler returns `None`
    /// until every byte of the stream is present.
    #[test]
    fn assembler_refuses_an_incomplete_stream(frame in multi_fragment_frame()) {
        let frames = frame.to_frames().unwrap();
        prop_assume!(frames.len() >= 2);

        let mut asm = ScreenAssembler::new();
        for f in &frames[..frames.len() - 1] {
            prop_assert!(asm.push(f).unwrap().is_none());
        }
    }

    /// A whole `TMV1` frame over the payload cap is refused, never allocated.
    #[test]
    fn screen_decode_respects_the_payload_cap(w in 1u16..=65535, h in 1u16..=65535) {
        // Build a header for a rect whose pixel bytes overflow the cap and give
        // it no body; the decoder must reject on the rect, before copying bytes.
        let rect = ScreenRect::new(0, 0, w, h).unwrap();
        prop_assume!(rect.pixel_bytes(PixelFormat::Bgra8) > SCREEN_PAYLOAD_MAX);

        let mut payload = Vec::with_capacity(31);
        payload.extend_from_slice(b"TMV1");
        payload.push(1); // proto
        payload.extend_from_slice(&0u64.to_be_bytes()); // seq
        payload.extend_from_slice(&0i32.to_be_bytes()); // x
        payload.extend_from_slice(&0i32.to_be_bytes()); // y
        payload.extend_from_slice(&w.to_be_bytes());
        payload.extend_from_slice(&h.to_be_bytes());
        payload.push(0); // BGRA
        payload.push(0); // raw
        payload.extend_from_slice(&0u32.to_be_bytes()); // stride
        prop_assert!(ScreenFrame::decode_payload(&payload).is_err());
    }
}
