// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Unit tests for screen streaming frame/control codecs.

use super::*;
use crate::error::CoreError;
use crate::frame::{InnerFrame, FLAG_MUST_UNDERSTAND, INNER_PAYLOAD_MAX};
use crate::ports::{ScreenSink, ScreenSource};

fn rect() -> ScreenRect {
    ScreenRect::new(0, 0, 4, 2).unwrap()
}

fn raw_frame(seq: u64) -> ScreenFrame {
    ScreenFrame::raw(seq, rect(), PixelFormat::Bgra8, vec![7u8; 32]).unwrap()
}

#[test]
fn rect_rejects_zero_dimensions() {
    assert!(ScreenRect::new(0, 0, 0, 10).is_err());
    assert!(ScreenRect::new(0, 0, 10, 0).is_err());
    assert_eq!(rect().pixel_bytes(PixelFormat::Bgra8), 32);
}

#[test]
fn raw_frame_length_must_match_rect() {
    assert!(ScreenFrame::raw(1, rect(), PixelFormat::Bgra8, vec![0u8; 31]).is_err());
    assert!(ScreenFrame::raw(1, rect(), PixelFormat::Bgra8, vec![0u8; 32]).is_ok());
}

#[test]
fn frame_round_trips_through_the_wire() {
    let f = raw_frame(42);
    let wire = f.to_frame().unwrap();
    assert_eq!(wire.ty, TYPE_SCREEN_FRAME);
    let back = ScreenFrame::from_frame(&wire).unwrap();
    assert_eq!(back, f);
}

#[test]
fn decode_rejects_a_short_header_and_a_bad_magic() {
    assert_eq!(
        ScreenFrame::decode_payload(&[0u8; 10]).unwrap_err(),
        CoreError::TruncatedFrame
    );
    let mut payload = raw_frame(1).encode_payload().unwrap();
    payload[0] = b'X';
    assert_eq!(
        ScreenFrame::decode_payload(&payload).unwrap_err(),
        CoreError::ScreenFrame
    );
}

#[test]
fn decode_rejects_a_raw_body_that_does_not_match_the_rect() {
    let mut payload = raw_frame(1).encode_payload().unwrap();
    payload.push(0);
    assert_eq!(
        ScreenFrame::decode_payload(&payload).unwrap_err(),
        CoreError::ScreenFrame
    );
}

#[test]
fn decode_rejects_an_unknown_compression_tag() {
    let mut payload = raw_frame(1).encode_payload().unwrap();
    payload[26] = 255;
    assert_eq!(
        ScreenFrame::decode_payload(&payload).unwrap_err(),
        CoreError::ScreenFrame
    );
}

#[test]
fn control_round_trips_and_rejects_a_short_body() {
    for msg in [
        ControlMsg::Start,
        ControlMsg::Stop,
        ControlMsg::Ack { seq: 9 },
        ControlMsg::KeyframeRequest,
        ControlMsg::SwitchDisplay { display_id: 2 },
    ] {
        let wire = msg.to_frame().unwrap();
        assert_eq!(ControlMsg::from_frame(&wire).unwrap(), msg);
    }
    assert_eq!(
        ControlMsg::decode_payload(&[0u8; 5]).unwrap_err(),
        CoreError::TruncatedFrame
    );
}

#[test]
fn start_is_refused_until_the_local_machine_allows() {
    let mut s = ScreenSender::new();
    s.on_peer_connected();
    assert_eq!(s.state(), ScreenState::Requested);
    // Arriving at the channel is not consent: refusal is answered with the
    // Stop reply so the controller learns the Start did not take effect.
    assert_eq!(
        s.on_control(ControlMsg::Start).unwrap(),
        Some(ControlMsg::Stop)
    );
    assert_eq!(s.state(), ScreenState::Requested);
    assert_eq!(s.stats().refused, 1);
    // A push while un-allowed is dropped, not queued.
    assert!(s.push(raw_frame(1)).is_none());
    assert!(s.drain().is_empty());

    s.allow();
    assert_eq!(s.state(), ScreenState::Allowed);
    s.on_control(ControlMsg::Start).unwrap();
    assert_eq!(s.state(), ScreenState::Streaming);
    assert_eq!(s.push(raw_frame(1)), Some(1));
    assert_eq!(s.drain().len(), 1);
}

#[test]
fn revoke_stops_streaming_and_clears_the_queue() {
    let mut s = ScreenSender::new();
    s.on_peer_connected();
    s.allow();
    s.on_control(ControlMsg::Start).unwrap();
    s.push(raw_frame(1));
    s.revoke();
    assert_eq!(s.state(), ScreenState::Requested);
    assert!(s.drain().is_empty());
    // After a revoke, Start is refused again (with the Stop refusal reply).
    assert_eq!(
        s.on_control(ControlMsg::Start).unwrap(),
        Some(ControlMsg::Stop)
    );
    assert_eq!(s.state(), ScreenState::Requested);
    assert_eq!(s.stats().refused, 1);
}

#[test]
fn leaving_the_channel_resets_the_sequence() {
    let mut s = ScreenSender::new();
    s.on_peer_connected();
    s.allow();
    s.on_control(ControlMsg::Start).unwrap();
    assert_eq!(s.push(raw_frame(1)), Some(1));
    s.on_peer_gone();
    assert_eq!(s.state(), ScreenState::Idle);
    assert_eq!(s.next_seq(), 1);
    // A late frame from the dead peer is dropped.
    assert!(s.push(raw_frame(2)).is_none());
}

#[test]
fn receiver_drops_stale_and_counts_the_drop() {
    let mut r = ScreenReceiver::new();
    assert_eq!(r.apply(&raw_frame(1)).unwrap(), Some(1));
    assert_eq!(r.apply(&raw_frame(1)).unwrap(), None);
    assert_eq!(r.apply(&raw_frame(0)).unwrap(), None);
    assert_eq!(r.stats().dropped, 2);
    assert_eq!(r.stats().received, 1);
    assert_eq!(r.last_seq(), 1);
}

#[test]
fn receiver_refuses_a_forward_jump_beyond_the_window() {
    let mut r = ScreenReceiver::new();
    r.apply(&raw_frame(1)).unwrap();
    let far = SCREEN_SEQ_WINDOW + 10;
    assert_eq!(
        r.apply(&raw_frame(far)).unwrap_err(),
        CoreError::ScreenSeqJump
    );
    // The stride is untouched by the refusal.
    assert_eq!(r.last_seq(), 1);
}

#[test]
fn receiver_forgets_the_cursor_on_peer_gone() {
    let mut r = ScreenReceiver::new();
    r.apply(&raw_frame(5)).unwrap();
    r.on_peer_gone();
    assert_eq!(r.last_seq(), 0);
    assert!(r.trace().is_empty());
    // seq 1 is a fresh start again, not a stale drop.
    assert_eq!(r.apply(&raw_frame(1)).unwrap(), Some(1));
}

#[test]
fn receiver_trace_is_bounded() {
    let mut r = ScreenReceiver::new();
    for seq in 1..=(SCREEN_TRACE_MAX as u64 + 20) {
        r.apply(&raw_frame(seq)).unwrap();
    }
    assert_eq!(r.trace().len(), SCREEN_TRACE_MAX);
    assert_eq!(*r.trace().last().unwrap(), SCREEN_TRACE_MAX as u64 + 20);
}

#[test]
fn memory_source_is_deterministic_and_can_inject_a_failure() {
    let a = MemoryScreenSource::new(4, 2).unwrap();
    let b = MemoryScreenSource::new(4, 2).unwrap();
    assert_eq!(a.grab().unwrap(), b.grab().unwrap());
    assert_eq!(a.grab().unwrap().unwrap().seq, 2);

    let src = MemoryScreenSource::new(4, 2).unwrap();
    src.fail_on_grab(2);
    assert!(src.grab().is_ok());
    assert!(src.grab().is_err());
    assert!(src.grab().is_ok());
}

#[test]
fn memory_sink_records_and_surfaces_a_failure() {
    let sink = MemoryScreenSink::new();
    sink.present(&raw_frame(1)).unwrap();
    sink.present(&raw_frame(2)).unwrap();
    assert_eq!(sink.presented(), vec![1, 2]);
    assert_eq!(sink.last().unwrap().seq, 2);

    sink.set_fail(true);
    assert!(sink.present(&raw_frame(3)).is_err());
    assert_eq!(sink.count(), 2);
}

#[test]
fn pixel_format_swap_round_trips() {
    let mut px = vec![1u8, 2, 3, 4];
    PixelFormat::Bgra8.converted(PixelFormat::Rgba8, &mut px);
    assert_eq!(px, vec![3u8, 2, 1, 4]);
    PixelFormat::Rgba8.converted(PixelFormat::Bgra8, &mut px);
    assert_eq!(px, vec![1u8, 2, 3, 4]);
}

#[test]
fn effective_stride_defaults_to_packed() {
    let f = raw_frame(1);
    assert_eq!(f.stride, 0);
    assert_eq!(f.effective_stride(), 4 * 4);
    assert_eq!(f.raw_len(), 32);
}

#[test]
fn screen_types_are_recognized_for_the_must_understand_exemption() {
    assert!(is_screen_type(TYPE_SCREEN_FRAME));
    assert!(is_screen_type(TYPE_SCREEN_CONTROL));
    assert!(!is_screen_type(0x0999));
}

#[test]
fn small_frame_takes_the_whole_frame_fast_path() {
    let f = raw_frame(7);
    let frames = f.to_frames().unwrap();
    assert_eq!(frames.len(), 1);
    assert!(frames[0].payload.starts_with(&SCREEN_MAGIC));
    let mut a = ScreenAssembler::new();
    let out = a.push(&frames[0]).unwrap();
    assert_eq!(out, Some(f));
}

