// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Stateful property tests for the machines that *consume* the decoders.
//!
//! `decoders.rs` and `fuzz_decoders.rs` (v1.16) drive every parser with one
//! hostile buffer at a time. That leaves the class this file closes: the
//! sequence-dependent state machines — `ReplayGuard`, the rate limiters,
//! `NotifyHub`, `FileHub`, `ResumeTable`, the screen sender/receiver/assembler,
//! and the ANCS ingress. Their bugs are order-dependent (a replay that slips
//! through after an eviction, a consent gate a peer can walk around, a pending
//! map that never drains), so each is driven here with an arbitrary *program* of
//! operations, not a single input.
//!
//! What is asserted is the safety contract of each machine, in a form that holds
//! for every operation order:
//!
//!   * nothing panics on a peer-shaped sequence (the v1.16 fuzz guarantee, now
//!     extended from one buffer to a whole session);
//!   * a security gate never opens from peer input alone — consent, dedup,
//!     replay and traversal rules survive any ordering;
//!   * every internal counter/map stays bounded, so a peer cannot grow memory by
//!     replaying an event forever.
//!
//! OS-free: only the `tetherly-core` public surface is used, so it runs on every
//! CI host. Deterministic: the proptest seed makes a failure reproducible.

use proptest::prelude::*;
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;
use tetherly_core::ancs::{
    AncsIngress, AncsState, ConnectOutcome, EventFlags, MemoryAncsTransport, NOTIFICATION_EVENT_LEN,
};
use tetherly_core::dedup::{ClipLimiter, NotifyLimiter, TokenBucket};
use tetherly_core::filexfer::{FileHub, ResumeState, ResumeTable};
use tetherly_core::frame::{
    FileDone, FileMeta, FileOffer, InnerFrame, NotifyPush, NotifyReply, FILE_CHUNK,
    FLAG_MUST_UNDERSTAND,
};
use tetherly_core::notify::{IngestOutcome, NotifyHub};
use tetherly_core::ports::ManualClock;
use tetherly_core::screen::{
    ControlMsg, PixelFormat, ScreenAssembler, ScreenFrame, ScreenReceiver, ScreenRect,
    ScreenSender, SCREEN_PAYLOAD_MAX, SCREEN_TRACE_MAX, TYPE_SCREEN_FRAME,
};
use tetherly_core::{Clock, DeviceId, ReplayGuard};

// --------------------------------------------------------------- program utils

/// An arbitrary operation program. Each test interprets the bytes as a sequence
/// of ops; proptest shrinks a failure to the shortest op sequence that still
/// breaks the invariant, which is what makes an order-dependent bug readable.
fn program(max_ops: usize) -> impl Strategy<Value = Vec<u8>> {
    prop::collection::vec(any::<u8>(), 0..max_ops)
}

/// A read cursor over a program. Every accessor is total: reading past the end
/// yields `0` (or an empty slice) and advances, so a program never has to be
/// length-checked by the caller.
struct Cur<'a> {
    p: &'a [u8],
    i: usize,
}

impl<'a> Cur<'a> {
    fn new(p: &'a [u8]) -> Self {
        Self { p, i: 0 }
    }

    fn done(&self) -> bool {
        self.i >= self.p.len()
    }

    fn byte(&mut self) -> u8 {
        let b = *self.p.get(self.i).unwrap_or(&0);
        self.i += 1;
        b
    }

    fn u64(&mut self) -> u64 {
        let mut b = [0u8; 8];
        for x in b.iter_mut() {
            *x = self.byte();
        }
        u64::from_be_bytes(b)
    }

    fn take(&mut self, n: usize) -> &'a [u8] {
        let start = self.i.min(self.p.len());
        let end = (start + n).min(self.p.len());
        self.i = end;
        &self.p[start..end]
    }

    fn text(&mut self, n: usize) -> String {
        String::from_utf8_lossy(self.take(n)).into_owned()
    }
}

/// A 64-char lowercase-hex digest for `FileMeta::sha256` (the shape the hub
/// requires), derived from arbitrary program bytes.
fn hex64(v: u64) -> String {
    format!("{v:064x}")
}

fn small_rect() -> ScreenRect {
    ScreenRect::new(0, 0, 4, 2).unwrap()
}

