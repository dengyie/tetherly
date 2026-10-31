// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Unit tests for screen streaming pipelines (ADR-030 ~ ADR-126).

use super::*;
use crate::error::CoreError;
use std::collections::{HashMap, VecDeque};

#[test]
fn screen_harq_selective_nack_retransmission_pipeline() {
    // 1. Parsing and enum tags for ScreenHarqMode
    assert_eq!(
        "auto".parse::<ScreenHarqMode>().unwrap(),
        ScreenHarqMode::Auto
    );
    assert_eq!(
        "aggressive".parse::<ScreenHarqMode>().unwrap(),
        ScreenHarqMode::Aggressive
    );
    assert_eq!(
        "agg".parse::<ScreenHarqMode>().unwrap(),
        ScreenHarqMode::Aggressive
    );
    assert_eq!(
        "conservative".parse::<ScreenHarqMode>().unwrap(),
        ScreenHarqMode::Conservative
    );
    assert_eq!(
        "cons".parse::<ScreenHarqMode>().unwrap(),
        ScreenHarqMode::Conservative
    );
    assert_eq!(
        "disabled".parse::<ScreenHarqMode>().unwrap(),
        ScreenHarqMode::Disabled
    );
    assert_eq!(
        "off".parse::<ScreenHarqMode>().unwrap(),
        ScreenHarqMode::Disabled
    );
    assert!("unknown".parse::<ScreenHarqMode>().is_err());

    assert_eq!(ScreenHarqMode::from_tag(0), ScreenHarqMode::Auto);
    assert_eq!(ScreenHarqMode::from_tag(1), ScreenHarqMode::Aggressive);
    assert_eq!(ScreenHarqMode::from_tag(2), ScreenHarqMode::Conservative);
    assert_eq!(ScreenHarqMode::from_tag(3), ScreenHarqMode::Disabled);
    assert_eq!(ScreenHarqMode::Auto.as_str(), "auto");
    assert_eq!(ScreenHarqMode::Aggressive.as_str(), "aggressive");
    assert_eq!(ScreenHarqMode::Conservative.as_str(), "conservative");
    assert_eq!(ScreenHarqMode::Disabled.as_str(), "disabled");

    // 2. ScreenNack wire protocol encode & decode round-trip
    let nack = ScreenNack {
        frame_seq: 42,
        block_id: 3,
        missing_mask: 0b1010,
        missing_indices: vec![1, 3],
        is_fec: true,
        requires_keyframe_fallback: false,
        rtt_estimate_us: 25_000,
    };
    let encoded = nack.encode();
    assert_eq!(&encoded[0..5], &SCREEN_NACK_MAGIC);
    assert_eq!(encoded[5], SCREEN_NACK_PROTO);

    let decoded = ScreenNack::decode(&encoded).expect("decode nack");
    assert_eq!(decoded.frame_seq, 42);
    assert_eq!(decoded.block_id, 3);
    assert_eq!(decoded.missing_mask, 0b1010);
    assert_eq!(decoded.missing_indices, vec![1, 3]);
    assert!(decoded.is_fec);
    assert!(!decoded.requires_keyframe_fallback);
    assert_eq!(decoded.rtt_estimate_us, 25_000);

    // Corrupt magic rejection
    let mut corrupt = encoded.clone();
    corrupt[0] = b'X';
    assert!(ScreenNack::decode(&corrupt).is_err());

    // Corrupt length rejection
    assert!(ScreenNack::decode(&encoded[0..20]).is_err());

    // InnerFrame packaging round-trip
    let inner = nack.to_inner().expect("to inner");
    assert_eq!(inner.ty, TYPE_SCREEN_NACK);
    let decoded_inner = ScreenNack::decode(&inner.payload).expect("decode inner payload");
    assert_eq!(decoded_inner, nack);

    // 3. ScreenRetransmitBuffer sliding window and selective retry limits
    let mut buf = ScreenRetransmitBuffer::new(ScreenHarqMode::Auto);
    assert_eq!(
        buf.stats().buffer_capacity,
        SCREEN_HARQ_DEFAULT_BUFFER_CAPACITY
    );
    assert_eq!(buf.len(), 0);

    let shard0 = ScreenFecShard {
        frame_seq: 100,
        block_id: 1,
        k_data_shards: 4,
        m_parity_shards: 2,
        shard_index: 0,
        is_parity: false,
        transport_seq: 1,
        data: vec![10; 100],
    };
    let shard1 = ScreenFecShard {
        shard_index: 1,
        data: vec![20; 100],
        ..shard0.clone()
    };
    let shard2 = ScreenFecShard {
        shard_index: 2,
        data: vec![30; 100],
        ..shard0.clone()
    };

    let f0 = shard0.to_inner_frame();
    let f1 = shard1.to_inner_frame();
    let f2 = shard2.to_inner_frame();

    buf.record_outbound_frame(&f0, 1_000_000);
    buf.record_outbound_frame(&f1, 1_000_000);
    buf.record_outbound_frame(&f2, 1_000_000);
    assert_eq!(buf.len(), 3);

    // NACK requesting missing shard 1
    let req_shard1 = ScreenNack {
        frame_seq: 100,
        block_id: 1,
        missing_mask: 0b0010,
        missing_indices: vec![1],
        is_fec: true,
        requires_keyframe_fallback: false,
        rtt_estimate_us: 10_000,
    };
    let retx = buf.handle_nack(&req_shard1, 1_015_000);
    assert_eq!(retx.len(), 1);
    let retx_shard = ScreenFecShard::from_inner_frame(&retx[0]).unwrap();
    assert_eq!(retx_shard.shard_index, 1);
    assert_eq!(retx_shard.data, vec![20; 100]);
    assert_eq!(buf.stats().shards_retransmitted, 1);
    assert_eq!(buf.stats().nacks_received, 1);

    // Second retry under Auto mode (retry ceiling = 2)
    let retx2 = buf.handle_nack(&req_shard1, 1_030_000);
    assert_eq!(retx2.len(), 1);
    assert_eq!(buf.stats().shards_retransmitted, 2);

    // Third retry exceeds Auto mode ceiling (2) -> refused/timeouts incremented
    let retx3 = buf.handle_nack(&req_shard1, 1_045_000);
    assert_eq!(retx3.len(), 0);
    assert_eq!(buf.stats().retransmit_timeouts, 1);

    // Age expiration (> 300ms)
    let req_shard0 = ScreenNack {
        frame_seq: 100,
        block_id: 1,
        missing_mask: 0b0001,
        missing_indices: vec![0],
        is_fec: true,
        requires_keyframe_fallback: false,
        rtt_estimate_us: 10_000,
    };
    let retx_expired = buf.handle_nack(&req_shard0, 1_400_000); // 400ms later
    assert_eq!(retx_expired.len(), 0);
    assert_eq!(buf.stats().retransmit_timeouts, 2);

    // 4. Receiver NACK generation and RTT debounce
    let mut receiver = ScreenReceiver::with_harq_mode(ScreenHarqMode::Auto);
    assert_eq!(receiver.harq_mode(), ScreenHarqMode::Auto);

    // Ingest shard 0 (k=2, m=1), missing shard 1
    let r_shard0 = ScreenFecShard {
        frame_seq: 200,
        block_id: 0,
        k_data_shards: 2,
        m_parity_shards: 1,
        shard_index: 0,
        is_parity: false,
        transport_seq: 10,
        data: vec![55; 100],
    };
    let _ = receiver.on_fec_shard(r_shard0);

    let nack1 = receiver.generate_nack_on_fec(2_000_000, 20_000);
    assert!(nack1.is_some());
    let nack1 = nack1.unwrap();
    assert_eq!(nack1.frame_seq, 200);
    assert_eq!(nack1.block_id, 0);
    assert_eq!(nack1.missing_indices, vec![1, 2]); // missing shard 1 and parity 0 (idx 2)
    assert_eq!(receiver.harq_stats().nacks_sent, 1);

    // Immediate duplicate NACK within RTT window (< 20ms) is debounced
    let nack_dup = receiver.generate_nack_on_fec(2_005_000, 20_000);
    assert!(nack_dup.is_none());
    assert_eq!(receiver.harq_stats().nacks_sent, 1);

    // After RTT window passes (> 20ms) -> NACK allowed again
    let nack2 = receiver.generate_nack_on_fec(2_025_000, 20_000);
    assert!(nack2.is_some());
    assert_eq!(receiver.harq_stats().nacks_sent, 2);

    // 5. Sender end-to-end integration
    let mut sender = ScreenSender::with_harq(ScreenHarqMode::Auto);
    let sf = ScreenFrame {
        seq: 300,
        bytes: vec![123; 500],
        rect: ScreenRect {
            x: 0,
            y: 0,
            w: 10,
            h: 10,
        },
        format: PixelFormat::Bgra8,
        codec_tag: SCREEN_COMPRESS_NONE,
        stride: 40,
    };
    let encoded_frames = sender
        .encode_frame_to_inner_frames_with_time(&sf, 0.05, 3_000_000)
        .expect("encode with time");
    assert!(!encoded_frames.is_empty());
    assert!(!sender.retransmit_buffer().is_empty());

    let fallback_nack = ScreenNack {
        frame_seq: 300,
        block_id: 0,
        missing_mask: 0b1111,
        missing_indices: vec![0, 1, 2, 3],
        is_fec: true,
        requires_keyframe_fallback: true,
        rtt_estimate_us: 15_000,
    };
    let _ = sender.handle_nack(&fallback_nack, 3_010_000);
    assert_eq!(sender.stats().harq_nacks_received, 1);
    assert!(!sender.stats().is_squelched);
}

#[test]
fn screen_twcc_transport_wide_congestion_control_pipeline() {
    // 1. Parsing and enum tags for ScreenTwccMode
    assert_eq!(
        "auto".parse::<ScreenTwccMode>().unwrap(),
        ScreenTwccMode::Auto
    );
    assert_eq!(
        "aggressive".parse::<ScreenTwccMode>().unwrap(),
        ScreenTwccMode::Aggressive
    );
    assert_eq!(
        "agg".parse::<ScreenTwccMode>().unwrap(),
        ScreenTwccMode::Aggressive
    );
    assert_eq!(
        "conservative".parse::<ScreenTwccMode>().unwrap(),
        ScreenTwccMode::Conservative
    );
    assert_eq!(
        "cons".parse::<ScreenTwccMode>().unwrap(),
        ScreenTwccMode::Conservative
    );
    assert_eq!(
        "disabled".parse::<ScreenTwccMode>().unwrap(),
        ScreenTwccMode::Disabled
    );
    assert_eq!(
        "off".parse::<ScreenTwccMode>().unwrap(),
        ScreenTwccMode::Disabled
    );
    assert!("unknown".parse::<ScreenTwccMode>().is_err());

    assert_eq!(ScreenTwccMode::from_tag(0), ScreenTwccMode::Auto);
    assert_eq!(ScreenTwccMode::from_tag(1), ScreenTwccMode::Conservative);
    assert_eq!(ScreenTwccMode::from_tag(2), ScreenTwccMode::Aggressive);
    assert_eq!(ScreenTwccMode::from_tag(3), ScreenTwccMode::Disabled);

    assert_eq!(ScreenTwccMode::Auto.interval_us(), 10_000);
    assert_eq!(ScreenTwccMode::Aggressive.interval_us(), 5_000);
    assert_eq!(ScreenTwccMode::Conservative.interval_us(), 25_000);
    assert_eq!(ScreenTwccMode::Disabled.interval_us(), u64::MAX);

    assert_eq!(ScreenTwccMode::Auto.packet_threshold(), 40);
    assert_eq!(ScreenTwccMode::Aggressive.packet_threshold(), 20);
    assert_eq!(ScreenTwccMode::Conservative.packet_threshold(), 80);
    assert_eq!(ScreenTwccMode::Disabled.packet_threshold(), usize::MAX);

    // 2. ScreenTwccFeedback wire protocol codec
    let fb = ScreenTwccFeedback {
        feedback_seq: 12,
        base_seq: 100,
        packet_count: 3,
        reference_time_us: 1_000_000,
        records: vec![
            ScreenTwccPacketRecord {
                transport_seq: 100,
                recv_delta_us: 0,
            },
            ScreenTwccPacketRecord {
                transport_seq: 101,
                recv_delta_us: 250,
            },
            ScreenTwccPacketRecord {
                transport_seq: 102,
                recv_delta_us: SCREEN_TWCC_MISSING_DELTA, // lost packet
            },
        ],
    };
    let encoded = fb.encode();
    assert_eq!(&encoded[0..5], &SCREEN_TWCC_MAGIC);
    assert_eq!(encoded[5], SCREEN_TWCC_PROTO);
    assert_eq!(
        encoded.len(),
        SCREEN_TWCC_HEADER_LEN + 3 * SCREEN_TWCC_RECORD_LEN
    );

    let decoded = ScreenTwccFeedback::decode(&encoded).expect("decode twcc");
    assert_eq!(decoded, fb);

    // Corrupt magic & length checks
    let mut corrupt = encoded.clone();
    corrupt[0] = b'X';
    assert!(ScreenTwccFeedback::decode(&corrupt).is_err());
    assert!(ScreenTwccFeedback::decode(&encoded[..19]).is_err());
    assert!(ScreenTwccFeedback::decode(&encoded[..23]).is_err());

    // InnerFrame round-trip
    let inner = fb.to_inner().expect("to inner");
    assert_eq!(inner.ty, TYPE_SCREEN_TWCC);
    assert_eq!(
        ScreenTwccFeedback::from_inner_frame(&inner).expect("from inner"),
        fb
    );

    // 3. ScreenTwccReceiverTracker
    let mut rx = ScreenTwccReceiverTracker::new(ScreenTwccMode::Auto);
    assert!(!rx.should_send_feedback(1_000_000));
    assert!(rx.build_feedback(1_000_000).is_none());

    // Record packets
    rx.on_packet_received(10, 1_000_000);
    rx.on_packet_received(11, 1_000_200);
    rx.on_packet_received(12, 1_000_500);

    // Before interval or threshold -> false / none
    assert!(!rx.should_send_feedback(1_005_000));

    // After interval (10ms) -> generates feedback
    assert!(rx.should_send_feedback(1_011_000));
    let fb_out = rx.build_feedback(1_011_000).expect("should emit feedback");
    assert_eq!(fb_out.base_seq, 10);
    assert_eq!(fb_out.packet_count, 3);
    assert_eq!(fb_out.records.len(), 3);
    assert_eq!(fb_out.records[0].recv_delta_us, 0);
    assert_eq!(fb_out.records[1].recv_delta_us, 200);
    assert_eq!(fb_out.records[2].recv_delta_us, 500);

    // Generating again immediately returns None (buffer cleared)
    assert!(rx.build_feedback(1_012_000).is_none());

    // Packet threshold trigger (e.g. Aggressive mode: 20 packets)
    rx.set_mode(ScreenTwccMode::Aggressive);
    for seq in 20..40 {
        rx.on_packet_received(seq, 1_020_000 + (seq as u64) * 10);
    }
    assert!(rx.should_send_feedback(1_020_100));
    let fb_agg = rx.build_feedback(1_020_100).expect("threshold triggered");
    assert_eq!(fb_agg.packet_count, 20);

    // 4. ScreenTwccSenderTracker
    let mut tx = ScreenTwccSenderTracker::new(ScreenTwccMode::Auto);
    let s1 = tx.allocate_seq(1_000_000, 1000);
    let s2 = tx.allocate_seq(1_001_000, 1000);
    let s3 = tx.allocate_seq(1_002_000, 1000);
    assert_eq!(s1, 1);
    assert_eq!(s2, 2);
    assert_eq!(s3, 3);

    // Create feedback matching s1, s2, s3 (s2 received with 1ms delay, s3 lost)
    let fb_test = ScreenTwccFeedback {
        feedback_seq: 1,
        base_seq: 1,
        packet_count: 3,
        reference_time_us: 1_010_000,
        records: vec![
            ScreenTwccPacketRecord {
                transport_seq: 1,
                recv_delta_us: 0,
            },
            ScreenTwccPacketRecord {
                transport_seq: 2,
                recv_delta_us: 1_000,
            },
            ScreenTwccPacketRecord {
                transport_seq: 3,
                recv_delta_us: SCREEN_TWCC_MISSING_DELTA,
            },
        ],
    };

    let rep = tx.on_feedback(&fb_test, 1_020_000);
    assert_eq!(rep.feedbacks_received, 1);
    assert_eq!(rep.packets_lost, 1);
    assert_eq!(rep.packets_acknowledged, 2);
    assert!(rep.loss_ratio > 0.3);
    assert!(rep.delivery_rate_bps > 0);

    // 5. End-to-end integration with ScreenSender & ScreenReceiver
    let mut sender = ScreenSender::new();
    sender.set_twcc_mode(ScreenTwccMode::Auto);
    sender.set_fec_mode(ScreenFecMode::Aggressive);
    assert_eq!(sender.twcc_mode(), ScreenTwccMode::Auto);
    assert_eq!(sender.stats().twcc_mode, ScreenTwccMode::Auto.tag());

    sender.on_peer_connected();
    sender.allow();
    sender.on_control(ControlMsg::Start).unwrap();

    let frame = ScreenFrame {
        seq: 50,
        bytes: vec![0xAB; 2000],
        rect: ScreenRect {
            x: 0,
            y: 0,
            w: 20,
            h: 20,
        },
        format: PixelFormat::Bgra8,
        codec_tag: SCREEN_COMPRESS_NONE,
        stride: 80,
    };

    let inner_frames = sender
        .encode_frame_to_inner_frames_with_time(&frame, 0.05, 2_000_000)
        .expect("encode");
    assert!(!inner_frames.is_empty());

    let mut receiver = ScreenReceiver::new();
    receiver.set_twcc_mode(ScreenTwccMode::Auto);

    let mut shards_received = 0;
    for in_frame in &inner_frames {
        if in_frame.ty == TYPE_SCREEN_FEC {
            let shard = ScreenFecShard::from_inner_frame(in_frame).expect("shard");
            assert!(shard.transport_seq > 0);
            let _ = receiver.on_fec_shard_with_time(shard, 2_005_000);
            shards_received += 1;
        }
    }
    assert!(shards_received > 0);

    let fb_rx = receiver
        .generate_twcc_feedback(2_016_000)
        .expect("rx feedback");
    let sender_rep = sender.handle_twcc_feedback(&fb_rx, 2_020_000);
    assert_eq!(sender_rep.feedbacks_received, 1);
    assert_eq!(sender.stats().twcc_feedbacks_received, 1);
    assert_eq!(receiver.stats().twcc_feedbacks_sent, 1);

    // Test peer disconnect resets
    sender.on_peer_gone();
    receiver.on_peer_gone();
    assert_eq!(sender.stats().twcc_feedbacks_received, 0);
    assert_eq!(receiver.stats().twcc_feedbacks_sent, 0);
}

#[test]
fn screen_active_bandwidth_probing_and_fast_rampup_pipeline() {
    // 1. Parsing and enum tags for ScreenProbeMode
    assert_eq!(
        "auto".parse::<ScreenProbeMode>().unwrap(),
        ScreenProbeMode::Auto
    );
    assert_eq!(
        "aggressive".parse::<ScreenProbeMode>().unwrap(),
        ScreenProbeMode::Aggressive
    );
    assert_eq!(
        "agg".parse::<ScreenProbeMode>().unwrap(),
        ScreenProbeMode::Aggressive
    );
    assert_eq!(
        "conservative".parse::<ScreenProbeMode>().unwrap(),
        ScreenProbeMode::Conservative
    );
    assert_eq!(
        "cons".parse::<ScreenProbeMode>().unwrap(),
        ScreenProbeMode::Conservative
    );
    assert_eq!(
        "disabled".parse::<ScreenProbeMode>().unwrap(),
        ScreenProbeMode::Disabled
    );
    assert_eq!(
        "off".parse::<ScreenProbeMode>().unwrap(),
        ScreenProbeMode::Disabled
    );
    assert!("invalid".parse::<ScreenProbeMode>().is_err());
    assert_eq!(ScreenProbeMode::from_tag(0), ScreenProbeMode::Auto);
    assert_eq!(ScreenProbeMode::from_tag(1), ScreenProbeMode::Aggressive);
    assert_eq!(ScreenProbeMode::from_tag(2), ScreenProbeMode::Conservative);
    assert_eq!(ScreenProbeMode::from_tag(3), ScreenProbeMode::Disabled);

    // 2. Wire Protocol and framing: ScreenProbePacket encode/decode round-trip
    let original = ScreenProbePacket {
        flags: 0x01,
        cluster_id: 42,
        cluster_index: 3,
        cluster_size: 8,
        transport_seq: 105,
        target_bps: 12_000_000,
        send_time_us: 1_234_567_890,
        padding: vec![0xEE; 256],
    };
    let encoded = original.encode();
    assert_eq!(encoded.len(), SCREEN_PROBE_HEADER_LEN + 256);
    assert_eq!(&encoded[0..5], &SCREEN_PROBE_MAGIC);
    assert_eq!(encoded[5], SCREEN_PROBE_PROTO);

    let decoded = ScreenProbePacket::decode(&encoded).expect("decode probe packet");
    assert_eq!(decoded.flags, original.flags);
    assert_eq!(decoded.cluster_id, original.cluster_id);
    assert_eq!(decoded.cluster_index, original.cluster_index);
    assert_eq!(decoded.cluster_size, original.cluster_size);
    assert_eq!(decoded.transport_seq, original.transport_seq);
    assert_eq!(decoded.target_bps, original.target_bps);
    assert_eq!(decoded.send_time_us, original.send_time_us);
    assert_eq!(decoded.padding, original.padding);

    // Corrupted magic and truncated slice error tests
    let mut bad_magic = encoded.clone();
    bad_magic[0] = b'X';
    assert!(ScreenProbePacket::decode(&bad_magic).is_err());

    let mut bad_proto = encoded.clone();
    bad_proto[5] = 0x99;
    assert!(ScreenProbePacket::decode(&bad_proto).is_err());

    assert!(ScreenProbePacket::decode(&encoded[..SCREEN_PROBE_HEADER_LEN - 1]).is_err());
    assert!(ScreenProbePacket::decode(&encoded[..SCREEN_PROBE_HEADER_LEN + 10]).is_err());

    // InnerFrame round-trip
    let inner = original.to_inner().expect("to inner");
    assert_eq!(inner.ty, TYPE_SCREEN_PROBE);
    assert_eq!(
        ScreenProbePacket::from_inner_frame(&inner).expect("from inner"),
        original
    );

    // 3. ScreenProbeController state machine and cluster generation
    let mut controller = ScreenProbeController::new(ScreenProbeMode::Auto);
    assert_eq!(controller.mode(), ScreenProbeMode::Auto);
    assert_eq!(controller.state(), ScreenProbeState::Idle);

    // Trigger probe burst for 12 Mbps
    let mut mock_seq = 1000u16;
    let ok = controller.trigger_probe(12_000_000, 1_000_000, &mut mock_seq);
    assert!(ok);
    assert_eq!(controller.stats().probes_triggered, 1);
    assert_eq!(controller.stats().last_target_bps, 12_000_000);
    assert_eq!(controller.state(), ScreenProbeState::Probing);

    let mut popped = Vec::new();
    while let Some(p) = controller.pop_probe_packet() {
        popped.push(p);
    }
    assert_eq!(popped.len(), SCREEN_PROBE_CLUSTER_SIZE_AUTO as usize);
    assert_eq!(controller.state(), ScreenProbeState::AwaitingFeedback);

    for (idx, p) in popped.iter().enumerate() {
        assert_eq!(p.cluster_id, 1);
        assert_eq!(p.cluster_index, idx as u8);
        assert_eq!(p.cluster_size, SCREEN_PROBE_CLUSTER_SIZE_AUTO);
        assert_eq!(p.target_bps, 12_000_000);
        assert_eq!(p.transport_seq, 1001 + idx as u16);
    }

    // Evaluating feedback: successful burst delivery with high delivery rate and low loss
    let mut rate_ctrl = ScreenRateController::new(SCREEN_BITRATE_DEFAULT_BPS);
    let mut records = Vec::new();
    for p in &popped {
        records.push(ScreenTwccPacketRecord {
            transport_seq: p.transport_seq,
            recv_delta_us: (p.cluster_index as i16) * 500, // 500us intervals -> 3500us span for 8 packets -> high delivery rate
        });
    }
    let fb_success = ScreenTwccFeedback {
        feedback_seq: 1,
        base_seq: popped[0].transport_seq,
        packet_count: popped.len() as u16,
        reference_time_us: 1_010_000,
        records,
    };

    let ramp_target = controller.on_twcc_feedback(&fb_success, &mut rate_ctrl, 1_015_000);
    assert!(ramp_target.is_some());
    assert_eq!(ramp_target.unwrap(), 12_000_000);
    assert_eq!(controller.state(), ScreenProbeState::Completed);
    assert_eq!(controller.stats().probes_completed, 1);
    assert_eq!(rate_ctrl.current_bps(), 12_000_000);

    // Failed probe evaluation (high loss > 15%)
    let ok_manual = controller.trigger_probe(15_000_000, 2_000_000, &mut mock_seq);
    assert!(ok_manual);
    assert_eq!(controller.state(), ScreenProbeState::Probing);

    let mut popped_fail = Vec::new();
    while let Some(p) = controller.pop_probe_packet() {
        popped_fail.push(p);
    }
    assert_eq!(popped_fail.len(), SCREEN_PROBE_CLUSTER_SIZE_AUTO as usize);

    // Create feedback where half the packets are marked missing
    let mut records_fail = Vec::new();
    for p in &popped_fail {
        let delta = if p.cluster_index % 2 == 1 {
            SCREEN_TWCC_MISSING_DELTA
        } else {
            (p.cluster_index as i16) * 1000
        };
        records_fail.push(ScreenTwccPacketRecord {
            transport_seq: p.transport_seq,
            recv_delta_us: delta,
        });
    }
    let fb_fail = ScreenTwccFeedback {
        feedback_seq: 2,
        base_seq: popped_fail[0].transport_seq,
        packet_count: popped_fail.len() as u16,
        reference_time_us: 2_010_000,
        records: records_fail,
    };

    let fail_target = controller.on_twcc_feedback(&fb_fail, &mut rate_ctrl, 2_015_000);
    assert!(fail_target.is_none());
    assert_eq!(controller.state(), ScreenProbeState::Failed);
    assert_eq!(controller.stats().probes_failed, 1);

    // 4. ScreenRateController fast_ramp_up integration
    let mut rate_ctrl2 = ScreenRateController::new(SCREEN_BITRATE_DEFAULT_BPS);
    assert_eq!(rate_ctrl2.current_bps(), SCREEN_BITRATE_DEFAULT_BPS);

    rate_ctrl2.fast_ramp_up(15_000_000);
    assert_eq!(rate_ctrl2.current_bps(), 15_000_000);

    // Clamping to max allowed bitrate (SCREEN_BITRATE_MAX_BPS)
    rate_ctrl2.fast_ramp_up(500_000_000);
    assert_eq!(rate_ctrl2.current_bps(), SCREEN_BITRATE_MAX_BPS);

    // Fast ramp-up only increases, does not decrease
    let stepped_down = rate_ctrl2.fast_ramp_up(100);
    assert!(!stepped_down);
    assert_eq!(rate_ctrl2.current_bps(), SCREEN_BITRATE_MAX_BPS);

    // 5. End-to-end Sender & Receiver Active Probing Pipeline
    let mut sender = ScreenSender::new();
    sender.set_probe_mode(ScreenProbeMode::Auto);
    sender.set_twcc_mode(ScreenTwccMode::Auto);
    sender.set_fec_mode(ScreenFecMode::Auto);
    assert_eq!(sender.probe_mode(), ScreenProbeMode::Auto);

    sender.on_peer_connected();
    sender.allow();
    sender.on_control(ControlMsg::Start).unwrap();

    // Trigger active bandwidth probe burst
    let triggered = sender.trigger_probe(8_000_000, 2_000_000);
    assert!(triggered);

    // Check that sender generated probe packets
    let mut probe_packets = Vec::new();
    while let Some(p) = sender.pop_probe_packet() {
        probe_packets.push(p);
    }
    assert_eq!(probe_packets.len(), SCREEN_PROBE_CLUSTER_SIZE_AUTO as usize);

    let mut receiver = ScreenReceiver::new();
    receiver.set_probe_mode(ScreenProbeMode::Auto);
    receiver.set_twcc_mode(ScreenTwccMode::Auto);

    for (idx, p) in probe_packets.into_iter().enumerate() {
        receiver.on_probe_packet_with_time(p, 2_005_000 + (idx as u64) * 200);
    }
    assert_eq!(
        receiver.probe_stats().probes_triggered,
        SCREEN_PROBE_CLUSTER_SIZE_AUTO as u64
    );

    let fb = receiver
        .generate_twcc_feedback(2_016_000)
        .expect("twcc feedback covering probe packets");
    let rep = sender.handle_twcc_feedback(&fb, 2_020_000);
    assert_eq!(rep.feedbacks_received, 1);

    // Peer disconnect resets
    sender.on_peer_gone();
    receiver.on_peer_gone();
    assert_eq!(sender.stats().probes_triggered, 0);
    assert_eq!(receiver.stats().probes_triggered, 0);
}