#[test]
fn large_frame_fragments_and_reassembles_round_trip() {
    // 4 MiB raw BGRA body — far past the 64 KiB inner budget.
    let rect = ScreenRect::new(0, 0, 1024, 1024).unwrap();
    let bytes = vec![0xABu8; rect.pixel_bytes(PixelFormat::Bgra8)];
    let f = ScreenFrame::raw(9, rect, PixelFormat::Bgra8, bytes).unwrap();
    let frames = f.to_frames().unwrap();
    assert!(frames.len() > 2, "must be fragmented, got {}", frames.len());
    for fr in &frames {
        assert!(fr.payload.starts_with(&SCREEN_FRAG_MAGIC));
        assert!(fr.payload.len() <= INNER_PAYLOAD_MAX);
    }
    let mut a = ScreenAssembler::new();
    let mut out = None;
    for fr in &frames {
        out = a.push(fr).unwrap();
    }
    let out = out.expect("last fragment yields the frame");
    assert_eq!(out, f);
    assert_eq!(out.bytes.len(), rect.pixel_bytes(PixelFormat::Bgra8));
}

#[test]
fn assembler_rejects_a_lying_fragment_stream() {
    let rect = ScreenRect::new(0, 0, 1024, 1024).unwrap();
    let f = ScreenFrame::raw(1, rect, PixelFormat::Bgra8, vec![0u8; 4 * 1024 * 1024]).unwrap();
    let frames = f.to_frames().unwrap();
    assert!(frames.len() > 1, "must be fragmented");
    // Corrupt the first fragment: total_len far above the cap.
    let mut bad = frames[0].payload.clone();
    bad[4..8].copy_from_slice(&(SCREEN_PAYLOAD_MAX as u32 + 1000).to_be_bytes());
    let mut a = ScreenAssembler::new();
    assert_eq!(
        a.push(&InnerFrame {
            ty: TYPE_SCREEN_FRAME,
            flags: FLAG_MUST_UNDERSTAND,
            msg_id: 0,
            payload: bad,
        })
        .unwrap_err(),
        CoreError::ScreenFragment
    );
    // A non-TMV1 non-TMF1 payload is refused too.
    let mut a = ScreenAssembler::new();
    assert_eq!(
        a.push(&InnerFrame {
            ty: TYPE_SCREEN_FRAME,
            flags: FLAG_MUST_UNDERSTAND,
            msg_id: 0,
            payload: b"not a frame".to_vec(),
        })
        .unwrap_err(),
        CoreError::ScreenFragment
    );
}

#[test]
fn assembler_rejects_an_index_past_the_stream_end() {
    // A sender that keeps `total` small but claims a high `idx` must be
    // refused before `total - off` can underflow (a remote panic).
    let mut a = ScreenAssembler::new();
    let mut frag = vec![0u8; SCREEN_FRAG_HEADER + 16];
    frag[..4].copy_from_slice(&SCREEN_FRAG_MAGIC);
    frag[4..8].copy_from_slice(&16u32.to_be_bytes()); // total = 16
    frag[8..12].copy_from_slice(&1u32.to_be_bytes()); // idx = 1 -> off = 64K
    frag[12..16].copy_from_slice(&2u32.to_be_bytes()); // count = 2
    frag[16..24].copy_from_slice(&5u64.to_be_bytes());
    assert_eq!(
        a.push(&InnerFrame {
            ty: TYPE_SCREEN_FRAME,
            flags: FLAG_MUST_UNDERSTAND,
            msg_id: 0,
            payload: frag,
        })
        .unwrap_err(),
        CoreError::ScreenFragment
    );
}

#[test]
fn assembler_stale_stream_is_superseded_by_a_new_seq() {
    let rect = ScreenRect::new(0, 0, 1024, 1024).unwrap();
    let f1 = ScreenFrame::raw(1, rect, PixelFormat::Bgra8, vec![0x11u8; 4 * 1024 * 1024]).unwrap();
    let f2 = ScreenFrame::raw(2, rect, PixelFormat::Bgra8, vec![0x22u8; 4 * 1024 * 1024]).unwrap();
    let (mut f1f, mut f2f) = (f1.to_frames().unwrap(), f2.to_frames().unwrap());
    let mut a = ScreenAssembler::new();
    // Half of stream 1, then a chunk of stream 2 — the old buffer resets.
    for fr in f1f.drain(..f1f.len() / 2) {
        assert!(a.push(&fr).unwrap().is_none());
    }
    assert!(a.push(&f2f.remove(0)).unwrap().is_none());
    let mut out = None;
    for fr in f2f {
        out = a.push(&fr).unwrap();
    }
    assert_eq!(out, Some(f2));
}

#[test]
fn assembler_rejects_a_short_count_that_would_leave_the_body_unfilled() {
    // `count` smaller than the chunks implied by `total` would otherwise let
    // the stream "complete" with holes in `body` — a silently corrupt frame.
    let rect = ScreenRect::new(0, 0, 1024, 1024).unwrap();
    let f = ScreenFrame::raw(1, rect, PixelFormat::Bgra8, vec![0u8; 4 * 1024 * 1024]).unwrap();
    let frames = f.to_frames().unwrap();
    assert!(frames.len() > 1);
    let mut bad = frames[0].payload.clone();
    bad[12..16].copy_from_slice(&1u32.to_be_bytes()); // count = 1, total is many
    let mut a = ScreenAssembler::new();
    assert_eq!(
        a.push(&InnerFrame {
            ty: TYPE_SCREEN_FRAME,
            flags: FLAG_MUST_UNDERSTAND,
            msg_id: 0,
            payload: bad,
        })
        .unwrap_err(),
        CoreError::ScreenFragment
    );
}

#[test]
fn assembler_whole_frame_fast_path_drops_stale_fragments() {
    // Half a fragmented stream, then a whole frame: the whole frame must
    // win and the leftover fragments must not resurrect.
    let rect = ScreenRect::new(0, 0, 1024, 1024).unwrap();
    let big = ScreenFrame::raw(1, rect, PixelFormat::Bgra8, vec![0x33u8; 4 * 1024 * 1024]).unwrap();
    let frags = big.to_frames().unwrap();
    let small = raw_frame(2);
    let whole = small.to_frames().unwrap();
    assert!(frags.len() > 1 && whole.len() == 1);

    let mut a = ScreenAssembler::new();
    assert!(a.push(&frags[0]).unwrap().is_none());
    assert_eq!(a.push(&whole[0]).unwrap(), Some(small.clone()));
    // Feeding the remaining old fragments must re-anchor against a fresh
    // stream (they may start a new partial), never blend with the whole
    // frame that already shipped.
    for fr in &frags[1..] {
        let _ = a.push(fr);
    }
}

#[test]
fn screen_matrix_coordinator_health_state_transitions() {
    let mut coord = ScreenMatrixCoordinator::default();
    assert_eq!(coord.health(), NetworkHealth::Healthy);
    assert_eq!(coord.mode(), ScreenMatrixMode::Auto);

    // Low RTT, 0 inflight -> Healthy
    let out1 = coord.on_ack(1_000_000, 50_000, 0, 60, 100, 60);
    assert_eq!(out1.health, NetworkHealth::Healthy);

    // Elevated RTT (150ms) -> Warning
    let out2 = coord.on_ack(1_100_000, 150_000, 0, 60, 100, 60);
    assert_eq!(out2.health, NetworkHealth::Warning);

    // High RTT (280ms) -> Congested
    let out3 = coord.on_ack(1_200_000, 280_000, 0, 60, 100, 60);
    assert_eq!(out3.health, NetworkHealth::Congested);

    // High in-flight (8) -> Critical
    let out4 = coord.on_ack(1_300_000, 100_000, 8, 60, 100, 60);
    assert_eq!(out4.health, NetworkHealth::Critical);
}

#[test]
fn screen_matrix_coordinator_demotion_ladder_on_drops() {
    let mut coord = ScreenMatrixCoordinator::default();
    let now_us = 10_000_000;

    // 1 drop -> Congested -> demote FPS from 60 to 30
    let out1 = coord.on_drop(now_us, 60, 100);
    assert_eq!(out1.health, NetworkHealth::Congested);
    assert_eq!(out1.new_fps, Some(30));
    assert_eq!(out1.new_scale_pct, None);

    // 2nd drop (exceeds cooldown: +500ms) -> Critical -> demote scale to 75 and FPS to 24
    let out2 = coord.on_drop(now_us + 500_000, 30, 100);
    assert_eq!(out2.health, NetworkHealth::Critical);
    assert_eq!(out2.new_scale_pct, Some(75));
    assert_eq!(out2.new_fps, Some(24));
    assert!(out2.force_idr);
}

#[test]
fn screen_matrix_coordinator_promotion_ladder_on_sustained_healthy_acks() {
    let mut coord = ScreenMatrixCoordinator::default();
    let mut now_us = 10_000_000;

    // Start degraded at 50% scale, 15 FPS
    let mut current_scale = 50;
    let current_fps = 15;

    // Send 14 healthy acks (not yet 15)
    for _ in 0..14 {
        now_us += 100_000;
        let out = coord.on_ack(now_us, 30_000, 0, current_fps, current_scale, 60);
        assert_eq!(out.new_scale_pct, None);
        assert_eq!(out.new_fps, None);
    }

    // 15th ack (+3.5s elapsed): should promote scale first: 50% -> 75%
    now_us += 3_500_000;
    let out_promo1 = coord.on_ack(now_us, 30_000, 0, current_fps, current_scale, 60);
    assert_eq!(out_promo1.new_scale_pct, Some(75));
    assert!(out_promo1.force_idr);
    current_scale = 75;

    // Another 15 acks (+3.5s): promote scale 75% -> 100%
    for _ in 0..14 {
        now_us += 100_000;
        let _ = coord.on_ack(now_us, 30_000, 0, current_fps, current_scale, 60);
    }
    now_us += 3_500_000;
    let out_promo2 = coord.on_ack(now_us, 30_000, 0, current_fps, current_scale, 60);
    assert_eq!(out_promo2.new_scale_pct, Some(100));
    current_scale = 100;

    // Once scale is 100%, next promotion promotes FPS: 15 -> 24
    for _ in 0..14 {
        now_us += 100_000;
        let _ = coord.on_ack(now_us, 30_000, 0, current_fps, current_scale, 60);
    }
    now_us += 3_500_000;
    let out_promo3 = coord.on_ack(now_us, 30_000, 0, current_fps, current_scale, 60);
    assert_eq!(out_promo3.new_fps, Some(24));
    assert_eq!(out_promo3.new_scale_pct, None);
}