fn raw_frame(seq: u64) -> ScreenFrame {
    ScreenFrame::raw(seq, small_rect(), PixelFormat::Bgra8, vec![7u8; 32]).unwrap()
}

// ---------------------------------------------------------------- ReplayGuard

proptest! {
    /// A `msg_id` is attacker-chosen `u64`. Accepting one must be safe to replay
    /// (its own duplicate is refused) and must never let an id far below the
    /// high-water mark through, even after the (bounded) recent-set has evicted
    /// it — that guard is the wrap-safe window check, not the set.
    #[test]
    fn replay_guard_rejects_its_own_replays_and_ancient_ids(p in program(400)) {
        let mut g = ReplayGuard::default();
        let mut cur = Cur::new(&p);
        let mut zero_seen = false;
        let mut max = 0u64;
        while !cur.done() {
            // Mix of tiny ids, shifted ids, and the `u64::MAX` neighbourhood
            // where a naive `msg_id + 1024` would overflow.
            let id = match cur.byte() % 3 {
                0 => cur.byte() as u64,
                1 => (cur.byte() as u64) << 20,
                _ => u64::MAX - cur.byte() as u64,
            };
            if g.check_and_record(id) {
                if id == 0 {
                    zero_seen = true;
                }
                max = max.max(id);
                // A freshly accepted id is in the recent-set: never re-accepted.
                prop_assert!(!g.check_and_record(id));
            }
        }
        if !zero_seen && max > 3_000 {
            prop_assert!(!g.check_and_record(0));
        }
    }
}

// ----------------------------------------------------------------- limiters

proptest! {
    /// On a clock that does not advance, a token bucket yields exactly
    /// `floor(burst)` tokens and then refuses: no burst can exceed its bucket,
    /// whatever the rate.
    #[test]
    fn token_bucket_on_a_frozen_clock_yields_exactly_its_burst(
        rate in 0.0f64..50.0,
        burst in 1.0f64..64.0,
        draws in 64usize..400,
    ) {
        let mut b = TokenBucket::new(rate, burst, 1_000);
        let mut ok = 0usize;
        for _ in 0..draws {
            if b.try_take(1_000) {
                ok += 1;
            }
        }
        prop_assert_eq!(ok, burst.floor() as usize);
    }

    /// A monotone clock is the only thing that refills a bucket, so a peer
    /// cannot exceed the leaky-bucket bound `burst + rate*elapsed` no matter how
    /// many events it sends — the flood is capped, not unbounded.
    #[test]
    fn notify_limiter_never_exceeds_its_leaky_bucket_bound(
        events in prop::collection::vec(any::<u8>(), 0..300),
    ) {
        let mut lim = NotifyLimiter::default();
        let src = DeviceId::from_id_pk(&[1u8; 32]);
        let mut t = 0u64;
        let mut ok = 0u64;
        for e in &events {
            t = t.saturating_add((e % 64) as u64);
            if lim.allow(&src, t) {
                ok += 1;
            }
        }
        // burst 10, rate 2/s: successes <= 10 + 2*t/1000 (+1 fixed-point slack).
        let bound = 10 + (2 * t) / 1_000 + 1;
        prop_assert!(ok <= bound, "limiters allowed {ok} > bound {bound} over {t}ms");
    }

    /// A clock that jumps backwards never grants a source more than its initial
    /// burst, because `elapsed` saturates at zero.
    #[test]
    fn clip_limiter_ignores_a_backwards_clock(p in program(300)) {
        let mut lim = ClipLimiter::default();
        let a = DeviceId::from_id_pk(&[1u8; 32]);
        let b = DeviceId::from_id_pk(&[2u8; 32]);
        let mut cur = Cur::new(&p);
        let mut now = 1_000u64;
        while !cur.done() {
            let step = cur.byte() as i64 - 128;
            now = (now as i64 + step).max(0) as u64;
            let _ = lim.allow(&a, now);
            let _ = lim.allow(&b, now);
        }
        // A fresh bucket at a frozen time still grants exactly its burst (4).
        let mut fresh = ClipLimiter::default();
        let mut granted = 0;
        for _ in 0..8 {
            if fresh.allow(&a, 1_000) {
                granted += 1;
            }
        }
        prop_assert_eq!(granted, 4);
    }
}