#[test]
fn screen_audio_sync_pipeline_and_drift_compensation() {
    // 1. Enum parsing and tags
    assert_eq!(
        "auto".parse::<ScreenAudioSyncMode>().unwrap(),
        ScreenAudioSyncMode::Auto
    );
    assert_eq!(
        "strict".parse::<ScreenAudioSyncMode>().unwrap(),
        ScreenAudioSyncMode::Strict
    );
    assert_eq!(
        "tight".parse::<ScreenAudioSyncMode>().unwrap(),
        ScreenAudioSyncMode::Strict
    );
    assert_eq!(
        "loose".parse::<ScreenAudioSyncMode>().unwrap(),
        ScreenAudioSyncMode::Loose
    );
    assert_eq!(
        "relaxed".parse::<ScreenAudioSyncMode>().unwrap(),
        ScreenAudioSyncMode::Loose
    );
    assert_eq!(
        "disabled".parse::<ScreenAudioSyncMode>().unwrap(),
        ScreenAudioSyncMode::Disabled
    );
    assert_eq!(
        "off".parse::<ScreenAudioSyncMode>().unwrap(),
        ScreenAudioSyncMode::Disabled
    );
    assert!("invalid".parse::<ScreenAudioSyncMode>().is_err());
    assert_eq!(ScreenAudioSyncMode::from_tag(0), ScreenAudioSyncMode::Auto);
    assert_eq!(
        ScreenAudioSyncMode::from_tag(1),
        ScreenAudioSyncMode::Strict
    );
    assert_eq!(ScreenAudioSyncMode::from_tag(2), ScreenAudioSyncMode::Loose);
    assert_eq!(
        ScreenAudioSyncMode::from_tag(3),
        ScreenAudioSyncMode::Disabled
    );

    // 2. Wire Protocol and framing: ScreenAudioSyncFrame encode/decode round-trip
    let original = ScreenAudioSyncFrame {
        flags: 0x01,
        sync_seq: 42,
        sender_wallclock_us: 1_000_000,
        video_pts_us: 1_020_000,
        audio_pts_us: 1_005_000,
    };
    let encoded = original.encode();
    assert_eq!(encoded.len(), SCREEN_SYNC_HEADER_LEN);
    assert_eq!(&encoded[0..5], &SCREEN_SYNC_MAGIC);
    assert_eq!(encoded[5], SCREEN_SYNC_PROTO);

    let decoded = ScreenAudioSyncFrame::decode(&encoded).expect("decode sync frame");
    assert_eq!(decoded.flags, original.flags);
    assert_eq!(decoded.sync_seq, original.sync_seq);
    assert_eq!(decoded.sender_wallclock_us, original.sender_wallclock_us);
    assert_eq!(decoded.video_pts_us, original.video_pts_us);
    assert_eq!(decoded.audio_pts_us, original.audio_pts_us);

    // Corrupted magic and truncated errors
    let mut bad_magic = encoded.clone();
    bad_magic[0] = b'X';
    assert!(ScreenAudioSyncFrame::decode(&bad_magic).is_err());

    let mut bad_proto = encoded.clone();
    bad_proto[5] = 0x99;
    assert!(ScreenAudioSyncFrame::decode(&bad_proto).is_err());

    assert!(ScreenAudioSyncFrame::decode(&encoded[..SCREEN_SYNC_HEADER_LEN - 1]).is_err());

    // InnerFrame round-trip
    let inner = original.to_inner().expect("to inner");
    assert_eq!(inner.ty, TYPE_SCREEN_SYNC);
    assert_eq!(
        ScreenAudioSyncFrame::from_inner_frame(&inner).expect("from inner"),
        original
    );

    // 3. ScreenAudioSyncCoordinator generation and interval gating
    let mut coord = ScreenAudioSyncCoordinator::new(ScreenAudioSyncMode::Auto);
    coord.record_video_pts(500_000);
    coord.record_audio_pts(490_000);

    let beacon1 = coord.maybe_generate_sync_beacon(1_000_000);
    assert!(beacon1.is_some());
    let b1 = beacon1.unwrap();
    assert_eq!(b1.sync_seq, 1);
    assert_eq!(b1.video_pts_us, 500_000);
    assert_eq!(b1.audio_pts_us, 490_000);
    assert_eq!(coord.stats().beacons_sent, 1);

    // Call before interval has passed (< 500ms) returns None
    let beacon_early = coord.maybe_generate_sync_beacon(1_200_000);
    assert!(beacon_early.is_none());

    // Call after interval (>= 500ms) generates next beacon
    let beacon2 = coord.maybe_generate_sync_beacon(1_600_000);
    assert!(beacon2.is_some());
    assert_eq!(beacon2.unwrap().sync_seq, 2);
    assert_eq!(coord.stats().beacons_sent, 2);

    // 4. Receiver-side skew calculation, drift tracking, and playout buffer compensation
    let mut receiver_coord = ScreenAudioSyncCoordinator::new(ScreenAudioSyncMode::Auto);
    let mut playout_buf = ScreenPlayoutBuffer::new(ScreenPlayoutMode::Adaptive);
    let initial_delay = playout_buf.target_delay_us();

    // Beacon with video lead > 15ms tolerance (video 600ms, audio 570ms -> skew = +30ms)
    let lead_beacon = ScreenAudioSyncFrame {
        flags: 0,
        sync_seq: 10,
        sender_wallclock_us: 2_000_000,
        video_pts_us: 600_000,
        audio_pts_us: 570_000,
    };
    let skew = receiver_coord.on_sync_beacon(&lead_beacon, 2_005_000, Some(&mut playout_buf));
    assert_eq!(skew, 30_000);
    assert_eq!(receiver_coord.stats().current_skew_us, 30_000);
    assert!(!receiver_coord.stats().in_sync); // |30ms| > 15ms tolerance
    assert_eq!(receiver_coord.stats().adjustments_applied, 1);
    assert!(playout_buf.target_delay_us() > initial_delay); // delay increased to slow down video

    // Second beacon within tolerance (video 700ms, audio 695ms -> skew = +5ms <= 15ms)
    let sync_beacon = ScreenAudioSyncFrame {
        flags: 0,
        sync_seq: 11,
        sender_wallclock_us: 2_500_000,
        video_pts_us: 700_000,
        audio_pts_us: 695_000,
    };
    let skew2 = receiver_coord.on_sync_beacon(&sync_beacon, 2_505_050, Some(&mut playout_buf));
    assert_eq!(skew2, 5_000);
    assert!(receiver_coord.stats().in_sync);

    // Check drift estimation over time
    assert!(receiver_coord.stats().clock_drift_ppm.abs() >= 0.0);

    // 5. End-to-end ScreenSender & ScreenReceiver Pipeline
    let mut sender = ScreenSender::with_sync(ScreenAudioSyncMode::Auto);
    let mut receiver = ScreenReceiver::with_sync_mode(ScreenAudioSyncMode::Auto);
    assert_eq!(sender.sync_mode(), ScreenAudioSyncMode::Auto);
    assert_eq!(receiver.sync_mode(), ScreenAudioSyncMode::Auto);

    sender.record_audio_pts(1_000_000);
    sender.record_video_pts(1_025_000);

    let beacon = sender.trigger_sync_beacon(2_000_000);
    assert_eq!(beacon.sync_seq, 1);
    assert_eq!(sender.stats().sync_beacons_sent, 1);

    let popped = sender.pop_sync_beacon().expect("popped beacon");
    assert_eq!(popped, beacon);

    let skew_rcv = receiver.on_sync_beacon_with_time(&popped, 2_002_000);
    assert_eq!(skew_rcv, 25_000);
    assert_eq!(receiver.stats().sync_beacons_received, 1);
    assert_eq!(receiver.stats().sync_skew_us, 25_000);

    // Disconnect resets sync state
    sender.on_peer_gone();
    receiver.on_peer_gone();
    assert_eq!(sender.stats().sync_beacons_sent, 0);
    assert_eq!(receiver.stats().sync_beacons_received, 0);
}

#[test]
fn screen_multi_display_and_dynamic_switch_pipeline() {
    // 1. ScreenDisplayInfo constructor and defaults
    let d0 = ScreenDisplayInfo::primary_default();
    assert_eq!(d0.display_id, 0);
    assert!(d0.is_primary);
    assert!(!d0.is_virtual);
    assert_eq!(d0.width, 1920);
    assert_eq!(d0.height, 1080);
    assert_eq!(d0.scale_factor_pct, 100);

    let d1 = ScreenDisplayInfo::new(1, "Display 1 (Secondary)", 2560, 1440, 125, false, false);
    let d2 = ScreenDisplayInfo::new(2, "Virtual Display (ADR-073)", 1920, 1200, 100, false, true);

    // 2. ControlMsg wire round-trip
    let ctrl_switch = ControlMsg::SwitchDisplay { display_id: 1 };
    assert_eq!(ctrl_switch.tag(), 4);
    let frame = ctrl_switch.to_frame().expect("to_frame");
    assert_eq!(frame.ty, TYPE_SCREEN_CONTROL);
    let decoded = ControlMsg::from_frame(&frame).expect("from_frame");
    assert_eq!(decoded, ControlMsg::SwitchDisplay { display_id: 1 });

    // 3. ScreenSender display management and switching
    let mut sender = ScreenSender::new();
    sender.allow();
    sender.on_control(ControlMsg::Start).unwrap();
    assert_eq!(sender.active_display_id(), 0);
    assert_eq!(sender.displays().len(), 1);

    sender.set_displays(vec![d0.clone(), d1.clone(), d2.clone()]);
    assert_eq!(sender.displays().len(), 3);
    assert_eq!(sender.stats().display_count, 3);

    // Switch to display 1
    sender
        .switch_display(1, 1_000_000)
        .expect("switch display 1");
    assert_eq!(sender.active_display_id(), 1);
    assert_eq!(sender.stats().active_display_id, 1);
    assert_eq!(sender.stats().display_switches_count, 1);
    assert_eq!(sender.stats().keyframes_requested, 1); // Triggers keyframe refresh

    let report = sender.display_report();
    assert_eq!(report.active_display_id, 1);
    assert_eq!(report.displays.len(), 3);
    assert_eq!(report.switches_count, 1);
    assert_eq!(report.last_switch_us, 1_000_000);

    // Receiving ControlMsg::SwitchDisplay on sender
    sender
        .on_control_with_time(ControlMsg::SwitchDisplay { display_id: 2 }, 1_500_000)
        .expect("on_control switch");
    assert_eq!(sender.active_display_id(), 2);
    assert_eq!(sender.stats().active_display_id, 2);
    assert_eq!(sender.stats().display_switches_count, 2);
    assert_eq!(sender.stats().keyframes_requested, 2);

    // 4. ScreenReceiver display management
    let mut receiver = ScreenReceiver::new();
    assert_eq!(receiver.active_display_id(), 0);
    receiver.set_displays(vec![d0, d1, d2]);
    assert_eq!(receiver.displays().len(), 3);
    assert_eq!(receiver.stats().display_count, 3);

    receiver.switch_display(2, 1_500_000);
    assert_eq!(receiver.active_display_id(), 2);
    assert_eq!(receiver.stats().active_display_id, 2);
    assert_eq!(receiver.stats().display_switches_count, 1);

    let rx_report = receiver.display_report();
    assert_eq!(rx_report.active_display_id, 2);
    assert_eq!(rx_report.switches_count, 1);

    // 5. MemoryScreenSource multi-display support
    use crate::ports::ScreenSource;
    let mem_src = MemoryScreenSource::new(1920, 1080).expect("mem_src");
    assert_eq!(mem_src.active_display_id(), 0);
    assert_eq!(mem_src.displays().len(), 1);
    assert!(mem_src.switch_display(99).is_err()); // Reject unknown display ID

    mem_src.set_displays(vec![
        ScreenDisplayInfo::primary_default(),
        ScreenDisplayInfo::new(1, "Display 1", 2560, 1440, 100, false, false),
    ]);
    assert_eq!(mem_src.displays().len(), 2);
    mem_src.switch_display(1).expect("mem_src switch");
    assert_eq!(mem_src.active_display_id(), 1);

    // 6. Peer disconnect resets sender & receiver display state
    sender.on_peer_gone();
    receiver.on_peer_gone();
    assert_eq!(sender.stats().active_display_id, 0);
    assert_eq!(sender.stats().display_switches_count, 0);
    assert_eq!(receiver.stats().active_display_id, 0);
    assert_eq!(receiver.stats().display_switches_count, 0);
}

#[test]
fn screen_privacy_mode_and_input_lockout_pipeline() {
    // 1. ControlMsg::SetPrivacyMode wire encode/decode round-trip (tag 5, 14 bytes)
    let ctrl_privacy = ControlMsg::SetPrivacyMode {
        enabled: true,
        lock_input: true,
    };
    assert_eq!(ctrl_privacy.tag(), 5);
    let payload = ctrl_privacy.encode_payload();
    assert_eq!(payload.len(), 14);
    let decoded = ControlMsg::decode_payload(&payload).expect("decode SetPrivacyMode");
    assert_eq!(
        decoded,
        ControlMsg::SetPrivacyMode {
            enabled: true,
            lock_input: true,
        }
    );

    let ctrl_privacy_off = ControlMsg::SetPrivacyMode {
        enabled: false,
        lock_input: false,
    };
    let payload_off = ctrl_privacy_off.encode_payload();
    let decoded_off = ControlMsg::decode_payload(&payload_off).expect("decode SetPrivacyMode off");
    assert_eq!(
        decoded_off,
        ControlMsg::SetPrivacyMode {
            enabled: false,
            lock_input: false,
        }
    );

    // 2. ScreenSender privacy mode management and telemetry
    let mut sender = ScreenSender::new();
    let rep0 = sender.privacy_report();
    assert!(rep0.ok);
    assert!(!rep0.enabled);
    assert!(!rep0.lock_input);
    assert_eq!(rep0.blanking_method, "virtual_overlay");
    assert!(!sender.stats().privacy_enabled);
    assert!(!sender.stats().privacy_input_locked);

    // Set privacy mode directly
    sender.set_privacy_mode(true, true, 1_000_000);
    let rep1 = sender.privacy_report();
    assert!(rep1.enabled);
    assert!(rep1.lock_input);
    assert_eq!(rep1.last_change_us, 1_000_000);
    assert!(sender.stats().privacy_enabled);
    assert!(sender.stats().privacy_input_locked);

    // Update via ControlMsg::SetPrivacyMode
    sender
        .on_control_with_time(
            ControlMsg::SetPrivacyMode {
                enabled: true,
                lock_input: false,
            },
            2_000_000,
        )
        .expect("on_control privacy mode");
    let rep2 = sender.privacy_report();
    assert!(rep2.enabled);
    assert!(!rep2.lock_input);
    assert_eq!(rep2.last_change_us, 2_000_000);
    assert!(sender.stats().privacy_enabled);
    assert!(!sender.stats().privacy_input_locked);

    // 3. Fail-safe disconnect recovery: on_peer_gone disengages privacy & input lockout
    sender.on_peer_gone();
    let rep3 = sender.privacy_report();
    assert!(!rep3.enabled);
    assert!(!rep3.lock_input);
    assert!(!sender.stats().privacy_enabled);
    assert!(!sender.stats().privacy_input_locked);

    // 4. ScreenReceiver privacy mode management
    let mut receiver = ScreenReceiver::new();
    let rx_rep0 = receiver.privacy_report();
    assert!(!rx_rep0.enabled);
    assert!(!rx_rep0.lock_input);

    receiver.set_privacy_mode(true, true, 3_000_000);
    let rx_rep1 = receiver.privacy_report();
    assert!(rx_rep1.enabled);
    assert!(rx_rep1.lock_input);
    assert_eq!(rx_rep1.last_change_us, 3_000_000);
    assert!(receiver.stats().privacy_enabled);
    assert!(receiver.stats().privacy_input_locked);

    // Receiver on_peer_gone reset
    receiver.on_peer_gone();
    let rx_rep2 = receiver.privacy_report();
    assert!(!rx_rep2.enabled);
    assert!(!rx_rep2.lock_input);
    assert!(!receiver.stats().privacy_enabled);
    assert!(!receiver.stats().privacy_input_locked);
}

#[test]
fn screen_latency_breakdown_and_profiler_pipeline() {
    // 1. ScreenProfilerMode & ScreenBottleneckStage parse & display
    assert_eq!(
        "auto".parse::<ScreenProfilerMode>().unwrap(),
        ScreenProfilerMode::Auto
    );
    assert_eq!(
        "detailed".parse::<ScreenProfilerMode>().unwrap(),
        ScreenProfilerMode::Detailed
    );
    assert_eq!(
        "always".parse::<ScreenProfilerMode>().unwrap(),
        ScreenProfilerMode::Detailed
    );
    assert_eq!(
        "disabled".parse::<ScreenProfilerMode>().unwrap(),
        ScreenProfilerMode::Disabled
    );
    assert!("invalid".parse::<ScreenProfilerMode>().is_err());
    assert_eq!(ScreenProfilerMode::Auto.to_string(), "auto");
    assert_eq!(ScreenProfilerMode::Detailed.to_string(), "detailed");
    assert_eq!(ScreenProfilerMode::Disabled.to_string(), "disabled");

    assert_eq!(
        "capture".parse::<ScreenBottleneckStage>().unwrap(),
        ScreenBottleneckStage::Capture
    );
    assert_eq!(
        "encode".parse::<ScreenBottleneckStage>().unwrap(),
        ScreenBottleneckStage::Encode
    );
    assert_eq!(
        "pacer".parse::<ScreenBottleneckStage>().unwrap(),
        ScreenBottleneckStage::Pacer
    );
    assert_eq!(
        "network".parse::<ScreenBottleneckStage>().unwrap(),
        ScreenBottleneckStage::Network
    );
    assert_eq!(
        "jitter".parse::<ScreenBottleneckStage>().unwrap(),
        ScreenBottleneckStage::Jitter
    );
    assert_eq!(
        "decode".parse::<ScreenBottleneckStage>().unwrap(),
        ScreenBottleneckStage::Decode
    );
    assert_eq!(
        "render".parse::<ScreenBottleneckStage>().unwrap(),
        ScreenBottleneckStage::Render
    );
    assert_eq!(
        "balanced".parse::<ScreenBottleneckStage>().unwrap(),
        ScreenBottleneckStage::Balanced
    );
    assert!("invalid".parse::<ScreenBottleneckStage>().is_err());
    assert_eq!(ScreenBottleneckStage::Network.to_string(), "network");

    // 2. ScreenProfileFrame wire encode / decode roundtrip (56 bytes, TYPE_SCREEN_PROFILE)
    let pframe = ScreenProfileFrame {
        flags: 0x01,
        frame_seq: 100,
        capture_us: 2_000,
        encode_us: 4_500,
        pacer_us: 500,
        sender_ts_us: 1_000_000,
        network_us: 12_000,
        jitter_us: 2_500,
        decode_us: 1_800,
        render_us: 700,
        total_e2e_us: 24_000,
    };
    let encoded = pframe.encode();
    assert_eq!(encoded.len(), SCREEN_PROFILE_HEADER_LEN);
    assert_eq!(&encoded[0..5], &SCREEN_PROFILE_MAGIC);
    assert_eq!(encoded[5], SCREEN_PROFILE_PROTO);

    let decoded = ScreenProfileFrame::decode(&encoded).expect("decode profile frame");
    assert_eq!(decoded.frame_seq, 100);
    assert_eq!(decoded.capture_us, 2_000);
    assert_eq!(decoded.encode_us, 4_500);
    assert_eq!(decoded.pacer_us, 500);
    assert_eq!(decoded.sender_ts_us, 1_000_000);
    assert_eq!(decoded.network_us, 12_000);
    assert_eq!(decoded.jitter_us, 2_500);
    assert_eq!(decoded.decode_us, 1_800);
    assert_eq!(decoded.render_us, 700);
    assert_eq!(decoded.total_e2e_us, 24_000);

    // Wire protocol errors: truncated, invalid magic, invalid proto
    assert!(ScreenProfileFrame::decode(&encoded[..SCREEN_PROFILE_HEADER_LEN - 1]).is_err());
    let mut bad_magic = encoded.clone();
    bad_magic[0] = b'X';
    assert!(ScreenProfileFrame::decode(&bad_magic).is_err());
    let mut bad_proto = encoded.clone();
    bad_proto[5] = 0x99;
    assert!(ScreenProfileFrame::decode(&bad_proto).is_err());

    // InnerFrame roundtrip
    let inner = pframe.to_inner().expect("to inner");
    assert_eq!(inner.ty, TYPE_SCREEN_PROFILE);
    let from_inner = ScreenProfileFrame::from_inner_frame(&inner).expect("from inner");
    assert_eq!(from_inner, pframe);

    // 3. ControlMsg::ResetLatencyProfile roundtrip (tag 6, 14 bytes)
    let reset_msg = ControlMsg::ResetLatencyProfile;
    assert_eq!(reset_msg.tag(), 6);
    let reset_payload = reset_msg.encode_payload();
    assert_eq!(reset_payload.len(), 14);
    let decoded_reset =
        ControlMsg::decode_payload(&reset_payload).expect("decode ResetLatencyProfile");
    assert_eq!(decoded_reset, ControlMsg::ResetLatencyProfile);

    // 4. ScreenFrameProfiler telemetry, bottleneck detection & percentiles
    let mut profiler = ScreenFrameProfiler::new(ScreenProfilerMode::Auto);
    assert_eq!(profiler.report().frames_profiled, 0);
    assert_eq!(profiler.report().bottleneck, "balanced");

    // Record balanced frame: total 10000 us, each stage <= 30%
    let b_balanced = ScreenLatencyBreakdown {
        frame_seq: 1,
        capture_us: 1500,
        encode_us: 2000,
        pacer_us: 1000,
        network_us: 2500,
        jitter_us: 1000,
        decode_us: 1000,
        render_us: 1000,
        total_e2e_us: 10000,
    };
    profiler.record_breakdown(b_balanced);
    assert_eq!(profiler.report().frames_profiled, 1);
    assert_eq!(profiler.report().bottleneck, "balanced");

    // Record network bottleneck frame: network_us = 12000 us out of 20000 us (60% >= 35%)
    let b_network = ScreenLatencyBreakdown {
        frame_seq: 2,
        capture_us: 1000,
        encode_us: 2000,
        pacer_us: 500,
        network_us: 12000,
        jitter_us: 1500,
        decode_us: 1500,
        render_us: 1500,
        total_e2e_us: 20000,
    };
    profiler.record_breakdown(b_network);
    assert_eq!(profiler.report().frames_profiled, 2);
    assert_eq!(profiler.report().bottleneck, "network");

    // Record encode bottleneck frame: encode_us = 10000 us out of 20000 us (50% >= 35%)
    let b_encode = ScreenLatencyBreakdown {
        frame_seq: 3,
        capture_us: 1000,
        encode_us: 10000,
        pacer_us: 500,
        network_us: 3000,
        jitter_us: 2000,
        decode_us: 2000,
        render_us: 1500,
        total_e2e_us: 20000,
    };
    profiler.record_breakdown(b_encode);
    assert_eq!(profiler.report().frames_profiled, 3);
    assert_eq!(profiler.report().bottleneck, "encode");

    // Percentiles and stats verification
    let rep = profiler.report();
    assert!(rep.avg_e2e_us > 0);
    assert!(rep.p50_e2e_us > 0);
    assert!(rep.p95_e2e_us >= rep.p50_e2e_us);

    // Reset clears metrics
    profiler.reset();
    let rep_reset = profiler.report();
    assert_eq!(rep_reset.frames_profiled, 0);
    assert_eq!(rep_reset.avg_e2e_us, 0);
    assert_eq!(rep_reset.bottleneck, "balanced");

    // 5. ScreenSender & ScreenReceiver latency profiler integration
    let mut sender = ScreenSender::new();
    assert_eq!(sender.latency_mode(), ScreenProfilerMode::Auto);
    sender.set_latency_mode(ScreenProfilerMode::Detailed);
    assert_eq!(sender.latency_mode(), ScreenProfilerMode::Detailed);

    sender.on_profile_frame(&pframe);
    let s_rep = sender.latency_report();
    assert_eq!(s_rep.frames_profiled, 1);
    assert_eq!(s_rep.current.frame_seq, 100);

    sender.on_peer_gone();
    assert_eq!(sender.latency_report().frames_profiled, 0);

    let mut receiver = ScreenReceiver::new();
    assert_eq!(receiver.latency_mode(), ScreenProfilerMode::Auto);
    receiver.set_latency_mode(ScreenProfilerMode::Detailed);
    assert_eq!(receiver.latency_mode(), ScreenProfilerMode::Detailed);

    receiver.on_peer_gone();
    assert_eq!(receiver.latency_report().frames_profiled, 0);
}