#[test]
fn screen_matrix_coordinator_modes_manual_and_conservative() {
    let mut coord_manual = ScreenMatrixCoordinator::new(ScreenMatrixMode::Manual);
    let out_m = coord_manual.on_drop(1_000_000, 60, 100);
    assert_eq!(out_m.health, NetworkHealth::Congested);
    assert_eq!(out_m.new_fps, None);
    assert_eq!(out_m.new_scale_pct, None);

    let mut coord_cons = ScreenMatrixCoordinator::new(ScreenMatrixMode::Conservative);
    let out_c1 = coord_cons.on_drop(1_000_000, 60, 100);
    assert_eq!(out_c1.new_fps, Some(30));
    assert_eq!(out_c1.new_scale_pct, None); // Conservative never downscales resolution

    let out_c2 = coord_cons.on_drop(2_000_000, 30, 100);
    assert_eq!(out_c2.health, NetworkHealth::Critical);
    assert_eq!(out_c2.new_fps, Some(24));
    assert_eq!(out_c2.new_scale_pct, None); // Still no scale change
}

#[test]
fn screen_sender_4d_autonomous_matrix_closed_loop() {
    let mut sender = ScreenSender::new();
    assert_eq!(sender.stats().matrix_mode(), ScreenMatrixMode::Auto);
    assert_eq!(sender.stats().network_health(), NetworkHealth::Healthy);
    assert_eq!(sender.stats().fps_target, 30);
    assert_eq!(sender.stats().scale_pct, 100);

    sender.set_fps(60);
    assert_eq!(sender.stats().fps_target, 60);

    // Record a congestion drop at t=10_000_000
    sender.record_drop_with_time(10_000_000);
    assert_eq!(sender.stats().network_health(), NetworkHealth::Congested);
    assert_eq!(sender.stats().fps_target, 30); // 60 -> 30 demoted
    assert_eq!(sender.stats().scale_pct, 100);

    // Mode switch check
    sender.set_matrix_mode(ScreenMatrixMode::Conservative);
    assert_eq!(sender.stats().matrix_mode(), ScreenMatrixMode::Conservative);
}

#[test]
fn sender_tracks_in_flight_and_computes_rtt_on_ack() {
    let mut s = ScreenSender::new();
    s.on_peer_connected();
    s.allow();
    s.on_control(ControlMsg::Start).unwrap();

    // Push 3 frames with timestamps (t = 1000, 2000, 3000 us)
    assert_eq!(s.push_with_time(raw_frame(1), 1000), Some(1));
    assert_eq!(s.push_with_time(raw_frame(2), 2000), Some(2));
    assert_eq!(s.push_with_time(raw_frame(3), 3000), Some(3));

    assert_eq!(s.stats().sent, 3);
    assert_eq!(s.stats().in_flight, 3);
    assert_eq!(s.in_flight_count(), 3);
    assert!(!s.is_congested());

    // Ack frame 2 at t = 2050 us (RTT for frame 2 = 50 us; frame 1 is also cumulatively acknowledged)
    s.on_control_with_time(ControlMsg::Ack { seq: 2 }, 2050)
        .unwrap();
    assert_eq!(s.stats().acked, 1);
    assert_eq!(s.stats().last_acked_seq, 2);
    assert_eq!(s.stats().in_flight, 1); // Only frame 3 remains in-flight
    assert_eq!(s.stats().rtt_latest_us, Some(50));
    assert_eq!(s.stats().rtt_p95_us, Some(50));

    // Ack frame 3 at t = 3100 us (RTT for frame 3 = 100 us)
    s.on_control_with_time(ControlMsg::Ack { seq: 3 }, 3100)
        .unwrap();
    assert_eq!(s.stats().acked, 2);
    assert_eq!(s.stats().last_acked_seq, 3);
    assert_eq!(s.stats().in_flight, 0);
    assert_eq!(s.stats().rtt_latest_us, Some(100));
    assert_eq!(s.stats().rtt_p95_us, Some(100));
    assert!(!s.is_congested());
}

#[test]
fn sender_congestion_detection_and_in_flight_bounds() {
    let mut s = ScreenSender::new();
    s.on_peer_connected();
    s.allow();
    s.on_control(ControlMsg::Start).unwrap();

    // Push up to congestion threshold (8 frames)
    for i in 1..=8 {
        s.push_with_time(raw_frame(i), i * 1000);
    }
    assert_eq!(s.in_flight_count(), 8);
    assert!(s.is_congested()); // In-flight threshold reached

    // Ack frame 8: clears in-flight queue, RTT = 20 us
    s.on_control_with_time(ControlMsg::Ack { seq: 8 }, 8020)
        .unwrap();
    assert_eq!(s.in_flight_count(), 0);
    assert!(!s.is_congested());

    // Push frame 9, ack with high RTT (300ms)
    s.push_with_time(raw_frame(9), 10_000);
    s.on_control_with_time(ControlMsg::Ack { seq: 9 }, 10_000 + 300_000)
        .unwrap();
    assert_eq!(s.stats().rtt_latest_us, Some(300_000));
    assert_eq!(s.in_flight_count(), 0);
    assert!(!s.is_congested(), "drained queue must not be congested");

    // When a new frame 10 is pushed under high RTT, while it is in-flight, channel becomes congested
    s.push_with_time(raw_frame(10), 400_000);
    assert_eq!(s.in_flight_count(), 1);
    assert!(
        s.is_congested(),
        "in-flight frame with high RTT triggers congestion pacing"
    );
}

#[test]
fn keyframe_request_and_gap_detection_tracking() {
    let mut sender = ScreenSender::new();
    sender.on_peer_connected();
    sender.allow();
    sender.on_control(ControlMsg::Start).unwrap();

    assert_eq!(sender.stats().keyframes_requested, 0);
    sender.on_control(ControlMsg::KeyframeRequest).unwrap();
    assert_eq!(sender.stats().keyframes_requested, 1);
    sender.on_control(ControlMsg::KeyframeRequest).unwrap();
    assert_eq!(sender.stats().keyframes_requested, 2);

    let mut receiver = ScreenReceiver::new();
    assert_eq!(receiver.stats().gaps, 0);

    // Frame 1 arrives
    assert_eq!(receiver.apply(&raw_frame(1)).unwrap(), Some(1));
    assert_eq!(receiver.stats().gaps, 0);

    // Frame 2 arrives (consecutive, no gap)
    assert_eq!(receiver.apply(&raw_frame(2)).unwrap(), Some(2));
    assert_eq!(receiver.stats().gaps, 0);

    // Frame 5 arrives (frames 3 and 4 were dropped -> 1 gap detected)
    assert_eq!(receiver.apply(&raw_frame(5)).unwrap(), Some(5));
    assert_eq!(receiver.stats().gaps, 1);

    // Frame 6 arrives (consecutive, no additional gap)
    assert_eq!(receiver.apply(&raw_frame(6)).unwrap(), Some(6));
    assert_eq!(receiver.stats().gaps, 1);

    // Frame 9 arrives (frames 7 and 8 dropped -> gap count becomes 2)
    assert_eq!(receiver.apply(&raw_frame(9)).unwrap(), Some(9));
    assert_eq!(receiver.stats().gaps, 2);
}

#[test]
fn rate_controller_aimd_adaptation() {
    let mut s = ScreenSender::new();
    s.on_peer_connected();
    s.allow();
    s.on_control(ControlMsg::Start).unwrap();

    assert_eq!(s.bitrate_bps(), SCREEN_BITRATE_DEFAULT_BPS);
    assert_eq!(s.stats().bitrate_bps, SCREEN_BITRATE_DEFAULT_BPS);

    // 1. High RTT congestion triggers Multiplicative Decrease (80% of 4_000_000 = 3_200_000)
    s.push_with_time(raw_frame(1), 1_000);
    s.on_control_with_time(ControlMsg::Ack { seq: 1 }, 1_000 + 300_000)
        .unwrap();
    assert_eq!(s.bitrate_bps(), 3_200_000);

    // 2. Cooldown damping: another high RTT within 400ms does NOT decrease again
    s.push_with_time(raw_frame(2), 310_000);
    s.on_control_with_time(ControlMsg::Ack { seq: 2 }, 310_000 + 260_000)
        .unwrap();
    assert_eq!(s.bitrate_bps(), 3_200_000);

    // 3. Sender drop past cooldown triggers another decrease (80% of 3_200_000 = 2_560_000)
    s.record_drop_with_time(1_000_000);
    assert_eq!(s.bitrate_bps(), 2_560_000);

    // 4. Clean network (low RTT, in_flight=0): 5 consecutive good frames trigger Additive Increase (+150k)
    let mut now = 1_500_000;
    for i in 3..8 {
        s.push_with_time(raw_frame(i), now);
        now += 50_000;
        s.on_control_with_time(ControlMsg::Ack { seq: i }, now)
            .unwrap();
    }
    assert_eq!(
        s.bitrate_bps(),
        2_560_000 + SCREEN_BITRATE_STEP_INCREASE_BPS
    );

    // 5. Manual override clamps to bounds
    s.set_bitrate_bps(50_000_000);
    assert_eq!(s.bitrate_bps(), SCREEN_BITRATE_MAX_BPS);

    s.set_bitrate_bps(10_000);
    assert_eq!(s.bitrate_bps(), SCREEN_BITRATE_MIN_BPS);
}