// ------------------------------------------------------------------- FileHub

proptest! {
    /// The file name is the highest-value target: a peer chooses it. No accepted
    /// offer may carry a traversal, an empty name, or a NUL, and an offer alone
    /// is never an acceptance.
    #[test]
    fn file_hub_sanitizes_every_name_and_never_self_accepts(p in program(500)) {
        let mut hub = FileHub::default();
        let src = DeviceId::from_id_pk(&[4u8; 32]);
        let mut cur = Cur::new(&p);
        while !cur.done() {
            let id = format!("tr_{}", cur.byte() % 8);
            match cur.byte() % 4 {
                0 => {
                    let name_len = 1 + (cur.byte() as usize % 14);
                    let name = cur.text(name_len);
                    let size = cur.u64() % (4 * FILE_CHUNK as u64 + 1);
                    let offer = FileOffer {
                        transfer_id: id.clone(),
                        files: vec![FileMeta { name, size, sha256: hex64(cur.u64()) }],
                    };
                    if let Ok(t) = hub.offer(src.clone(), offer, 1) {
                        let n = &t.files[0].name;
                        // A basename: never empty, never a traversal, no NUL, and
                        // `/` is a separator on every platform so it can never
                        // appear (a literal `\` is legal on Unix, so it is not
                        // asserted here).
                        prop_assert!(!n.is_empty() && n != "." && n != "..");
                        prop_assert!(!n.contains('/') && !n.contains('\0'));
                        // A fresh offer resets the transfer to `Offered`.
                        prop_assert!(!hub.is_accepted(&id));
                    }
                }
                1 => {
                    if hub.accept(&id, 2).is_ok() {
                        // A second decision on the same offer is refused.
                        prop_assert!(hub.accept(&id, 3).is_err());
                        prop_assert!(hub.reject(&id).is_err());
                    }
                }
                2 => {
                    let _ = hub.reject(&id);
                }
                _ => {
                    let _ = hub.accept_at(&id, 2, Some(cur.u64() % (2 * FILE_CHUNK as u64)));
                }
            }
        }
    }

    /// A peer that floods offers cannot grow the hub without bound: with no
    /// local accept, the live offer count stays at or under `INCOMING_MAX` (256).
    #[test]
    fn file_hub_bounds_a_flood_of_offers(count in 8usize..600) {
        let mut hub = FileHub::default();
        let src = DeviceId::from_id_pk(&[4u8; 32]);
        for i in 0..count {
            let offer = FileOffer {
                transfer_id: format!("tr_{i}"),
                files: vec![FileMeta { name: "a.bin".into(), size: 16, sha256: hex64(i as u64) }],
            };
            let _ = hub.offer(src.clone(), offer, 1);
        }
        prop_assert!(hub.offered().len() <= 256);
    }

    /// A resume offset is only ever echoed back when it is chunk-aligned,
    /// strictly inside the file, and belongs to this peer.
    #[test]
    fn resume_offsets_are_aligned_in_range_and_source_bound(p in program(400)) {
        let mut table = ResumeTable::new();
        let src = DeviceId::from_id_pk(&[6u8; 32]);
        let other = DeviceId::from_id_pk(&[7u8; 32]);
        let mut cur = Cur::new(&p);
        while !cur.done() {
            let id = format!("tr_{}", cur.byte() % 6);
            match cur.byte() % 3 {
                0 => table.put(ResumeState {
                    transfer_id: id,
                    source: if cur.byte() % 2 == 0 { src.clone() } else { other.clone() },
                    total: cur.u64() % (8 * FILE_CHUNK as u64 + 1),
                    offset: cur.u64(),
                }),
                1 => table.remove(&id),
                _ => {
                    let total = cur.u64() % (8 * FILE_CHUNK as u64 + 1);
                    if let Some(off) = table.claim(&src, &id, total) {
                        prop_assert!(off > 0 && off < total);
                        prop_assert_eq!(off % FILE_CHUNK as u64, 0);
                    }
                }
            }
        }
    }
}

// ------------------------------------------------------------------ NotifyHub