// ADR-100 Screen Streaming Bottleneck-Driven Autonomous Tuning Closed Loop
#[test]
fn screen_bottleneck_autotuning_closed_loop() {
    // 1. ScreenTunerMode parse & display
    assert_eq!(
        "standard".parse::<ScreenTunerMode>().unwrap(),
        ScreenTunerMode::Standard
    );
    assert_eq!(
        "auto".parse::<ScreenTunerMode>().unwrap(),
        ScreenTunerMode::Standard
    );
    assert_eq!(
        "aggressive".parse::<ScreenTunerMode>().unwrap(),
        ScreenTunerMode::Aggressive
    );
    assert_eq!(
        "disabled".parse::<ScreenTunerMode>().unwrap(),
        ScreenTunerMode::Disabled
    );
    assert!("invalid".parse::<ScreenTunerMode>().is_err());
    assert_eq!(ScreenTunerMode::Standard.to_string(), "standard");
    assert_eq!(ScreenTunerMode::Aggressive.trigger_frames(), 2);
    assert_eq!(ScreenTunerMode::Standard.trigger_frames(), 3);
    assert_eq!(ScreenTunerMode::Standard.recovery_frames(), 10);

    // 2. ControlMsg::SetTuningMode / ResetTuningProfile wire roundtrip (14-byte control envelope)
    let set_frame = ControlMsg::SetTuningMode {
        mode: ScreenTunerMode::Aggressive.tag(),
    }
    .to_frame()
    .unwrap();
    assert_eq!(set_frame.ty, TYPE_SCREEN_CONTROL);
    assert_eq!(set_frame.payload.len(), 14);
    match ControlMsg::decode_payload(&set_frame.payload).unwrap() {
        ControlMsg::SetTuningMode { mode } => {
            assert_eq!(ScreenTunerMode::from_tag(mode), ScreenTunerMode::Aggressive)
        }
        other => panic!("expected SetTuningMode, got {other:?}"),
    }
    let rst_frame = ControlMsg::ResetTuningProfile.to_frame().unwrap();
    assert!(matches!(
        ControlMsg::decode_payload(&rst_frame.payload).unwrap(),
        ControlMsg::ResetTuningProfile
    ));

    // 3. Hysteresis: fewer than trigger frames of the same bottleneck must not act
    let ctx = ScreenTuningContext {
        bitrate_bps: SCREEN_BITRATE_DEFAULT_BPS,
        bitrate_min_bps: SCREEN_BITRATE_MIN_BPS,
        bitrate_max_bps: SCREEN_BITRATE_MAX_BPS,
        fps: 30,
        scale_pct: 100,
        qp_min: 12,
        qp_max: 36,
        playout_delay_us: 0,
    };

    let mut sender = ScreenSender::new();
    assert_eq!(sender.tuning_mode(), ScreenTunerMode::Standard);
    for seq in 1..=2u64 {
        sender.on_profile_frame(&ScreenProfileFrame {
            flags: 0,
            frame_seq: seq,
            capture_us: 1_000,
            encode_us: 30_000,
            pacer_us: 1_000,
            sender_ts_us: seq,
            network_us: 1_000,
            jitter_us: 1_000,
            decode_us: 1_000,
            render_us: 1_000,
            total_e2e_us: 36_000,
        });
    }
    assert_eq!(sender.tuning_report().actions_applied, 0);
    assert_eq!(sender.tuning_report().bottleneck_streak, 2);

    // 4. Third consecutive encode-heavy frame triggers RaiseQp (12 -> 40 clamped to 51? no: 36+4=40)
    sender.on_profile_frame(&ScreenProfileFrame {
        flags: 0,
        frame_seq: 3,
        capture_us: 1_000,
        encode_us: 30_000,
        pacer_us: 1_000,
        sender_ts_us: 3,
        network_us: 1_000,
        jitter_us: 1_000,
        decode_us: 1_000,
        render_us: 1_000,
        total_e2e_us: 36_000,
    });
    let rep = sender.tuning_report();
    assert_eq!(
        rep.actions_applied, 1,
        "encode bottleneck must trigger on 3rd frame"
    );
    assert_eq!(rep.last_action, "raise_qp");
    assert_eq!(rep.last_stage, "encode");
    let decision = rep.last_decision.unwrap();
    assert_eq!(decision.new_qp_max, Some(39), "qp_max 35 + 4 relief = 39");
    assert_eq!(sender.stats.qp_max, 39);
    // Sender knobs applied: stats mirror the tuner telemetry
    assert_eq!(sender.stats.tuning_actions_total, 1);

    // 5. Direct ScreenTuner policy checks per stage
    let mut tuner = ScreenTuner::new(ScreenTunerMode::Standard);
    let _ = tuner.evaluate(1_000, 1, ScreenBottleneckStage::Network, &ctx);
    let _ = tuner.evaluate(1_000, 2, ScreenBottleneckStage::Network, &ctx);
    let out = tuner.evaluate(1_000, 3, ScreenBottleneckStage::Network, &ctx);
    assert_eq!(out.decision.action, "reduce_bitrate");
    let expect_bps = SCREEN_BITRATE_DEFAULT_BPS
        - SCREEN_BITRATE_DEFAULT_BPS * SCREEN_TUNER_BITRATE_DROP_PCT / 100;
    assert_eq!(out.bitrate_bps, Some(expect_bps));

    // Cooldown: the action fires on the 3rd frame; an immediate re-trigger must hold
    let mut tuner = ScreenTuner::new(ScreenTunerMode::Standard);
    for seq in 1..=3u64 {
        let _ = tuner.evaluate(1_000, seq, ScreenBottleneckStage::Pacer, &ctx);
    }
    let out = tuner.evaluate(1_000 + 1, 4, ScreenBottleneckStage::Pacer, &ctx);
    assert_eq!(
        out.decision.action, "hold",
        "cooldown must suppress immediate re-trigger"
    );
    // After the cooldown elapses the still-running streak triggers again
    let out = tuner.evaluate(3_000_000, 5, ScreenBottleneckStage::Pacer, &ctx);
    assert_eq!(
        out.decision.action, "reduce_bitrate",
        "post-cooldown streak must trigger"
    );
    // ... and the cooldown window restarts from the new action
    let out = tuner.evaluate(3_000_001, 6, ScreenBottleneckStage::Pacer, &ctx);
    assert_eq!(out.decision.action, "hold");

    // Mixed bottleneck types reset the streak
    let mut tuner = ScreenTuner::new(ScreenTunerMode::Standard);
    let _ = tuner.evaluate(1_000, 1, ScreenBottleneckStage::Network, &ctx);
    let _ = tuner.evaluate(1_000, 2, ScreenBottleneckStage::Capture, &ctx);
    let out = tuner.evaluate(1_000, 3, ScreenBottleneckStage::Capture, &ctx);
    assert_eq!(out.decision.action, "hold", "streak resets on stage switch");

    // Capture bottleneck reduces FPS
    let mut tuner = ScreenTuner::new(ScreenTunerMode::Aggressive);
    let _ = tuner.evaluate(1_000, 1, ScreenBottleneckStage::Capture, &ctx);
    let out = tuner.evaluate(2_000_000, 2, ScreenBottleneckStage::Capture, &ctx);
    assert_eq!(out.decision.action, "reduce_fps");
    assert_eq!(out.fps, Some(screen_matrix_next_lower_fps(30)));

    // Jitter bottleneck raises playout delay (receiver side)
    let mut tuner = ScreenTuner::new(ScreenTunerMode::Aggressive);
    let _ = tuner.evaluate(1_000, 1, ScreenBottleneckStage::Jitter, &ctx);
    let out = tuner.evaluate(2_000_000, 2, ScreenBottleneckStage::Jitter, &ctx);
    assert_eq!(out.decision.action, "raise_playout_delay");
    assert_eq!(
        out.playout_delay_delta_us,
        Some(SCREEN_TUNER_PLAYOUT_RELIEF_DELTA_US)
    );

    // Disabled mode never acts
    let mut tuner = ScreenTuner::new(ScreenTunerMode::Disabled);
    for seq in 1..=10u64 {
        let out = tuner.evaluate(1_000 * seq, seq, ScreenBottleneckStage::Network, &ctx);
        assert_eq!(out.decision.action, "hold");
    }
    assert_eq!(tuner.report().actions_applied, 0);

    // 6. Anti-oscillation recovery: after sustained balance the degraded knobs step back up in reverse order
    let mut tuner = ScreenTuner::new(ScreenTunerMode::Aggressive);
    // Network trigger -> reduce_bitrate
    let _ = tuner.evaluate(1_000, 1, ScreenBottleneckStage::Network, &ctx);
    let out = tuner.evaluate(2_000_000, 2, ScreenBottleneckStage::Network, &ctx);
    assert_eq!(out.decision.action, "reduce_bitrate");
    // Capture trigger -> reduce_fps
    let _ = tuner.evaluate(4_000_000, 3, ScreenBottleneckStage::Capture, &ctx);
    let out = tuner.evaluate(6_000_000, 4, ScreenBottleneckStage::Capture, &ctx);
    assert_eq!(out.decision.action, "reduce_fps");

    // Balanced frames below the recovery threshold keep holding
    for seq in 5..=9u64 {
        let out = tuner.evaluate(6_000_000 + seq, seq, ScreenBottleneckStage::Balanced, &ctx);
        assert_eq!(out.decision.action, "hold");
    }
    assert_eq!(tuner.report().balanced_streak, 5);

    // 6th balanced frame (aggressive needs 6) recovers the most recent degradation first: fps
    let rec_ctx = ScreenTuningContext {
        bitrate_bps: expect_bps,
        bitrate_min_bps: SCREEN_BITRATE_MIN_BPS,
        bitrate_max_bps: SCREEN_BITRATE_MAX_BPS,
        fps: screen_matrix_next_lower_fps(30),
        scale_pct: 100,
        qp_min: 12,
        qp_max: 36,
        playout_delay_us: 0,
    };
    let out = tuner.evaluate(10_000_000, 10, ScreenBottleneckStage::Balanced, &rec_ctx);
    assert_eq!(
        out.decision.action, "recover_fps",
        "recovery must undo the most recent degradation first"
    );
    assert!(out.decision.recovered);
    // Next recovery (after the aggressive 1s cooldown elapses) restores bitrate
    let out = tuner.evaluate(11_100_000, 11, ScreenBottleneckStage::Balanced, &rec_ctx);
    assert_eq!(out.decision.action, "recover_bitrate");
    let expect_recover =
        (expect_bps * (100 + SCREEN_TUNER_BITRATE_RECOVER_PCT) / 100).min(SCREEN_BITRATE_MAX_BPS);
    assert_eq!(out.bitrate_bps, Some(expect_recover));
    assert_eq!(tuner.report().recoveries_applied, 2);

    // 7. ScreenSender reset_tuning clears counters but keeps the mode
    sender.set_tuning_mode(ScreenTunerMode::Aggressive);
    assert_eq!(sender.tuning_mode(), ScreenTunerMode::Aggressive);
    let rep = sender.reset_tuning();
    assert_eq!(rep.actions_applied, 0);
    assert_eq!(rep.last_action, "hold");
    assert_eq!(sender.tuning_mode(), ScreenTunerMode::Aggressive);

    // 8. Receiver-side tuner: Jitter bottleneck adjusts the playout buffer target delay
    let mut receiver = ScreenReceiver::new();
    assert_eq!(receiver.tuning_mode(), ScreenTunerMode::Standard);
    receiver.set_tuning_mode(ScreenTunerMode::Aggressive);
    let jitter_profile = |seq: u64| ScreenProfileFrame {
        flags: 0,
        frame_seq: seq,
        capture_us: 1_000,
        encode_us: 1_000,
        pacer_us: 1_000,
        sender_ts_us: seq,
        network_us: 1_000,
        jitter_us: 30_000,
        decode_us: 1_000,
        render_us: 1_000,
        total_e2e_us: 36_000,
    };
    receiver.on_profile_frame(&jitter_profile(1));
    assert_eq!(receiver.tuning_report().actions_applied, 0);
    let delay_before = receiver.playout_stats().target_delay_us;
    receiver.on_profile_frame(&jitter_profile(2));
    let rep = receiver.tuning_report();
    assert_eq!(
        rep.actions_applied, 1,
        "aggressive mode triggers on the 2nd frame"
    );
    assert_eq!(rep.last_action, "raise_playout_delay");
    let delay_after = receiver.playout_stats().target_delay_us;
    assert!(
        delay_after > delay_before,
        "playout target delay must deepen on jitter bottleneck ({delay_before} -> {delay_after})"
    );

    // 9. ScreenStats default tuning telemetry
    let stats = ScreenStats::default();
    assert_eq!(stats.tuning_mode, ScreenTunerMode::Standard.tag());
    assert_eq!(
        stats.tuning_last_stage,
        ScreenBottleneckStage::Balanced.tag()
    );
    assert_eq!(stats.tuning_last_action, ScreenTuningAction::Hold.tag());

    // 10. ScreenTuningAction parse & tag roundtrips
    for tag in 0..=10u8 {
        let action = ScreenTuningAction::from_tag(tag);
        assert_eq!(action.tag(), tag);
    }
    assert_eq!(
        "recover_bitrate".parse::<ScreenTuningAction>().unwrap(),
        ScreenTuningAction::RecoverBitrate
    );
    assert!(ScreenTuningAction::RecoverBitrate.is_recovery());
    assert!(!ScreenTuningAction::ReduceFps.is_recovery());
}

// ADR-101 Screen Streaming Session Recording Container & Replay Pipeline
#[test]
fn screen_recording_container_roundtrip_and_recording_disclosure() {
    // 1. ControlMsg::SetRecordingState wire roundtrip (Tag = 9, 14-byte envelope)
    let on_frame = ControlMsg::SetRecordingState { active: true }
        .to_frame()
        .unwrap();
    assert_eq!(on_frame.ty, TYPE_SCREEN_CONTROL);
    assert_eq!(on_frame.payload.len(), 14);
    assert!(matches!(
        ControlMsg::decode_payload(&on_frame.payload).unwrap(),
        ControlMsg::SetRecordingState { active: true }
    ));
    let off_frame = ControlMsg::SetRecordingState { active: false }
        .to_frame()
        .unwrap();
    assert!(matches!(
        ControlMsg::decode_payload(&off_frame.payload).unwrap(),
        ControlMsg::SetRecordingState { active: false }
    ));

    // 2. Writer -> Reader container roundtrip with three raw frames
    let rect0 = ScreenRect::new(0, 0, 8, 4).unwrap();
    let frame_bytes = rect0.pixel_bytes(PixelFormat::Bgra8);
    let frames: Vec<ScreenFrame> = (1..=3)
        .map(|seq| {
            ScreenFrame::raw(seq, rect0, PixelFormat::Bgra8, vec![7u8; frame_bytes]).unwrap()
        })
        .collect();
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let meta = ScreenRecordingMeta::from_frame(&frames[0], 1_000);
        assert_eq!(meta.width, rect0.w as u32);
        assert_eq!(meta.height, rect0.h as u32);
        let mut writer = ScreenRecordingWriter::new(meta, &mut buf).unwrap();
        let pts = [1_000u64, 16_000, 33_000];
        for (i, f) in frames.iter().enumerate() {
            writer.append(f, pts[i]).unwrap();
        }
        assert_eq!(writer.frames_written(), 3);
        let (fw, bw) = writer.finish().unwrap();
        assert_eq!(fw, 3);
        assert!(bw > 0);
    }
    buf.set_position(0);
    let mut reader = ScreenRecordingReader::new(&mut buf).unwrap();
    let meta = reader.meta().clone();
    assert_eq!(meta.started_at_us, 1_000);
    assert_eq!(meta.format_tag, PixelFormat::Bgra8.tag());
    assert_eq!(meta.codec_tag, SCREEN_COMPRESS_NONE);
    let mut read_back = Vec::new();
    while let Some(rec) = reader.next_record().unwrap() {
        read_back.push(rec);
    }
    assert_eq!(read_back.len(), 3);
    assert_eq!(reader.records_read(), 3);
    for (i, rec) in read_back.iter().enumerate() {
        assert_eq!(rec.seq, frames[i].seq);
        assert_eq!(rec.pts_us, [1_000u64, 16_000, 33_000][i]);
        // decode_frame must restore the exact original ScreenFrame
        let restored = reader.decode_frame(rec).unwrap();
        assert_eq!(restored.seq, frames[i].seq);
        assert_eq!(restored.rect, frames[i].rect);
        assert_eq!(restored.format, frames[i].format);
        assert_eq!(restored.bytes, frames[i].bytes);
        assert_eq!(restored.codec_tag, frames[i].codec_tag);
    }

    // 3. Truncated record -> error, not a hang
    let mut buf2 = std::io::Cursor::new(Vec::new());
    {
        let mut writer =
            ScreenRecordingWriter::new(ScreenRecordingMeta::from_frame(&frames[0], 5), &mut buf2)
                .unwrap();
        writer.append(&frames[0], 5).unwrap();
        writer.finish().unwrap();
    }
    let mut raw = buf2.into_inner();
    raw.truncate(SCREEN_RECORDING_HEADER_LEN + 8);
    let mut reader2 = ScreenRecordingReader::new(&raw[..]).unwrap();
    assert!(reader2.next_record().is_err());

    // 4. Bad magic / proto rejected
    let mut bad = ScreenRecordingMeta::from_frame(&frames[0], 5).encode();
    bad[0] = b'X';
    assert!(ScreenRecordingMeta::decode(&bad).is_err());
    let mut bad_proto = ScreenRecordingMeta::from_frame(&frames[0], 5).encode();
    bad_proto[4] = 0x7F;
    let mut cursor = std::io::Cursor::new(bad_proto);
    assert!(ScreenRecordingReader::new(&mut cursor).is_err());
}

// ADR-102 Screen Session In-Container Playback Engine & Virtual Screen Source Feeder
#[test]
fn screen_session_playback_engine_and_virtual_source_feeder() {
    use crate::ports::ScreenSource;

    // 1. State string parsing & roundtrips
    assert_eq!(ScreenPlaybackState::Idle.as_str(), "idle");
    assert_eq!(ScreenPlaybackState::Playing.as_str(), "playing");
    assert_eq!(ScreenPlaybackState::Paused.as_str(), "paused");
    assert_eq!(ScreenPlaybackState::Finished.as_str(), "finished");
    assert_eq!(ScreenPlaybackState::Stopped.as_str(), "stopped");
    assert_eq!(
        "playing".parse::<ScreenPlaybackState>().unwrap(),
        ScreenPlaybackState::Playing
    );
    assert_eq!(
        "paused".parse::<ScreenPlaybackState>().unwrap(),
        ScreenPlaybackState::Paused
    );
    assert!("invalid_state".parse::<ScreenPlaybackState>().is_err());

    // 2. Prepare in-memory TREC container with 3 frames
    let rect0 = ScreenRect::new(0, 0, 16, 8).unwrap();
    let frame_bytes = rect0.pixel_bytes(PixelFormat::Bgra8);
    let frames: Vec<ScreenFrame> = (1..=3)
        .map(|seq| {
            let mut data = vec![0u8; frame_bytes];
            for (i, b) in data.iter_mut().enumerate() {
                *b = ((seq as usize * 17 + i) & 0xFF) as u8;
            }
            ScreenFrame::raw(seq, rect0, PixelFormat::Bgra8, data).unwrap()
        })
        .collect();

    let mut trec_bytes = Vec::new();
    {
        let meta = ScreenRecordingMeta::from_frame(&frames[0], 1_000);
        let mut writer =
            ScreenRecordingWriter::new(meta, std::io::Cursor::new(&mut trec_bytes)).unwrap();
        writer.append(&frames[0], 1_000).unwrap();
        writer.append(&frames[1], 17_000).unwrap();
        writer.append(&frames[2], 33_000).unwrap();
        writer.finish().unwrap();
    }

    // 3. Reader scan_summary fast summary
    let cursor = std::io::Cursor::new(trec_bytes.clone());
    let mut reader = ScreenRecordingReader::new(cursor).unwrap();
    let (scanned_frames, duration_us) = reader.scan_summary().unwrap();
    assert_eq!(scanned_frames, 3);
    assert_eq!(duration_us, 32_000);

    // 4. ScreenPlaybackEngine: step-by-step presentation, pause, resume, finish
    let sink = MemoryScreenSink::new();
    let mut engine = ScreenPlaybackEngine::from_bytes(trec_bytes.clone(), 0.0, false).unwrap();
    assert_eq!(engine.state(), ScreenPlaybackState::Idle);
    assert_eq!(engine.total_frames(), 3);
    let rep0 = engine.report();
    assert_eq!(rep0.state, "idle");
    assert_eq!(rep0.total_frames, 3);
    assert_eq!(rep0.duration_ms, 32);
    assert_eq!(rep0.width, 16);
    assert_eq!(rep0.height, 8);

    // Step frame 1
    let f1 = engine.step(&sink).unwrap().expect("frame 1");
    assert_eq!(f1.seq, 1);
    assert_eq!(engine.state(), ScreenPlaybackState::Playing);
    assert_eq!(engine.current_frame(), 1);
    assert_eq!(sink.count(), 1);

    // Pause -> stepping does nothing
    engine.pause();
    assert_eq!(engine.state(), ScreenPlaybackState::Paused);
    assert!(engine.step(&sink).unwrap().is_none());
    assert_eq!(sink.count(), 1);

    // Resume -> step frame 2
    engine.resume();
    assert_eq!(engine.state(), ScreenPlaybackState::Playing);
    let f2 = engine.step(&sink).unwrap().expect("frame 2");
    assert_eq!(f2.seq, 2);
    assert_eq!(engine.current_frame(), 2);
    assert_eq!(sink.count(), 2);

    // Step frame 3
    let f3 = engine.step(&sink).unwrap().expect("frame 3");
    assert_eq!(f3.seq, 3);
    assert_eq!(engine.current_frame(), 3);
    assert_eq!(sink.count(), 3);

    // Step frame 4 (EOF) -> state Finished
    assert!(engine.step(&sink).unwrap().is_none());
    assert_eq!(engine.state(), ScreenPlaybackState::Finished);

    // 5. ScreenPlaybackEngine with loop enabled
    let sink_loop = MemoryScreenSink::new();
    let mut loop_engine = ScreenPlaybackEngine::from_bytes(trec_bytes.clone(), 0.0, true).unwrap();
    for _ in 0..7 {
        let frame = loop_engine.step(&sink_loop).unwrap().expect("looped frame");
        assert!(frame.seq >= 1 && frame.seq <= 3);
    }
    assert_eq!(sink_loop.count(), 7);
    loop_engine.stop();
    assert_eq!(loop_engine.state(), ScreenPlaybackState::Stopped);

    // 6. ScreenRecordingSource: Virtual ScreenSource adapter
    let source = ScreenRecordingSource::from_bytes(trec_bytes.clone(), true).unwrap();
    assert_eq!(source.meta().width, 16);
    assert_eq!(source.meta().height, 8);
    let displays = source.displays();
    assert_eq!(displays.len(), 1);
    assert_eq!(displays[0].width, 16);
    assert_eq!(displays[0].height, 8);
    assert!(displays[0].is_primary);

    // Grab 1, 2, 3 and loop to 1
    let g1 = source.grab().unwrap().expect("grab 1");
    assert_eq!(g1.seq, 1);
    let g2 = source.grab().unwrap().expect("grab 2");
    assert_eq!(g2.seq, 2);
    let g3 = source.grab().unwrap().expect("grab 3");
    assert_eq!(g3.seq, 3);
    let g4 = source.grab().unwrap().expect("grab 4 (looped)");
    assert_eq!(g4.seq, 1);
    assert_eq!(source.frames_grabbed(), 4);

    // Disable loop on source -> EOF after frames 2 and 3
    source.set_loop_enabled(false);
    assert_eq!(source.grab().unwrap().expect("grab 5").seq, 2);
    assert_eq!(source.grab().unwrap().expect("grab 6").seq, 3);
    assert!(
        source.grab().unwrap().is_none(),
        "expected EOF when loop disabled"
    );
}

// ADR-103 Screen Live Snapshot BMP Encoding & Recording Frame Extraction
#[test]
fn screen_snapshot_bmp_encoding_and_recording_extraction() {
    // 1. BMP Encoding for Bgra8
    let w = 4u16;
    let h = 4u16;
    let rect = ScreenRect::new(0, 0, w, h).unwrap();
    let pixel_count = (w as usize) * (h as usize);
    let mut bgra_data = Vec::with_capacity(pixel_count * 4);
    for i in 0..pixel_count {
        bgra_data.extend_from_slice(&[
            (i * 10) as u8,     // B
            (i * 10 + 1) as u8, // G
            (i * 10 + 2) as u8, // R
            255,                // A
        ]);
    }
    let frame_bgra = ScreenFrame::raw(1, rect, PixelFormat::Bgra8, bgra_data.clone()).unwrap();
    let bmp_bytes = frame_bgra.to_bmp().expect("to_bmp bgra");

    assert_eq!(bmp_bytes.len(), 54 + pixel_count * 4);
    // Header verification
    assert_eq!(&bmp_bytes[0..2], b"BM");
    let file_size = u32::from_le_bytes(bmp_bytes[2..6].try_into().unwrap());
    assert_eq!(file_size, (54 + pixel_count * 4) as u32);
    let offset = u32::from_le_bytes(bmp_bytes[10..14].try_into().unwrap());
    assert_eq!(offset, 54);

    let bi_size = u32::from_le_bytes(bmp_bytes[14..18].try_into().unwrap());
    assert_eq!(bi_size, 40);
    let bi_width = i32::from_le_bytes(bmp_bytes[18..22].try_into().unwrap());
    assert_eq!(bi_width, 4);
    let bi_height = i32::from_le_bytes(bmp_bytes[22..26].try_into().unwrap());
    assert_eq!(bi_height, -4, "Top-down DIB must have negative biHeight");
    let bi_planes = u16::from_le_bytes(bmp_bytes[26..28].try_into().unwrap());
    assert_eq!(bi_planes, 1);
    let bi_bit_count = u16::from_le_bytes(bmp_bytes[28..30].try_into().unwrap());
    assert_eq!(bi_bit_count, 32);
    let bi_compression = u32::from_le_bytes(bmp_bytes[30..34].try_into().unwrap());
    assert_eq!(bi_compression, 0); // BI_RGB
    let bi_size_image = u32::from_le_bytes(bmp_bytes[34..38].try_into().unwrap());
    assert_eq!(bi_size_image, (pixel_count * 4) as u32);

    // Pixels in BMP match original Bgra8 bytes
    assert_eq!(&bmp_bytes[54..], &bgra_data[..]);

    // 2. BMP Encoding for Rgba8 (channel swap)
    let mut rgba_data = Vec::with_capacity(pixel_count * 4);
    for i in 0..pixel_count {
        rgba_data.extend_from_slice(&[
            (i * 10 + 2) as u8, // R
            (i * 10 + 1) as u8, // G
            (i * 10) as u8,     // B
            255,                // A
        ]);
    }
    let frame_rgba = ScreenFrame::raw(2, rect, PixelFormat::Rgba8, rgba_data).unwrap();
    let bmp_from_rgba = frame_rgba.to_bmp().expect("to_bmp rgba");
    assert_eq!(
        &bmp_from_rgba[54..],
        &bgra_data[..],
        "Rgba8 must be converted to Bgra8 in BMP"
    );

    // 3. Rejection of compressed frame
    let mut compressed_frame = frame_bgra.clone();
    compressed_frame.codec_tag = SCREEN_COMPRESS_ZSTD;
    assert!(compressed_frame.to_bmp().is_err());

    // 4. Recording container extract_frame
    let mut trec_bytes = Vec::new();
    {
        let meta = ScreenRecordingMeta::from_frame(&frame_bgra, 1_000);
        let mut writer =
            ScreenRecordingWriter::new(meta, std::io::Cursor::new(&mut trec_bytes)).unwrap();
        writer.append(&frame_bgra, 1_000).unwrap();

        let f2 =
            ScreenFrame::raw(2, rect, PixelFormat::Bgra8, vec![42u8; pixel_count * 4]).unwrap();
        writer.append(&f2, 16_000).unwrap();

        let f3 =
            ScreenFrame::raw(3, rect, PixelFormat::Bgra8, vec![99u8; pixel_count * 4]).unwrap();
        writer.append(&f3, 33_000).unwrap();
        writer.finish().unwrap();
    }

    let mut reader = ScreenRecordingReader::new(std::io::Cursor::new(trec_bytes)).unwrap();

    // Target seq = 2
    let extracted_2 = reader
        .extract_frame(Some(2), None)
        .unwrap()
        .expect("extract frame 2");
    assert_eq!(extracted_2.seq, 2);
    assert_eq!(extracted_2.bytes[0], 42);

    // Target pts >= 20_000 (should match frame 3 at 33_000)
    let extracted_pts = reader
        .extract_frame(None, Some(20_000))
        .unwrap()
        .expect("extract frame 3 by pts");
    assert_eq!(extracted_pts.seq, 3);
    assert_eq!(extracted_pts.bytes[0], 99);

    // Default: first frame
    let extracted_first = reader
        .extract_frame(None, None)
        .unwrap()
        .expect("extract first");
    assert_eq!(extracted_first.seq, 1);

    // Non-existent seq -> None
    let not_found = reader.extract_frame(Some(999), None).unwrap();
    assert!(not_found.is_none());

    // 5. Default ScreenSnapshotReport
    let report = ScreenSnapshotReport::default();
    assert!(report.ok);
    assert_eq!(report.format, "bmp");
    assert!(!report.privacy_blanked);
    assert!(report.path.is_none());
}

#[test]
fn screen_dynamic_security_watermark_rendering_and_signaling() {
    // 1. Line protocol Tag 10 ControlMsg::SetWatermarkState wire roundtrip
    let msg1 = ControlMsg::SetWatermarkState {
        enabled: true,
        host_enforced: false,
    };
    let payload1 = msg1.encode_payload();
    assert_eq!(payload1.len(), 14);
    assert_eq!(payload1[5], 10);
    let decoded1 = ControlMsg::decode_payload(&payload1).expect("decode tag 10");
    match decoded1 {
        ControlMsg::SetWatermarkState {
            enabled,
            host_enforced,
        } => {
            assert!(enabled);
            assert!(!host_enforced);
        }
        _ => panic!("unexpected decoded variant"),
    }

    let msg2 = ControlMsg::SetWatermarkState {
        enabled: false,
        host_enforced: true,
    };
    let frame2 = msg2.to_frame().expect("to_frame");
    let decoded2 = ControlMsg::from_frame(&frame2).expect("from_frame");
    match decoded2 {
        ControlMsg::SetWatermarkState {
            enabled,
            host_enforced,
        } => {
            assert!(!enabled);
            assert!(host_enforced);
        }
        _ => panic!("unexpected decoded variant"),
    }

    // 2. Pattern parsing and default config
    assert_eq!(
        "grid".parse::<WatermarkPattern>().unwrap(),
        WatermarkPattern::Grid
    );
    assert_eq!(
        "single".parse::<WatermarkPattern>().unwrap(),
        WatermarkPattern::Single
    );
    assert_eq!(
        "banner".parse::<WatermarkPattern>().unwrap(),
        WatermarkPattern::Banner
    );
    assert!("invalid".parse::<WatermarkPattern>().is_err());

    let mut config = ScreenWatermarkConfig::default();
    assert!(!config.enabled);
    assert!(!config.host_enforced);
    assert_eq!(config.pattern, WatermarkPattern::Grid);

    // 3. Pixel rendering tests
    let rect = ScreenRect::new(0, 0, 160, 80).unwrap();
    let pixel_count = (160 * 80) as usize;
    let black_pixels = vec![0u8; pixel_count * 4];

    // Frame in Bgra8 format
    let mut frame_bgra =
        ScreenFrame::raw(1, rect, PixelFormat::Bgra8, black_pixels.clone()).unwrap();

    // Disabled watermark -> Ok(false)
    let applied_disabled = frame_bgra
        .apply_watermark(&config, "peer-abc", "2026-10-27")
        .unwrap();
    assert!(!applied_disabled);
    assert_eq!(frame_bgra.bytes, black_pixels);

    // Enabled watermark with 50% opacity
    config.enabled = true;
    config.opacity = 128;
    config.color_rgb = [200, 100, 50]; // R=200, G=100, B=50
    config.text_template = "CONFIDENTIAL-{peer}".to_string();

    let applied = frame_bgra.apply_watermark(&config, "dev1", "now").unwrap();
    assert!(applied);
    // At least some pixels should now be non-zero
    let non_zero_count = frame_bgra.bytes.iter().filter(|&&b| b > 0).count();
    assert!(
        non_zero_count > 50,
        "watermark text should render non-zero pixels"
    );

    // Single pattern
    config.pattern = WatermarkPattern::Single;
    let mut frame_single =
        ScreenFrame::raw(2, rect, PixelFormat::Bgra8, black_pixels.clone()).unwrap();
    assert!(frame_single
        .apply_watermark(&config, "dev1", "now")
        .unwrap());

    // Banner pattern
    config.pattern = WatermarkPattern::Banner;
    let mut frame_banner =
        ScreenFrame::raw(3, rect, PixelFormat::Bgra8, black_pixels.clone()).unwrap();
    assert!(frame_banner
        .apply_watermark(&config, "dev1", "now")
        .unwrap());

    // Rgba8 pixel format
    config.pattern = WatermarkPattern::Grid;
    let mut frame_rgba = ScreenFrame::raw(4, rect, PixelFormat::Rgba8, black_pixels).unwrap();
    assert!(frame_rgba.apply_watermark(&config, "dev1", "now").unwrap());

    // Compressed frame rejection
    let mut frame_compressed = frame_rgba.clone();
    frame_compressed.codec_tag = SCREEN_COMPRESS_ZSTD;
    assert!(frame_compressed
        .apply_watermark(&config, "dev1", "now")
        .is_err());
}

