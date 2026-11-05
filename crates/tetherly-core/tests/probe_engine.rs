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