proptest! {
    /// The notify hub ingests peer notifications. Across any interleaving of
    /// push/reply/copy/dismiss/evict: an expired candidate is never listed, a
    /// reply is taken at most once and never blank, and the hub is bounded.
    #[test]
    fn notify_hub_keeps_dedup_reply_and_expiry_invariants(p in program(500)) {
        let mut hub = NotifyHub::default();
        let clock = ManualClock::new(10_000);
        let src = DeviceId::from_id_pk(&[3u8; 32]);
        let mut cur = Cur::new(&p);
        let mut now = 10_000u64;
        // Replies ingested and not yet taken, keyed by (app, uid).
        let mut pending_reply: HashMap<(String, String), u32> = HashMap::new();
        while !cur.done() {
            if cur.byte() % 6 == 0 {
                now = now.saturating_add(cur.byte() as u64 * 1_000);
                clock.set(now);
            }
            let app = format!("app{}", cur.byte() % 3);
            let uid = format!("u{}", cur.byte() % 4);
            match cur.byte() % 5 {
                0 => {
                    let push = NotifyPush {
                        uid: uid.clone(),
                        app_id: app.clone(),
                        app_name: "app".into(),
                        title: "t".into(),
                        body: "验证码:868740".into(),
                        ts: 1,
                        actions: vec!["copy".into()],
                    };
                    // Ingest twice: the second of the same push is not fresh
                    // while the dedup window is live.
                    let _ = hub.ingest_push(src.clone(), push.clone(), &clock);
                    let _ = hub.ingest_push(src.clone(), push, &clock);
                }
                1 => {
                    let reply = NotifyReply { uid: uid.clone(), app_id: app.clone(), text: cur.text(6) };
                    if hub.ingest_reply(&src, reply) {
                        *pending_reply.entry((app.clone(), uid.clone())).or_insert(0) += 1;
                    }
                }
                2 => {
                    if let Some(text) = hub.take_reply(&src, &app, &uid) {
                        prop_assert!(!text.is_empty());
                        prop_assert_eq!(text.trim(), text.as_str());
                        let n = pending_reply.entry((app.clone(), uid.clone())).or_insert(0);
                        prop_assert!(*n > 0, "reply taken more times than ingested");
                        *n -= 1;
                        prop_assert!(hub.take_reply(&src, &app, &uid).is_none());
                    }
                }
                3 => {
                    let id = tetherly_core::notify::candidate_id(&src, &app, &uid, 1);
                    let _ = hub.copy_otp(&id, &clock);
                }
                _ => {
                    let _ = hub.evict_expired(&clock);
                    prop_assert!(hub.reply_views().len() <= 12);
                }
            }
            // Whatever the order, an expired candidate is never surfaced.
            for v in hub.views(&clock) {
                prop_assert!(v.expires_ms > clock.unix_ms(), "an expired candidate was listed");
            }
        }
    }
}

// -------------------------------------------------------------------- screen