// ADR-105 Screen Streaming Collaborative Annotation, Whiteboard Overlay & Transient Laser Pointer Pipeline
#[test]
fn screen_collaborative_annotation_and_laser_pointer_pipeline() {
    // 1. Tag 11 ControlMsg::SetLaserPointer wire roundtrip
    let laser_msg = ControlMsg::SetLaserPointer {
        x: 320,
        y: 240,
        active: true,
        color_rgb: [255, 60, 100],
    };
    let payload = laser_msg.encode_payload();
    assert_eq!(payload.len(), 14);
    assert_eq!(payload[5], 11);
    let decoded = ControlMsg::decode_payload(&payload).expect("decode tag 11");
    match decoded {
        ControlMsg::SetLaserPointer {
            x,
            y,
            active,
            color_rgb,
        } => {
            assert_eq!(x, 320);
            assert_eq!(y, 240);
            assert!(active);
            assert_eq!(color_rgb, [255, 60, 100]);
        }
        _ => panic!("expected SetLaserPointer"),
    }

    // 2. Tag 12 ControlMsg::ClearAnnotations wire roundtrip
    let clear_msg = ControlMsg::ClearAnnotations;
    let payload = clear_msg.encode_payload();
    assert_eq!(payload.len(), 14);
    assert_eq!(payload[5], 12);
    let decoded = ControlMsg::decode_payload(&payload).expect("decode tag 12");
    assert!(matches!(decoded, ControlMsg::ClearAnnotations));

    // 3. ScreenAnnotationMsg inner frame roundtrip
    let stroke_item = AnnotationItem {
        id: 1,
        author: "visitor1".into(),
        kind: AnnotationKind::Stroke {
            points: vec![(10, 10), (50, 10), (50, 50)],
        },
        color_rgb: [255, 0, 0],
        thickness: 4,
        alpha: 200,
        ttl_ms: 5000,
        created_at_ms: 1000,
    };
    let msg = ScreenAnnotationMsg::Add(stroke_item.clone());
    let frame = msg.to_frame().expect("to_frame");
    assert_eq!(frame.ty, TYPE_SCREEN_ANNOTATION);
    let decoded_msg = ScreenAnnotationMsg::from_frame(&frame).expect("from_frame");
    match decoded_msg {
        ScreenAnnotationMsg::Add(item) => {
            assert_eq!(item.id, 1);
            assert_eq!(item.author, "visitor1");
            assert_eq!(item.color_rgb, [255, 0, 0]);
            assert_eq!(item.thickness, 4);
            assert_eq!(item.alpha, 200);
            assert_eq!(item.ttl_ms, 5000);
        }
        _ => panic!("expected Add"),
    }

    // Clear annotation msg
    let clear_frame = ScreenAnnotationMsg::Clear.to_frame().expect("to_frame");
    let decoded_clear = ScreenAnnotationMsg::from_frame(&clear_frame).expect("from_frame");
    assert!(matches!(decoded_clear, ScreenAnnotationMsg::Clear));

    // 4. Rasterization & Annotation Application on ScreenFrame
    let rect = ScreenRect::new(0, 0, 100, 100).unwrap();
    let pixel_count = 100 * 100;
    let initial_bgra = vec![0u8; pixel_count * 4]; // All black, transparent
    let mut frame = ScreenFrame::raw(1, rect, PixelFormat::Bgra8, initial_bgra).unwrap();

    let rect_item = AnnotationItem {
        id: 2,
        author: "visitor2".into(),
        kind: AnnotationKind::Rect {
            x: 20,
            y: 20,
            w: 30,
            h: 30,
        },
        color_rgb: [0, 255, 0],
        thickness: 2,
        alpha: 255,
        ttl_ms: 0, // Permanent
        created_at_ms: 1000,
    };

    let arrow_item = AnnotationItem {
        id: 3,
        author: "visitor1".into(),
        kind: AnnotationKind::Arrow {
            from: (10, 80),
            to: (60, 80),
        },
        color_rgb: [0, 0, 255],
        thickness: 2,
        alpha: 255,
        ttl_ms: 3000, // Expires at 4000ms
        created_at_ms: 1000,
    };

    let text_item = AnnotationItem {
        id: 4,
        author: "visitor1".into(),
        kind: AnnotationKind::Text {
            x: 5,
            y: 5,
            text: "TEST".into(),
            scale: 1,
        },
        color_rgb: [255, 255, 255],
        thickness: 1,
        alpha: 255,
        ttl_ms: 2000, // Expires at 3000ms
        created_at_ms: 1000,
    };

    // Apply at now_ms = 1500 (none expired)
    let applied = frame
        .apply_annotations(
            &[
                stroke_item.clone(),
                rect_item.clone(),
                arrow_item.clone(),
                text_item.clone(),
            ],
            1500,
        )
        .expect("apply annotations");
    assert_eq!(applied, 4);

    // Verify pixels were changed
    let has_non_zero = frame.bytes.iter().any(|&b| b > 0);
    assert!(has_non_zero);

    // Apply at now_ms = 3500 (text_item expired at 3000ms, others active)
    let applied_later = frame
        .apply_annotations(
            &[stroke_item, rect_item.clone(), arrow_item, text_item],
            3500,
        )
        .expect("apply annotations at 3500ms");
    assert_eq!(applied_later, 3, "expired text item should be skipped");

    // 5. Laser Pointer rendering
    let active_laser = LaserPointerState {
        x: 50,
        y: 50,
        active: true,
        color_rgb: [255, 50, 50],
        radius: 12,
    };
    let laser_drawn = frame
        .apply_laser_pointer(&active_laser)
        .expect("laser pointer");
    assert!(laser_drawn);

    let inactive_laser = LaserPointerState {
        x: 50,
        y: 50,
        active: false,
        color_rgb: [255, 50, 50],
        radius: 12,
    };
    let inactive_drawn = frame
        .apply_laser_pointer(&inactive_laser)
        .expect("inactive laser");
    assert!(!inactive_drawn);

    // Compressed frame rejection
    let mut compressed = frame.clone();
    compressed.codec_tag = SCREEN_COMPRESS_ZSTD;
    assert!(compressed.apply_laser_pointer(&active_laser).is_err());
    assert!(compressed.apply_annotations(&[], 0).is_ok()); // empty is ok
    assert!(compressed.apply_annotations(&[rect_item], 0).is_err());
}

// ADR-106 Screen Streaming Multi-Party Shared Whiteboard Synchronization, Chronological Undo/Redo & Vector Snapshot Pipeline
#[test]
fn screen_whiteboard_multi_user_sync_and_undo_redo_pipeline() {
    // 1. Tag 13, 14, 15 ControlMsg wire roundtrips
    let undo_msg = ControlMsg::WhiteboardUndo {
        author_hash: 0x1122334455667788,
    };
    let payload13 = undo_msg.encode_payload();
    assert_eq!(payload13.len(), 14);
    assert_eq!(payload13[5], 13);
    let decoded13 = ControlMsg::decode_payload(&payload13).expect("decode tag 13");
    match decoded13 {
        ControlMsg::WhiteboardUndo { author_hash } => {
            assert_eq!(author_hash, 0x1122334455667788);
        }
        _ => panic!("expected WhiteboardUndo"),
    }

    let redo_msg = ControlMsg::WhiteboardRedo {
        author_hash: 0x99AABBCCDDEEFF00,
    };
    let payload14 = redo_msg.encode_payload();
    assert_eq!(payload14.len(), 14);
    assert_eq!(payload14[5], 14);
    let decoded14 = ControlMsg::decode_payload(&payload14).expect("decode tag 14");
    match decoded14 {
        ControlMsg::WhiteboardRedo { author_hash } => {
            assert_eq!(author_hash, 0x99AABBCCDDEEFF00);
        }
        _ => panic!("expected WhiteboardRedo"),
    }

    let policy_msg = ControlMsg::SetWhiteboardPolicy {
        read_only: true,
        allow_peer_clear: false,
        max_items: 450,
    };
    let payload15 = policy_msg.encode_payload();
    assert_eq!(payload15.len(), 14);
    assert_eq!(payload15[5], 15);
    let decoded15 = ControlMsg::decode_payload(&payload15).expect("decode tag 15");
    match decoded15 {
        ControlMsg::SetWhiteboardPolicy {
            read_only,
            allow_peer_clear,
            max_items,
        } => {
            assert!(read_only);
            assert!(!allow_peer_clear);
            assert_eq!(max_items, 450);
        }
        _ => panic!("expected SetWhiteboardPolicy"),
    }

    // 2. hash_author helper properties
    assert_eq!(hash_author(""), 0);
    let alice_hash = hash_author("alice");
    let bob_hash = hash_author("bob");
    assert_ne!(alice_hash, 0);
    assert_ne!(bob_hash, 0);
    assert_ne!(alice_hash, bob_hash);
    assert_eq!(hash_author("alice"), alice_hash);

    // 3. ScreenAnnotationMsg inner frame roundtrips
    let undo_ann = ScreenAnnotationMsg::Undo {
        author: "alice".into(),
    };
    let frame_undo = undo_ann.to_frame().expect("to_frame");
    let decoded_undo = ScreenAnnotationMsg::from_frame(&frame_undo).expect("from_frame");
    match decoded_undo {
        ScreenAnnotationMsg::Undo { author } => assert_eq!(author, "alice"),
        _ => panic!("expected Undo"),
    }

    let policy_ann = ScreenAnnotationMsg::Policy(WhiteboardPolicy {
        read_only: true,
        allow_peer_clear: false,
        max_items: 500,
    });
    let frame_pol = policy_ann.to_frame().expect("to_frame");
    let decoded_pol = ScreenAnnotationMsg::from_frame(&frame_pol).expect("from_frame");
    match decoded_pol {
        ScreenAnnotationMsg::Policy(pol) => {
            assert!(pol.read_only);
            assert!(!pol.allow_peer_clear);
            assert_eq!(pol.max_items, 500);
        }
        _ => panic!("expected Policy"),
    }

    // 4. WhiteboardHistory: multi-author undo/redo, capacity and snapshot
    let mut history = WhiteboardHistory::new(WhiteboardPolicy::default());

    let item_alice = AnnotationItem {
        id: 0,
        author: "alice".into(),
        kind: AnnotationKind::Rect {
            x: 10,
            y: 10,
            w: 20,
            h: 20,
        },
        color_rgb: [255, 0, 0],
        thickness: 2,
        alpha: 255,
        ttl_ms: 0,
        created_at_ms: 100,
    };
    let id1 = history.add_item(item_alice).expect("add alice");
    assert_eq!(id1, 1);

    let item_bob = AnnotationItem {
        id: 0,
        author: "bob".into(),
        kind: AnnotationKind::Arrow {
            from: (50, 50),
            to: (80, 80),
        },
        color_rgb: [0, 255, 0],
        thickness: 3,
        alpha: 255,
        ttl_ms: 0,
        created_at_ms: 200,
    };
    let id2 = history.add_item(item_bob).expect("add bob");
    assert_eq!(id2, 2);
    assert_eq!(history.active_count(), 2);

    // Undo for Alice only: Alice's item is removed, Bob's item remains active!
    let undone_alice = history.undo(Some("alice")).expect("undo alice");
    assert_eq!(undone_alice.author, "alice");
    assert_eq!(history.active_count(), 1);
    assert_eq!(history.active_items()[0].author, "bob");
    assert_eq!(history.redo_count(), 1);

    // Redo for Alice
    let redone_alice = history.redo(Some("alice")).expect("redo alice");
    assert_eq!(redone_alice.author, "alice");
    assert_eq!(history.active_count(), 2);
    assert_eq!(history.redo_count(), 0);

    // Global undo: pops the last item (Alice)
    let global_undone = history.undo(None).expect("global undo");
    assert_eq!(global_undone.author, "alice");
    assert_eq!(history.active_count(), 1);

    // Snapshot Export & Import
    let snapshot = history.export_snapshot("sess-1".into(), 300);
    assert_eq!(snapshot.session_id, "sess-1");
    assert_eq!(snapshot.active_items.len(), 1);
    assert_eq!(snapshot.active_items[0].author, "bob");

    let mut fresh_history = WhiteboardHistory::new(WhiteboardPolicy::default());
    let imported = fresh_history
        .import_snapshot(snapshot, true)
        .expect("import");
    assert_eq!(imported, 1);
    assert_eq!(fresh_history.active_count(), 1);
    assert_eq!(fresh_history.active_items()[0].author, "bob");

    // Read-only policy rejection
    fresh_history.set_policy(WhiteboardPolicy {
        read_only: true,
        allow_peer_clear: false,
        max_items: 10,
    });
    let rejected = fresh_history.add_item(AnnotationItem {
        id: 0,
        author: "eve".into(),
        kind: AnnotationKind::Text {
            x: 0,
            y: 0,
            text: "test".into(),
            scale: 1,
        },
        color_rgb: [0, 0, 0],
        thickness: 1,
        alpha: 255,
        ttl_ms: 0,
        created_at_ms: 0,
    });
    assert!(rejected.is_err());
}

#[test]
fn screen_roi_cropping_and_viewport_signaling() {
    // 1. Line protocol 14-byte envelope tag 16, 17, 18 roundtrips
    let msg_roi = ControlMsg::SetScreenRoi {
        x: 100,
        y: 200,
        w: 800,
        h: 600,
    };
    assert_eq!(msg_roi.tag(), 16);
    let payload = msg_roi.encode_payload();
    assert_eq!(payload.len(), 14);
    let decoded = ControlMsg::decode_payload(&payload).expect("decode roi");
    assert_eq!(decoded, msg_roi);

    let msg_reset = ControlMsg::ResetScreenRoi;
    assert_eq!(msg_reset.tag(), 17);
    let payload = msg_reset.encode_payload();
    let decoded = ControlMsg::decode_payload(&payload).expect("decode reset");
    assert_eq!(decoded, msg_reset);

    let msg_zoom = ControlMsg::SetScreenRoiZoom { zoom_percent: 250 };
    assert_eq!(msg_zoom.tag(), 18);
    let payload = msg_zoom.encode_payload();
    let decoded = ControlMsg::decode_payload(&payload).expect("decode zoom");
    assert_eq!(decoded, msg_zoom);

    // 2. ScreenRoi sanitization & clamping
    let mut roi = ScreenRoi {
        active: true,
        x: 3800,
        y: 2100,
        w: 1921,
        h: 1081,
        zoom_percent: 30, // below 50 -> clamped to 50
    };
    roi.sanitize(3840, 2160);
    assert!(roi.active);
    assert_eq!(roi.zoom_percent, 50);
    assert_eq!(roi.x, 3800);
    assert_eq!(roi.y, 2100);
    assert_eq!(roi.w, 40); // 3840 - 3800 = 40 (even)
    assert_eq!(roi.h, 60); // 2160 - 2100 = 60 (even)

    // 3. Coordinate mapping
    let test_roi = ScreenRoi {
        active: true,
        x: 100,
        y: 100,
        w: 200,
        h: 200,
        zoom_percent: 100,
    };
    assert_eq!(test_roi.map_screen_to_roi(150, 150), Some((50, 50)));
    assert_eq!(test_roi.map_screen_to_roi(50, 150), None);
    assert_eq!(test_roi.map_screen_to_roi(350, 150), None);
    assert_eq!(test_roi.map_roi_to_screen(50, 50), (150, 150));

    // Inactive ROI map pass-through
    let inactive_roi = ScreenRoi {
        active: false,
        ..test_roi
    };
    assert_eq!(inactive_roi.map_screen_to_roi(50, 50), Some((50, 50)));
    assert_eq!(inactive_roi.map_roi_to_screen(50, 50), (50, 50));

    // 4. Pixel-perfect row-stride cropping (32x32 -> 16x16)
    let src_w = 32u16;
    let src_h = 32u16;
    let mut src_bytes = vec![0u8; (src_w as usize) * (src_h as usize) * 4];
    for y in 0..src_h as usize {
        for x in 0..src_w as usize {
            let idx = (y * (src_w as usize) + x) * 4;
            src_bytes[idx] = x as u8;
            src_bytes[idx + 1] = y as u8;
            src_bytes[idx + 2] = (x ^ y) as u8;
            src_bytes[idx + 3] = 255;
        }
    }
    let full_frame = ScreenFrame::raw(
        1,
        ScreenRect::new(0, 0, src_w, src_h).unwrap(),
        PixelFormat::Bgra8,
        src_bytes,
    )
    .expect("raw frame");

    // Pass-through when inactive
    let cropped_inactive = full_frame.crop_roi(&inactive_roi).expect("inactive crop");
    assert_eq!(cropped_inactive.rect.w, 32);
    assert_eq!(cropped_inactive.rect.h, 32);
    assert_eq!(cropped_inactive.bytes.len(), full_frame.bytes.len());

    // Crop center 16x16
    let crop_active_roi = ScreenRoi {
        active: true,
        x: 8,
        y: 8,
        w: 16,
        h: 16,
        zoom_percent: 200,
    };
    let cropped = full_frame.crop_roi(&crop_active_roi).expect("crop active");
    assert_eq!(cropped.rect.x, 8);
    assert_eq!(cropped.rect.y, 8);
    assert_eq!(cropped.rect.w, 16);
    assert_eq!(cropped.rect.h, 16);
    assert_eq!(cropped.bytes.len(), 16 * 16 * 4);

    // Verify pixel content in cropped sub-rectangle
    for cy in 0..16usize {
        for cx in 0..16usize {
            let c_idx = (cy * 16 + cx) * 4;
            let sx = (8 + cx) as u8;
            let sy = (8 + cy) as u8;
            assert_eq!(cropped.bytes[c_idx], sx);
            assert_eq!(cropped.bytes[c_idx + 1], sy);
            assert_eq!(cropped.bytes[c_idx + 2], sx ^ sy);
            assert_eq!(cropped.bytes[c_idx + 3], 255);
        }
    }
}

#[test]
fn screen_adaptive_color_depth_and_grayscale_signaling_and_quantization() {
    // 1. Line protocol 14-byte envelope tag 19, 20 roundtrips
    let msg_color = ControlMsg::SetScreenColorMode {
        mode: ScreenColorMode::Grayscale8.tag(),
        threshold: 128,
    };
    assert_eq!(msg_color.tag(), 19);
    let payload = msg_color.encode_payload();
    assert_eq!(payload.len(), 14);
    assert_eq!(payload[5], 19);
    let decoded = ControlMsg::decode_payload(&payload).expect("decode color mode");
    assert_eq!(decoded, msg_color);

    let msg_reset = ControlMsg::ResetScreenColorMode;
    assert_eq!(msg_reset.tag(), 20);
    let payload = msg_reset.encode_payload();
    assert_eq!(payload.len(), 14);
    assert_eq!(payload[5], 20);
    let decoded = ControlMsg::decode_payload(&payload).expect("decode reset color");
    assert_eq!(decoded, msg_reset);

    // 2. ScreenColorMode parsing & Display
    assert_eq!(
        "full".parse::<ScreenColorMode>().unwrap(),
        ScreenColorMode::FullColor32
    );
    assert_eq!(
        "reduced".parse::<ScreenColorMode>().unwrap(),
        ScreenColorMode::ReducedColor16
    );
    assert_eq!(
        "grayscale".parse::<ScreenColorMode>().unwrap(),
        ScreenColorMode::Grayscale8
    );
    assert_eq!(
        "monochrome".parse::<ScreenColorMode>().unwrap(),
        ScreenColorMode::Monochrome1
    );
    assert_eq!(
        ScreenColorMode::from_tag(1),
        ScreenColorMode::ReducedColor16
    );
    assert_eq!(ScreenColorMode::from_tag(2), ScreenColorMode::Grayscale8);
    assert_eq!(ScreenColorMode::from_tag(3), ScreenColorMode::Monochrome1);
    assert_eq!(ScreenColorMode::from_tag(0), ScreenColorMode::FullColor32);
    assert_eq!(ScreenColorMode::Grayscale8.as_str(), "grayscale8");

    // 3. ScreenColorConfig defaults & clamping
    let cfg_default = ScreenColorConfig::default();
    assert_eq!(cfg_default.mode, ScreenColorMode::FullColor32);
    assert_eq!(cfg_default.threshold, 128);
    assert!(!cfg_default.is_active());

    let cfg_mono = ScreenColorConfig::new(ScreenColorMode::Monochrome1, Some(200));
    assert!(cfg_mono.is_active());
    assert_eq!(cfg_mono.threshold, 200);

    let cfg_clamped = ScreenColorConfig::new(ScreenColorMode::Monochrome1, Some(255));
    assert_eq!(cfg_clamped.threshold, 254);

    // 4. Test In-Place Grayscale8 Transformation on raw BGRA8 frame
    let rect = ScreenRect::new(0, 0, 2, 2).unwrap();
    // Pixel 0: Red (B=0, G=0, R=255)
    // Pixel 1: Green (B=0, G=255, R=0)
    // Pixel 2: Blue (B=255, G=0, R=0)
    // Pixel 3: White (B=255, G=255, R=255)
    let bgra_pixels = vec![
        0, 0, 255, 255, // Red
        0, 255, 0, 255, // Green
        255, 0, 0, 255, // Blue
        255, 255, 255, 255, // White
    ];
    let mut frame_bgra = ScreenFrame::raw(1, rect, PixelFormat::Bgra8, bgra_pixels).unwrap();
    let cfg_gray = ScreenColorConfig::new(ScreenColorMode::Grayscale8, None);
    frame_bgra
        .apply_color_mode(&cfg_gray)
        .expect("apply grayscale");

    // Y_red = (77 * 255 + 0 + 0) >> 8 = 76
    // Y_green = (150 * 255) >> 8 = 149
    // Y_blue = (29 * 255) >> 8 = 28
    // Y_white = (77*255 + 150*255 + 29*255) >> 8 = 255
    let px0_y = frame_bgra.bytes[0];
    assert_eq!(px0_y, 76);
    assert_eq!(frame_bgra.bytes[1], 76); // G == Y
    assert_eq!(frame_bgra.bytes[2], 76); // R == Y
    assert_eq!(frame_bgra.bytes[3], 255); // Alpha preserved

    let px1_y = frame_bgra.bytes[4];
    assert_eq!(px1_y, 149);
    assert_eq!(frame_bgra.bytes[5], 149);
    assert_eq!(frame_bgra.bytes[6], 149);

    let px2_y = frame_bgra.bytes[8];
    assert_eq!(px2_y, 28);
    assert_eq!(frame_bgra.bytes[9], 28);
    assert_eq!(frame_bgra.bytes[10], 28);

    let px3_y = frame_bgra.bytes[12];
    assert_eq!(px3_y, 255);
    assert_eq!(frame_bgra.bytes[13], 255);
    assert_eq!(frame_bgra.bytes[14], 255);

    // 5. Test Monochrome1 bi-level binarization with threshold 100
    let rgba_pixels = vec![
        255, 0, 0, 255, // Red (Y=76 < 100 -> 0)
        0, 255, 0, 255, // Green (Y=149 >= 100 -> 255)
        0, 0, 255, 255, // Blue (Y=28 < 100 -> 0)
        255, 255, 255, 255, // White (Y=255 >= 100 -> 255)
    ];
    let mut frame_mono = ScreenFrame::raw(2, rect, PixelFormat::Rgba8, rgba_pixels).unwrap();
    let cfg_mono100 = ScreenColorConfig::new(ScreenColorMode::Monochrome1, Some(100));
    frame_mono
        .apply_color_mode(&cfg_mono100)
        .expect("apply mono");

    // Red -> 0
    assert_eq!(frame_mono.bytes[0..3], [0, 0, 0]);
    // Green -> 255
    assert_eq!(frame_mono.bytes[4..7], [255, 255, 255]);
    // Blue -> 0
    assert_eq!(frame_mono.bytes[8..11], [0, 0, 0]);
    // White -> 255
    assert_eq!(frame_mono.bytes[12..15], [255, 255, 255]);

    // 6. Test ReducedColor16 (RGB565 quantization)
    let test_pixels = vec![123, 234, 45, 255];
    let mut frame_reduced = ScreenFrame::raw(
        3,
        ScreenRect::new(0, 0, 1, 1).unwrap(),
        PixelFormat::Rgba8,
        test_pixels,
    )
    .unwrap();
    let cfg_reduced = ScreenColorConfig::new(ScreenColorMode::ReducedColor16, None);
    frame_reduced
        .apply_color_mode(&cfg_reduced)
        .expect("apply reduced");

    let r_orig = 123u8;
    let g_orig = 234u8;
    let b_orig = 45u8;
    let r_expected = (r_orig & 0xF8) | (r_orig >> 5);
    let g_expected = (g_orig & 0xFC) | (g_orig >> 6);
    let b_expected = (b_orig & 0xF8) | (b_orig >> 5);
    assert_eq!(frame_reduced.bytes[0], r_expected);
    assert_eq!(frame_reduced.bytes[1], g_expected);
    assert_eq!(frame_reduced.bytes[2], b_expected);
    assert_eq!(frame_reduced.bytes[3], 255);
}

#[test]
fn screen_dynamic_delta_patch_and_temporal_compression_algorithms() {
    // 1. Line protocol 14-byte envelope tag 21, 22 roundtrips
    let msg_delta = ControlMsg::SetScreenDeltaMode {
        enabled: true,
        keyframe_interval: 120,
    };
    assert_eq!(msg_delta.tag(), 21);
    let payload = msg_delta.encode_payload();
    assert_eq!(payload.len(), 14);
    assert_eq!(payload[5], 21);
    let decoded = ControlMsg::decode_payload(&payload).expect("decode delta mode");
    assert_eq!(decoded, msg_delta);

    let msg_reset = ControlMsg::ResetScreenDeltaMode;
    assert_eq!(msg_reset.tag(), 22);
    let payload = msg_reset.encode_payload();
    assert_eq!(payload.len(), 14);
    assert_eq!(payload[5], 22);
    let decoded = ControlMsg::decode_payload(&payload).expect("decode reset delta");
    assert_eq!(decoded, msg_reset);

    // 2. ScreenDeltaConfig defaults & clamping
    let cfg_default = ScreenDeltaConfig::default();
    assert!(!cfg_default.enabled);
    assert_eq!(
        cfg_default.keyframe_interval,
        SCREEN_DELTA_DEFAULT_KEYFRAME_INTERVAL
    );

    let cfg_custom = ScreenDeltaConfig::new(true, Some(30));
    assert!(cfg_custom.enabled);
    assert_eq!(cfg_custom.keyframe_interval, 30);

    let cfg_clamped_low = ScreenDeltaConfig::new(true, Some(0));
    assert_eq!(cfg_clamped_low.keyframe_interval, 1);

    let cfg_clamped_high = ScreenDeltaConfig::new(true, Some(999));
    assert_eq!(cfg_clamped_high.keyframe_interval, 600);

    // 3. ScreenDeltaReport default
    let report = ScreenDeltaReport::default();
    assert!(report.ok);
    assert!(!report.enabled);
    assert_eq!(report.keyframe_interval, 60);
    assert_eq!(report.saved_bytes, 0);

    // 4. Test extract_sub_rect and apply_sub_rect algorithms
    let w = 8u16;
    let h = 8u16;
    let rect = ScreenRect::new(0, 0, w, h).unwrap();
    let mut base_pixels = vec![0u8; (w as usize) * (h as usize) * 4];
    for y in 0..h {
        for x in 0..w {
            let idx = ((y as usize) * (w as usize) + (x as usize)) * 4;
            base_pixels[idx] = (x * 10) as u8; // B
            base_pixels[idx + 1] = (y * 10) as u8; // G
            base_pixels[idx + 2] = ((x + y) * 5) as u8; // R
            base_pixels[idx + 3] = 255; // A
        }
    }
    let full_frame = ScreenFrame::raw(1, rect, PixelFormat::Bgra8, base_pixels.clone()).unwrap();

    // Extract dirty patch at (2, 2) size 4x3
    let patch = full_frame
        .extract_sub_rect(2, 2, 4, 3)
        .expect("extract sub rect");
    assert_eq!(patch.rect.x, 2);
    assert_eq!(patch.rect.y, 2);
    assert_eq!(patch.rect.w, 4);
    assert_eq!(patch.rect.h, 3);
    assert_eq!(patch.bytes.len(), 4 * 3 * 4);

    // Verify patch pixels match expected region from full_frame
    for py in 0..3 {
        for px in 0..4 {
            let patch_idx = (py * 4 + px) * 4;
            let full_x = 2 + px;
            let full_y = 2 + py;
            let full_idx = (full_y * 8 + full_x) * 4;
            assert_eq!(
                patch.bytes[patch_idx..patch_idx + 4],
                base_pixels[full_idx..full_idx + 4]
            );
        }
    }

    // Now test apply_sub_rect onto an empty (zeroed) receiver canvas
    let canvas_pixels = vec![0u8; (w as usize) * (h as usize) * 4];
    let mut canvas = ScreenFrame::raw(0, rect, PixelFormat::Bgra8, canvas_pixels).unwrap();
    canvas
        .apply_sub_rect(&patch)
        .expect("apply patch to canvas");

    // Check that patch region in canvas is updated, and outside is still zero
    for y in 0..h {
        for x in 0..w {
            let idx = ((y as usize) * (w as usize) + (x as usize)) * 4;
            if (2..6).contains(&x) && (2..5).contains(&y) {
                assert_eq!(canvas.bytes[idx..idx + 4], base_pixels[idx..idx + 4]);
            } else {
                assert_eq!(canvas.bytes[idx..idx + 4], [0, 0, 0, 0]);
            }
        }
    }

    // Test error cases for apply_sub_rect:
    // Out-of-bounds patch
    let oob_patch = ScreenFrame {
        seq: 2,
        rect: ScreenRect::new(6, 6, 4, 4).unwrap(), // 6+4=10 > 8
        format: PixelFormat::Bgra8,
        codec_tag: SCREEN_COMPRESS_NONE,
        stride: 16,
        bytes: vec![0u8; 4 * 4 * 4],
    };
    assert!(canvas.apply_sub_rect(&oob_patch).is_err());

    // Format mismatch
    let format_mismatch_patch = ScreenFrame {
        seq: 3,
        rect: ScreenRect::new(0, 0, 2, 2).unwrap(),
        format: PixelFormat::Rgba8,
        codec_tag: SCREEN_COMPRESS_NONE,
        stride: 8,
        bytes: vec![0u8; 2 * 2 * 4],
    };
    assert!(canvas.apply_sub_rect(&format_mismatch_patch).is_err());

    // 5. Wire frame serialization with SCREEN_COMPRESS_DELTA
    let mut delta_wire_frame = patch.clone();
    delta_wire_frame.codec_tag = SCREEN_COMPRESS_DELTA;
    let encoded = delta_wire_frame
        .encode_payload()
        .expect("encode delta frame");
    let decoded = ScreenFrame::decode_payload(&encoded).expect("decode delta frame");
    assert_eq!(decoded.codec_tag, SCREEN_COMPRESS_DELTA);
    assert_eq!(decoded.rect, patch.rect);
    assert_eq!(decoded.bytes, patch.bytes);
}