#[test]
fn fps_pacer_and_frame_rate_throttling() {
    let mut pacer = ScreenFpsPacer::default();
    assert_eq!(pacer.fps_target(), SCREEN_FPS_DEFAULT);
    assert_eq!(pacer.interval_us(), 1_000_000 / 30);

    // 1. Clamping bounds: upper limit (120 FPS) and lower limit (5 FPS)
    assert_eq!(pacer.set_fps(200), SCREEN_FPS_MAX);
    assert_eq!(pacer.fps_target(), SCREEN_FPS_MAX);
    assert_eq!(pacer.interval_us(), 1_000_000 / 120);

    assert_eq!(pacer.set_fps(1), SCREEN_FPS_MIN);
    assert_eq!(pacer.fps_target(), SCREEN_FPS_MIN);
    assert_eq!(pacer.interval_us(), 1_000_000 / 5);

    // Set to 30 FPS (interval = 33_333 us)
    pacer.set_fps(30);
    assert_eq!(pacer.fps_target(), 30);
    let interval = pacer.interval_us();

    // 2. Initial frame tick consumes immediately
    assert!(pacer.is_ready(100_000));
    assert!(pacer.check_and_consume(100_000));

    // 3. Early tick before interval elapses is throttled
    assert!(!pacer.is_ready(100_000 + 15_000));
    assert!(!pacer.check_and_consume(100_000 + 15_000));

    // 4. Tick after interval elapses is permitted
    assert!(pacer.is_ready(100_000 + interval));
    assert!(pacer.check_and_consume(100_000 + interval));

    // 5. Sender integration and stats
    let mut s = ScreenSender::new();
    assert_eq!(s.fps_target(), SCREEN_FPS_DEFAULT);
    assert_eq!(s.stats().fps_target, SCREEN_FPS_DEFAULT);

    assert_eq!(s.set_fps(60), 60);
    assert_eq!(s.fps_target(), 60);
    assert_eq!(s.stats().fps_target, 60);

    assert!(s.consume_pacing(500_000));
    assert!(!s.consume_pacing(500_000 + 5_000));
    assert!(s.consume_pacing(500_000 + 17_000));

    s.on_peer_gone();
    // Reset resets last frame timestamp, allowing immediate next tick
    assert!(s.is_pacing_ready(600_000));
}

#[test]
fn resolution_scale_controller_and_downscaling() {
    let mut ctrl = ScreenScaleController::default();
    assert_eq!(ctrl.scale_pct(), SCREEN_SCALE_DEFAULT_PCT);

    // 1. Clamping bounds
    assert_eq!(ctrl.set_scale_pct(150), SCREEN_SCALE_MAX_PCT);
    assert_eq!(ctrl.scale_pct(), SCREEN_SCALE_MAX_PCT);

    assert_eq!(ctrl.set_scale_pct(10), SCREEN_SCALE_MIN_PCT);
    assert_eq!(ctrl.scale_pct(), SCREEN_SCALE_MIN_PCT);

    // 2. Dimension calculation with even constraint (minimum 2x2)
    assert_eq!(
        ScreenScaleController::scale_dims(1920, 1080, 100),
        (1920, 1080)
    );
    assert_eq!(
        ScreenScaleController::scale_dims(1920, 1080, 50),
        (960, 540)
    );
    assert_eq!(
        ScreenScaleController::scale_dims(1920, 1080, 75),
        (1440, 810)
    );
    assert_eq!(
        ScreenScaleController::scale_dims(1920, 1080, 25),
        (480, 270)
    );

    // Odd dimensions rounded to even
    let (w, h) = ScreenScaleController::scale_dims(1366, 768, 50);
    assert_eq!(w % 2, 0);
    assert_eq!(h % 2, 0);
    assert_eq!((w, h), (682, 384));

    // 3. Downscale raw frame
    let rect = ScreenRect::new(0, 0, 100, 100).unwrap();
    let mut bgra = vec![0u8; 100 * 100 * 4];
    for i in 0..100 * 100 {
        bgra[i * 4] = 255; // Blue
        bgra[i * 4 + 1] = 128; // Green
        bgra[i * 4 + 2] = 64; // Red
        bgra[i * 4 + 3] = 255; // Alpha
    }
    let frame = ScreenFrame::raw(42, rect, PixelFormat::Bgra8, bgra).unwrap();

    // 50% scale -> 50x50 frame
    let scaled = downscale_frame(&frame, 50).unwrap();
    assert_eq!(scaled.rect.w, 50);
    assert_eq!(scaled.rect.h, 50);
    assert_eq!(scaled.bytes.len(), 50 * 50 * 4);
    assert_eq!(scaled.seq, 42);
    // Verify color preservation in downscaled pixels
    assert_eq!(scaled.bytes[0], 255);
    assert_eq!(scaled.bytes[1], 128);
    assert_eq!(scaled.bytes[2], 64);
    assert_eq!(scaled.bytes[3], 255);

    // 100% scale is a no-op clone
    let native = downscale_frame(&frame, 100).unwrap();
    assert_eq!(native.rect.w, 100);
    assert_eq!(native.rect.h, 100);

    // 4. Sender integration and stats
    let mut s = ScreenSender::new();
    assert_eq!(s.scale_pct(), SCREEN_SCALE_DEFAULT_PCT);
    assert_eq!(s.stats().scale_pct, SCREEN_SCALE_DEFAULT_PCT);

    assert_eq!(s.set_scale_pct(50), 50);
    assert_eq!(s.scale_pct(), 50);
    assert_eq!(s.stats().scale_pct, 50);
}

#[test]
fn quality_controller_and_content_profiles() {
    let mut ctrl = ScreenQualityController::default();
    assert_eq!(ctrl.profile(), ScreenContentProfile::Text);
    assert_eq!(ctrl.min_qp(), SCREEN_QP_TEXT_DEFAULT.0);
    assert_eq!(ctrl.max_qp(), SCREEN_QP_TEXT_DEFAULT.1);
    assert_eq!(ctrl.qp_range(), SCREEN_QP_TEXT_DEFAULT);
    assert_eq!(ctrl.effective_profile(), ScreenContentProfile::Text);
    assert!(!ctrl.custom_qp());

    // 1. Switch to Motion profile updates default QP range to motion defaults
    assert_eq!(
        ctrl.set_profile(ScreenContentProfile::Motion),
        ScreenContentProfile::Motion
    );
    assert_eq!(ctrl.profile(), ScreenContentProfile::Motion);
    assert_eq!(ctrl.min_qp(), SCREEN_QP_MOTION_DEFAULT.0);
    assert_eq!(ctrl.max_qp(), SCREEN_QP_MOTION_DEFAULT.1);
    assert_eq!(ctrl.effective_profile(), ScreenContentProfile::Motion);

    // 2. Switch to Auto profile resolves effective profile to Text for desktop sharing
    assert_eq!(
        ctrl.set_profile(ScreenContentProfile::Auto),
        ScreenContentProfile::Auto
    );
    assert_eq!(ctrl.effective_profile(), ScreenContentProfile::Text);
    assert_eq!(ctrl.qp_range(), SCREEN_QP_TEXT_DEFAULT);

    // 3. Custom QP range configuration and clamping
    let custom = ctrl.set_qp_range(10, 30);
    assert_eq!(custom, (10, 30));
    assert_eq!(ctrl.qp_range(), (10, 30));
    assert!(ctrl.custom_qp());

    // Clamping invalid inputs (min > max clamped, bounds 0..51)
    let clamped = ctrl.set_qp_range(60, 20);
    assert_eq!(clamped, (51, 51));

    // 4. Reset default QP restores profile defaults
    let reset = ctrl.reset_default_qp();
    assert_eq!(reset, SCREEN_QP_TEXT_DEFAULT);
    assert!(!ctrl.custom_qp());

    // 5. String parsing and display
    assert_eq!(
        "text".parse::<ScreenContentProfile>().unwrap(),
        ScreenContentProfile::Text
    );
    assert_eq!(
        "desktop".parse::<ScreenContentProfile>().unwrap(),
        ScreenContentProfile::Text
    );
    assert_eq!(
        "motion".parse::<ScreenContentProfile>().unwrap(),
        ScreenContentProfile::Motion
    );
    assert_eq!(
        "video".parse::<ScreenContentProfile>().unwrap(),
        ScreenContentProfile::Motion
    );
    assert_eq!(
        "auto".parse::<ScreenContentProfile>().unwrap(),
        ScreenContentProfile::Auto
    );
    assert!("invalid".parse::<ScreenContentProfile>().is_err());
    assert_eq!(ScreenContentProfile::Text.to_string(), "text");
    assert_eq!(ScreenContentProfile::Motion.to_string(), "motion");
    assert_eq!(ScreenContentProfile::Auto.to_string(), "auto");

    // 6. Sender integration and stats
    let mut s = ScreenSender::new();
    assert_eq!(s.content_profile(), ScreenContentProfile::Text);
    assert_eq!(s.stats().content_profile(), ScreenContentProfile::Text);
    assert_eq!(s.stats().profile_tag, 0);
    assert_eq!(s.stats().qp_min, SCREEN_QP_TEXT_DEFAULT.0);
    assert_eq!(s.stats().qp_max, SCREEN_QP_TEXT_DEFAULT.1);

    assert_eq!(
        s.set_content_profile(ScreenContentProfile::Motion),
        ScreenContentProfile::Motion
    );
    assert_eq!(s.content_profile(), ScreenContentProfile::Motion);
    assert_eq!(s.stats().profile_tag, 1);
    assert_eq!(s.stats().content_profile(), ScreenContentProfile::Motion);
    assert_eq!(s.stats().qp_min, SCREEN_QP_MOTION_DEFAULT.0);
    assert_eq!(s.stats().qp_max, SCREEN_QP_MOTION_DEFAULT.1);

    s.set_qp_range(8, 28);
    assert_eq!(s.qp_range(), (8, 28));
    assert_eq!(s.stats().qp_min, 8);
    assert_eq!(s.stats().qp_max, 28);
}