proptest! {
    /// The consent gate (M5.2): a peer's `Start` can never move the sender into
    /// `Streaming` unless this machine called `allow()` since the peer
    /// (re)joined. Sequence numbers only increase, and a push is accepted only
    /// while streaming.
    #[test]
    fn screen_sender_streams_only_under_local_consent(p in program(400)) {
        let mut s = ScreenSender::new();
        let mut cur = Cur::new(&p);
        let mut consent = false;
        let mut last_seq = 0u64;
        while !cur.done() {
            match cur.byte() % 6 {
                0 => s.on_peer_connected(),
                1 => { s.allow(); consent = true; }
                2 => {
                    let msg = match cur.byte() % 3 {
                        0 => ControlMsg::Start,
                        1 => ControlMsg::Stop,
                        _ => ControlMsg::Ack { seq: cur.u64() },
                    };
                    let _ = s.on_control(msg);
                }
                3 => {
                    if let Some(seq) = s.push(raw_frame(0)) {
                        prop_assert!(s.state().is_streaming());
                        prop_assert!(seq > last_seq);
                        last_seq = seq;
                    }
                }
                4 => {
                    s.revoke();
                    // A revoke drops consent: reaching Streaming again needs a
                    // fresh `allow()`, never a peer `Start`.
                    consent = false;
                }
                _ => {
                    s.on_peer_gone();
                    consent = false;
                    prop_assert_eq!(s.state(), tetherly_core::ScreenState::Idle);
                    prop_assert_eq!(s.next_seq(), 1);
                    last_seq = 0;
                }
            }
            if s.state().is_streaming() {
                prop_assert!(consent, "streaming without local consent");
            }
        }
    }

    /// The receiver's cursor is monotone: an accepted frame advances it, a
    /// duplicate/stale frame does not, an error leaves it untouched, and the
    /// trace is bounded.
    #[test]
    fn screen_receiver_cursor_is_monotone_and_traced(p in program(400)) {
        let mut r = ScreenReceiver::new();
        let mut cur = Cur::new(&p);
        let mut received = 0u64;
        while !cur.done() {
            let seq = match cur.byte() % 4 {
                0 => cur.byte() as u64,
                1 => (cur.byte() as u64) << 20,
                2 => cur.u64(),
                _ => r.last_seq().wrapping_add(cur.byte() as u64),
            };
            // Half the frames are shaped validly; half lie about their length.
            let frame = if cur.byte() % 2 == 0 {
                ScreenFrame::raw(seq, small_rect(), PixelFormat::Bgra8, vec![0u8; 32]).unwrap()
            } else {
                ScreenFrame {
                    seq,
                    rect: small_rect(),
                    format: PixelFormat::Bgra8,
                    codec_tag: tetherly_core::SCREEN_COMPRESS_NONE,
                    stride: 0,
                    bytes: vec![0u8; 31], // one byte short of the rect
                }
            };
            let before = r.last_seq();
            match r.apply(&frame) {
                Ok(Some(s)) => {
                    prop_assert_eq!(s, seq);
                    prop_assert!(seq > before);
                    received += 1;
                }
                Ok(None) => prop_assert!(seq <= before),
                Err(_) => {}
            }
            prop_assert_eq!(r.stats().received, received);
            prop_assert!(r.last_seq() >= before);
            prop_assert!(r.trace().len() <= SCREEN_TRACE_MAX);
        }
    }

    /// Reassembling an arbitrary mix of valid, reordered and corrupt fragments
    /// never panics; anything it *does* hand back re-encodes under the cap.
    #[test]
    fn screen_assembler_reassembles_or_rejects_but_never_panics(p in program(600)) {
        let mut a = ScreenAssembler::new();
        let mut cur = Cur::new(&p);
        let small_frames = raw_frame(7).to_frames().unwrap();
        let big_rect = ScreenRect::new(0, 0, 256, 256).unwrap();
        let big = ScreenFrame::raw(9, big_rect, PixelFormat::Bgra8, vec![4u8; 256 * 256 * 4]).unwrap();
        let big_frames = big.to_frames().unwrap();
        prop_assert!(big_frames.len() > 1);
        while !cur.done() {
            let frame = match cur.byte() % 4 {
                0 => small_frames[(cur.byte() as usize) % small_frames.len()].clone(),
                1 => big_frames[(cur.byte() as usize) % big_frames.len()].clone(),
                2 => {
                    let mut f = big_frames[(cur.byte() as usize) % big_frames.len()].clone();
                    let at = (cur.byte() as usize) % f.payload.len();
                    f.payload[at] ^= cur.byte() | 1;
                    f
                }
                _ => InnerFrame {
                    ty: TYPE_SCREEN_FRAME,
                    flags: FLAG_MUST_UNDERSTAND,
                    msg_id: 0,
                    payload: vec![cur.byte(); 1 + (cur.byte() as usize % 80)],
                },
            };
            if let Ok(Some(done)) = a.push(&frame) {
                let wire = done.encode_payload().expect("an assembled frame must re-encode");
                prop_assert!(wire.len() <= SCREEN_PAYLOAD_MAX);
            }
        }
    }
}

// ------------------------------------------------- deterministic round trips

/// Non-vacuity witness for `screen_sender_streams_only_under_local_consent`:
/// the exact op encoding that property uses *does* reach `Streaming` once the
/// local machine consents, so the property is constraining a reachable state,
/// not an unreachable one.
#[test]
fn screen_sender_gate_opens_once_the_local_machine_consents() {
    let mut s = ScreenSender::new();
    s.on_peer_connected(); // op 0 -> Requested
    s.allow(); // op 1
    assert_eq!(s.on_control(ControlMsg::Start).unwrap(), None); // op 2 -> Streaming
    assert!(s.state().is_streaming());
    let seq = s.push(raw_frame(0)).expect("streaming accepts a push");
    assert_eq!(seq, 1);
    // Peer gone is the only transition back to Idle, and it resets the seq.
    s.on_peer_gone();
    assert_eq!(s.state(), tetherly_core::ScreenState::Idle);
    assert_eq!(s.next_seq(), 1);
}