#[test]
fn screen_objective_visual_quality_metric_and_psnr_ssim_algorithms() {
    // 1. Line protocol 14-byte envelope tag 23, 24 roundtrips
    let msg_metric = ControlMsg::SetScreenQualityMetricMode {
        enabled: true,
        sample_interval: 15,
    };
    assert_eq!(msg_metric.tag(), 23);
    let payload = msg_metric.encode_payload();
    assert_eq!(payload.len(), 14);
    assert_eq!(payload[5], 23);
    let decoded = ControlMsg::decode_payload(&payload).expect("decode metric mode");
    assert_eq!(decoded, msg_metric);

    let msg_reset = ControlMsg::ResetScreenQualityMetricMode;
    assert_eq!(msg_reset.tag(), 24);
    let payload = msg_reset.encode_payload();
    assert_eq!(payload.len(), 14);
    assert_eq!(payload[5], 24);
    let decoded = ControlMsg::decode_payload(&payload).expect("decode reset metric");
    assert_eq!(decoded, msg_reset);

    // 2. ScreenQualityMetricConfig defaults & clamping
    let cfg_default = ScreenQualityMetricConfig::default();
    assert!(!cfg_default.enabled);
    assert_eq!(
        cfg_default.sample_interval,
        SCREEN_METRIC_DEFAULT_SAMPLE_INTERVAL
    );

    let cfg_custom = ScreenQualityMetricConfig::new(true, Some(25));
    assert!(cfg_custom.enabled);
    assert_eq!(cfg_custom.sample_interval, 25);

    let cfg_clamped_low = ScreenQualityMetricConfig::new(true, Some(0));
    assert_eq!(cfg_clamped_low.sample_interval, 1);

    let cfg_clamped_high = ScreenQualityMetricConfig::new(true, Some(200));
    assert_eq!(cfg_clamped_high.sample_interval, 120);

    // 3. ScreenQualityMetricReport default
    let report = ScreenQualityMetricReport::default();
    assert!(report.ok);
    assert!(!report.enabled);
    assert_eq!(
        report.sample_interval,
        SCREEN_METRIC_DEFAULT_SAMPLE_INTERVAL
    );
    assert_eq!(report.evaluated_frames, 0);
    assert_eq!(report.avg_mse, 0.0);
    assert_eq!(report.avg_psnr_db, 100.0);
    assert_eq!(report.avg_ssim, 1.0);
    assert_eq!(report.last_psnr_db, 100.0);
    assert_eq!(report.last_ssim, 1.0);
    assert!(!report.quality_warning);

    // 4. Test compute_mse, compute_psnr, compute_ssim
    let w = 32u16;
    let h = 32u16;
    let rect = ScreenRect::new(0, 0, w, h).unwrap();
    let mut base_pixels = vec![0u8; (w as usize) * (h as usize) * 4];
    for y in 0..h {
        for x in 0..w {
            let idx = ((y as usize) * (w as usize) + (x as usize)) * 4;
            base_pixels[idx] = (x * 7) as u8; // B
            base_pixels[idx + 1] = (y * 7) as u8; // G
            base_pixels[idx + 2] = ((x + y) * 3) as u8; // R
            base_pixels[idx + 3] = 255; // A
        }
    }
    let frame_ref = ScreenFrame::raw(1, rect, PixelFormat::Bgra8, base_pixels.clone()).unwrap();
    let frame_identical =
        ScreenFrame::raw(2, rect, PixelFormat::Bgra8, base_pixels.clone()).unwrap();

    // Exact match: MSE = 0, PSNR = 100 dB, SSIM = 1.0
    let mse_ident = frame_identical
        .compute_mse(&frame_ref)
        .expect("mse identical");
    assert!(mse_ident <= 1e-10);
    let psnr_ident = frame_identical
        .compute_psnr(&frame_ref)
        .expect("psnr identical");
    assert_eq!(psnr_ident, 100.0);
    let ssim_ident = frame_identical
        .compute_ssim(&frame_ref)
        .expect("ssim identical");
    assert!((ssim_ident - 1.0).abs() < 1e-6);

    // Slightly modified frame (small perturbation on a few pixels)
    let mut perturbed_pixels = base_pixels.clone();
    perturbed_pixels[0] = perturbed_pixels[0].wrapping_add(20);
    perturbed_pixels[1] = perturbed_pixels[1].wrapping_add(15);
    perturbed_pixels[4] = perturbed_pixels[4].wrapping_add(10);
    let frame_perturbed = ScreenFrame::raw(3, rect, PixelFormat::Bgra8, perturbed_pixels).unwrap();

    let mse_pert = frame_perturbed
        .compute_mse(&frame_ref)
        .expect("mse perturbed");
    assert!(mse_pert > 0.0);
    let psnr_pert = frame_perturbed
        .compute_psnr(&frame_ref)
        .expect("psnr perturbed");
    assert!(
        psnr_pert < 100.0 && psnr_pert > 35.0,
        "psnr_pert was {}",
        psnr_pert
    );
    let ssim_pert = frame_perturbed
        .compute_ssim(&frame_ref)
        .expect("ssim perturbed");
    assert!(
        ssim_pert > 0.95 && ssim_pert < 1.0,
        "ssim_pert was {}",
        ssim_pert
    );

    // Heavily distorted frame (all inverted colors)
    let mut inverted_pixels = base_pixels.clone();
    for p in inverted_pixels.chunks_exact_mut(4) {
        p[0] = 255 - p[0];
        p[1] = 255 - p[1];
        p[2] = 255 - p[2];
    }
    let frame_inverted = ScreenFrame::raw(4, rect, PixelFormat::Bgra8, inverted_pixels).unwrap();
    let psnr_inv = frame_inverted
        .compute_psnr(&frame_ref)
        .expect("psnr inverted");
    assert!(psnr_inv < 25.0, "psnr_inv was {}", psnr_inv);
    let ssim_inv = frame_inverted
        .compute_ssim(&frame_ref)
        .expect("ssim inverted");
    assert!(ssim_inv < 0.50, "ssim_inv was {}", ssim_inv);

    // Error case: dimension mismatch
    let rect_diff = ScreenRect::new(0, 0, 32, 16).unwrap();
    let frame_diff =
        ScreenFrame::raw(5, rect_diff, PixelFormat::Bgra8, vec![0u8; 32 * 16 * 4]).unwrap();
    assert!(frame_diff.compute_mse(&frame_ref).is_err());
    assert!(frame_diff.compute_psnr(&frame_ref).is_err());
    assert!(frame_diff.compute_ssim(&frame_ref).is_err());

    // Error case: format mismatch
    let frame_rgba = ScreenFrame::raw(6, rect, PixelFormat::Rgba8, base_pixels).unwrap();
    assert!(frame_rgba.compute_mse(&frame_ref).is_err());
    assert!(frame_rgba.compute_psnr(&frame_ref).is_err());
    assert!(frame_rgba.compute_ssim(&frame_ref).is_err());
}

#[test]
fn screen_autonomous_qoe_closed_loop_adaptation_algorithms_and_signaling() {
    // 1. Line protocol 14-byte envelope tag 25, 26 roundtrips
    let msg_qoe = ControlMsg::SetScreenQoeMode {
        mode: ScreenQoeMode::Balanced.tag(),
        target_psnr_db: 32,
        target_ssim_pct: 90,
    };
    assert_eq!(msg_qoe.tag(), 25);
    let payload = msg_qoe.encode_payload();
    assert_eq!(payload.len(), 14);
    assert_eq!(payload[5], 25);
    let decoded = ControlMsg::decode_payload(&payload).expect("decode qoe mode");
    assert_eq!(decoded, msg_qoe);

    let msg_reset = ControlMsg::ResetScreenQoeMode;
    assert_eq!(msg_reset.tag(), 26);
    let payload = msg_reset.encode_payload();
    assert_eq!(payload.len(), 14);
    assert_eq!(payload[5], 26);
    let decoded_reset = ControlMsg::decode_payload(&payload).expect("decode qoe reset");
    assert_eq!(decoded_reset, msg_reset);

    // 2. ScreenQoeMode string parsing & defaults
    assert_eq!(
        "balanced".parse::<ScreenQoeMode>().unwrap(),
        ScreenQoeMode::Balanced
    );
    assert_eq!(
        "conservative".parse::<ScreenQoeMode>().unwrap(),
        ScreenQoeMode::Conservative
    );
    assert_eq!(
        "economy".parse::<ScreenQoeMode>().unwrap(),
        ScreenQoeMode::Economy
    );
    assert_eq!("off".parse::<ScreenQoeMode>().unwrap(), ScreenQoeMode::Off);

    // 3. ScreenQoeConfig defaults and clamping
    let cfg_default = ScreenQoeConfig::default();
    assert_eq!(cfg_default.mode, ScreenQoeMode::Off);
    assert_eq!(cfg_default.target_psnr_db, 30);
    assert_eq!(cfg_default.target_ssim_pct, 85);
    assert!(!cfg_default.is_active());

    let cfg_custom = ScreenQoeConfig::new(ScreenQoeMode::Balanced, Some(35), Some(92));
    assert_eq!(cfg_custom.target_psnr_db, 35);
    assert_eq!(cfg_custom.target_ssim_pct, 92);
    assert!(cfg_custom.is_active());

    // 4. ScreenQoeController evaluation state machine
    let mut controller = ScreenQoeController::new(cfg_custom);
    let mut metric = ScreenQualityMetricReport {
        ok: true,
        enabled: true,
        sample_interval: 10,
        evaluated_frames: 10,
        avg_mse: 0.0,
        avg_psnr_db: 40.0,
        avg_ssim: 0.98,
        last_psnr_db: 40.0,
        last_ssim: 0.98,
        quality_warning: false,
    };

    // Quality satisfies target -> Hold
    let action1 = controller.evaluate(&metric, 1_000_000);
    assert_eq!(action1, ScreenQoeAction::Hold);
    assert_eq!(controller.actions_total(), 0);

    // Quality drops: high SSIM deficit (downsampling blurriness) -> UpgradeScale
    metric.last_ssim = 0.70; // target is 0.92 -> deficit 0.22 > 0.15
    let action2 = controller.evaluate(&metric, 1_100_000);
    assert_eq!(action2, ScreenQoeAction::UpgradeScale);
    assert_eq!(controller.actions_total(), 1);
    assert_eq!(controller.last_action(), ScreenQoeAction::UpgradeScale);

    // Cooldown prevents immediate re-triggering
    metric.last_ssim = 0.65;
    let action_cooldown = controller.evaluate(&metric, 1_200_000);
    assert_eq!(action_cooldown, ScreenQoeAction::Hold);
    assert_eq!(controller.actions_total(), 1);

    // After cooldown expires (cooldown is 500,000 us -> 1_100_000 + 600_000 = 1_700_000)
    // Large PSNR deficit -> LowerQp
    metric.last_ssim = 0.90; // deficit 0.02
    metric.last_psnr_db = 22.0; // target 35.0 -> deficit 13.0 > 8.0
    let action3 = controller.evaluate(&metric, 1_700_000);
    assert_eq!(action3, ScreenQoeAction::LowerQp);
    assert_eq!(controller.actions_total(), 2);

    // Report generation
    let rep = controller.report(1_700_000);
    assert_eq!(rep.mode, "balanced");
    assert_eq!(rep.target_psnr_db, 35);
    assert_eq!(rep.target_ssim_pct, 92);
    assert_eq!(rep.actions_total, 2);
    assert_eq!(rep.last_action, "lower_qp");

    // Reset restores defaults
    controller.reset();
    assert_eq!(controller.actions_total(), 0);
    assert_eq!(controller.last_action(), ScreenQoeAction::Hold);
    assert!(!controller.config().is_active());
}

#[test]
fn screen_progressive_intra_refresh_algorithms_and_signaling() {
    // 1. Line protocol 14-byte envelope tag 27, 28 roundtrips
    let msg_pir = ControlMsg::SetScreenPirMode {
        mode: ScreenPirMode::Continuous.tag(),
        bands: 12,
    };
    assert_eq!(msg_pir.tag(), 27);
    let payload = msg_pir.encode_payload();
    assert_eq!(payload.len(), 14);
    assert_eq!(payload[5], 27);
    let decoded = ControlMsg::decode_payload(&payload).expect("decode pir mode");
    assert_eq!(decoded, msg_pir);

    let msg_reset = ControlMsg::ResetScreenPirMode;
    assert_eq!(msg_reset.tag(), 28);
    let payload = msg_reset.encode_payload();
    assert_eq!(payload.len(), 14);
    assert_eq!(payload[5], 28);
    let decoded = ControlMsg::decode_payload(&payload).expect("decode reset pir");
    assert_eq!(decoded, msg_reset);

    // 2. ScreenPirMode string parsing & defaults
    assert_eq!(
        "continuous".parse::<ScreenPirMode>().unwrap(),
        ScreenPirMode::Continuous
    );
    assert_eq!(
        "on-demand".parse::<ScreenPirMode>().unwrap(),
        ScreenPirMode::OnDemand
    );
    assert_eq!(
        "ondemand".parse::<ScreenPirMode>().unwrap(),
        ScreenPirMode::OnDemand
    );
    assert_eq!(
        "disabled".parse::<ScreenPirMode>().unwrap(),
        ScreenPirMode::Disabled
    );
    assert_eq!(
        "off".parse::<ScreenPirMode>().unwrap(),
        ScreenPirMode::Disabled
    );

    // 3. ScreenPirConfig defaults & clamping
    let cfg_default = ScreenPirConfig::default();
    assert_eq!(cfg_default.mode, ScreenPirMode::Disabled);
    assert_eq!(cfg_default.bands, SCREEN_PIR_DEFAULT_BANDS);
    assert!(!cfg_default.is_enabled());

    let cfg_custom = ScreenPirConfig::new(ScreenPirMode::Continuous, Some(16));
    assert_eq!(cfg_custom.mode, ScreenPirMode::Continuous);
    assert_eq!(cfg_custom.bands, 16);
    assert!(cfg_custom.is_enabled());

    let cfg_clamped_low = ScreenPirConfig::new(ScreenPirMode::OnDemand, Some(1));
    assert_eq!(cfg_clamped_low.bands, SCREEN_PIR_MIN_BANDS);

    let cfg_clamped_high = ScreenPirConfig::new(ScreenPirMode::OnDemand, Some(64));
    assert_eq!(cfg_clamped_high.bands, SCREEN_PIR_MAX_BANDS);

    // 4. Band rectangle geometry calculation & full area coverage
    let w = 1920u32;
    let h = 1080u32;
    let bands = 8u8;
    let mut total_h = 0u32;
    for i in 0..bands {
        let (rx, ry, rw, rh) = compute_pir_band_rect(w, h, bands, i);
        assert_eq!(rx, 0);
        assert_eq!(rw, w);
        assert_eq!(ry, total_h);
        total_h += rh;
    }
    assert_eq!(total_h, h);

    // Check with odd height to verify remainder absorption
    let h_odd = 1085u32;
    let mut total_h_odd = 0u32;
    for i in 0..bands {
        let (rx, ry, rw, rh) = compute_pir_band_rect(w, h_odd, bands, i);
        assert_eq!(rx, 0);
        assert_eq!(rw, w);
        assert_eq!(ry, total_h_odd);
        total_h_odd += rh;
    }
    assert_eq!(total_h_odd, h_odd);

    // 5. Default Report
    let rep = ScreenPirReport::default();
    assert!(rep.ok);
    assert_eq!(rep.mode, "disabled");
    assert_eq!(rep.bands, SCREEN_PIR_DEFAULT_BANDS);
    assert_eq!(rep.sweeps_completed, 0);
    assert_eq!(rep.slices_sent, 0);
    assert_eq!(rep.current_band, 0);
    assert!(!rep.active_sweep);
}

#[test]
fn screen_long_term_reference_algorithms_and_signaling() {
    // 1. Line protocol 14-byte envelope tag 29, 30, 31, 32 roundtrips
    let msg_ltr = ControlMsg::SetScreenLtrMode {
        enabled: true,
        auto_interval: 120,
    };
    assert_eq!(msg_ltr.tag(), 29);
    let payload = msg_ltr.encode_payload();
    assert_eq!(payload.len(), 14);
    assert_eq!(payload[5], 29);
    let decoded = ControlMsg::decode_payload(&payload).expect("decode ltr mode");
    assert_eq!(decoded, msg_ltr);

    let msg_reset = ControlMsg::ResetScreenLtrMode;
    assert_eq!(msg_reset.tag(), 30);
    let payload = msg_reset.encode_payload();
    assert_eq!(payload.len(), 14);
    assert_eq!(payload[5], 30);
    let decoded = ControlMsg::decode_payload(&payload).expect("decode reset ltr");
    assert_eq!(decoded, msg_reset);

    let msg_rec = ControlMsg::LtrRecoveryRequest { ltr_seq: 4242 };
    assert_eq!(msg_rec.tag(), 31);
    let payload = msg_rec.encode_payload();
    assert_eq!(payload.len(), 14);
    assert_eq!(payload[5], 31);
    let decoded = ControlMsg::decode_payload(&payload).expect("decode ltr recovery request");
    assert_eq!(decoded, msg_rec);

    let msg_sync = ControlMsg::LtrAnchorSync { ltr_seq: 8888 };
    assert_eq!(msg_sync.tag(), 32);
    let payload = msg_sync.encode_payload();
    assert_eq!(payload.len(), 14);
    assert_eq!(payload[5], 32);
    let decoded = ControlMsg::decode_payload(&payload).expect("decode ltr anchor sync");
    assert_eq!(decoded, msg_sync);

    // 2. ScreenLtrConfig defaults & clamping
    let cfg_default = ScreenLtrConfig::default();
    assert!(!cfg_default.enabled);
    assert_eq!(cfg_default.auto_interval, SCREEN_LTR_DEFAULT_AUTO_INTERVAL);
    assert!(!cfg_default.is_enabled());

    let cfg_custom = ScreenLtrConfig::new(true, Some(30));
    assert!(cfg_custom.enabled);
    assert_eq!(cfg_custom.auto_interval, 30);
    assert!(cfg_custom.is_enabled());

    let cfg_clamped_low = ScreenLtrConfig::new(true, Some(1));
    assert_eq!(cfg_clamped_low.auto_interval, SCREEN_LTR_MIN_INTERVAL);

    let cfg_clamped_high = ScreenLtrConfig::new(true, Some(1000));
    assert_eq!(cfg_clamped_high.auto_interval, SCREEN_LTR_MAX_INTERVAL);

    let cfg_manual = ScreenLtrConfig::new(true, Some(0));
    assert_eq!(cfg_manual.auto_interval, 0);

    // 3. Default Report
    let rep = ScreenLtrReport::default();
    assert!(rep.ok);
    assert!(!rep.enabled);
    assert_eq!(rep.auto_interval, SCREEN_LTR_DEFAULT_AUTO_INTERVAL);
    assert_eq!(rep.current_ltr_seq, 0);
    assert_eq!(rep.anchors_established, 0);
    assert_eq!(rep.recoveries_requested, 0);
    assert_eq!(rep.recoveries_completed, 0);
    assert_eq!(rep.saved_burst_bytes, 0);

    // 4. SCREEN_COMPRESS_LTR codec tag roundtrip
    let rect = ScreenRect::new(0, 0, 16, 16).unwrap();
    let pixels = vec![0xABu8; 16 * 16 * 4];
    let mut frame = ScreenFrame::raw(100, rect, PixelFormat::Bgra8, pixels).unwrap();
    frame.codec_tag = SCREEN_COMPRESS_LTR;
    let wire_payload = frame.encode_payload().unwrap();
    let decoded_frame =
        ScreenFrame::decode_payload(&wire_payload).expect("decode frame with SCREEN_COMPRESS_LTR");
    assert_eq!(decoded_frame.codec_tag, SCREEN_COMPRESS_LTR);
    assert_eq!(decoded_frame.seq, 100);

    // 5. LTR Recovery sub-rect blitting
    let canvas_rect = ScreenRect::new(0, 0, 32, 32).unwrap();
    let canvas_pixels = vec![0x10u8; 32 * 32 * 4];
    let mut golden_canvas =
        ScreenFrame::raw(50, canvas_rect, PixelFormat::Bgra8, canvas_pixels).unwrap();

    let patch_rect = ScreenRect::new(8, 8, 8, 8).unwrap();
    let patch_pixels = vec![0xFFu8; 8 * 8 * 4];
    let patch_frame = ScreenFrame::raw(101, patch_rect, PixelFormat::Bgra8, patch_pixels).unwrap();

    golden_canvas
        .apply_sub_rect(&patch_frame)
        .expect("apply ltr recovery patch");
    // Verify patched area
    let idx = ((8 * 32 + 8) * 4) as usize;
    assert_eq!(golden_canvas.bytes[idx], 0xFF);
    // Verify unpatched area
    let unpatched_idx = 0usize;
    assert_eq!(golden_canvas.bytes[unpatched_idx], 0x10);
}

#[test]
fn screen_temporal_scalability_svc_algorithms_and_signaling() {
    // 1. ScreenSvcMode enum conversions and parsing
    assert_eq!(ScreenSvcMode::from_u8(0), ScreenSvcMode::Disabled);
    assert_eq!(ScreenSvcMode::from_u8(1), ScreenSvcMode::Adaptive);
    assert_eq!(ScreenSvcMode::from_u8(2), ScreenSvcMode::BaseOnly);
    assert_eq!(ScreenSvcMode::from_u8(99), ScreenSvcMode::Disabled);

    assert_eq!(ScreenSvcMode::Disabled.as_u8(), 0);
    assert_eq!(ScreenSvcMode::Adaptive.as_u8(), 1);
    assert_eq!(ScreenSvcMode::BaseOnly.as_u8(), 2);

    assert_eq!(ScreenSvcMode::Disabled.as_str(), "disabled");
    assert_eq!(ScreenSvcMode::Adaptive.as_str(), "adaptive");
    assert_eq!(ScreenSvcMode::BaseOnly.as_str(), "base_only");

    assert_eq!(
        ScreenSvcMode::parse_mode("adaptive"),
        ScreenSvcMode::Adaptive
    );
    assert_eq!(
        ScreenSvcMode::parse_mode("base_only"),
        ScreenSvcMode::BaseOnly
    );
    assert_eq!(
        ScreenSvcMode::parse_mode("disabled"),
        ScreenSvcMode::Disabled
    );

    // 2. ScreenSvcConfig methods
    let default_cfg = ScreenSvcConfig::default();
    assert_eq!(default_cfg.mode, ScreenSvcMode::Disabled);
    assert!(!default_cfg.drop_t1);
    assert!(!default_cfg.is_enabled());
    assert!(!default_cfg.should_drop_t1());

    let adaptive_cfg = ScreenSvcConfig::new(ScreenSvcMode::Adaptive, false);
    assert!(adaptive_cfg.is_enabled());
    assert!(!adaptive_cfg.should_drop_t1());

    let adaptive_drop_cfg = ScreenSvcConfig::new(ScreenSvcMode::Adaptive, true);
    assert!(adaptive_drop_cfg.is_enabled());
    assert!(adaptive_drop_cfg.should_drop_t1());

    let base_only_cfg = ScreenSvcConfig::new(ScreenSvcMode::BaseOnly, false);
    assert!(base_only_cfg.is_enabled());
    assert!(base_only_cfg.should_drop_t1()); // BaseOnly always drops T1

    // 3. ScreenSvcReport defaults
    let rep = ScreenSvcReport::default();
    assert!(rep.ok);
    assert!(!rep.enabled);
    assert_eq!(rep.mode, "disabled");
    assert!(!rep.drop_t1);
    assert_eq!(rep.t0_frames_sent, 0);
    assert_eq!(rep.t1_frames_sent, 0);
    assert_eq!(rep.t1_frames_dropped, 0);
    assert_eq!(rep.saved_bandwidth_bytes, 0);

    // 4. Tag 33: SetScreenSvcMode
    let msg_set = ControlMsg::SetScreenSvcMode {
        mode: 1,
        drop_t1: true,
    };
    assert_eq!(msg_set.tag(), 33);
    let payload_set = msg_set.encode_payload();
    assert_eq!(payload_set.len(), 14);
    assert_eq!(payload_set[5], 33);
    let decoded_set = ControlMsg::decode_payload(&payload_set).expect("decode set svc mode");
    assert_eq!(decoded_set, msg_set);

    // 5. Tag 34: ResetScreenSvcMode
    let msg_reset = ControlMsg::ResetScreenSvcMode;
    assert_eq!(msg_reset.tag(), 34);
    let payload_reset = msg_reset.encode_payload();
    assert_eq!(payload_reset.len(), 14);
    assert_eq!(payload_reset[5], 34);
    let decoded_reset = ControlMsg::decode_payload(&payload_reset).expect("decode reset svc mode");
    assert_eq!(decoded_reset, msg_reset);

    // 6. Tag 35: SvcT1DropSync
    let msg_sync = ControlMsg::SvcT1DropSync {
        dropped_count: 12345,
    };
    assert_eq!(msg_sync.tag(), 35);
    let payload_sync = msg_sync.encode_payload();
    assert_eq!(payload_sync.len(), 14);
    assert_eq!(payload_sync[5], 35);
    let decoded_sync = ControlMsg::decode_payload(&payload_sync).expect("decode svc drop sync");
    assert_eq!(decoded_sync, msg_sync);

    // 7. SCREEN_COMPRESS_SVC_T1 Codec Tag roundtrip
    let rect = ScreenRect::new(0, 0, 16, 16).unwrap();
    let pixels = vec![0x77u8; 16 * 16 * 4];
    let mut frame = ScreenFrame::raw(101, rect, PixelFormat::Bgra8, pixels).unwrap();
    frame.codec_tag = SCREEN_COMPRESS_SVC_T1;
    let wire = frame.encode_payload().unwrap();
    let decoded =
        ScreenFrame::decode_payload(&wire).expect("decode frame with SCREEN_COMPRESS_SVC_T1");
    assert_eq!(decoded.codec_tag, SCREEN_COMPRESS_SVC_T1);
    assert_eq!(decoded.seq, 101);
}