#[test]
fn screen_playout_modes_and_target_delays() {
    let mut buf = ScreenPlayoutBuffer::new(ScreenPlayoutMode::Adaptive);
    assert_eq!(buf.mode(), ScreenPlayoutMode::Adaptive);
    assert_eq!(buf.target_delay_us(), SCREEN_PLAYOUT_ADAPTIVE_BASE_DELAY_US);

    // Switch to ZeroLatency
    buf.set_mode(ScreenPlayoutMode::ZeroLatency);
    assert_eq!(buf.mode(), ScreenPlayoutMode::ZeroLatency);
    assert_eq!(buf.target_delay_us(), SCREEN_PLAYOUT_MIN_DELAY_US);

    // Switch to Smooth
    buf.set_mode(ScreenPlayoutMode::Smooth);
    assert_eq!(buf.mode(), ScreenPlayoutMode::Smooth);
    assert_eq!(buf.target_delay_us(), SCREEN_PLAYOUT_SMOOTH_BASE_DELAY_US);

    // Custom delay override
    buf.set_custom_target_delay_us(Some(50_000));
    assert_eq!(buf.custom_target_delay_us(), Some(50_000));
    assert_eq!(buf.target_delay_us(), 50_000);

    // Clear custom delay restores mode default
    buf.set_custom_target_delay_us(None);
    assert_eq!(buf.custom_target_delay_us(), None);
    assert_eq!(buf.target_delay_us(), SCREEN_PLAYOUT_SMOOTH_BASE_DELAY_US);

    // Parsing and string conversion
    assert_eq!(
        ScreenPlayoutMode::parse("adaptive"),
        Some(ScreenPlayoutMode::Adaptive)
    );
    assert_eq!(
        ScreenPlayoutMode::parse("auto"),
        Some(ScreenPlayoutMode::Adaptive)
    );
    assert_eq!(
        ScreenPlayoutMode::parse("zerolatency"),
        Some(ScreenPlayoutMode::ZeroLatency)
    );
    assert_eq!(
        ScreenPlayoutMode::parse("zero"),
        Some(ScreenPlayoutMode::ZeroLatency)
    );
    assert_eq!(
        ScreenPlayoutMode::parse("smooth"),
        Some(ScreenPlayoutMode::Smooth)
    );
    assert_eq!(ScreenPlayoutMode::parse("invalid"), None);

    assert_eq!(ScreenPlayoutMode::from_tag(0), ScreenPlayoutMode::Adaptive);
    assert_eq!(
        ScreenPlayoutMode::from_tag(1),
        ScreenPlayoutMode::ZeroLatency
    );
    assert_eq!(ScreenPlayoutMode::from_tag(2), ScreenPlayoutMode::Smooth);
}

#[test]
fn screen_playout_jitter_calculation_and_pacing() {
    let mut buf = ScreenPlayoutBuffer::new(ScreenPlayoutMode::Adaptive);

    // 1. Initial frame arrival at t=100_000 us
    let f1 = raw_frame(1);
    buf.push_frame(f1, 100_000);
    assert_eq!(buf.queue_len(), 1);
    assert_eq!(buf.current_jitter_us(), 0);
    // Initial playout deadline = 100_000 + 15_000 = 115_000 us
    assert_eq!(buf.next_scheduled_playout_us(), Some(115_000));

    // Pop before scheduled playout returns WaitUntil
    match buf.pop_frame(110_000) {
        ScreenPlayoutAction::WaitUntil(ts) => assert_eq!(ts, 115_000),
        other => panic!("expected WaitUntil, got {other:?}"),
    }

    // Pop at or after scheduled playout returns Play
    match buf.pop_frame(115_000) {
        ScreenPlayoutAction::Play(f) => assert_eq!(f.seq, 1),
        other => panic!("expected Play, got {other:?}"),
    }
    assert_eq!(buf.queue_len(), 0);

    // 2. Second frame arrives with severe jitter (gap of 90ms instead of nominal 33.3ms)
    let f2 = raw_frame(2);
    buf.push_frame(f2, 190_000);
    // Interarrival = 90_000, nominal = 33_333, diff = 56_667. Jitter updated via EWMA.
    assert!(buf.current_jitter_us() > 0);
    // Adaptive target delay expands beyond base 15ms due to jitter
    assert!(buf.target_delay_us() > SCREEN_PLAYOUT_ADAPTIVE_BASE_DELAY_US);
}

#[test]
fn screen_playout_late_frame_dropping_and_overflow() {
    let mut buf = ScreenPlayoutBuffer::new(ScreenPlayoutMode::ZeroLatency);

    // Push 3 frames
    buf.push_frame(raw_frame(1), 10_000);
    buf.push_frame(raw_frame(2), 20_000);
    buf.push_frame(raw_frame(3), 30_000);

    // Advance clock way past late threshold (e.g. t = 500_000 us)
    // Frames 1 and 2 are hopelessly late; pop should drop late frames and play frame 3
    match buf.pop_frame(500_000) {
        ScreenPlayoutAction::Play(f) => assert_eq!(f.seq, 3),
        other => panic!("expected Play frame 3, got {other:?}"),
    }
    let st = buf.stats();
    assert_eq!(st.dropped_late_frames, 2);
    assert_eq!(st.played_frames, 1);

    // Overflow test: push 15 frames into max_capacity 12 buffer
    for i in 10..25 {
        buf.push_frame(raw_frame(i), 1_000_000 + (i - 10) * 10_000);
    }
    assert_eq!(buf.queue_len(), 12);
    assert_eq!(buf.stats().dropped_overflow_frames, 3);
}

#[test]
fn screen_receiver_playout_buffer_integration() {
    let mut recv = ScreenReceiver::with_playout_mode(ScreenPlayoutMode::Smooth);
    assert_eq!(recv.playout_mode(), ScreenPlayoutMode::Smooth);

    let f = raw_frame(1);
    let accepted = recv.apply(&f).unwrap();
    assert_eq!(accepted, Some(1));

    recv.push_playout(f, 50_000);
    assert_eq!(recv.playout_stats().buffered_frames, 1);

    let stats = recv.stats();
    assert_eq!(stats.playout_mode, ScreenPlayoutMode::Smooth as u8);
    assert_eq!(
        stats.playout_delay_us,
        SCREEN_PLAYOUT_SMOOTH_BASE_DELAY_US as u32
    );

    // On peer gone resets playout queue
    recv.on_peer_gone();
    assert_eq!(recv.playout_stats().buffered_frames, 0);
}

#[test]
fn screen_cursor_update_codec_and_roundtrip() {
    // 1. Basic standard cursor without custom bitmap
    let update1 = ScreenCursorUpdate {
        seq: 42,
        x: 1024,
        y: 768,
        visible: true,
        shape: CursorShape::Hand,
        hotspot_x: 6,
        hotspot_y: 2,
        width: 32,
        height: 32,
        custom_rgba: None,
    };

    let payload = update1.encode_payload().unwrap();
    assert_eq!(payload.len(), ScreenCursorUpdate::HEADER_LEN);
    let decoded1 = ScreenCursorUpdate::decode_payload(&payload).unwrap();
    assert_eq!(decoded1, update1);

    // InnerFrame conversion
    let frame = update1.to_frame().unwrap();
    assert_eq!(frame.ty, TYPE_SCREEN_CURSOR);
    let decoded_frame = ScreenCursorUpdate::from_frame(&frame).unwrap();
    assert_eq!(decoded_frame, update1);

    // 2. Custom cursor with RGBA bitmap payload
    let custom_bytes = vec![255u8; 16 * 16 * 4];
    let update2 = ScreenCursorUpdate {
        seq: 99,
        x: 500,
        y: 300,
        visible: true,
        shape: CursorShape::Custom,
        hotspot_x: 0,
        hotspot_y: 0,
        width: 16,
        height: 16,
        custom_rgba: Some(custom_bytes.clone()),
    };

    let payload2 = update2.encode_payload().unwrap();
    assert_eq!(
        payload2.len(),
        ScreenCursorUpdate::HEADER_LEN + custom_bytes.len()
    );
    let decoded2 = ScreenCursorUpdate::decode_payload(&payload2).unwrap();
    assert_eq!(decoded2, update2);

    // 3. Negative coordinates (multi-monitor topology)
    let update3 = ScreenCursorUpdate {
        seq: 101,
        x: -1920,
        y: -1080,
        visible: false,
        shape: CursorShape::IBeam,
        hotspot_x: 4,
        hotspot_y: 8,
        width: 16,
        height: 24,
        custom_rgba: None,
    };
    let payload3 = update3.encode_payload().unwrap();
    let decoded3 = ScreenCursorUpdate::decode_payload(&payload3).unwrap();
    assert_eq!(decoded3.x, -1920);
    assert_eq!(decoded3.y, -1080);
    assert!(!decoded3.visible);
    assert_eq!(decoded3.shape, CursorShape::IBeam);
}

#[test]
fn screen_cursor_controller_dedup_and_rate_limiting() {
    let mut ctrl = ScreenCursorController::new(ScreenCursorMode::Decoupled);
    assert_eq!(ctrl.mode(), ScreenCursorMode::Decoupled);
    assert_eq!(ctrl.min_interval_us(), SCREEN_CURSOR_MIN_INTERVAL_US);

    // 1. First update succeeds
    let u1 = ctrl.create_update(
        100,
        100,
        true,
        CursorShape::Arrow,
        (0, 0),
        (32, 32),
        None,
        10_000,
    );
    assert!(u1.is_some());
    assert_eq!(u1.unwrap().seq, 1);
    assert_eq!(ctrl.stats().updates_sent, 1);

    // 2. Exact position deduplication drops redundant call
    let u2 = ctrl.create_update(
        100,
        100,
        true,
        CursorShape::Arrow,
        (0, 0),
        (32, 32),
        None,
        10_000 + 50_000,
    );
    assert!(u2.is_none());
    assert_eq!(ctrl.stats().updates_dropped_dedup, 1);

    // 3. Movement before min_interval_us (8ms) elapses is rate-limited
    let u3 = ctrl.create_update(
        105,
        102,
        true,
        CursorShape::Arrow,
        (0, 0),
        (32, 32),
        None,
        10_000 + 4_000, // only 4ms elapsed
    );
    assert!(u3.is_none());
    assert_eq!(ctrl.stats().updates_dropped_rate, 1);

    // 4. Movement after min_interval_us elapses succeeds
    let u4 = ctrl.create_update(
        110,
        105,
        true,
        CursorShape::Arrow,
        (0, 0),
        (32, 32),
        None,
        10_000 + 10_000, // 10ms elapsed >= 8ms
    );
    assert!(u4.is_some());
    assert_eq!(u4.unwrap().seq, 2);
    assert_eq!(ctrl.stats().updates_sent, 2);

    // 5. Shape change bypasses rate limiting immediately
    let u5 = ctrl.create_update(
        112,
        106,
        true,
        CursorShape::Hand, // shape changed!
        (6, 2),
        (32, 32),
        None,
        10_000 + 11_000, // only 1ms after last send
    );
    assert!(u5.is_some());
    assert_eq!(u5.unwrap().shape, CursorShape::Hand);
    assert_eq!(ctrl.stats().updates_sent, 3);

    // 6. Embedded mode produces no outbound cursor updates
    ctrl.set_mode(ScreenCursorMode::Embedded);
    let u6 = ctrl.create_update(
        200,
        200,
        true,
        CursorShape::Arrow,
        (0, 0),
        (32, 32),
        None,
        1_000_000,
    );
    assert!(u6.is_none());
}