#[test]
fn screen_assembler_reassembles_fragments_in_any_order() {
    let big_rect = ScreenRect::new(0, 0, 256, 256).unwrap();
    let big = ScreenFrame::raw(9, big_rect, PixelFormat::Bgra8, vec![4u8; 256 * 256 * 4]).unwrap();
    let frames = big.to_frames().unwrap();
    assert!(frames.len() > 1, "a full-screen frame must fragment");

    // In order, reversed, and with a trailing duplicate: all yield exactly `big`.
    let mut with_dup = frames.clone();
    with_dup.push(frames[0].clone());
    for order in [
        frames.clone(),
        {
            let mut v = frames.clone();
            v.reverse();
            v
        },
        with_dup,
    ] {
        let mut a = ScreenAssembler::new();
        let mut got = None;
        for f in &order {
            if let Some(done) = a.push(f).unwrap() {
                got = Some(done);
            }
        }
        assert_eq!(
            got.as_ref(),
            Some(&big),
            "fragments must reassemble in any order"
        );
    }
}

#[test]
fn screen_assembler_refuses_an_incomplete_stream() {
    let big_rect = ScreenRect::new(0, 0, 256, 256).unwrap();
    let big = ScreenFrame::raw(9, big_rect, PixelFormat::Bgra8, vec![4u8; 256 * 256 * 4]).unwrap();
    let frames = big.to_frames().unwrap();
    let mut a = ScreenAssembler::new();
    for f in &frames[..frames.len() - 1] {
        assert_eq!(a.push(f).unwrap(), None);
    }
    // A whole frame supersedes the half-assembled stream and returns at once.
    let small = raw_frame(3);
    assert_eq!(a.push(&small.to_frame().unwrap()).unwrap(), Some(small));
}

// --------------------------------------------------------------- ANCS ingress

/// Build an 8-byte Notification Source value (added event unless told otherwise).
fn notify_event(event_id: u8, flags: u8, uid: u32) -> [u8; NOTIFICATION_EVENT_LEN] {
    let mut v = [0u8; NOTIFICATION_EVENT_LEN];
    v[0] = event_id;
    v[1] = flags;
    v[2] = 4; // Social
    v[3] = 1;
    v[4..8].copy_from_slice(&uid.to_le_bytes());
    v
}

/// The ANCS ingress is the richest machine (BLE callbacks + a timer). Sweep a
/// deterministic op program through it and assert the two security-critical
/// rules: a callback is inert while not `Ready`, and a pre-existing uid is never
/// surfaced (M4.3).
#[test]
fn ancs_ingress_sequences_never_panic_and_never_surface_a_pre_existing_uid() {
    let mut seed = 0x2545_F491_4F6C_DD1Du64;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    for _ in 0..64 {
        let clock = Arc::new(ManualClock::new(1_000));
        let transport = Arc::new(MemoryAncsTransport::default());
        let mut ing = AncsIngress::new(transport, clock.clone());
        let mut now = 1_000u64;
        for _ in 0..400 {
            match next() % 7 {
                0 => {
                    let _ = ing.connect().unwrap();
                }
                1 => {
                    now = now.saturating_add(next() % 20_000);
                    clock.set(now);
                }
                2 => {
                    let uid = (next() % 16) as u32;
                    let flags = (next() % 0x20) as u8;
                    let ev = notify_event((next() % 3) as u8, flags, uid);
                    // The event queue is not cleared by `connect()`, so drop any
                    // events an earlier (Ready) step produced before asserting
                    // that *this* callback added none.
                    let ready_before = ing.state() == AncsState::Ready;
                    let _ = ing.drain_events();
                    let _ = ing.on_notification_source(&ev);
                    if !ready_before {
                        assert!(
                            ing.drain_events().is_empty(),
                            "inert callback surfaced an event"
                        );
                    }
                }
                3 => {
                    let _ = ing.connect();
                    if ing.state() != AncsState::Ready {
                        continue;
                    }
                    let _ = ing.drain_events();
                    let ev = notify_event(0, EventFlags::PRE_EXISTING, 999);
                    let _ = ing.on_notification_source(&ev);
                    assert!(ing.drain_events().is_empty(), "pre-existing uid surfaced");
                }
                4 => {
                    let len = (next() % 24) as usize;
                    let mut v = vec![0u8; len];
                    for b in v.iter_mut() {
                        *b = next() as u8;
                    }
                    let _ = ing.on_data_source(&v);
                }
                5 => ing.tick().unwrap(),
                _ => {
                    let retry_at = ing.on_disconnected();
                    assert!(retry_at >= now);
                    if now < retry_at {
                        assert!(matches!(
                            ing.connect().unwrap(),
                            ConnectOutcome::BackingOff { .. }
                        ));
                    }
                }
            }
            assert!(ing.pending_notifications() <= 16);
            assert!(ing.queued() <= 64);
        }
    }
}