#[test]
fn screen_motion_vector_estimation_and_scroll_acceleration_algorithms() {
    // 1. ScreenMotionConfig defaults & clamping
    let default_cfg = ScreenMotionConfig::default();
    assert!(!default_cfg.enabled);
    assert_eq!(
        default_cfg.max_displacement,
        SCREEN_MOTION_DEFAULT_MAX_DISPLACEMENT
    );

    let custom_cfg = ScreenMotionConfig::new(true, 128);
    assert!(custom_cfg.enabled);
    assert_eq!(custom_cfg.max_displacement, 128);

    let low_clamped = ScreenMotionConfig::new(true, 2);
    assert_eq!(low_clamped.max_displacement, SCREEN_MOTION_MIN_DISPLACEMENT);

    let high_clamped = ScreenMotionConfig::new(true, 500);
    assert_eq!(
        high_clamped.max_displacement,
        SCREEN_MOTION_MAX_DISPLACEMENT
    );

    // 2. ScreenMotionReport defaults
    let rep = ScreenMotionReport::default();
    assert!(rep.ok);
    assert!(!rep.enabled);
    assert_eq!(rep.max_displacement, SCREEN_MOTION_DEFAULT_MAX_DISPLACEMENT);
    assert_eq!(rep.motion_frames_sent, 0);
    assert_eq!(rep.fallback_frames_sent, 0);
    assert_eq!(rep.saved_bandwidth_bytes, 0);

    // 3. Tag 36: SetScreenMotionMode
    let msg_set = ControlMsg::SetScreenMotionMode {
        enabled: true,
        max_displacement: 96,
    };
    assert_eq!(msg_set.tag(), 36);
    let payload_set = msg_set.encode_payload();
    assert_eq!(payload_set.len(), 14);
    assert_eq!(payload_set[5], 36);
    let decoded_set = ControlMsg::decode_payload(&payload_set).expect("decode set motion mode");
    assert_eq!(decoded_set, msg_set);

    // 4. Tag 37: ResetScreenMotionMode
    let msg_reset = ControlMsg::ResetScreenMotionMode;
    assert_eq!(msg_reset.tag(), 37);
    let payload_reset = msg_reset.encode_payload();
    assert_eq!(payload_reset.len(), 14);
    assert_eq!(payload_reset[5], 37);
    let decoded_reset =
        ControlMsg::decode_payload(&payload_reset).expect("decode reset motion mode");
    assert_eq!(decoded_reset, msg_reset);

    // 5. Tag 38: MotionScrollSync
    let msg_sync = ControlMsg::MotionScrollSync {
        scroll_frames_sent: 9999,
    };
    assert_eq!(msg_sync.tag(), 38);
    let payload_sync = msg_sync.encode_payload();
    assert_eq!(payload_sync.len(), 14);
    assert_eq!(payload_sync[5], 38);
    let decoded_sync =
        ControlMsg::decode_payload(&payload_sync).expect("decode motion scroll sync");
    assert_eq!(decoded_sync, msg_sync);

    // 6. compute_exposed_strip tests
    assert_eq!(compute_exposed_strip(100, 100, 0, 0), None);
    // Shift up by 16: exposed strip is bottom 16 rows (y=84, h=16)
    let strip_up = compute_exposed_strip(100, 100, 0, -16).unwrap();
    assert_eq!(strip_up.x, 0);
    assert_eq!(strip_up.y, 84);
    assert_eq!(strip_up.width, 100);
    assert_eq!(strip_up.height, 16);

    // Shift down by 20: exposed strip is top 20 rows (y=0, h=20)
    let strip_down = compute_exposed_strip(100, 100, 0, 20).unwrap();
    assert_eq!(strip_down.x, 0);
    assert_eq!(strip_down.y, 0);
    assert_eq!(strip_down.width, 100);
    assert_eq!(strip_down.height, 20);

    // Shift left by 15: exposed strip is right 15 cols (x=85, w=15)
    let strip_left = compute_exposed_strip(100, 100, -15, 0).unwrap();
    assert_eq!(strip_left.x, 85);
    assert_eq!(strip_left.y, 0);
    assert_eq!(strip_left.width, 15);
    assert_eq!(strip_left.height, 100);

    // Shift right by 10: exposed strip is left 10 cols (x=0, w=10)
    let strip_right = compute_exposed_strip(100, 100, 10, 0).unwrap();
    assert_eq!(strip_right.x, 0);
    assert_eq!(strip_right.y, 0);
    assert_eq!(strip_right.width, 10);
    assert_eq!(strip_right.height, 100);

    // 7. ScreenFrame::scroll_canvas memory movement tests
    let w = 32u16;
    let h = 32u16;
    let mut base_pixels = vec![0u8; (w as usize) * (h as usize) * 4];
    for y in 0..h as usize {
        for x in 0..w as usize {
            let idx = (y * (w as usize) + x) * 4;
            base_pixels[idx] = (x & 0xFF) as u8;
            base_pixels[idx + 1] = (y & 0xFF) as u8;
            base_pixels[idx + 2] = 0xAA;
            base_pixels[idx + 3] = 255;
        }
    }
    let rect = ScreenRect::new(0, 0, w, h).unwrap();
    let mut frame_canvas =
        ScreenFrame::raw(1, rect, PixelFormat::Rgba8, base_pixels.clone()).unwrap();

    // Scroll up by 4 pixels (dy = -4)
    frame_canvas.scroll_canvas(0, -4);
    // At row 0, pixels should now equal original row 4
    let row0_offset = 0;
    let orig_row4_offset = 4 * (w as usize) * 4;
    assert_eq!(
        &frame_canvas.bytes[row0_offset..row0_offset + (w as usize) * 4],
        &base_pixels[orig_row4_offset..orig_row4_offset + (w as usize) * 4]
    );

    // Scroll down by 4 pixels (dy = 4) on fresh canvas
    let mut frame_canvas_down =
        ScreenFrame::raw(2, rect, PixelFormat::Rgba8, base_pixels.clone()).unwrap();
    frame_canvas_down.scroll_canvas(0, 4);
    // At row 4, pixels should now equal original row 0
    let row4_offset = 4 * (w as usize) * 4;
    let orig_row0_offset = 0;
    assert_eq!(
        &frame_canvas_down.bytes[row4_offset..row4_offset + (w as usize) * 4],
        &base_pixels[orig_row0_offset..orig_row0_offset + (w as usize) * 4]
    );

    // 8. estimate_scroll_motion tests
    let w_est = 64u16;
    let h_est = 64u16;
    let mut prev_pixels = vec![0u8; (w_est as usize) * (h_est as usize) * 4];
    for y in 0..h_est as usize {
        for x in 0..w_est as usize {
            let idx = (y * (w_est as usize) + x) * 4;
            prev_pixels[idx] = ((x * 3) & 0xFF) as u8;
            prev_pixels[idx + 1] = ((y * 7) & 0xFF) as u8;
            prev_pixels[idx + 2] = ((x ^ y) & 0xFF) as u8;
            prev_pixels[idx + 3] = 255;
        }
    }
    let rect_est = ScreenRect::new(0, 0, w_est, h_est).unwrap();
    let prev_frame =
        ScreenFrame::raw(10, rect_est, PixelFormat::Rgba8, prev_pixels.clone()).unwrap();

    // Create current frame scrolled up by 8 pixels (dy = -8)
    let mut curr_pixels = vec![0u8; (w_est as usize) * (h_est as usize) * 4];
    let dy_scroll = -8i16;
    let abs_dy = 8usize;
    // Shift up
    for y in 0..(h_est as usize - abs_dy) {
        let src_idx = (y + abs_dy) * (w_est as usize) * 4;
        let dst_idx = y * (w_est as usize) * 4;
        curr_pixels[dst_idx..dst_idx + (w_est as usize) * 4]
            .copy_from_slice(&prev_pixels[src_idx..src_idx + (w_est as usize) * 4]);
    }
    // New exposed bottom rows filled with distinct pattern
    for y in (h_est as usize - abs_dy)..(h_est as usize) {
        for x in 0..w_est as usize {
            let idx = (y * (w_est as usize) + x) * 4;
            curr_pixels[idx] = 0xFE;
            curr_pixels[idx + 1] = 0xDC;
            curr_pixels[idx + 2] = 0xBA;
            curr_pixels[idx + 3] = 255;
        }
    }
    let curr_frame =
        ScreenFrame::raw(11, rect_est, PixelFormat::Rgba8, curr_pixels.clone()).unwrap();

    let motion = estimate_scroll_motion(&prev_frame, &curr_frame, 32, 0.85);
    assert!(motion.is_some(), "motion must be detected for scroll dy=-8");
    let mv = motion.unwrap();
    assert_eq!(mv.dx, 0);
    assert_eq!(mv.dy, dy_scroll);
    assert!(mv.match_ratio >= 0.85);

    // Static frame check: prev == curr returns None
    assert_eq!(
        estimate_scroll_motion(&prev_frame, &prev_frame, 32, 0.85),
        None
    );

    // 9. encode_motion_scroll and decode_motion_scroll full roundtrip & canvas synthesis
    let exposed_rect = compute_exposed_strip(w_est, h_est, mv.dx, mv.dy).unwrap();
    let exposed_patch = curr_frame
        .extract_sub_rect(
            exposed_rect.x,
            exposed_rect.y,
            exposed_rect.width,
            exposed_rect.height,
        )
        .expect("extract exposed sub rect");

    let motion_wire_frame = ScreenFrame::encode_motion_scroll(
        rect_est,
        PixelFormat::Rgba8,
        12,
        mv.dx,
        mv.dy,
        exposed_rect,
        &exposed_patch.bytes,
    );
    assert_eq!(motion_wire_frame.codec_tag, SCREEN_COMPRESS_MOTION_SCROLL);

    // Wire serialization roundtrip
    let wire_bytes = motion_wire_frame.encode_payload().unwrap();
    let decoded_wire_frame =
        ScreenFrame::decode_payload(&wire_bytes).expect("decode motion wire frame");
    assert_eq!(decoded_wire_frame.codec_tag, SCREEN_COMPRESS_MOTION_SCROLL);
    assert_eq!(decoded_wire_frame.seq, 12);

    let (dec_dx, dec_dy, dec_exp, dec_patch) = decoded_wire_frame.decode_motion_scroll().unwrap();
    assert_eq!(dec_dx, mv.dx);
    assert_eq!(dec_dy, mv.dy);
    assert_eq!(dec_exp, exposed_rect);

    // Synthesize receiver canvas from prev_frame
    let mut receiver_canvas = prev_frame.clone();
    receiver_canvas.scroll_canvas(dec_dx, dec_dy);
    let patch_rect = ScreenRect::new(
        dec_exp.x as i32,
        dec_exp.y as i32,
        dec_exp.width,
        dec_exp.height,
    )
    .unwrap();
    let patch_frame = ScreenFrame::raw(
        decoded_wire_frame.seq,
        patch_rect,
        PixelFormat::Rgba8,
        dec_patch.to_vec(),
    )
    .unwrap();
    receiver_canvas.apply_sub_rect(&patch_frame).unwrap();

    // Assert 100% exact pixel match with curr_frame
    assert_eq!(receiver_canvas.bytes, curr_frame.bytes);
}

#[test]
fn screen_spatial_tile_grid_decomposition_and_streaming_algorithms() {
    // 1. ScreenTileConfig defaults & clamping
    let default_cfg = ScreenTileConfig::default();
    assert!(!default_cfg.enabled);
    assert_eq!(default_cfg.tile_size, SCREEN_TILE_DEFAULT_SIZE);
    assert!(!default_cfg.is_enabled());

    let custom_cfg = ScreenTileConfig::new(true, 128);
    assert!(custom_cfg.enabled);
    assert_eq!(custom_cfg.tile_size, 128);
    assert!(custom_cfg.is_enabled());

    let low_clamped = ScreenTileConfig::new(true, 10);
    assert_eq!(low_clamped.tile_size, SCREEN_TILE_MIN_SIZE);

    let high_clamped = ScreenTileConfig::new(true, 512);
    assert_eq!(high_clamped.tile_size, SCREEN_TILE_MAX_SIZE);

    let zero_cfg = ScreenTileConfig::new(true, 0);
    assert_eq!(zero_cfg.tile_size, SCREEN_TILE_DEFAULT_SIZE);

    // 2. ScreenTileReport defaults
    let rep = ScreenTileReport::default();
    assert!(rep.ok);
    assert!(!rep.enabled);
    assert_eq!(rep.tile_size, SCREEN_TILE_DEFAULT_SIZE);
    assert_eq!(rep.grid_frames_sent, 0);
    assert_eq!(rep.dirty_tiles_sent, 0);
    assert_eq!(rep.skipped_tiles_count, 0);
    assert_eq!(rep.saved_bandwidth_bytes, 0);

    // 3. Line protocol 14-byte envelope tag 39, 40, 41 roundtrips
    // Tag 39: SetScreenTileMode
    let msg_set = ControlMsg::SetScreenTileMode {
        enabled: true,
        tile_size: 96,
    };
    assert_eq!(msg_set.tag(), 39);
    let payload_set = msg_set.encode_payload();
    assert_eq!(payload_set.len(), 14);
    assert_eq!(payload_set[5], 39);
    let decoded_set = ControlMsg::decode_payload(&payload_set).expect("decode set tile mode");
    assert_eq!(decoded_set, msg_set);

    // Tag 40: ResetScreenTileMode
    let msg_reset = ControlMsg::ResetScreenTileMode;
    assert_eq!(msg_reset.tag(), 40);
    let payload_reset = msg_reset.encode_payload();
    assert_eq!(payload_reset.len(), 14);
    assert_eq!(payload_reset[5], 40);
    let decoded_reset = ControlMsg::decode_payload(&payload_reset).expect("decode reset tile mode");
    assert_eq!(decoded_reset, msg_reset);

    // Tag 41: TileGridSync
    let msg_sync = ControlMsg::TileGridSync { tiles_sent: 12345 };
    assert_eq!(msg_sync.tag(), 41);
    let payload_sync = msg_sync.encode_payload();
    assert_eq!(payload_sync.len(), 14);
    assert_eq!(payload_sync[5], 41);
    let decoded_sync = ControlMsg::decode_payload(&payload_sync).expect("decode tile grid sync");
    assert_eq!(decoded_sync, msg_sync);

    // 4. compute_tile_grid_dims calculations
    assert_eq!(compute_tile_grid_dims(0, 100, 64), (0, 0, 0));
    assert_eq!(compute_tile_grid_dims(100, 0, 64), (0, 0, 0));
    // Exact multiples: 128x128 with T=64 -> 2x2 = 4
    assert_eq!(compute_tile_grid_dims(128, 128, 64), (2, 2, 4));
    // Uneven edges: 130x70 with T=64 -> cols=ceil(130/64)=3, rows=ceil(70/64)=2 -> 6 tiles
    assert_eq!(compute_tile_grid_dims(130, 70, 64), (3, 2, 6));

    // 5. extract_dirty_tiles & spatial decomposition
    let w = 128u16;
    let h = 128u16;
    let bpp = 4usize;
    let rect = ScreenRect::new(0, 0, w, h).unwrap();
    let base_pixels = vec![0x20u8; (w as usize) * (h as usize) * bpp];

    let prev_frame = ScreenFrame::raw(1, rect, PixelFormat::Bgra8, base_pixels.clone()).unwrap();

    // Case A: Identical frames -> 0 dirty tiles, total_tiles = 4
    let (total_a, dirty_a) = extract_dirty_tiles(&prev_frame, &prev_frame, 64);
    assert_eq!(total_a, 4);
    assert!(dirty_a.is_empty());

    // Case B: Discrete micro-updates at top-left (x=10, y=10 -> tile 0) and bottom-right (x=120, y=120 -> tile 3)
    let mut curr_pixels = base_pixels.clone();
    // Invert pixel in tile 0: (10, 10)
    let idx0 = (10 * (w as usize) + 10) * bpp;
    curr_pixels[idx0] = 0xFF;
    curr_pixels[idx0 + 1] = 0xAA;

    // Invert pixel in tile 3: (120, 120)
    let idx3 = (120 * (w as usize) + 120) * bpp;
    curr_pixels[idx3] = 0xEE;
    curr_pixels[idx3 + 2] = 0xCC;

    let curr_frame = ScreenFrame::raw(2, rect, PixelFormat::Bgra8, curr_pixels.clone()).unwrap();

    let (total_b, dirty_b) = extract_dirty_tiles(&prev_frame, &curr_frame, 64);
    assert_eq!(total_b, 4);
    assert_eq!(dirty_b.len(), 2, "only 2 tiles should be dirty");
    assert_eq!(dirty_b[0].index, 0);
    assert_eq!(dirty_b[0].x, 0);
    assert_eq!(dirty_b[0].y, 0);
    assert_eq!(dirty_b[0].w, 64);
    assert_eq!(dirty_b[0].h, 64);

    assert_eq!(dirty_b[1].index, 3);
    assert_eq!(dirty_b[1].x, 64);
    assert_eq!(dirty_b[1].y, 64);
    assert_eq!(dirty_b[1].w, 64);
    assert_eq!(dirty_b[1].h, 64);

    // 6. encode_tile_grid and decode_tile_grid wire serialization roundtrip
    let wire_frame =
        ScreenFrame::encode_tile_grid(100, rect, PixelFormat::Bgra8, 64, total_b, &dirty_b);
    assert_eq!(wire_frame.codec_tag, SCREEN_COMPRESS_TILE_GRID);
    assert_eq!(wire_frame.seq, 100);

    let wire_bytes = wire_frame.encode_payload().unwrap();
    let decoded_wire_frame = ScreenFrame::decode_payload(&wire_bytes).expect("decode wire payload");
    assert_eq!(decoded_wire_frame.codec_tag, SCREEN_COMPRESS_TILE_GRID);

    let (dec_tile_size, dec_total, dec_dirty) = decoded_wire_frame.decode_tile_grid().unwrap();
    assert_eq!(dec_tile_size, 64);
    assert_eq!(dec_total, 4);
    assert_eq!(dec_dirty.len(), 2);
    assert_eq!(dec_dirty[0], dirty_b[0]);
    assert_eq!(dec_dirty[1], dirty_b[1]);

    // 7. Receiver in-place blitting via apply_dirty_tile
    let mut receiver_canvas = prev_frame.clone();
    for tile in &dec_dirty {
        receiver_canvas
            .apply_dirty_tile(tile)
            .expect("apply dirty tile");
    }

    // 100% bit-exact reproduction verification!
    assert_eq!(receiver_canvas.bytes, curr_frame.bytes);

    // Boundary error checks: tile exceeds canvas boundary
    let oob_tile = DirtyTile {
        index: 99,
        x: 100,
        y: 100,
        w: 64, // 100 + 64 = 164 > 128
        h: 64,
        data: vec![0u8; 64 * 64 * bpp],
    };
    assert!(receiver_canvas.apply_dirty_tile(&oob_tile).is_err());

    // Truncated tile data error check
    let short_tile = DirtyTile {
        index: 0,
        x: 0,
        y: 0,
        w: 64,
        h: 64,
        data: vec![0u8; 10], // expected 64*64*4 = 16384 bytes
    };
    assert!(receiver_canvas.apply_dirty_tile(&short_tile).is_err());
}

#[test]
fn screen_tile_cache_deduplication_and_streaming_algorithms() {
    // 1. ScreenTileCacheConfig defaults, clamping, and helpers
    let def_cfg = ScreenTileCacheConfig::default();
    assert!(!def_cfg.is_enabled());
    assert_eq!(def_cfg.max_entries, SCREEN_TILE_CACHE_DEFAULT_MAX_ENTRIES);

    let clamped_low = ScreenTileCacheConfig::new(true, 10);
    assert!(clamped_low.is_enabled());
    assert_eq!(clamped_low.max_entries, SCREEN_TILE_CACHE_MIN_ENTRIES);

    let clamped_high = ScreenTileCacheConfig::new(true, 10000);
    assert_eq!(clamped_high.max_entries, SCREEN_TILE_CACHE_MAX_ENTRIES);

    let custom_cfg = ScreenTileCacheConfig::new(true, 256);
    assert_eq!(custom_cfg.max_entries, 256);

    // 2. ScreenTileCacheReport defaults
    let def_rep = ScreenTileCacheReport::default();
    assert!(def_rep.ok);
    assert!(!def_rep.enabled);
    assert_eq!(def_rep.current_entries, 0);
    assert_eq!(def_rep.cache_hits, 0);
    assert_eq!(def_rep.cache_misses, 0);

    // 3. ControlMsg Tag 42, 43, 44 wire serialization roundtrips
    // Tag 42: SetScreenTileCacheMode
    let msg_set = ControlMsg::SetScreenTileCacheMode {
        enabled: true,
        max_entries: 512,
    };
    assert_eq!(msg_set.tag(), 42);
    let payload_set = msg_set.encode_payload();
    assert_eq!(payload_set.len(), 14);
    assert_eq!(payload_set[5], 42);
    let decoded_set = ControlMsg::decode_payload(&payload_set).expect("decode set tile cache mode");
    assert_eq!(decoded_set, msg_set);

    // Tag 43: ResetScreenTileCacheMode
    let msg_reset = ControlMsg::ResetScreenTileCacheMode;
    assert_eq!(msg_reset.tag(), 43);
    let payload_reset = msg_reset.encode_payload();
    assert_eq!(payload_reset.len(), 14);
    assert_eq!(payload_reset[5], 43);
    let decoded_reset =
        ControlMsg::decode_payload(&payload_reset).expect("decode reset tile cache mode");
    assert_eq!(decoded_reset, msg_reset);

    // Tag 44: TileCacheSync
    let msg_sync = ControlMsg::TileCacheSync { hits: 98765 };
    assert_eq!(msg_sync.tag(), 44);
    let payload_sync = msg_sync.encode_payload();
    assert_eq!(payload_sync.len(), 14);
    assert_eq!(payload_sync[5], 44);
    let decoded_sync = ControlMsg::decode_payload(&payload_sync).expect("decode tile cache sync");
    assert_eq!(decoded_sync, msg_sync);

    // 4. fnv1a_hash64 fast content hashing
    let tile1 = vec![0x11u8; 1024];
    let tile2 = vec![0x11u8; 1024];
    let tile3 = vec![0x22u8; 1024];
    let hash1 = fnv1a_hash64(&tile1);
    let hash2 = fnv1a_hash64(&tile2);
    let hash3 = fnv1a_hash64(&tile3);
    assert_eq!(hash1, hash2, "Identical content must yield identical hash");
    assert_ne!(hash1, hash3, "Different content must yield different hash");

    // 5. ScreenTileDictionary LRU and eviction logic
    let dict = ScreenTileDictionary::new(3);
    assert_eq!(dict.max_entries(), 64, "Should clamp to minimum 64");
    // Test with small dictionary capacity by constructing directly or resizing
    let mut small_dict = ScreenTileDictionary {
        max_entries: 3,
        hash_to_id: HashMap::with_capacity(3),
        id_to_hash: vec![None; 3],
        lru_order: VecDeque::with_capacity(3),
    };

    // Insert item A: miss -> slot 0
    let (hit, id_a) = small_dict.lookup_or_insert(1001);
    assert!(!hit);
    assert_eq!(id_a, 0);
    assert_eq!(small_dict.current_entries(), 1);

    // Lookup item A: hit -> slot 0
    let (hit, id_a_hit) = small_dict.lookup_or_insert(1001);
    assert!(hit);
    assert_eq!(id_a_hit, 0);

    // Insert item B: miss -> slot 1
    let (hit, id_b) = small_dict.lookup_or_insert(1002);
    assert!(!hit);
    assert_eq!(id_b, 1);

    // Insert item C: miss -> slot 2 (now full: [A, B, C])
    let (hit, id_c) = small_dict.lookup_or_insert(1003);
    assert!(!hit);
    assert_eq!(id_c, 2);
    assert_eq!(small_dict.current_entries(), 3);

    // Access B to make it more recently used: order becomes [A, C, B]
    let (hit, _) = small_dict.lookup_or_insert(1002);
    assert!(hit);

    // Insert item D: miss -> evicts LRU (item A, slot 0) -> slot 0 re-used
    let (hit, id_d) = small_dict.lookup_or_insert(1004);
    assert!(!hit);
    assert_eq!(id_d, 0);
    assert_eq!(small_dict.current_entries(), 3);

    // Verify item A is now gone
    let (hit, _) = small_dict.lookup_or_insert(1001);
    assert!(!hit, "Item A should have been evicted");

    // 6. encode_tile_cache and decode_tile_cache wire serialization roundtrip
    let w = 128u16;
    let h = 128u16;
    let bpp = 4usize;
    let rect = ScreenRect::new(0, 0, w, h).unwrap();

    let entry0 = CachedTileEntry {
        index: 0,
        x: 0,
        y: 0,
        w: 64,
        h: 64,
        cache_hit: false,
        cache_id: 0,
        data: vec![0xAA; 64 * 64 * bpp],
    };

    let entry1 = CachedTileEntry {
        index: 1,
        x: 64,
        y: 0,
        w: 64,
        h: 64,
        cache_hit: true,
        cache_id: 0, // Zero payload reference to slot 0!
        data: Vec::new(),
    };

    let entries = vec![entry0.clone(), entry1.clone()];
    let wire_frame = ScreenFrame::encode_tile_cache(200, rect, PixelFormat::Bgra8, 64, 4, &entries);
    assert_eq!(wire_frame.codec_tag, SCREEN_COMPRESS_TILE_CACHE);
    assert_eq!(wire_frame.seq, 200);

    let wire_bytes = wire_frame.encode_payload().unwrap();
    let decoded_wire_frame = ScreenFrame::decode_payload(&wire_bytes).expect("decode wire payload");
    assert_eq!(decoded_wire_frame.codec_tag, SCREEN_COMPRESS_TILE_CACHE);

    let (dec_tile_size, dec_total, dec_entries) = decoded_wire_frame.decode_tile_cache().unwrap();
    assert_eq!(dec_tile_size, 64);
    assert_eq!(dec_total, 4);
    assert_eq!(dec_entries.len(), 2);
    assert_eq!(dec_entries[0], entry0);
    assert_eq!(dec_entries[1], entry1);

    // 7. Receiver canvas replay with apply_cached_tile
    let mut canvas = ScreenFrame::raw(
        0,
        rect,
        PixelFormat::Bgra8,
        vec![0x00; (w as usize) * (h as usize) * bpp],
    )
    .unwrap();
    let mut receiver_cache: Vec<Option<Vec<u8>>> = vec![None; 512];

    for entry in &dec_entries {
        if !entry.cache_hit {
            // Cache miss: store in receiver dictionary slot
            receiver_cache[entry.cache_id as usize] = Some(entry.data.clone());
            canvas
                .apply_cached_tile(entry.x, entry.y, entry.w, entry.h, &entry.data)
                .expect("apply miss tile");
        } else {
            // Cache hit: fetch from receiver dictionary slot
            let cached_pixels = receiver_cache[entry.cache_id as usize]
                .as_ref()
                .expect("found in cache");
            canvas
                .apply_cached_tile(entry.x, entry.y, entry.w, entry.h, cached_pixels)
                .expect("apply hit tile");
        }
    }

    // Verify both tile 0 and tile 1 on canvas have 0xAA pixels!
    let tile0_offset = 0;
    assert_eq!(
        &canvas.bytes[tile0_offset..tile0_offset + 64 * bpp],
        &vec![0xAA; 64 * bpp][..]
    );
    let tile1_offset = 64 * bpp;
    assert_eq!(
        &canvas.bytes[tile1_offset..tile1_offset + 64 * bpp],
        &vec![0xAA; 64 * bpp][..]
    );
}

#[test]
fn screen_foveated_focus_scheduling_algorithms() {
    // 1. ScreenFocusConfig defaults, clamping, and helpers
    let def_cfg = ScreenFocusConfig::default();
    assert!(!def_cfg.is_enabled());
    assert_eq!(def_cfg.mode, ScreenFocusMode::Cursor);
    assert_eq!(def_cfg.radius, SCREEN_FOCUS_DEFAULT_RADIUS);
    assert_eq!(
        def_cfg.peripheral_skip_ratio,
        SCREEN_FOCUS_DEFAULT_SKIP_RATIO
    );
    assert_eq!(def_cfg.box_x, 0);
    assert_eq!(def_cfg.box_y, 0);
    assert_eq!(def_cfg.box_w, 512);
    assert_eq!(def_cfg.box_h, 512);

    let clamped_low = ScreenFocusConfig::new(true, ScreenFocusMode::Cursor, 10, 20, None);
    assert_eq!(clamped_low.radius, SCREEN_FOCUS_MIN_RADIUS);
    assert_eq!(
        clamped_low.peripheral_skip_ratio,
        SCREEN_FOCUS_MAX_SKIP_RATIO
    );

    let clamped_high =
        ScreenFocusConfig::new(true, ScreenFocusMode::Box, 5000, 5, Some((10, 20, 10, 10)));
    assert_eq!(clamped_high.radius, SCREEN_FOCUS_MAX_RADIUS);
    assert_eq!(clamped_high.peripheral_skip_ratio, 5);
    assert_eq!(clamped_high.box_x, 10);
    assert_eq!(clamped_high.box_y, 20);
    assert_eq!(clamped_high.box_w, 32); // Min box width clamped to 32
    assert_eq!(clamped_high.box_h, 32); // Min box height clamped to 32

    // 2. ScreenFocusMode enum representations
    assert_eq!(ScreenFocusMode::Cursor.as_str(), "cursor");
    assert_eq!(ScreenFocusMode::Box.as_str(), "box");
    assert_eq!(ScreenFocusMode::from_u8(0), ScreenFocusMode::Cursor);
    assert_eq!(ScreenFocusMode::from_u8(1), ScreenFocusMode::Box);
    assert_eq!(ScreenFocusMode::Cursor.to_u8(), 0);
    assert_eq!(ScreenFocusMode::Box.to_u8(), 1);

    // 3. ScreenFocusReport defaults
    let def_rep = ScreenFocusReport::default();
    assert!(def_rep.ok);
    assert!(!def_rep.enabled);
    assert_eq!(def_rep.focus_tiles_sent, 0);
    assert_eq!(def_rep.peripheral_tiles_sent, 0);
    assert_eq!(def_rep.peripheral_tiles_skipped, 0);

    // 4. Geometry and distance check: ScreenFocusMode::Cursor
    // Cursor at (100, 100), radius = 64
    let cursor_cfg = ScreenFocusConfig {
        enabled: true,
        mode: ScreenFocusMode::Cursor,
        radius: 64,
        peripheral_skip_ratio: 2,
        box_x: 0,
        box_y: 0,
        box_w: 512,
        box_h: 512,
    };
    let cursor_pos = Some((100, 100));

    // Tile A: x=80, y=80, w=32, h=32 -> center is (96, 96), dx=-4, dy=-4 -> dist_sq = 32 <= 64^2 (4096) -> In Focus!
    assert!(is_tile_in_focus(80, 80, 32, 32, &cursor_cfg, cursor_pos));

    // Tile B: x=300, y=300, w=32, h=32 -> center is (316, 316), dx=216, dy=216 -> dist_sq > 4096 -> Peripheral!
    assert!(!is_tile_in_focus(300, 300, 32, 32, &cursor_cfg, cursor_pos));

    // Tile C: Boundary check at exactly radius distance: center at (164, 100), dx=64, dy=0 -> dist_sq = 4096 == 4096 -> In Focus!
    assert!(is_tile_in_focus(148, 84, 32, 32, &cursor_cfg, cursor_pos));

    // 5. Geometry and boundary check: ScreenFocusMode::Box
    let box_cfg = ScreenFocusConfig {
        enabled: true,
        mode: ScreenFocusMode::Box,
        radius: 128,
        peripheral_skip_ratio: 2,
        box_x: 100,
        box_y: 100,
        box_w: 200,
        box_h: 200,
    };

    // Tile inside box: center (150, 150) -> In Focus!
    assert!(is_tile_in_focus(134, 134, 32, 32, &box_cfg, None));

    // Tile outside box: center (50, 50) -> Peripheral!
    assert!(!is_tile_in_focus(34, 34, 32, 32, &box_cfg, None));

    // Tile outside box right edge: center (350, 150) -> Peripheral!
    assert!(!is_tile_in_focus(334, 134, 32, 32, &box_cfg, None));

    // 6. ControlMsg Tag 45~49 line protocol wire serialization roundtrip
    // Tag 45: SetScreenFocusMode
    let msg_set = ControlMsg::SetScreenFocusMode {
        enabled: true,
        mode: 1, // Box
        radius: 384,
        peripheral_skip_ratio: 3,
    };
    let payload_set = msg_set.encode_payload();
    assert_eq!(payload_set.len(), 14);
    assert_eq!(payload_set[5], 45); // Tag 45
    let decoded_set = ControlMsg::decode_payload(&payload_set).unwrap();
    assert_eq!(
        decoded_set,
        ControlMsg::SetScreenFocusMode {
            enabled: true,
            mode: 1,
            radius: 384,
            peripheral_skip_ratio: 3,
        }
    );

    // Tag 46: SetScreenFocusBox
    let msg_box = ControlMsg::SetScreenFocusBox {
        box_x: 64,
        box_y: 128,
        box_w: 800,
        box_h: 600,
    };
    let payload_box = msg_box.encode_payload();
    assert_eq!(payload_box.len(), 14);
    assert_eq!(payload_box[5], 46); // Tag 46
    let decoded_box = ControlMsg::decode_payload(&payload_box).unwrap();
    assert_eq!(
        decoded_box,
        ControlMsg::SetScreenFocusBox {
            box_x: 64,
            box_y: 128,
            box_w: 800,
            box_h: 600,
        }
    );

    // Tag 47: ResetScreenFocusMode
    let msg_reset = ControlMsg::ResetScreenFocusMode;
    let payload_reset = msg_reset.encode_payload();
    assert_eq!(payload_reset.len(), 14);
    assert_eq!(payload_reset[5], 47); // Tag 47
    let decoded_reset = ControlMsg::decode_payload(&payload_reset).unwrap();
    assert_eq!(decoded_reset, ControlMsg::ResetScreenFocusMode);

    // Tag 48: FocusSync
    let msg_sync = ControlMsg::FocusSync {
        focus_tiles_sent: 123456,
    };
    let payload_sync = msg_sync.encode_payload();
    assert_eq!(payload_sync.len(), 14);
    assert_eq!(payload_sync[5], 48); // Tag 48
    let decoded_sync = ControlMsg::decode_payload(&payload_sync).unwrap();
    assert_eq!(
        decoded_sync,
        ControlMsg::FocusSync {
            focus_tiles_sent: 123456,
        }
    );

    // Tag 49: FocusPeripheralSync
    let msg_peri = ControlMsg::FocusPeripheralSync {
        peripheral_skipped: 987654,
    };
    let payload_peri = msg_peri.encode_payload();
    assert_eq!(payload_peri.len(), 14);
    assert_eq!(payload_peri[5], 49); // Tag 49
    let decoded_peri = ControlMsg::decode_payload(&payload_peri).unwrap();
    assert_eq!(
        decoded_peri,
        ControlMsg::FocusPeripheralSync {
            peripheral_skipped: 987654,
        }
    );
}