#[test]
fn screen_cursor_sender_and_receiver_integration() {
    let mut sender = ScreenSender::new();
    let mut receiver = ScreenReceiver::new();

    // 1. Initial default mode is decoupled
    assert_eq!(sender.cursor_mode(), ScreenCursorMode::Decoupled);
    assert_eq!(receiver.cursor_mode(), ScreenCursorMode::Decoupled);

    // Cursor push before streaming returns None
    assert!(sender
        .push_cursor_with_time(
            50,
            50,
            true,
            CursorShape::Arrow,
            (0, 0),
            (32, 32),
            None,
            1000
        )
        .is_none());

    // Connect, allow, and start streaming
    sender.on_peer_connected();
    sender.allow();
    sender.on_control(ControlMsg::Start).unwrap();
    assert!(sender.state().is_streaming());

    // 2. Push cursor update on sender
    let update = sender
        .push_cursor_with_time(
            300,
            400,
            true,
            CursorShape::IBeam,
            (4, 8),
            (16, 24),
            None,
            20_000,
        )
        .expect("should produce update");

    assert_eq!(update.x, 300);
    assert_eq!(update.y, 400);
    assert_eq!(update.shape, CursorShape::IBeam);
    assert_eq!(sender.stats().cursor_updates_sent, 1);
    assert_eq!(sender.stats().cursor_x, 300);
    assert_eq!(sender.stats().cursor_y, 400);

    // 3. Receive cursor update on receiver
    receiver.on_cursor(update.clone());
    assert_eq!(receiver.stats().cursor_updates_received, 1);
    assert_eq!(receiver.stats().cursor_x, 300);
    assert_eq!(receiver.stats().cursor_y, 400);
    assert_eq!(receiver.stats().cursor_shape, CursorShape::IBeam.tag());
    assert_eq!(receiver.current_cursor(), Some(&update));

    // 4. Mode switching updates stats
    sender.set_cursor_mode(ScreenCursorMode::Hidden);
    assert_eq!(sender.cursor_mode(), ScreenCursorMode::Hidden);
    assert_eq!(sender.stats().cursor_mode, ScreenCursorMode::Hidden.tag());

    receiver.set_cursor_mode(ScreenCursorMode::Embedded);
    assert_eq!(receiver.cursor_mode(), ScreenCursorMode::Embedded);
    assert_eq!(
        receiver.stats().cursor_mode,
        ScreenCursorMode::Embedded.tag()
    );

    // 5. On peer gone resets receiver cursor
    receiver.on_peer_gone();
    assert_eq!(receiver.current_cursor(), None);
}

#[test]
fn screen_damage_tracker_clean_and_dirty_eval() {
    let mut tracker = ScreenDamageTracker::new();
    let w = 128u32;
    let h = 128u32;
    let block_w = w.div_ceil(SCREEN_DAMAGE_BLOCK_SIZE as u32);
    let block_h = h.div_ceil(SCREEN_DAMAGE_BLOCK_SIZE as u32);
    let total_blocks = (block_w * block_h) as usize;
    assert_eq!(total_blocks, 16);

    // 1. Initial frame comparison with empty previous buffer -> full dirty
    let mut curr = vec![0x11u8; (w * h * 4) as usize];
    let rep1 = tracker.evaluate_rgba(&[], &curr, w, h);
    assert!(rep1.is_dirty());
    assert_eq!(rep1.num_dirty_blocks, 1); // fallback returns 1 whole dirty block covering full rect
    assert_eq!(rep1.dirty_pixels, 128 * 128);
    assert!((rep1.dirty_ratio - 1.0).abs() < 1e-4);
    assert_eq!(rep1.rect(), Some(ScreenDamageRect::new(0, 0, 128, 128)));

    // 2. Second frame comparison with identical buffer -> clean (zero dirty)
    let rep2 = tracker.evaluate_rgba(&curr, &curr, w, h);
    assert!(!rep2.is_dirty());
    assert_eq!(rep2.num_dirty_blocks, 0);
    assert_eq!(rep2.total_blocks, 16);
    assert_eq!(rep2.dirty_pixels, 0);
    assert_eq!(rep2.dirty_ratio, 0.0);
    assert_eq!(rep2.rect(), None);

    // 3. Modify single pixel in macroblock (1, 1) -> (x: 32..64, y: 32..64)
    let prev = curr.clone();
    let target_idx = ((35 * w + 40) * 4) as usize;
    curr[target_idx] = 0x99;
    let rep3 = tracker.evaluate_rgba(&prev, &curr, w, h);
    assert!(rep3.is_dirty());
    assert_eq!(rep3.num_dirty_blocks, 1);
    assert_eq!(rep3.total_blocks, 16);
    assert_eq!(rep3.dirty_pixels, 32 * 32);
    assert_eq!(rep3.dirty_ratio, 1.0 / 16.0);
    assert_eq!(rep3.rect(), Some(ScreenDamageRect::new(32, 32, 32, 32)));

    // 4. Modify top-left (0, 0) and bottom-right (127, 127) -> bounding box covers whole screen
    let prev = curr.clone();
    curr[0] = 0xAA;
    let last_idx = ((127 * w + 127) * 4) as usize;
    curr[last_idx] = 0xBB;
    let rep4 = tracker.evaluate_rgba(&prev, &curr, w, h);
    assert!(rep4.is_dirty());
    assert_eq!(rep4.num_dirty_blocks, 2);
    assert_eq!(rep4.rect(), Some(ScreenDamageRect::new(0, 0, 128, 128)));
}

#[test]
fn screen_squelch_modes_and_decisions() {
    // 1. Parsing and serialization
    assert_eq!(
        "auto".parse::<ScreenSquelchMode>().unwrap(),
        ScreenSquelchMode::Auto
    );
    assert_eq!(
        "aggressive".parse::<ScreenSquelchMode>().unwrap(),
        ScreenSquelchMode::Aggressive
    );
    assert_eq!(
        "agg".parse::<ScreenSquelchMode>().unwrap(),
        ScreenSquelchMode::Aggressive
    );
    assert_eq!(
        "disabled".parse::<ScreenSquelchMode>().unwrap(),
        ScreenSquelchMode::Disabled
    );
    assert_eq!(
        "off".parse::<ScreenSquelchMode>().unwrap(),
        ScreenSquelchMode::Disabled
    );
    assert!("invalid".parse::<ScreenSquelchMode>().is_err());
    assert_eq!(ScreenSquelchMode::from_tag(0), ScreenSquelchMode::Auto);
    assert_eq!(
        ScreenSquelchMode::from_tag(1),
        ScreenSquelchMode::Aggressive
    );
    assert_eq!(ScreenSquelchMode::from_tag(2), ScreenSquelchMode::Disabled);

    // 2. Auto mode lifecycle
    let mut ctrl = ScreenSquelchController::new(ScreenSquelchMode::Auto);
    let t0 = 1_000_000u64;

    // Dirty frame -> SendFrame
    assert_eq!(
        ctrl.evaluate_decision(true, 0.5, t0),
        ScreenSquelchDecision::SendFrame
    );
    assert!(!ctrl.stats().is_squelched);
    assert_eq!(ctrl.stats().frames_passed, 1);
    assert_eq!(ctrl.stats().consecutive_static_frames, 0);

    // Static frame within 500ms grace window (t0 + 200ms) -> SendFrame
    let t1 = t0 + 200_000;
    assert_eq!(
        ctrl.evaluate_decision(false, 0.0, t1),
        ScreenSquelchDecision::SendFrame
    );
    assert!(!ctrl.stats().is_squelched);
    assert_eq!(ctrl.stats().frames_passed, 2);
    assert_eq!(ctrl.stats().consecutive_static_frames, 1);

    // Static frame at t0 + 500ms -> Squelch activated, frame skipped
    let t2 = t0 + 500_000;
    assert_eq!(
        ctrl.evaluate_decision(false, 0.0, t2),
        ScreenSquelchDecision::SquelchSkip
    );
    assert!(ctrl.stats().is_squelched);
    assert_eq!(ctrl.stats().frames_squelched, 1);
    assert_eq!(ctrl.stats().consecutive_static_frames, 2);

    // Repeated static frames remain squelched
    for i in 1..10 {
        let t = t2 + i * 33_333;
        assert_eq!(
            ctrl.evaluate_decision(false, 0.0, t),
            ScreenSquelchDecision::SquelchSkip
        );
    }
    assert_eq!(ctrl.stats().frames_squelched, 10);

    // Keepalive heartbeat fires after 2.0s since last sent frame
    let t_keepalive = t0 + 200_000 + SCREEN_SQUELCH_AUTO_KEEPALIVE_US;
    assert_eq!(
        ctrl.evaluate_decision(false, 0.0, t_keepalive),
        ScreenSquelchDecision::SendFrame
    );
    assert_eq!(ctrl.stats().keepalive_frames, 1);
    assert_eq!(ctrl.stats().frames_passed, 3);
    assert!(ctrl.stats().is_squelched);

    // User input or force_wake awakens immediately
    ctrl.force_wake(t_keepalive + 10_000);
    assert!(!ctrl.stats().is_squelched);
    assert_eq!(
        ctrl.evaluate_decision(false, 0.0, t_keepalive + 10_000),
        ScreenSquelchDecision::SendFrame
    );

    // 3. Aggressive mode squelches after 150ms
    let mut agg = ScreenSquelchController::new(ScreenSquelchMode::Aggressive);
    assert_eq!(
        agg.evaluate_decision(true, 1.0, 100_000),
        ScreenSquelchDecision::SendFrame
    );
    assert_eq!(
        agg.evaluate_decision(false, 0.0, 100_000 + 100_000),
        ScreenSquelchDecision::SendFrame
    );
    // Squelched at 150ms
    assert_eq!(
        agg.evaluate_decision(false, 0.0, 100_000 + 150_000),
        ScreenSquelchDecision::SquelchSkip
    );
    assert!(agg.stats().is_squelched);

    // 4. Disabled mode never squelches
    let mut dis = ScreenSquelchController::new(ScreenSquelchMode::Disabled);
    assert_eq!(
        dis.evaluate_decision(true, 1.0, 100_000),
        ScreenSquelchDecision::SendFrame
    );
    assert_eq!(
        dis.evaluate_decision(false, 0.0, 100_000 + 600_000),
        ScreenSquelchDecision::SendFrame
    );
    assert!(!dis.stats().is_squelched);
}

