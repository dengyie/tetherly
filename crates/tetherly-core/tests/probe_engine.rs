// Probe: simulate the production crossing flow for Mac top edge -> Windows
use tetherly_core::input::InputServer;
use tetherly_core::{CursorSeat, InputKind, ScreenEdge, SCREEN_UNITS};

#[test]
fn probe_top_edge_crossing_entry_x() {
    // Mac: 1512x982 (points), engine sized as spawn_input_capture does
    let mut srv = InputServer::new(1512, 982);
    srv.set_edge(ScreenEdge::Top);

    let x = 300;
    let mut y = 500;
    let mut entered = None;
    for _ in 0..200 {
        y -= 3;
        let edge = srv.local_move_delta(x, y, 0, -3);
        if let Some(e) = edge {
            assert_eq!(e, ScreenEdge::Top);
            entered = Some(srv.drain());
            break;
        }
        srv.drain();
    }
    let evs = entered.expect("should cross");
    let mut saw_enter = false;
    for ev in &evs {
        match &ev.kind {
            InputKind::Enter => saw_enter = true,
            InputKind::Move { x, y } if saw_enter => {
                println!("entry Move: x={x} y={y} (expect x≈1985, y=SCREEN_UNITS)");
                assert_eq!(*y, SCREEN_UNITS, "entry y must be bottom");
                assert!(
                    (*x - 1985).abs() <= 10,
                    "entry x must be proportional, got {x}"
                );
                return;
            }
            _ => {}
        }
    }
    panic!("no Move after Enter in {evs:?}");
}

// Probe: a full Mac->Win->Mac round trip must be position-stable. The
// controller engine (norm/denorm) and the target's pixel mapping must not
// shear in one direction, else every hop drifts a pixel and repeated
// crossings walk visibly — the reported production bug. The out and back
// hops run the REAL engine: a Mac InputServer crossing Top, then a Win
// InputServer crossing Bottom (its local_move_delta Remote arm applies the
// same norm/denorm the controller side uses for landing).
#[test]
fn roundtrip_is_position_stable() {
    let mac_w = 1512i32;
    let mac_h = 982i32;
    let win_w = 1920i32;
    let win_h = 1080i32;

    // Target-side pixel mapping (SystemInputSink): rounded units -> pixels.
    fn sink_px(rem: i32, size: i32) -> i32 {
        ((rem.clamp(0, SCREEN_UNITS) as i64 * (size as i64 - 1) + SCREEN_UNITS as i64 / 2)
            / SCREEN_UNITS as i64) as i32
    }

    let mut worst = 0i32;
    for x in [1, 100, 300, 455, 700, 756, 1200, 1510] {
        // Mac controller: cross the Top edge at x.
        let mut mac = InputServer::new(mac_w, mac_h);
        mac.set_edge(ScreenEdge::Top);
        let edge = mac.local_move_delta(x, 0, 0, -6);
        assert_eq!(edge, Some(ScreenEdge::Top));
        let entry_move = mac
            .drain()
            .into_iter()
            .find_map(|ev| match ev.kind {
                InputKind::Move { x, y } => Some((x, y)),
                _ => None,
            })
            .expect("Enter+Move pushed on crossing");

        // Win target: cursor lands at entry_move, then user pushes back
        // through Win's Bottom edge (the shared edge back to Mac).
        let win_px = sink_px(entry_move.0, win_w);
        let mut win = InputServer::new(win_w, win_h);
        win.set_edge(ScreenEdge::Bottom);
        let back = win.local_move_delta(win_px, win_h, 0, 6);
        assert_eq!(back, Some(ScreenEdge::Bottom));
        let return_move = win
            .drain()
            .into_iter()
            .find_map(|ev| match ev.kind {
                InputKind::Move { x, y } => Some((x, y)),
                _ => None,
            })
            .expect("Enter+Move pushed on return crossing");

        // Mac target: cursor lands on Mac at the mapped position.
        let mac_px = sink_px(return_move.0, mac_w);
        worst = worst.max((x - mac_px).abs());
        println!(
            "x={x} entry_rem={} win_px={win_px} back_rem={} mac_px={mac_px}",
            entry_move.0, return_move.0
        );
    }
    assert!(
        worst <= 1,
        "round trip must not drift more than 1px; worst={worst}"
    );
}