#[test]
fn screen_tile_palette_compression_and_decompression_algorithms() {
    // 1. Config defaults and clamping
    let def_cfg = ScreenTilePaletteConfig::default();
    assert!(!def_cfg.enabled);
    assert_eq!(def_cfg.max_colors, 16);
    assert!(!def_cfg.enable_rle);

    let custom_cfg = ScreenTilePaletteConfig::new(true, Some(24), Some(true));
    assert!(custom_cfg.enabled);
    assert_eq!(custom_cfg.max_colors, 16); // clamped to 16
    assert!(custom_cfg.enable_rle);

    let min_cfg = ScreenTilePaletteConfig::new(true, Some(1), None);
    assert_eq!(min_cfg.max_colors, 2); // clamped to 2

    let cfg = ScreenTilePaletteConfig::new(true, Some(16), None);

    // 2. Solid Color Tile (K = 1) -> 5 bytes payload (1 byte subcodec + 4 bytes color)
    let w = 64u16;
    let h = 64u16;
    let total_pixels = (w as usize) * (h as usize);
    let solid_color = [0x12, 0x34, 0x56, 0xFF];
    let mut solid_buf = Vec::with_capacity(total_pixels * 4);
    for _ in 0..total_pixels {
        solid_buf.extend_from_slice(&solid_color);
    }

    let (codec, payload) = compress_tile_palette(&solid_buf, w, h, &cfg);
    assert_eq!(codec, TILE_SUBCODEC_SOLID);
    assert_eq!(payload.len(), 5);
    assert_eq!(payload[0], TILE_SUBCODEC_SOLID);

    let decompressed = decompress_tile_palette(codec, &payload, w, h, total_pixels * 4).unwrap();
    assert_eq!(decompressed, solid_buf);

    // 3. Bi-Level Tile (K = 2) -> 521 bytes payload (1 + 8 + 512)
    let c0 = [0x00, 0x00, 0x00, 0xFF];
    let c1 = [0xFF, 0xFF, 0xFF, 0xFF];
    let mut bilevel_buf = Vec::with_capacity(total_pixels * 4);
    for i in 0..total_pixels {
        if i % 2 == 0 {
            bilevel_buf.extend_from_slice(&c0);
        } else {
            bilevel_buf.extend_from_slice(&c1);
        }
    }

    let (codec, payload) = compress_tile_palette(&bilevel_buf, w, h, &cfg);
    assert_eq!(codec, TILE_SUBCODEC_BILEVEL);
    assert_eq!(payload.len(), 1 + 8 + (total_pixels / 8));
    assert_eq!(payload[0], TILE_SUBCODEC_BILEVEL);

    let decompressed = decompress_tile_palette(codec, &payload, w, h, total_pixels * 4).unwrap();
    assert_eq!(decompressed, bilevel_buf);

    // 4. Indexed Palette Tile (3 <= K <= 16) -> 4 colors
    let colors = [
        [0xAA, 0x00, 0x00, 0xFF],
        [0x00, 0xBB, 0x00, 0xFF],
        [0x00, 0x00, 0xCC, 0xFF],
        [0xDD, 0xDD, 0x00, 0xFF],
    ];
    let mut palette_buf = Vec::with_capacity(total_pixels * 4);
    for i in 0..total_pixels {
        palette_buf.extend_from_slice(&colors[i % 4]);
    }

    let (codec, payload) = compress_tile_palette(&palette_buf, w, h, &cfg);
    assert_eq!(codec, TILE_SUBCODEC_PALETTE16);
    assert_eq!(payload.len(), 2 + 4 * 4 + (total_pixels / 2));
    assert_eq!(payload[0], TILE_SUBCODEC_PALETTE16);
    assert_eq!(payload[1], 4); // 4 colors

    let decompressed = decompress_tile_palette(codec, &payload, w, h, total_pixels * 4).unwrap();
    assert_eq!(decompressed, palette_buf);

    // 5. True Color Tile (K > 16) -> Short-circuit abort to RAW
    let mut truecolor_buf = Vec::with_capacity(total_pixels * 4);
    for i in 0..total_pixels {
        let val = (i % 256) as u8;
        truecolor_buf.extend_from_slice(&[val, val.wrapping_add(1), val.wrapping_add(2), 0xFF]);
    }

    let (codec, payload) = compress_tile_palette(&truecolor_buf, w, h, &cfg);
    assert_eq!(codec, TILE_SUBCODEC_RAW);
    assert_eq!(payload.len(), total_pixels * 4);
    assert_eq!(payload, truecolor_buf);

    // 6. ControlMsg Tag 50~52 line protocol roundtrip
    // Tag 50: SetScreenTilePaletteMode
    let msg_set = ControlMsg::SetScreenTilePaletteMode {
        enabled: true,
        max_colors: 12,
        enable_rle: true,
    };
    let payload_set = msg_set.encode_payload();
    assert_eq!(payload_set.len(), 14);
    assert_eq!(payload_set[5], 50); // Tag 50
    let decoded_set = ControlMsg::decode_payload(&payload_set).unwrap();
    assert_eq!(
        decoded_set,
        ControlMsg::SetScreenTilePaletteMode {
            enabled: true,
            max_colors: 12,
            enable_rle: true,
        }
    );

    // Tag 51: ResetScreenTilePaletteMode
    let msg_reset = ControlMsg::ResetScreenTilePaletteMode;
    let payload_reset = msg_reset.encode_payload();
    assert_eq!(payload_reset.len(), 14);
    assert_eq!(payload_reset[5], 51); // Tag 51
    let decoded_reset = ControlMsg::decode_payload(&payload_reset).unwrap();
    assert_eq!(decoded_reset, ControlMsg::ResetScreenTilePaletteMode);

    // Tag 52: TilePaletteSync
    let msg_sync = ControlMsg::TilePaletteSync {
        solid_tiles: 0,
        bilevel_tiles: 0,
        palette_tiles: 0,
        saved_bytes: 987654321,
    };
    let payload_sync = msg_sync.encode_payload();
    assert_eq!(payload_sync.len(), 14);
    assert_eq!(payload_sync[5], 52); // Tag 52
    let decoded_sync = ControlMsg::decode_payload(&payload_sync).unwrap();
    assert_eq!(
        decoded_sync,
        ControlMsg::TilePaletteSync {
            solid_tiles: 0,
            bilevel_tiles: 0,
            palette_tiles: 0,
            saved_bytes: 987654321,
        }
    );
}

#[test]
fn screen_tile_predictive_compression_and_decompression_algorithms() {
    // 1. Config defaults and helpers
    let def_cfg = ScreenTilePredictConfig::default();
    assert!(!def_cfg.enabled);
    assert_eq!(def_cfg.filter_mode, ScreenTilePredictFilter::Auto);
    assert!(def_cfg.enable_rct);

    let custom_cfg =
        ScreenTilePredictConfig::new(true, Some(ScreenTilePredictFilter::Sub), Some(false));
    assert!(custom_cfg.enabled);
    assert_eq!(custom_cfg.filter_mode, ScreenTilePredictFilter::Sub);
    assert!(!custom_cfg.enable_rct);

    // 2. Filter mode enum representation & parsing
    assert_eq!(ScreenTilePredictFilter::None.as_str(), "none");
    assert_eq!(ScreenTilePredictFilter::Sub.as_str(), "sub");
    assert_eq!(ScreenTilePredictFilter::Up.as_str(), "up");
    assert_eq!(ScreenTilePredictFilter::Average.as_str(), "average");
    assert_eq!(ScreenTilePredictFilter::Paeth.as_str(), "paeth");
    assert_eq!(ScreenTilePredictFilter::Auto.as_str(), "auto");

    assert_eq!(
        ScreenTilePredictFilter::from_u8(0),
        ScreenTilePredictFilter::None
    );
    assert_eq!(
        ScreenTilePredictFilter::from_u8(1),
        ScreenTilePredictFilter::Sub
    );
    assert_eq!(
        ScreenTilePredictFilter::from_u8(2),
        ScreenTilePredictFilter::Up
    );
    assert_eq!(
        ScreenTilePredictFilter::from_u8(3),
        ScreenTilePredictFilter::Average
    );
    assert_eq!(
        ScreenTilePredictFilter::from_u8(4),
        ScreenTilePredictFilter::Paeth
    );
    assert_eq!(
        ScreenTilePredictFilter::from_u8(5),
        ScreenTilePredictFilter::Auto
    );

    assert_eq!(
        ScreenTilePredictFilter::from_str_loose("left"),
        Some(ScreenTilePredictFilter::Sub)
    );
    assert_eq!(
        ScreenTilePredictFilter::from_str_loose("top"),
        Some(ScreenTilePredictFilter::Up)
    );
    assert_eq!(
        ScreenTilePredictFilter::from_str_loose("avg"),
        Some(ScreenTilePredictFilter::Average)
    );
    assert_eq!(
        ScreenTilePredictFilter::from_str_loose("diag"),
        Some(ScreenTilePredictFilter::Paeth)
    );
    assert_eq!(
        ScreenTilePredictFilter::from_str_loose("adaptive"),
        Some(ScreenTilePredictFilter::Auto)
    );

    // 3. Report defaults
    let def_rep = ScreenTilePredictReport::default();
    assert!(def_rep.ok);
    assert!(!def_rep.enabled);
    assert_eq!(def_rep.predict_tiles_count, 0);
    assert_eq!(def_rep.saved_bandwidth_bytes, 0);

    // 4. Reversible Color Transform (RCT) bit-for-bit exact reversibility
    let mut orig_pixels = Vec::new();
    for i in 0..64 * 64 {
        let r = ((i * 7) % 256) as u8;
        let g = ((i * 13) % 256) as u8;
        let b = ((i * 29) % 256) as u8;
        let a = 0xFFu8;
        orig_pixels.extend_from_slice(&[r, g, b, a]);
    }
    let mut rct_buf = vec![0u8; orig_pixels.len()];
    rct_transform(&orig_pixels, &mut rct_buf);
    let mut restored_pixels = vec![0u8; orig_pixels.len()];
    rct_inverse(&rct_buf, &mut restored_pixels);
    assert_eq!(restored_pixels, orig_pixels);

    // 5. Directional Spatial Prediction: Horizontal Line (h=1 -> Sub is strictly optimal for Auto)
    let w = 64u16;
    let mut h_line = Vec::with_capacity((w as usize) * 4);
    for col in 0..w {
        let v = (col.wrapping_mul(37) % 256) as u8;
        h_line.extend_from_slice(&[v, v, v, 0xFF]);
    }
    let (chosen_h, residual_h) = predict_filter_tile(&h_line, w, 1, ScreenTilePredictFilter::Auto);
    assert_eq!(chosen_h, ScreenTilePredictFilter::Sub);
    let restored_h = unpredict_filter_tile(&residual_h, w, 1, chosen_h);
    assert_eq!(restored_h, h_line);

    // 6. Directional Spatial Prediction: Vertical Line (w=1 -> Up is strictly optimal for Auto)
    let h = 64u16;
    let mut v_line = Vec::with_capacity((h as usize) * 4);
    for row in 0..h {
        let v = (row.wrapping_mul(37) % 256) as u8;
        v_line.extend_from_slice(&[v, v, v, 0xFF]);
    }
    let (chosen_v, residual_v) = predict_filter_tile(&v_line, 1, h, ScreenTilePredictFilter::Auto);
    assert_eq!(chosen_v, ScreenTilePredictFilter::Up);
    let restored_v = unpredict_filter_tile(&residual_v, 1, h, chosen_v);
    assert_eq!(restored_v, v_line);

    // 7. 2D 64x64 Tile with Explicit Filter Modes (None, Sub, Up, Average, Paeth)
    let mut tile_2d = Vec::with_capacity(64 * 64 * 4);
    for row in 0..64u16 {
        for col in 0..64u16 {
            let r = (row.wrapping_mul(3) ^ col.wrapping_mul(5)) as u8;
            let g = (row.wrapping_add(col).wrapping_mul(7)) as u8;
            let b = (row.wrapping_mul(11)) as u8;
            tile_2d.extend_from_slice(&[r, g, b, 0xFF]);
        }
    }
    for filter in [
        ScreenTilePredictFilter::None,
        ScreenTilePredictFilter::Sub,
        ScreenTilePredictFilter::Up,
        ScreenTilePredictFilter::Average,
        ScreenTilePredictFilter::Paeth,
        ScreenTilePredictFilter::Auto,
    ] {
        let (f_used, res) = predict_filter_tile(&tile_2d, 64, 64, filter);
        let restored = unpredict_filter_tile(&res, 64, 64, f_used);
        assert_eq!(
            restored, tile_2d,
            "Failed roundtrip for filter {:?}",
            filter
        );
    }

    // 8. Full Compress & Decompress pipeline test with dummy codec
    struct TestIdentityCodec;
    impl crate::ScreenCodec for TestIdentityCodec {
        fn compress(&self, raw: &[u8]) -> Result<Vec<u8>, CoreError> {
            Ok(raw.to_vec())
        }
        fn decompress(
            &self,
            compressed: &[u8],
            _expected_len: usize,
        ) -> Result<Vec<u8>, CoreError> {
            Ok(compressed.to_vec())
        }
    }
    let codec = TestIdentityCodec;
    let predict_cfg = ScreenTilePredictConfig {
        enabled: true,
        filter_mode: ScreenTilePredictFilter::Auto,
        enable_rct: true,
    };
    let (subc, payload) = compress_tile_predictive(&tile_2d, 64, 64, &predict_cfg, &codec);
    assert_eq!(subc, TILE_SUBCODEC_PREDICT_ZSTD);
    assert_eq!(payload[0], TILE_SUBCODEC_PREDICT_ZSTD);
    assert_eq!(payload[2], 1); // enable_rct flag is 1

    let decompressed = decompress_tile_predictive(&payload, 64, 64, 64 * 64 * 4, &codec).unwrap();
    assert_eq!(decompressed, tile_2d);

    // 9. ControlMsg Tag 53~55 wire protocol roundtrip
    // Tag 53: SetScreenTilePredictMode
    let msg_set = ControlMsg::SetScreenTilePredictMode {
        enabled: true,
        filter_mode: 3, // Average
        enable_rct: true,
    };
    let payload_set = msg_set.encode_payload();
    assert_eq!(payload_set.len(), 14);
    assert_eq!(payload_set[5], 53); // Tag 53
    let decoded_set = ControlMsg::decode_payload(&payload_set).unwrap();
    assert_eq!(
        decoded_set,
        ControlMsg::SetScreenTilePredictMode {
            enabled: true,
            filter_mode: 3,
            enable_rct: true,
        }
    );

    // Tag 54: ResetScreenTilePredictMode
    let msg_reset = ControlMsg::ResetScreenTilePredictMode;
    let payload_reset = msg_reset.encode_payload();
    assert_eq!(payload_reset.len(), 14);
    assert_eq!(payload_reset[5], 54); // Tag 54
    let decoded_reset = ControlMsg::decode_payload(&payload_reset).unwrap();
    assert_eq!(decoded_reset, ControlMsg::ResetScreenTilePredictMode);

    // Tag 55: TilePredictSync
    let msg_sync = ControlMsg::TilePredictSync {
        predict_tiles: 0,
        saved_bytes: 1234567890,
    };
    let payload_sync = msg_sync.encode_payload();
    assert_eq!(payload_sync.len(), 14);
    assert_eq!(payload_sync[5], 55); // Tag 55
    let decoded_sync = ControlMsg::decode_payload(&payload_sync).unwrap();
    assert_eq!(
        decoded_sync,
        ControlMsg::TilePredictSync {
            predict_tiles: 0,
            saved_bytes: 1234567890,
        }
    );
}

#[test]
fn screen_tile_presentation_transaction_and_double_buffer_algorithms() {
    // 1. Config defaults and helpers
    let def_cfg = ScreenTilePresentConfig::default();
    assert!(!def_cfg.enabled);
    assert_eq!(def_cfg.vsync_mode, ScreenTilePresentVsyncMode::Off);
    assert!(def_cfg.double_buffer);
    assert!(def_cfg.tear_prevention);

    let custom_cfg = ScreenTilePresentConfig::new(
        true,
        Some(ScreenTilePresentVsyncMode::Vsync60),
        Some(true),
        Some(true),
    );
    assert!(custom_cfg.enabled);
    assert_eq!(custom_cfg.vsync_mode, ScreenTilePresentVsyncMode::Vsync60);
    assert!(custom_cfg.double_buffer);
    assert!(custom_cfg.tear_prevention);

    // 2. VsyncMode enum representation & parsing
    assert_eq!(ScreenTilePresentVsyncMode::Off.as_str(), "off");
    assert_eq!(ScreenTilePresentVsyncMode::Vsync60.as_str(), "vsync60");
    assert_eq!(ScreenTilePresentVsyncMode::Vsync120.as_str(), "vsync120");
    assert_eq!(ScreenTilePresentVsyncMode::Adaptive.as_str(), "adaptive");

    assert_eq!(
        ScreenTilePresentVsyncMode::from_u8(0),
        ScreenTilePresentVsyncMode::Off
    );
    assert_eq!(
        ScreenTilePresentVsyncMode::from_u8(1),
        ScreenTilePresentVsyncMode::Vsync60
    );
    assert_eq!(
        ScreenTilePresentVsyncMode::from_u8(2),
        ScreenTilePresentVsyncMode::Vsync120
    );
    assert_eq!(
        ScreenTilePresentVsyncMode::from_u8(3),
        ScreenTilePresentVsyncMode::Adaptive
    );

    assert_eq!(
        ScreenTilePresentVsyncMode::from_str_loose("off"),
        Some(ScreenTilePresentVsyncMode::Off)
    );
    assert_eq!(
        ScreenTilePresentVsyncMode::from_str_loose("vsync60"),
        Some(ScreenTilePresentVsyncMode::Vsync60)
    );
    assert_eq!(
        ScreenTilePresentVsyncMode::from_str_loose("60hz"),
        Some(ScreenTilePresentVsyncMode::Vsync60)
    );
    assert_eq!(
        ScreenTilePresentVsyncMode::from_str_loose("vsync120"),
        Some(ScreenTilePresentVsyncMode::Vsync120)
    );
    assert_eq!(
        ScreenTilePresentVsyncMode::from_str_loose("adaptive"),
        Some(ScreenTilePresentVsyncMode::Adaptive)
    );
    assert_eq!(ScreenTilePresentVsyncMode::from_str_loose("unknown"), None);

    assert_eq!(
        ScreenTilePresentVsyncMode::Vsync60.target_interval_us(),
        16_666
    );
    assert_eq!(
        ScreenTilePresentVsyncMode::Vsync120.target_interval_us(),
        8_333
    );

    // 3. Report defaults
    let def_rep = ScreenTilePresentReport::default();
    assert!(def_rep.ok);
    assert!(!def_rep.enabled);
    assert_eq!(def_rep.committed_transactions, 0);
    assert_eq!(def_rep.swapped_frames, 0);
    assert_eq!(def_rep.torn_frames_prevented, 0);

    // 4. Double-Buffered Presenter lifecycle & Atomic Swap
    let mut presenter = ScreenTilePresenter::new(custom_cfg);
    let w = 128u16;
    let h = 128u16;
    let total_pixels = (w as usize) * (h as usize);
    let mut initial_pixels = vec![0u8; total_pixels * 4];
    for p in initial_pixels.chunks_exact_mut(4) {
        p[0] = 0x11;
        p[1] = 0x22;
        p[2] = 0x33;
        p[3] = 0xFF;
    }
    presenter.set_front_buffer(ScreenFrame {
        seq: 1,
        rect: ScreenRect::new(0, 0, w, h).unwrap(),
        format: PixelFormat::Rgba8,
        stride: (w as u32) * 4,
        codec_tag: 0,
        bytes: initial_pixels.clone(),
    });

    // Begin transaction for new frame
    presenter.begin_transaction(w, h);
    assert!(presenter.back_buffer().is_some());
    assert_eq!(presenter.back_buffer().unwrap().bytes, initial_pixels);

    // Stage Tile 0 (top-left 64x64) with red
    let t0 = DirtyTile {
        index: 0,
        x: 0,
        y: 0,
        w: 64,
        h: 64,
        data: [0xFF, 0x00, 0x00, 0xFF].repeat(64 * 64),
    };
    assert!(presenter.stage_dirty_tile(&t0));

    // Stage Tile 1 (top-right 64x64) with green
    let t1 = DirtyTile {
        index: 1,
        x: 64,
        y: 0,
        w: 64,
        h: 64,
        data: [0x00, 0xFF, 0x00, 0xFF].repeat(64 * 64),
    };
    assert!(presenter.stage_dirty_tile(&t1));

    // During staging: front buffer is completely untouched!
    assert_eq!(presenter.front_buffer().unwrap().bytes, initial_pixels);

    // Commit and swap
    let swapped = presenter.commit_and_swap(2, 100_000);
    assert!(swapped);
    let front = presenter.front_buffer().unwrap();
    assert_eq!(front.seq, 2);

    // Front buffer now reflects both tiles atomically!
    // Sample Tile 0 pixel at (0, 0)
    assert_eq!(&front.bytes[0..4], &[0xFF, 0x00, 0x00, 0xFF]);
    // Sample Tile 1 pixel at (64, 0)
    let t1_idx = 64 * 4;
    assert_eq!(&front.bytes[t1_idx..t1_idx + 4], &[0x00, 0xFF, 0x00, 0xFF]);

    let rep = presenter.report();
    assert!(rep.enabled);
    assert_eq!(rep.committed_transactions, 1);
    assert_eq!(rep.swapped_frames, 1);
    assert_eq!(rep.torn_frames_prevented, 1); // Both tiles were staged before swap -> 1 prevention
    assert!(rep.avg_swap_latency_us > 0);

    // 5. ControlMsg Tag 56~58 wire protocol roundtrip
    // Tag 56: SetScreenTilePresentMode
    let msg_set = ControlMsg::SetScreenTilePresentMode {
        enabled: true,
        vsync_mode: 1, // Vsync60
        double_buffer: true,
        tear_prevention: true,
    };
    let payload_set = msg_set.encode_payload();
    assert_eq!(payload_set.len(), 14);
    assert_eq!(payload_set[5], 56); // Tag 56
    let decoded_set = ControlMsg::decode_payload(&payload_set).unwrap();
    assert_eq!(
        decoded_set,
        ControlMsg::SetScreenTilePresentMode {
            enabled: true,
            vsync_mode: 1,
            double_buffer: true,
            tear_prevention: true,
        }
    );

    // Tag 57: ResetScreenTilePresentMode
    let msg_reset = ControlMsg::ResetScreenTilePresentMode;
    let payload_reset = msg_reset.encode_payload();
    assert_eq!(payload_reset.len(), 14);
    assert_eq!(payload_reset[5], 57); // Tag 57
    let decoded_reset = ControlMsg::decode_payload(&payload_reset).unwrap();
    assert_eq!(decoded_reset, ControlMsg::ResetScreenTilePresentMode);

    // Tag 58: TilePresentSync
    let msg_sync = ControlMsg::TilePresentSync {
        committed_txs: 42,
        swapped_frames: 42,
    };
    let payload_sync = msg_sync.encode_payload();
    assert_eq!(payload_sync.len(), 14);
    assert_eq!(payload_sync[5], 58); // Tag 58
    let decoded_sync = ControlMsg::decode_payload(&payload_sync).unwrap();
    assert_eq!(
        decoded_sync,
        ControlMsg::TilePresentSync {
            committed_txs: 42,
            swapped_frames: 0,
        }
    );
}

#[test]
fn screen_tile_nack_and_fast_retransmission_algorithms() {
    // 1. ScreenTileNackConfig validation and clamping
    let def_cfg = ScreenTileNackConfig::default();
    assert!(!def_cfg.enabled);
    assert_eq!(def_cfg.timeout_ms, 20);
    assert_eq!(def_cfg.max_retries, 2);

    let custom_cfg = ScreenTileNackConfig::new(true, Some(3), Some(20));
    assert!(custom_cfg.enabled);
    assert_eq!(custom_cfg.timeout_ms, 5); // Clamped to min 5ms
    assert_eq!(custom_cfg.max_retries, 10); // Clamped to max 10 retries

    // 2. ScreenTileNackReport defaults
    let rep = ScreenTileNackReport::default();
    assert!(rep.ok);
    assert!(!rep.enabled);
    assert_eq!(rep.timeout_ms, 20);
    assert_eq!(rep.max_retries, 2);
    assert_eq!(rep.nack_requests_sent, 0);
    assert_eq!(rep.tiles_retransmitted, 0);
    assert_eq!(rep.degraded_commits, 0);

    // 3. ScreenTileResendBuffer FIFO ring-buffer mechanics
    let mut resend_buf = ScreenTileResendBuffer::new(3);
    assert_eq!(resend_buf.capacity, 3);

    let t0 = DirtyTile {
        index: 0,
        x: 0,
        y: 0,
        w: 64,
        h: 64,
        data: vec![1, 2, 3, 4],
    };
    let t1 = DirtyTile {
        index: 1,
        x: 64,
        y: 0,
        w: 64,
        h: 64,
        data: vec![5, 6, 7, 8],
    };
    resend_buf.push_frame(10, vec![t0.clone(), t1.clone()]);

    let t2 = DirtyTile {
        index: 2,
        x: 0,
        y: 64,
        w: 64,
        h: 64,
        data: vec![9, 10, 11, 12],
    };
    let t3 = DirtyTile {
        index: 3,
        x: 64,
        y: 64,
        w: 64,
        h: 64,
        data: vec![13, 14, 15, 16],
    };
    resend_buf.push_frame(11, vec![t2.clone(), t3.clone()]);

    let t4 = DirtyTile {
        index: 4,
        x: 128,
        y: 0,
        w: 64,
        h: 64,
        data: vec![17, 18, 19, 20],
    };
    resend_buf.push_frame(12, vec![t4.clone()]);

    // Query existing frames
    let retrieved_t0 = resend_buf.get_tile(10, 0);
    assert!(retrieved_t0.is_some());
    assert_eq!(retrieved_t0.unwrap().data, t0.data);

    let retrieved_t3 = resend_buf.get_tile(11, 3);
    assert!(retrieved_t3.is_some());
    assert_eq!(retrieved_t3.unwrap().data, t3.data);

    let retrieved_t4 = resend_buf.get_tile(12, 4);
    assert!(retrieved_t4.is_some());
    assert_eq!(retrieved_t4.unwrap().data, t4.data);

    // Non-existent tile index in frame 10
    assert!(resend_buf.get_tile(10, 2).is_none());

    // Push frame 13 -> Frame 10 must be evicted (capacity = 3)
    let t5 = DirtyTile {
        index: 5,
        x: 128,
        y: 64,
        w: 64,
        h: 64,
        data: vec![21, 22, 23, 24],
    };
    resend_buf.push_frame(13, vec![t5.clone()]);
    assert_eq!(resend_buf.frames.len(), 3);
    assert!(resend_buf.get_tile(10, 0).is_none()); // Evicted!
    assert!(resend_buf.get_tile(11, 2).is_some()); // Still in buffer
    assert!(resend_buf.get_tile(13, 5).is_some()); // Newly added

    resend_buf.clear();
    assert_eq!(resend_buf.frames.len(), 0);
    assert!(resend_buf.get_tile(13, 5).is_none());

    // 4. ControlMsg Tag 59~61 wire protocol roundtrip
    // Tag 59: TileNackRequest
    let msg_nack = ControlMsg::TileNackRequest {
        seq: 54321,
        tile_index: 7,
        retry_count: 1,
    };
    let payload_nack = msg_nack.encode_payload();
    assert_eq!(payload_nack.len(), 14);
    assert_eq!(payload_nack[5], 59); // Tag 59
    let decoded_nack = ControlMsg::decode_payload(&payload_nack).unwrap();
    assert_eq!(
        decoded_nack,
        ControlMsg::TileNackRequest {
            seq: 54321,
            tile_index: 7,
            retry_count: 1,
        }
    );

    // Tag 60: ResetScreenTileNackMode
    let msg_reset = ControlMsg::ResetScreenTileNackMode;
    let payload_reset = msg_reset.encode_payload();
    assert_eq!(payload_reset.len(), 14);
    assert_eq!(payload_reset[5], 60); // Tag 60
    let decoded_reset = ControlMsg::decode_payload(&payload_reset).unwrap();
    assert_eq!(decoded_reset, ControlMsg::ResetScreenTileNackMode);

    // Tag 61: TileNackSync
    let msg_sync = ControlMsg::TileNackSync {
        nack_requests: 99,
        retransmitted_tiles: 42,
        degraded_commits: 3,
    };
    let payload_sync = msg_sync.encode_payload();
    assert_eq!(payload_sync.len(), 14);
    assert_eq!(payload_sync[5], 61); // Tag 61
    let decoded_sync = ControlMsg::decode_payload(&payload_sync).unwrap();
    assert_eq!(
        decoded_sync,
        ControlMsg::TileNackSync {
            nack_requests: 99,
            retransmitted_tiles: 0,
            degraded_commits: 0,
        }
    );
}