#[test]
fn screen_sender_squelch_integration() {
    let mut s = ScreenSender::new();
    assert_eq!(s.squelch_mode(), ScreenSquelchMode::Auto);
    assert_eq!(s.stats().squelch_mode, ScreenSquelchMode::Auto.tag());

    // Mode mutation
    s.set_squelch_mode(ScreenSquelchMode::Aggressive);
    assert_eq!(s.squelch_mode(), ScreenSquelchMode::Aggressive);
    assert_eq!(s.stats().squelch_mode, ScreenSquelchMode::Aggressive.tag());

    s.set_squelch_mode(ScreenSquelchMode::Auto);
    s.on_peer_connected();
    s.allow();
    s.on_control(ControlMsg::Start).unwrap();
    assert!(s.state().is_streaming());

    let rect = ScreenRect::new(0, 0, 64, 64).unwrap();
    let frame = ScreenFrame::raw(1, rect, PixelFormat::Bgra8, vec![0u8; 64 * 64 * 4]).unwrap();

    // 1. Initial dirty push
    let seq = s.push_with_damage_time(frame.clone(), true, 1.0, 1_000_000);
    assert_eq!(seq, Some(1));
    assert_eq!(s.stats().sent, 1);
    assert!(!s.stats().is_squelched);

    // 2. Static push within 500ms
    let seq = s.push_with_damage_time(frame.clone(), false, 0.0, 1_200_000);
    assert_eq!(seq, Some(2));
    assert_eq!(s.stats().sent, 2);

    // 3. Static push past 500ms -> Squelch skips
    let seq = s.push_with_damage_time(frame.clone(), false, 0.0, 1_600_000);
    assert_eq!(seq, None);
    assert_eq!(s.stats().sent, 2);
    assert!(s.stats().is_squelched);
    assert_eq!(s.stats().squelched_frames, 1);

    // 4. On-demand IDR keyframe wakes squelch
    s.on_control_with_time(ControlMsg::KeyframeRequest, 1_605_000)
        .unwrap();
    assert!(!s.stats().is_squelched);
    let seq = s.push_with_damage_time(frame.clone(), false, 0.0, 1_610_000);
    assert_eq!(seq, Some(3));
}

#[test]
fn gf_arithmetic_and_cauchy_matrix() {
    // 1. Inverses over GF(2^8)
    for a in 1..=255u8 {
        let inv = gf_inv(a);
        assert_ne!(inv, 0);
        assert_eq!(gf_mul(a, inv), 1, "gf_mul failed for a={a}");
    }
    assert_eq!(gf_inv(0), 0);

    // 2. Division
    assert_eq!(gf_div(100, 100), 1);
    assert_eq!(gf_div(42, 1), 42);
    assert_eq!(gf_div(0, 50), 0);

    // 3. Cauchy matrix non-zero and invertible submatrices
    let k = 4;
    let m = 2;
    for r in 0..m {
        for c in 0..k {
            let coeff = cauchy_coeff(r, c, k);
            assert_ne!(coeff, 0);
        }
    }
}

#[test]
fn screen_fec_encoder_decoder_recovery() {
    let frame_seq = 42;
    let block_id = 1;
    let data_shards = vec![
        b"Hello World - Shard 0 payload".to_vec(),
        b"Second buffer of variable length 12345".to_vec(),
        b"Short".to_vec(),
        b"The fourth and final data shard in this FEC test block!".to_vec(),
    ];
    let k = data_shards.len(); // 4
    let m = 2; // 2 parity shards (can tolerate any 2 lost shards)

    let shards = ScreenFecEncoder::encode_block(frame_seq, block_id, &data_shards, m).unwrap();
    assert_eq!(shards.len(), k + m); // 6 shards total

    // Case A: Zero loss (all data shards present)
    let dec_a = ScreenFecDecoder::decode_block(&shards[0..k]).unwrap();
    assert_eq!(dec_a, Some(data_shards.clone()));

    // Case B: 1 data shard lost (shard 1 missing, replaced by parity shard 0)
    let loss_1 = vec![
        shards[0].clone(),
        shards[2].clone(),
        shards[3].clone(),
        shards[4].clone(), // parity 0
    ];
    let dec_b = ScreenFecDecoder::decode_block(&loss_1).unwrap();
    assert_eq!(dec_b, Some(data_shards.clone()));

    // Case C: 2 data shards lost (shards 0 and 3 missing, parity 0 and 1 present)
    let loss_2 = vec![
        shards[1].clone(),
        shards[2].clone(),
        shards[4].clone(), // parity 0
        shards[5].clone(), // parity 1
    ];
    let dec_c = ScreenFecDecoder::decode_block(&loss_2).unwrap();
    assert_eq!(dec_c, Some(data_shards.clone()));

    // Case D: 3 data shards lost (only 3 total shards received, need 4) -> unrecoverable (None)
    let loss_3 = vec![shards[1].clone(), shards[4].clone(), shards[5].clone()];
    let dec_d = ScreenFecDecoder::decode_block(&loss_3).unwrap();
    assert_eq!(dec_d, None);
}

#[test]
fn screen_fec_shard_wire_codec() {
    let shard = ScreenFecShard {
        frame_seq: 12345678,
        block_id: 7,
        k_data_shards: 4,
        m_parity_shards: 2,
        shard_index: 1,
        is_parity: true,
        transport_seq: 101,
        data: vec![0xDE, 0xAD, 0xBE, 0xEF, 0x01, 0x02],
    };

    let inner = shard.to_inner_frame();
    assert_eq!(inner.ty, TYPE_SCREEN_FEC);

    let decoded = ScreenFecShard::from_inner_frame(&inner).unwrap();
    assert_eq!(decoded, shard);
}

#[test]
fn screen_fec_controller_redundancy_adaptation() {
    // 1. Parsing & Modes
    assert_eq!(
        "auto".parse::<ScreenFecMode>().unwrap(),
        ScreenFecMode::Auto
    );
    assert_eq!(
        "conservative".parse::<ScreenFecMode>().unwrap(),
        ScreenFecMode::Conservative
    );
    assert_eq!(
        "aggressive".parse::<ScreenFecMode>().unwrap(),
        ScreenFecMode::Aggressive
    );
    assert_eq!(
        "disabled".parse::<ScreenFecMode>().unwrap(),
        ScreenFecMode::Disabled
    );

    // 2. Disabled mode
    let mut ctrl = ScreenFecController::new(ScreenFecMode::Disabled);
    assert_eq!(ctrl.calculate_parity_shards(4, 0.10), 0);
    assert_eq!(ctrl.stats().redundancy_pct, 0);

    // 3. Conservative mode (10%)
    let mut cons = ScreenFecController::new(ScreenFecMode::Conservative);
    assert_eq!(cons.calculate_parity_shards(10, 0.0), 1);
    assert_eq!(cons.stats().redundancy_pct, 10);

    // 4. Aggressive mode (25%)
    let mut agg = ScreenFecController::new(ScreenFecMode::Aggressive);
    assert_eq!(agg.calculate_parity_shards(8, 0.0), 2);
    assert_eq!(agg.stats().redundancy_pct, 25);

    // 5. Auto mode adaptation
    let mut auto = ScreenFecController::new(ScreenFecMode::Auto);
    // < 0.5% loss -> 0% redundancy
    assert_eq!(auto.calculate_parity_shards(10, 0.002), 0);
    // 0.5%~3% loss -> 10% redundancy
    assert_eq!(auto.calculate_parity_shards(10, 0.015), 1);
    // 3%~8% loss -> 20% redundancy
    assert_eq!(auto.calculate_parity_shards(10, 0.05), 2);
    // >= 8% loss -> 30% redundancy
    assert_eq!(auto.calculate_parity_shards(10, 0.12), 3);
}

#[test]
fn screen_fec_assembler_and_sender_receiver_flow() {
    let mut sender = ScreenSender::new();
    sender.set_fec_mode(ScreenFecMode::Aggressive);
    sender.on_peer_connected();
    sender.allow();
    sender.on_control(ControlMsg::Start).unwrap();

    let rect = ScreenRect::new(0, 0, 100, 100).unwrap();
    let frame = ScreenFrame::raw(10, rect, PixelFormat::Bgra8, vec![0x77; 100 * 100 * 4]).unwrap();

    // Encode frame to FEC inner frames
    let inners = sender.encode_frame_to_inner_frames(&frame, 0.0).unwrap();
    assert!(!inners.is_empty());
    assert_eq!(inners[0].ty, TYPE_SCREEN_FEC);

    let shards: Vec<ScreenFecShard> = inners
        .iter()
        .map(|f| ScreenFecShard::from_inner_frame(f).unwrap())
        .collect();

    // Receiver assembly with 1 dropped data shard
    let mut receiver = ScreenReceiver::new();
    assert_eq!(receiver.stats().fec_recovered_shards, 0);

    let mut reconstructed_frame = None;
    for (i, shard) in shards.into_iter().enumerate() {
        // Intentionally skip shard 0 (simulate network loss)
        if i == 0 {
            continue;
        }
        if let Some(f) = receiver.on_fec_shard(shard).unwrap() {
            reconstructed_frame = Some(f);
            break;
        }
    }

    assert!(reconstructed_frame.is_some());
    let rf = reconstructed_frame.unwrap();
    assert_eq!(rf.seq, frame.seq);
    assert_eq!(rf.bytes.len(), frame.bytes.len());
    assert_eq!(receiver.stats().fec_recovered_shards, 1);
}