// Regression: a takeover demotes this node mid-roam and the engine seat
// stays Remote. A later re-attach used to find that stale seat, replay the
// old rem via resume_remote, and permanently block the fresh edge-crossing
// arm (it needs seat Local) — the cursor then drifted from the stale spot
// instead of entering proportionally. clear_stale_remote must drop the seat
// WITHOUT moving the hook position, so the next push crosses fresh.
#[test]
fn clear_stale_remote_resets_seat_but_keeps_position() {
    let mut srv = InputServer::new(1512, 982);
    srv.set_edge(ScreenEdge::Top);

    // Enter remote and roam somewhere: rem_x tracks hook x (777).
    assert_eq!(srv.local_move_delta(777, 0, 0, -6), Some(ScreenEdge::Top));
    let _ = srv.drain();
    assert_eq!(srv.seat(), CursorSeat::Remote);
    srv.local_move_delta(777, 30, 0, 30);
    let _ = srv.drain();

    // Simulate the demote-then-reattach: stale seat must clear.
    srv.clear_stale_remote();
    assert_eq!(srv.seat(), CursorSeat::Local);
    assert_eq!(srv.local_pos().0, 777, "hook position must be preserved");

    // The next physical push must fire a FRESH proportional crossing, not
    // an incremental move from the stale rem.
    assert_eq!(srv.local_move_delta(400, 0, 0, -6), Some(ScreenEdge::Top));
    let evs = srv.drain();
    let mv = evs
        .iter()
        .find_map(|ev| match ev.kind {
            InputKind::Move { x, y } => Some((x, y)),
            _ => None,
        })
        .expect("fresh Enter+Move after stale-seat clear");
    // norm(400) with rounding = round(400*10000/1511) = 2647 — proportional
    // to the NEW position, not a delta from the old roam.
    assert_eq!(mv.0, 2647, "entry rem must be proportional to fresh x");
    assert_eq!(mv.1, SCREEN_UNITS);
}

// Root-fix E regression: a stale remote-branch Move lingering in the
// pending queue must NOT ride along with the next crossing batch. The
// crossing arms clear pending first, so the batch is exactly
// [Enter, Move{fresh rem}] — a stale Move after the Enter used to
// re-place the cursor at an old position (production: snap to center).
#[test]
fn crossing_batch_carries_no_stale_frames() {
    let mut srv = InputServer::new(1470, 956);
    srv.set_edge(ScreenEdge::Top);

    // First crossing: enter remote and roam to the middle-bottom.
    assert_eq!(srv.local_move_delta(735, 0, 0, -6), Some(ScreenEdge::Top));
    let _ = srv.drain();
    // Roam up into the interior (rem_y drops below the hysteresis) and
    // sideways so the remote branch pushes Move frames.
    srv.local_move_delta(735, 40, 0, -40);
    srv.local_move_delta(400, 40, -30, 0);
    let _ = srv.drain();

    // Simulate the return (Leave batch): push back down through the entry
    // edge (rem_y returns to SCREEN_UNITS with dy > 4).
    assert_eq!(srv.local_move_delta(400, 40, 0, 60), Some(ScreenEdge::Top));
    let leave_batch = srv.drain();
    assert!(leave_batch.iter().any(|ev| ev.kind == InputKind::Leave));

    // A stale remote-branch Move lands in pending (race shape).
    srv.local_move_delta(735, 30, 0, 30);
    let _ = srv.drain();
    assert_eq!(srv.local_move_delta(300, 0, 0, -6), Some(ScreenEdge::Top));
    let batch = srv.drain();

    // The crossing batch must be exactly [Enter, Move{fresh rem}]: no stale
    // frame may appear after the Enter.
    let enter_idx = batch
        .iter()
        .position(|ev| ev.kind == InputKind::Enter)
        .expect("Enter present");
    assert_eq!(batch.len(), enter_idx + 2, "batch = [Enter, Move] exactly");
    match &batch[enter_idx + 1].kind {
        InputKind::Move { x, y } => {
            assert_eq!(*y, SCREEN_UNITS, "entry at bottom edge");
            assert_eq!(*x, 2042, "fresh proportional rem (norm(300) on 1470)");
        }
        other => panic!("expected Move, got {other:?}"),
    }
}

// v0.2.11 regression: the far-edge (shared-edge) return used to require raw
// |dy| > 4 within a SINGLE event. At real mouse report rates (125-1000 Hz,
// 1-3 px per event) a deliberate sustained push never fires that gate —
// production 2026-11-11: the user pushed the remote cursor at the shared
// edge repeatedly and BOTH nodes were stranded in Remote seat (the Leave
// frame never shipped, the target's MOUSE_RELAY stayed on, its physical
// mouse stayed dead). The return must respond to ACCUMULATED push at the
// shared edge instead of per-event flicks.

fn enter_remote_via_top(srv: &mut InputServer) {
    let x = 300;
    let mut y = 500;
    for _ in 0..200 {
        y -= 4;
        if srv.local_move_delta(x, y, 0, -4).is_some() {
            srv.drain();
            assert_eq!(srv.seat(), CursorSeat::Remote);
            return;
        }
        srv.drain();
    }
    panic!("never crossed into remote seat");
}

fn roam_into_interior(srv: &mut InputServer) {
    // 20 events of dy=-8 drop rem_y far below the 20-unit entry hysteresis.
    for _ in 0..20 {
        srv.local_move_delta(300, 500, 0, -8);
        srv.drain();
    }
    assert_eq!(srv.seat(), CursorSeat::Remote, "roam must stay remote");
}