#[test]
fn screen_tile_saliency_and_tiered_scheduling_algorithms() {
    // 1. TileSaliencyTier enum
    assert_eq!(TileSaliencyTier::Critical.to_u8(), 0);
    assert_eq!(TileSaliencyTier::Normal.to_u8(), 1);
    assert_eq!(TileSaliencyTier::Background.to_u8(), 2);
    assert_eq!(TileSaliencyTier::from_u8(0), TileSaliencyTier::Critical);
    assert_eq!(TileSaliencyTier::from_u8(1), TileSaliencyTier::Normal);
    assert_eq!(TileSaliencyTier::from_u8(2), TileSaliencyTier::Background);
    assert_eq!(TileSaliencyTier::Critical.as_str(), "critical");
    assert_eq!(TileSaliencyTier::Normal.as_str(), "normal");
    assert_eq!(TileSaliencyTier::Background.as_str(), "background");

    // 2. ScreenTileSaliencyConfig validation and defaults
    let def_cfg = ScreenTileSaliencyConfig::default();
    assert!(!def_cfg.enabled);
    assert_eq!(def_cfg.high_threshold, 120);
    assert_eq!(def_cfg.low_threshold, 40);
    assert!(def_cfg.drop_background_on_congestion);

    let custom_cfg = ScreenTileSaliencyConfig::new(true, Some(30), Some(150), Some(false));
    assert!(custom_cfg.enabled);
    assert_eq!(custom_cfg.high_threshold, 150);
    assert_eq!(custom_cfg.low_threshold, 30);
    assert!(!custom_cfg.drop_background_on_congestion);

    // 3. ScreenTileSaliencyReport defaults
    let rep = ScreenTileSaliencyReport::default();
    assert!(rep.ok);
    assert!(!rep.enabled);
    assert_eq!(rep.critical_tiles_sent, 0);
    assert_eq!(rep.normal_tiles_sent, 0);
    assert_eq!(rep.background_tiles_sent, 0);
    assert_eq!(rep.background_tiles_skipped, 0);

    // 4. Gradient energy on flat vs high-contrast tiles
    let flat_data = vec![128u8; 64 * 64 * 4];
    let flat_energy = ScreenTileSaliencyEvaluator::compute_gradient_energy(&flat_data, 64, 64);
    assert_eq!(flat_energy, 0);

    let mut high_contrast_data = vec![0u8; 64 * 64 * 4];
    for y in 0..64 {
        for x in 0..64 {
            let val = if (x / 2 + y / 2) % 2 == 0 { 255u8 } else { 0u8 };
            let off = (y * 64 + x) * 4;
            high_contrast_data[off] = val;
            high_contrast_data[off + 1] = val;
            high_contrast_data[off + 2] = val;
            high_contrast_data[off + 3] = 255;
        }
    }
    let hc_energy =
        ScreenTileSaliencyEvaluator::compute_gradient_energy(&high_contrast_data, 64, 64);
    assert!(
        hc_energy >= 150,
        "high contrast checkerboard energy: {}",
        hc_energy
    );

    // 5. Focus proximity
    let p_intersect = ScreenTileSaliencyEvaluator::compute_focus_proximity(
        100,
        100,
        64,
        64,
        (120, 120, 100, 100),
    );
    assert_eq!(p_intersect, 255);

    let p_distant = ScreenTileSaliencyEvaluator::compute_focus_proximity(
        100,
        100,
        64,
        64,
        (900, 900, 100, 100),
    );
    assert!(p_distant < 50, "distant proximity: {}", p_distant);

    // 6. Schedule and reorder tiles
    let tile_crit = DirtyTile {
        index: 0,
        x: 0,
        y: 0,
        w: 64,
        h: 64,
        data: high_contrast_data.clone(),
    };
    let tile_bg = DirtyTile {
        index: 1,
        x: 64,
        y: 0,
        w: 64,
        h: 64,
        data: flat_data.clone(),
    };
    let mut moderate_data = vec![128u8; 64 * 64 * 4];
    for y in 0..64 {
        for x in 0..64 {
            let off = (y * 64 + x) * 4;
            let v = ((x * 10) % 255) as u8;
            moderate_data[off] = v;
            moderate_data[off + 1] = v;
            moderate_data[off + 2] = v;
            moderate_data[off + 3] = 255;
        }
    }
    let tile_norm = DirtyTile {
        index: 2,
        x: 128,
        y: 0,
        w: 64,
        h: 64,
        data: moderate_data,
    };

    let cfg_active = ScreenTileSaliencyConfig::new(true, Some(120), Some(40), Some(true));
    // Input order: [tile_bg (idx 1), tile_norm (idx 2), tile_crit (idx 0)]
    let raw_list = vec![tile_bg.clone(), tile_norm.clone(), tile_crit.clone()];

    // Schedule without congestion
    let (reordered, crit_cnt, norm_cnt, bg_cnt, skipped_cnt) =
        ScreenTileSaliencyEvaluator::schedule_tiles(raw_list.clone(), None, &cfg_active, false);
    assert_eq!(crit_cnt, 1);
    assert_eq!(norm_cnt, 1);
    assert_eq!(bg_cnt, 1);
    assert_eq!(skipped_cnt, 0);
    assert_eq!(reordered.len(), 3);
    // Critical tile (idx 0) must be reordered to front!
    assert_eq!(reordered[0].index, 0);
    assert_eq!(reordered[1].index, 2);
    assert_eq!(reordered[2].index, 1);

    // Schedule with congestion -> background tile must be skipped!
    let (reordered_cong, c_cnt, n_cnt, b_cnt, s_cnt) =
        ScreenTileSaliencyEvaluator::schedule_tiles(raw_list, None, &cfg_active, true);
    assert_eq!(c_cnt, 1);
    assert_eq!(n_cnt, 1);
    assert_eq!(b_cnt, 1);
    assert_eq!(s_cnt, 1);
    assert_eq!(reordered_cong.len(), 2);
    assert_eq!(reordered_cong[0].index, 0);
    assert_eq!(reordered_cong[1].index, 2);

    // 7. ControlMsg Tag 62~64 wire codecs
    // Tag 62: SetScreenTileSaliencyMode
    let msg_set = ControlMsg::SetScreenTileSaliencyMode {
        enabled: true,
        high_threshold: 140,
        low_threshold: 50,
        drop_background_on_congestion: true,
    };
    let payload_set = msg_set.encode_payload();
    assert_eq!(payload_set.len(), 14);
    assert_eq!(payload_set[5], 62);
    let dec_set = ControlMsg::decode_payload(&payload_set).unwrap();
    assert_eq!(dec_set, msg_set);

    // Tag 63: ResetScreenTileSaliencyMode
    let msg_reset = ControlMsg::ResetScreenTileSaliencyMode;
    let payload_reset = msg_reset.encode_payload();
    assert_eq!(payload_reset.len(), 14);
    assert_eq!(payload_reset[5], 63);
    let dec_reset = ControlMsg::decode_payload(&payload_reset).unwrap();
    assert_eq!(dec_reset, msg_reset);

    // Tag 64: TileSaliencySync
    let msg_sync = ControlMsg::TileSaliencySync {
        critical_tiles: 888,
        normal_tiles: 555,
        background_tiles: 333,
        background_tiles_skipped: 42,
    };
    let payload_sync = msg_sync.encode_payload();
    assert_eq!(payload_sync.len(), 14);
    assert_eq!(payload_sync[5], 64);
    let dec_sync = ControlMsg::decode_payload(&payload_sync).unwrap();
    assert_eq!(
        dec_sync,
        ControlMsg::TileSaliencySync {
            critical_tiles: 888,
            normal_tiles: 0,
            background_tiles: 0,
            background_tiles_skipped: 0,
        }
    );
}

#[test]
fn screen_tile_lod_and_multiresolution_downsampling_algorithms() {
    // 1. TileLodLevel conversions and scale factors
    assert_eq!(TileLodLevel::from_u8(0), TileLodLevel::Lod0);
    assert_eq!(TileLodLevel::from_u8(1), TileLodLevel::Lod1);
    assert_eq!(TileLodLevel::from_u8(2), TileLodLevel::Lod2);
    assert_eq!(TileLodLevel::from_u8(99), TileLodLevel::Lod0);
    assert_eq!(TileLodLevel::Lod0.scale_factor(), 1);
    assert_eq!(TileLodLevel::Lod1.scale_factor(), 2);
    assert_eq!(TileLodLevel::Lod2.scale_factor(), 4);

    // 2. ScreenTileLodConfig and Report defaults
    let def_cfg = ScreenTileLodConfig::default();
    assert!(!def_cfg.enabled);
    assert_eq!(def_cfg.max_lod, 1);
    assert!(def_cfg.auto_lod_on_congestion);

    let def_rep = ScreenTileLodReport::default();
    assert!(def_rep.ok);
    assert!(!def_rep.enabled);
    assert_eq!(def_rep.lod0_tiles_sent, 0);
    assert_eq!(def_rep.lod1_tiles_sent, 0);
    assert_eq!(def_rep.lod2_tiles_sent, 0);
    assert_eq!(def_rep.bytes_saved, 0);

    // 3. Flat color downsampling & upsampling invariance
    let mut flat_red = vec![0u8; 64 * 64 * 4];
    for chunk in flat_red.chunks_exact_mut(4) {
        chunk[0] = 250;
        chunk[1] = 10;
        chunk[2] = 20;
        chunk[3] = 255;
    }

    // Downsample to Lod1 (32x32)
    let (down1, w1, h1) = downsample_tile_rgba(&flat_red, 64, 64, TileLodLevel::Lod1);
    assert_eq!(w1, 32);
    assert_eq!(h1, 32);
    assert_eq!(down1.len(), 32 * 32 * 4);
    assert_eq!(down1[0], 250);
    assert_eq!(down1[1], 10);
    assert_eq!(down1[2], 20);
    assert_eq!(down1[3], 255);

    // Upsample back to 64x64
    let up1 = upsample_tile_rgba(&down1, 32, 32, 64, 64);
    assert_eq!(up1.len(), 64 * 64 * 4);
    assert_eq!(up1[0], 250);
    assert_eq!(up1[1], 10);
    assert_eq!(up1[2], 20);
    assert_eq!(up1[3], 255);

    // Downsample to Lod2 (16x16)
    let (down2, w2, h2) = downsample_tile_rgba(&flat_red, 64, 64, TileLodLevel::Lod2);
    assert_eq!(w2, 16);
    assert_eq!(h2, 16);
    assert_eq!(down2.len(), 16 * 16 * 4);
    assert_eq!(down2[0], 250);

    // Upsample back to 64x64
    let up2 = upsample_tile_rgba(&down2, 16, 16, 64, 64);
    assert_eq!(up2.len(), 64 * 64 * 4);
    assert_eq!(up2[0], 250);

    // 4. Test compress_tile_lod and decompress_tile_lod roundtrip
    struct DummyCodec;
    impl crate::ScreenCodec for DummyCodec {
        fn compress(&self, raw: &[u8]) -> Result<Vec<u8>, CoreError> {
            Ok(raw.to_vec())
        }
        fn decompress(
            &self,
            compressed: &[u8],
            _expected_len: usize,
        ) -> Result<Vec<u8>, CoreError> {
            Ok(compressed.to_vec())
        }
        fn compress_frame(&self, raw: &[u8], _w: u16, _h: u16) -> Result<Vec<u8>, CoreError> {
            Ok(raw.to_vec())
        }
        fn decompress_frame(
            &self,
            data: &[u8],
            _w: u16,
            _h: u16,
            _expected_len: usize,
        ) -> Result<Vec<u8>, CoreError> {
            Ok(data.to_vec())
        }
    }
    let dummy = DummyCodec;

    let (codec_tag, payload) = compress_tile_lod(&flat_red, 64, 64, TileLodLevel::Lod1, &dummy);
    assert_eq!(codec_tag, TILE_SUBCODEC_LOD);
    assert_eq!(payload[0], TILE_SUBCODEC_LOD);
    assert_eq!(payload[1], 1); // Lod1

    let decompressed = decompress_tile_lod(&payload, 64, 64, 64 * 64 * 4, &dummy).unwrap();
    assert_eq!(decompressed.len(), 64 * 64 * 4);
    assert_eq!(decompressed[0], 250);
    assert_eq!(decompressed[1], 10);
    assert_eq!(decompressed[2], 20);
    assert_eq!(decompressed[3], 255);

    // 5. ControlMsg Tag 65~67 wire codecs
    // Tag 65: SetScreenTileLodMode
    let msg_set = ControlMsg::SetScreenTileLodMode {
        enabled: true,
        max_lod: 2,
        auto_lod_on_congestion: true,
    };
    let payload_set = msg_set.encode_payload();
    assert_eq!(payload_set.len(), 14);
    assert_eq!(payload_set[5], 65);
    let dec_set = ControlMsg::decode_payload(&payload_set).unwrap();
    assert_eq!(dec_set, msg_set);

    // Tag 66: ResetScreenTileLodMode
    let msg_reset = ControlMsg::ResetScreenTileLodMode;
    let payload_reset = msg_reset.encode_payload();
    assert_eq!(payload_reset.len(), 14);
    assert_eq!(payload_reset[5], 66);
    let dec_reset = ControlMsg::decode_payload(&payload_reset).unwrap();
    assert_eq!(dec_reset, msg_reset);

    // Tag 67: TileLodSync
    let msg_sync = ControlMsg::TileLodSync {
        lod0_tiles: 120,
        lod1_tiles: 34,
        lod2_tiles: 12,
        bytes_saved: 987654,
    };
    let payload_sync = msg_sync.encode_payload();
    assert_eq!(payload_sync.len(), 14);
    assert_eq!(payload_sync[5], 67);
    let dec_sync = ControlMsg::decode_payload(&payload_sync).unwrap();
    assert_eq!(
        dec_sync,
        ControlMsg::TileLodSync {
            lod0_tiles: 0,
            lod1_tiles: 0,
            lod2_tiles: 0,
            bytes_saved: 987654,
        }
    );
}

#[test]
fn screen_tile_batch_and_chunking_algorithms() {
    // 1. ScreenTileBatchConfig defaults and clamping
    let def_cfg = ScreenTileBatchConfig::default();
    assert!(!def_cfg.enabled);
    assert_eq!(
        def_cfg.target_chunk_bytes,
        SCREEN_TILE_BATCH_DEFAULT_CHUNK_BYTES
    );
    assert_eq!(
        def_cfg.pacing_interval_us,
        SCREEN_TILE_BATCH_DEFAULT_PACING_US
    );

    let custom_cfg = ScreenTileBatchConfig::new(true, Some(2000), Some(350));
    assert!(custom_cfg.enabled);
    assert_eq!(custom_cfg.target_chunk_bytes, 2000);
    assert_eq!(custom_cfg.pacing_interval_us, 350);

    let clamped_low = ScreenTileBatchConfig::new(true, Some(100), Some(0));
    assert_eq!(
        clamped_low.target_chunk_bytes,
        SCREEN_TILE_BATCH_MIN_CHUNK_BYTES
    );
    assert_eq!(clamped_low.pacing_interval_us, 0);

    let clamped_high = ScreenTileBatchConfig::new(true, Some(65000), Some(60000));
    assert_eq!(
        clamped_high.target_chunk_bytes,
        SCREEN_TILE_BATCH_MAX_CHUNK_BYTES
    );
    assert_eq!(
        clamped_high.pacing_interval_us,
        SCREEN_TILE_BATCH_MAX_PACING_US
    );

    // 2. ScreenTileBatchReport defaults
    let def_rep = ScreenTileBatchReport::default();
    assert!(def_rep.ok);
    assert!(!def_rep.enabled);
    assert_eq!(
        def_rep.target_chunk_bytes,
        SCREEN_TILE_BATCH_DEFAULT_CHUNK_BYTES
    );
    assert_eq!(
        def_rep.pacing_interval_us,
        SCREEN_TILE_BATCH_DEFAULT_PACING_US
    );
    assert_eq!(def_rep.batches_sent, 0);
    assert_eq!(def_rep.chunks_sent, 0);
    assert_eq!(def_rep.avg_chunk_bytes, 0);
    assert_eq!(def_rep.pacing_pauses, 0);

    // 3. ScreenFrame::encode_tile_batch_chunk and decode_tile_batch roundtrip
    let rect = ScreenRect::new(0, 0, 128, 64).unwrap();
    let t0 = DirtyTile {
        index: 0,
        x: 0,
        y: 0,
        w: 64,
        h: 64,
        data: vec![10u8; 128],
    };
    let t1 = DirtyTile {
        index: 1,
        x: 64,
        y: 0,
        w: 64,
        h: 64,
        data: vec![20u8; 256],
    };
    let tiles = vec![t0.clone(), t1.clone()];

    let frame = ScreenFrame::encode_tile_batch_chunk(
        42,
        rect,
        PixelFormat::Bgra8,
        1001,
        0,
        2,
        64,
        10,
        &tiles,
    );
    assert_eq!(frame.codec_tag, SCREEN_COMPRESS_TILE_BATCH);
    assert!(frame.compressed());

    let (batch_id, chunk_index, total_chunks, tile_size, total_tiles, dec_tiles) =
        frame.decode_tile_batch().expect("decode tile batch");
    assert_eq!(batch_id, 1001);
    assert_eq!(chunk_index, 0);
    assert_eq!(total_chunks, 2);
    assert_eq!(tile_size, 64);
    assert_eq!(total_tiles, 10);
    assert_eq!(dec_tiles.len(), 2);
    assert_eq!(dec_tiles[0].index, 0);
    assert_eq!(dec_tiles[0].data, vec![10u8; 128]);
    assert_eq!(dec_tiles[1].index, 1);
    assert_eq!(dec_tiles[1].data, vec![20u8; 256]);

    // Truncated payload check
    let mut bad_frame = frame.clone();
    bad_frame.bytes.truncate(10);
    assert!(bad_frame.decode_tile_batch().is_err());

    // 4. chunk_dirty_tiles algorithm
    // Empty tiles produces single empty chunk
    let empty_chunks = chunk_dirty_tiles(1, rect, PixelFormat::Bgra8, Vec::new(), 1200, 64, 0, 500);
    assert_eq!(empty_chunks.len(), 1);
    let (_, _, total_chunks, _, _, empty_tiles) = empty_chunks[0].decode_tile_batch().unwrap();
    assert_eq!(total_chunks, 1);
    assert!(empty_tiles.is_empty());

    // Multiple tiles exceeding target_chunk_bytes
    // 5 tiles, each with 300 bytes of data (+ 14 record header = 314 bytes per tile)
    // target = 600 bytes
    // chunk 0: batch_hdr(14) + tile0(314) = 328 bytes. tile1 would make it 328 + 314 = 642 > 600 -> closes chunk 0!
    // So 5 tiles should chunk into 5 separate chunks!
    let mut big_tiles = Vec::new();
    for i in 0..5u16 {
        big_tiles.push(DirtyTile {
            index: i,
            x: i * 10,
            y: 0,
            w: 64,
            h: 64,
            data: vec![i as u8; 300],
        });
    }
    let chunks = chunk_dirty_tiles(
        2,
        rect,
        PixelFormat::Bgra8,
        big_tiles.clone(),
        600,
        64,
        5,
        777,
    );
    assert_eq!(chunks.len(), 5);
    for (idx, chk) in chunks.iter().enumerate() {
        let (bid, c_idx, tot_c, _, tot_t, c_tiles) = chk.decode_tile_batch().unwrap();
        assert_eq!(bid, 777);
        assert_eq!(c_idx, idx as u16);
        assert_eq!(tot_c, 5);
        assert_eq!(tot_t, 5);
        assert_eq!(c_tiles.len(), 1);
        assert_eq!(c_tiles[0].index, idx as u16);
    }

    // 5. ControlMsg Tag 68~70 wire codecs
    // Tag 68: SetScreenTileBatchMode
    let msg_set = ControlMsg::SetScreenTileBatchMode {
        enabled: true,
        target_chunk_bytes: 1400,
        pacing_interval_us: 300,
    };
    let payload_set = msg_set.encode_payload();
    assert_eq!(payload_set.len(), 14);
    assert_eq!(payload_set[5], 68);
    let dec_set = ControlMsg::decode_payload(&payload_set).unwrap();
    assert_eq!(dec_set, msg_set);

    // Tag 69: ResetScreenTileBatchMode
    let msg_reset = ControlMsg::ResetScreenTileBatchMode;
    let payload_reset = msg_reset.encode_payload();
    assert_eq!(payload_reset.len(), 14);
    assert_eq!(payload_reset[5], 69);
    let dec_reset = ControlMsg::decode_payload(&payload_reset).unwrap();
    assert_eq!(dec_reset, msg_reset);

    // Tag 70: TileBatchSync
    let msg_sync = ControlMsg::TileBatchSync {
        batches_sent: 12345,
        chunks_sent: 54321,
        avg_chunk_bytes: 1180,
        pacing_pauses: 999,
    };
    let payload_sync = msg_sync.encode_payload();
    assert_eq!(payload_sync.len(), 14);
    assert_eq!(payload_sync[5], 70);
    let dec_sync = ControlMsg::decode_payload(&payload_sync).unwrap();
    assert_eq!(
        dec_sync,
        ControlMsg::TileBatchSync {
            batches_sent: 12345,
            chunks_sent: 0,
            avg_chunk_bytes: 0,
            pacing_pauses: 0,
        }
    );
}

#[test]
fn screen_tile_motion_and_displacement_compensation_algorithms() {
    // 1. ScreenTileMotionConfig defaults and bounds
    let cfg = ScreenTileMotionConfig::default();
    assert!(!cfg.enabled);
    assert_eq!(
        cfg.max_displacement,
        SCREEN_TILE_MOTION_DEFAULT_MAX_DISPLACEMENT
    );
    assert_eq!(
        cfg.min_match_ratio,
        SCREEN_TILE_MOTION_DEFAULT_MIN_MATCH_RATIO
    );

    // 2. Build reference canvas (192x192 Bgra8) with distinct pattern
    let canvas_w = 192u16;
    let canvas_h = 192u16;
    let mut canvas_raw = vec![0u8; (canvas_w as usize) * (canvas_h as usize) * 4];
    for y in 0..canvas_h as usize {
        for x in 0..canvas_w as usize {
            let off = (y * (canvas_w as usize) + x) * 4;
            canvas_raw[off] = (x % 256) as u8;
            canvas_raw[off + 1] = (y % 256) as u8;
            canvas_raw[off + 2] = ((x + y) % 256) as u8;
            canvas_raw[off + 3] = 255;
        }
    }
    let ref_canvas = ScreenFrame {
        seq: 1,
        rect: ScreenRect {
            x: 0,
            y: 0,
            w: canvas_w,
            h: canvas_h,
        },
        stride: (canvas_w as u32) * 4,
        format: PixelFormat::Bgra8,
        codec_tag: SCREEN_COMPRESS_NONE,
        bytes: canvas_raw.clone(),
    };

    // 3. Create a dirty tile at (64, 64) with size 64x64 that corresponds to
    // reference canvas shifted by dx = 0, dy = 12 (vertical scroll)
    let tile_x = 64u16;
    let tile_y = 64u16;
    let tile_w = 64u16;
    let tile_h = 64u16;
    let expected_dx = 0i16;
    let expected_dy = 12i16;

    let src_ref_x = (tile_x as i32 - expected_dx as i32) as usize;
    let src_ref_y = (tile_y as i32 - expected_dy as i32) as usize;
    let mut tile_raw = vec![0u8; (tile_w as usize) * (tile_h as usize) * 4];
    for r in 0..tile_h as usize {
        let src_off = (src_ref_y + r) * (canvas_w as usize) * 4 + src_ref_x * 4;
        let dst_off = r * (tile_w as usize) * 4;
        tile_raw[dst_off..dst_off + (tile_w as usize) * 4]
            .copy_from_slice(&canvas_raw[src_off..src_off + (tile_w as usize) * 4]);
    }

    // 4. Test estimate_tile_motion
    let motion = estimate_tile_motion(
        &tile_raw,
        tile_x,
        tile_y,
        tile_w,
        tile_h,
        &ref_canvas,
        32,
        0.90,
    );
    assert!(
        motion.is_some(),
        "Motion vector must be found for vertical shift"
    );
    let (est_dx, est_dy, ratio) = motion.unwrap();
    assert_eq!(est_dx, expected_dx);
    assert_eq!(est_dy, expected_dy);
    assert!(
        ratio >= 0.99,
        "Similarity ratio should be close to 1.0, got {}",
        ratio
    );

    // 5. Test compress_tile_motion with enabled config
    let mut active_cfg = ScreenTileMotionConfig {
        enabled: true,
        max_displacement: 32,
        min_match_ratio: 90,
    };
    let comp = compress_tile_motion(
        &tile_raw,
        tile_x,
        tile_y,
        tile_w,
        tile_h,
        &ref_canvas,
        &active_cfg,
    );
    assert!(comp.is_some());
    let (codec, payload) = comp.unwrap();
    assert_eq!(codec, TILE_SUBCODEC_MOTION);
    assert_eq!(payload.len(), 5);
    assert_eq!(payload[0], TILE_SUBCODEC_MOTION);
    assert_eq!(i16::from_be_bytes([payload[1], payload[2]]), expected_dx);
    assert_eq!(i16::from_be_bytes([payload[3], payload[4]]), expected_dy);

    // Test disabled config returns None
    active_cfg.enabled = false;
    assert!(compress_tile_motion(
        &tile_raw,
        tile_x,
        tile_y,
        tile_w,
        tile_h,
        &ref_canvas,
        &active_cfg,
    )
    .is_none());

    // 6. Test decompress_tile_motion
    let decomp = decompress_tile_motion(&payload, tile_x, tile_y, tile_w, tile_h, &ref_canvas)
        .expect("decompress tile motion");
    assert_eq!(decomp.len(), tile_raw.len());
    assert_eq!(
        decomp, tile_raw,
        "Decompressed pixels must match original dirty tile byte-for-byte"
    );

    // 7. Test horizontal shift (dx = -8, dy = 0)
    let h_dx = -8i16;
    let h_dy = 0i16;
    let src_h_x = (tile_x as i32 - h_dx as i32) as usize;
    let src_h_y = (tile_y as i32 - h_dy as i32) as usize;
    let mut h_tile_raw = vec![0u8; (tile_w as usize) * (tile_h as usize) * 4];
    for r in 0..tile_h as usize {
        let src_off = (src_h_y + r) * (canvas_w as usize) * 4 + src_h_x * 4;
        let dst_off = r * (tile_w as usize) * 4;
        h_tile_raw[dst_off..dst_off + (tile_w as usize) * 4]
            .copy_from_slice(&canvas_raw[src_off..src_off + (tile_w as usize) * 4]);
    }
    let h_motion = estimate_tile_motion(
        &h_tile_raw,
        tile_x,
        tile_y,
        tile_w,
        tile_h,
        &ref_canvas,
        32,
        0.90,
    )
    .expect("horizontal motion found");
    assert_eq!(h_motion.0, h_dx);
    assert_eq!(h_motion.1, h_dy);

    // 8. Test out-of-bounds displacement rejection in decompress_tile_motion
    let mut invalid_payload = vec![TILE_SUBCODEC_MOTION];
    invalid_payload.extend_from_slice(&(200i16).to_be_bytes()); // out of canvas
    invalid_payload.extend_from_slice(&(0i16).to_be_bytes());
    assert!(decompress_tile_motion(
        &invalid_payload,
        tile_x,
        tile_y,
        tile_w,
        tile_h,
        &ref_canvas,
    )
    .is_err());

    // 9. ControlMsg Tag 71~73 wire codecs
    // Tag 71: SetScreenTileMotionMode
    let msg_set = ControlMsg::SetScreenTileMotionMode {
        enabled: true,
        max_displacement: 64,
        min_match_ratio: 95,
    };
    let payload_set = msg_set.encode_payload();
    assert_eq!(payload_set.len(), 14);
    assert_eq!(payload_set[5], 71);
    let dec_set = ControlMsg::decode_payload(&payload_set).unwrap();
    assert_eq!(dec_set, msg_set);

    // Tag 72: ResetScreenTileMotionMode
    let msg_reset = ControlMsg::ResetScreenTileMotionMode;
    let payload_reset = msg_reset.encode_payload();
    assert_eq!(payload_reset.len(), 14);
    assert_eq!(payload_reset[5], 72);
    let dec_reset = ControlMsg::decode_payload(&payload_reset).unwrap();
    assert_eq!(dec_reset, msg_reset);

    // Tag 73: TileMotionSync
    let msg_sync = ControlMsg::TileMotionSync {
        motion_tiles: 8888,
        fallback_tiles: 2222,
        bytes_saved: 500000,
    };
    let payload_sync = msg_sync.encode_payload();
    assert_eq!(payload_sync.len(), 14);
    assert_eq!(payload_sync[5], 73);
    let dec_sync = ControlMsg::decode_payload(&payload_sync).unwrap();
    assert_eq!(
        dec_sync,
        ControlMsg::TileMotionSync {
            motion_tiles: 8888,
            fallback_tiles: 0,
            bytes_saved: 0,
        }
    );
}