#[test]
fn screen_packet_pacer_burst_smoothing_and_modes() {
    // 1. Parsing and serialization
    assert_eq!(
        "auto".parse::<ScreenPacerMode>().unwrap(),
        ScreenPacerMode::Auto
    );
    assert_eq!(
        "aggressive".parse::<ScreenPacerMode>().unwrap(),
        ScreenPacerMode::Aggressive
    );
    assert_eq!(
        "agg".parse::<ScreenPacerMode>().unwrap(),
        ScreenPacerMode::Aggressive
    );
    assert_eq!(
        "disabled".parse::<ScreenPacerMode>().unwrap(),
        ScreenPacerMode::Disabled
    );
    assert_eq!(
        "off".parse::<ScreenPacerMode>().unwrap(),
        ScreenPacerMode::Disabled
    );
    assert!("invalid".parse::<ScreenPacerMode>().is_err());
    assert_eq!(ScreenPacerMode::from_tag(0), ScreenPacerMode::Auto);
    assert_eq!(ScreenPacerMode::from_tag(1), ScreenPacerMode::Aggressive);
    assert_eq!(ScreenPacerMode::from_tag(2), ScreenPacerMode::Disabled);

    // 2. Auto mode pacing (burst limit = 4, target 30 FPS -> frame_duration 33333 us, budget = 26666 us)
    let mut pacer = ScreenPacketPacer::new(ScreenPacerMode::Auto);
    assert_eq!(pacer.mode(), ScreenPacerMode::Auto);

    // Under burst limit: 3 packets -> all zero delay
    let delays_under = pacer.compute_packet_delays(3, 30);
    assert_eq!(delays_under, vec![0, 0, 0]);
    assert_eq!(pacer.stats().packets_paced, 3);
    assert_eq!(pacer.stats().bursts_smoothed, 0);

    // Over burst limit: 8 packets -> 4 burst packets at 0, 4 paced packets with increasing delay
    let delays_over = pacer.compute_packet_delays(8, 30);
    assert_eq!(delays_over.len(), 8);
    assert_eq!(&delays_over[0..4], &[0, 0, 0, 0]);
    assert!(delays_over[4] > 0);
    assert!(delays_over[5] > delays_over[4]);
    assert!(delays_over[6] > delays_over[5]);
    assert!(delays_over[7] > delays_over[6]);
    assert!(delays_over[7] <= SCREEN_PACER_MAX_DELAY_US);
    assert_eq!(pacer.stats().bursts_smoothed, 1);
    assert_eq!(pacer.stats().packets_paced, 11);
    assert!(pacer.stats().avg_delay_us > 0);

    // 3. Aggressive mode (burst limit = 2)
    pacer.set_mode(ScreenPacerMode::Aggressive);
    let agg_delays = pacer.compute_packet_delays(5, 60);
    assert_eq!(agg_delays.len(), 5);
    assert_eq!(&agg_delays[0..2], &[0, 0]);
    assert!(agg_delays[2] > 0);
    assert!(agg_delays[3] > agg_delays[2]);
    assert!(agg_delays[4] > agg_delays[3]);

    // 4. Disabled mode -> all 0
    pacer.set_mode(ScreenPacerMode::Disabled);
    let dis_delays = pacer.compute_packet_delays(10, 30);
    assert_eq!(dis_delays, vec![0; 10]);

    // 5. Sender integration test
    let mut sender = ScreenSender::new();
    assert_eq!(sender.pacer_mode(), ScreenPacerMode::Auto);
    let sender_delays = sender.compute_packet_delays(6);
    assert_eq!(sender_delays.len(), 6);
    assert_eq!(&sender_delays[0..4], &[0, 0, 0, 0]);
    assert!(sender_delays[4] > 0);
    assert_eq!(sender.stats().pacer_mode, ScreenPacerMode::Auto.tag());
    assert_eq!(sender.stats().pacer_bursts_smoothed, 1);
    assert_eq!(sender.stats().pacer_packets_paced, 6);
}

#[test]
fn screen_trendline_filter_bufferbloat_detection_and_modes() {
    // 1. Parsing and enum tags
    assert_eq!(
        "auto".parse::<ScreenTrendlineMode>().unwrap(),
        ScreenTrendlineMode::Auto
    );
    assert_eq!(
        "aggressive".parse::<ScreenTrendlineMode>().unwrap(),
        ScreenTrendlineMode::Aggressive
    );
    assert_eq!(
        "agg".parse::<ScreenTrendlineMode>().unwrap(),
        ScreenTrendlineMode::Aggressive
    );
    assert_eq!(
        "conservative".parse::<ScreenTrendlineMode>().unwrap(),
        ScreenTrendlineMode::Conservative
    );
    assert_eq!(
        "cons".parse::<ScreenTrendlineMode>().unwrap(),
        ScreenTrendlineMode::Conservative
    );
    assert_eq!(
        "disabled".parse::<ScreenTrendlineMode>().unwrap(),
        ScreenTrendlineMode::Disabled
    );
    assert_eq!(
        "off".parse::<ScreenTrendlineMode>().unwrap(),
        ScreenTrendlineMode::Disabled
    );
    assert!("unknown".parse::<ScreenTrendlineMode>().is_err());

    assert_eq!(ScreenTrendlineMode::from_tag(0), ScreenTrendlineMode::Auto);
    assert_eq!(
        ScreenTrendlineMode::from_tag(1),
        ScreenTrendlineMode::Aggressive
    );
    assert_eq!(
        ScreenTrendlineMode::from_tag(2),
        ScreenTrendlineMode::Conservative
    );
    assert_eq!(
        ScreenTrendlineMode::from_tag(3),
        ScreenTrendlineMode::Disabled
    );

    assert_eq!(
        ScreenBandwidthUsageState::from_tag(0),
        ScreenBandwidthUsageState::Normal
    );
    assert_eq!(
        ScreenBandwidthUsageState::from_tag(1),
        ScreenBandwidthUsageState::Overusing
    );
    assert_eq!(
        ScreenBandwidthUsageState::from_tag(2),
        ScreenBandwidthUsageState::Underusing
    );
    assert_eq!(ScreenBandwidthUsageState::Normal.as_str(), "normal");
    assert_eq!(ScreenBandwidthUsageState::Overusing.as_str(), "overusing");
    assert_eq!(ScreenBandwidthUsageState::Underusing.as_str(), "underusing");

    // 2. Linear regression & constant delay -> normal state, slope ~ 0
    let mut filter = ScreenTrendlineFilter::new(ScreenTrendlineMode::Auto);
    assert_eq!(filter.mode(), ScreenTrendlineMode::Auto);
    assert_eq!(filter.state(), ScreenBandwidthUsageState::Normal);

    let frame_interval_us = 33_333; // ~30 fps
    let mut send_ts = 1_000_000;
    let rtt = 30_000; // 30ms

    for _ in 0..15 {
        let recv_ts = send_ts + rtt;
        let state = filter.on_ack(send_ts, recv_ts, Some(rtt));
        assert_eq!(state, ScreenBandwidthUsageState::Normal);
        send_ts += frame_interval_us;
    }
    assert_eq!(filter.stats().overuse_count, 0);
    assert_eq!(filter.stats().underuse_count, 0);
    assert!(filter.stats().current_slope_us_per_frame.abs() < 1.0);

    // 3. Bufferbloat queue buildup: increasing delay (+200 us per frame) -> triggers Overusing
    let mut queue_delay = rtt;
    let mut reached_overuse = false;
    for _ in 0..20 {
        queue_delay += 200;
        let recv_ts = send_ts + queue_delay;
        let state = filter.on_ack(send_ts, recv_ts, Some(queue_delay));
        if state == ScreenBandwidthUsageState::Overusing {
            reached_overuse = true;
        }
        send_ts += frame_interval_us;
    }
    assert!(reached_overuse);
    assert_eq!(filter.state(), ScreenBandwidthUsageState::Overusing);
    assert!(filter.stats().overuse_count >= 1);
    assert!(filter.stats().current_slope_us_per_frame > 10.0);

    // 4. Queue draining: decreasing delay (-200 us per frame) -> triggers Underusing
    let mut reached_underuse = false;
    for _ in 0..25 {
        queue_delay = queue_delay.saturating_sub(200);
        let recv_ts = send_ts + queue_delay;
        let state = filter.on_ack(send_ts, recv_ts, Some(queue_delay));
        if state == ScreenBandwidthUsageState::Underusing {
            reached_underuse = true;
        }
        send_ts += frame_interval_us;
    }
    assert!(reached_underuse);
    assert!(filter.stats().underuse_count >= 1);
    assert!(filter.stats().current_slope_us_per_frame < -10.0);

    // 5. Threshold adaptation across modes
    let mut filter_auto = ScreenTrendlineFilter::new(ScreenTrendlineMode::Auto);
    // Low RTT (50ms): factor clamp 0.5 -> threshold = 12.5 * 0.5 = 6.25
    filter_auto.on_ack(1_000_000, 1_050_000, Some(50_000));
    assert!((filter_auto.stats().threshold_us_per_frame - 6.25).abs() < 0.01);
    // High RTT (200ms): factor 2.0 -> threshold = 12.5 * 2.0 = 25.0
    filter_auto.on_ack(2_000_000, 2_200_000, Some(200_000));
    assert!((filter_auto.stats().threshold_us_per_frame - 25.0).abs() < 0.01);

    let mut filter_agg = ScreenTrendlineFilter::new(ScreenTrendlineMode::Aggressive);
    filter_agg.on_ack(1_000_000, 1_050_000, Some(50_000));
    assert!((filter_agg.stats().threshold_us_per_frame - 6.0).abs() < 0.01);

    let mut filter_cons = ScreenTrendlineFilter::new(ScreenTrendlineMode::Conservative);
    filter_cons.on_ack(1_000_000, 1_050_000, Some(50_000));
    assert!((filter_cons.stats().threshold_us_per_frame - 25.0).abs() < 0.01);

    let mut filter_dis = ScreenTrendlineFilter::new(ScreenTrendlineMode::Disabled);
    let dis_state = filter_dis.on_ack(1_000_000, 1_500_000, Some(500_000));
    assert_eq!(dis_state, ScreenBandwidthUsageState::Normal);

    // 6. Sender integration test
    let mut sender = ScreenSender::new();
    assert_eq!(sender.trendline_mode(), ScreenTrendlineMode::Auto);
    sender.set_trendline_mode(ScreenTrendlineMode::Aggressive);
    assert_eq!(sender.trendline_mode(), ScreenTrendlineMode::Aggressive);
    assert_eq!(
        sender.stats().trendline_mode,
        ScreenTrendlineMode::Aggressive.tag()
    );
    assert_eq!(
        sender.stats().trendline_state,
        ScreenBandwidthUsageState::Normal.tag()
    );
}