#[test]
fn sustained_slow_push_at_shared_edge_returns_to_local() {
    let mut srv = InputServer::new(1512, 982);
    srv.set_edge(ScreenEdge::Top);
    enter_remote_via_top(&mut srv);
    roam_into_interior(&mut srv);

    // Deliberate sustained push back at the shared edge: 2 px per event at
    // ~500 Hz (a real report rate). Must return well before 120 events.
    let mut returned = false;
    for _ in 0..120 {
        if srv.local_move_delta(300, 600, 0, 2).is_some() {
            returned = true;
            break;
        }
        srv.drain();
    }
    assert!(
        returned,
        "sustained 2px/event push at the shared edge must switch back to local"
    );
    assert_eq!(srv.seat(), CursorSeat::Local);
    let evs = srv.drain();
    assert!(
        evs.iter().any(|ev| matches!(ev.kind, InputKind::Leave)),
        "return must emit a Leave frame for the target to disarm"
    );
}

#[test]
fn single_flick_at_shared_edge_still_returns() {
    let mut srv = InputServer::new(1512, 982);
    srv.set_edge(ScreenEdge::Top);
    enter_remote_via_top(&mut srv);
    roam_into_interior(&mut srv);

    let mut returned = false;
    for _ in 0..90 {
        if srv.local_move_delta(300, 600, 0, 30).is_some() {
            returned = true;
            break;
        }
        srv.drain();
    }
    assert!(
        returned,
        "a hard flick at the shared edge must still return"
    );
    assert_eq!(srv.seat(), CursorSeat::Local);
}

/// Push +2px per event until the remote cursor sits saturated on the shared
/// edge (rem_y == SCREEN_UNITS). Events before saturation never accumulate.
fn push_until_shared_edge(srv: &mut InputServer) {
    for _ in 0..200 {
        if srv.remote_pos().1 >= SCREEN_UNITS {
            return;
        }
        srv.local_move_delta(300, 600, 0, 2);
        srv.drain();
    }
    panic!("never reached the shared edge");
}

#[test]
fn shared_edge_push_accum_resets_on_reverse_direction() {
    let mut srv = InputServer::new(1512, 982);
    srv.set_edge(ScreenEdge::Top);
    enter_remote_via_top(&mut srv);
    roam_into_interior(&mut srv);

    push_until_shared_edge(&mut srv);
    // Build 10px of accumulated push at the edge (reaching event + 4 more).
    for i in 0..4 {
        let fired = srv.local_move_delta(300, 600, 0, 2).is_some();
        assert!(!fired, "10px must stay under threshold (event {i})");
        srv.drain();
    }
    // One reverse event must discard the accumulation, so the fresh 6px
    // that follow stay under the 12px threshold.
    srv.local_move_delta(300, 600, 0, -4);
    srv.drain();
    for i in 0..3 {
        let fired = srv.local_move_delta(300, 600, 0, 2).is_some();
        assert!(
            !fired,
            "reverse direction must reset the push accumulator (event {i})"
        );
        srv.drain();
    }
    // Continuing the push still returns once it re-accumulates.
    let mut returned = false;
    for _ in 0..10 {
        if srv.local_move_delta(300, 600, 0, 2).is_some() {
            returned = true;
            break;
        }
        srv.drain();
    }
    assert!(returned, "fresh sustained push after reverse must return");
}

#[test]
fn shared_edge_push_window_expires() {
    let mut srv = InputServer::new(1512, 982);
    srv.set_edge(ScreenEdge::Top);
    enter_remote_via_top(&mut srv);
    roam_into_interior(&mut srv);

    push_until_shared_edge(&mut srv);
    // Accumulate 10px at the edge (stays under the 12px threshold).
    for i in 0..4 {
        let fired = srv.local_move_delta(300, 600, 0, 2).is_some();
        assert!(!fired, "10px must stay under threshold (event {i})");
        srv.drain();
    }
    // Let the window lapse: the stale accumulation must not combine with a
    // fresh push (10px + 6px would fire; fresh 6px must not).
    std::thread::sleep(std::time::Duration::from_millis(650));
    for i in 0..3 {
        let fired = srv.local_move_delta(300, 600, 0, 2).is_some();
        assert!(!fired, "stale push accumulation must expire (event {i})");
        srv.drain();
    }
    // A continuous fresh push still returns.
    let mut returned = false;
    for _ in 0..10 {
        if srv.local_move_delta(300, 600, 0, 2).is_some() {
            returned = true;
            break;
        }
        srv.drain();
    }
    assert!(returned, "fresh sustained push after expiry must return");
}

#[test]
fn button_held_blocks_shared_edge_return() {
    let mut srv = InputServer::new(1512, 982);
    srv.set_edge(ScreenEdge::Top);
    enter_remote_via_top(&mut srv);
    roam_into_interior(&mut srv);

    srv.note_button(1, true);
    for i in 0..120 {
        let fired = srv.local_move_delta(300, 600, 0, 2).is_some();
        assert!(
            !fired,
            "holding a button must block the return (drag protection, event {i})"
        );
        srv.drain();
    }
    srv.note_button(1, false);
    let mut returned = false;
    for _ in 0..20 {
        if srv.local_move_delta(300, 600, 0, 2).is_some() {
            returned = true;
            break;
        }
        srv.drain();
    }
    assert!(returned, "release must re-arm the return");
}