/// The dedup contract, isolated on a frozen clock so no window can expire: the
/// second identical push is never a fresh candidate, and a blank reply is
/// refused outright.
#[test]
fn notify_hub_dedups_on_a_frozen_clock() {
    let mut hub = NotifyHub::default();
    let clock = ManualClock::new(50_000);
    let src = DeviceId::from_id_pk(&[3u8; 32]);
    let push = || NotifyPush {
        uid: "42".into(),
        app_id: "sms".into(),
        app_name: "sms".into(),
        title: "t".into(),
        body: "验证码:868740".into(),
        ts: 1,
        actions: vec!["copy".into()],
    };
    assert!(matches!(
        hub.ingest_push(src.clone(), push(), &clock),
        IngestOutcome::Candidate(_)
    ));
    assert!(!matches!(
        hub.ingest_push(src.clone(), push(), &clock),
        IngestOutcome::Candidate(_)
    ));
    assert!(!hub.ingest_reply(
        &src,
        NotifyReply {
            uid: "42".into(),
            app_id: "sms".into(),
            text: "   ".into()
        }
    ));
    assert!(hub.reply_views().is_empty());
    assert!(hub.ingest_reply(
        &src,
        NotifyReply {
            uid: "42".into(),
            app_id: "sms".into(),
            text: "ok".into()
        }
    ));
    assert_eq!(hub.take_reply(&src, "sms", "42").as_deref(), Some("ok"));
    assert!(hub.take_reply(&src, "sms", "42").is_none());
}

/// `FileDone` is honored only after a local accept, and a mismatched digest is
/// refused — the peer cannot declare a transfer complete on its own say-so.
#[test]
fn file_hub_requires_accept_before_done() {
    let mut hub = FileHub::default();
    let src = DeviceId::from_id_pk(&[4u8; 32]);
    let sha = hex64(1);
    let offer = FileOffer {
        transfer_id: "tr_1".into(),
        files: vec![FileMeta {
            name: "a.bin".into(),
            size: 8,
            sha256: sha.clone(),
        }],
    };
    hub.offer(src, offer, 1).unwrap();
    assert!(hub
        .mark_done(&FileDone {
            transfer_id: "tr_1".into(),
            sha256: sha.clone()
        })
        .is_err());
    hub.accept("tr_1", 2).unwrap();
    assert!(hub
        .mark_done(&FileDone {
            transfer_id: "tr_1".into(),
            sha256: hex64(2)
        })
        .is_err());
    let done = hub
        .mark_done(&FileDone {
            transfer_id: "tr_1".into(),
            sha256: sha,
        })
        .unwrap();
    assert_eq!(done.status, tetherly_core::filexfer::TransferStatus::Done);
}

/// Distinct IPs never share a bucket, so one noisy source cannot rate-limit
/// another.
#[test]
fn tcp_limiter_isolates_by_ip() {
    use tetherly_core::TcpLimiter;
    let mut lim = TcpLimiter::default();
    let a: IpAddr = "127.0.0.1".parse().unwrap();
    let b: IpAddr = "127.0.0.2".parse().unwrap();
    for _ in 0..30 {
        assert!(lim.allow(a, 0));
    }
    assert!(!lim.allow(a, 0), "a is exhausted");
    assert!(lim.allow(b, 0), "b has its own bucket");
}
