// Probe: simulate the production crossing flow for Mac top edge -> Windows
use tetherly_core::input::InputServer;
use tetherly_core::{InputKind, ScreenEdge, SCREEN_UNITS};

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
                println!("entry Move: x={x} y={y} (expect x≈198, y=1000)");
                assert_eq!(*y, SCREEN_UNITS, "entry y must be bottom");
                assert!(
                    (*x - 198).abs() <= 1,
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
