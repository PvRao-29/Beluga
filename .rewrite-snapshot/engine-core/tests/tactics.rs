//! Search-level correctness: mates, tactical shots, and SEE.

use beluga_core::position::Position;
use beluga_core::search::{Search, MATE_IN_MAX};
use beluga_core::see;
use beluga_core::timeman::Limits;
use beluga_core::tt::Tt;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

fn search(fen: &str, depth: u32) -> (String, i32) {
    beluga_core::attacks::init();
    let tt = Tt::new(16);
    let stop = Arc::new(AtomicBool::new(false));
    let mut pos = Position::from_fen(fen).expect("valid fen");
    let limits = Limits {
        depth: Some(depth),
        ..Default::default()
    };
    let mut s = Search::new(&mut pos, &tt, stop, limits);
    let m = s.think();
    (m.to_uci(), s.root_score())
}

#[test]
fn finds_mate_in_one() {
    let (mv, score) = search("6k1/5ppp/8/8/8/8/8/R5K1 w - - 0 1", 4);
    assert_eq!(mv, "a1a8", "should play the back-rank mate");
    assert!(score >= MATE_IN_MAX, "score should be a mate, got {score}");
}

#[test]
fn finds_forced_mate_kqk() {
    // King + Queen vs King is a forced mate; the engine must report a mate score.
    let (_mv, score) = search("8/8/8/4k3/8/8/3QK3/8 w - - 0 1", 14);
    assert!(
        score >= MATE_IN_MAX,
        "KQ vs K should be a forced win, got {score}"
    );
}

#[test]
fn wins_hanging_queen() {
    // Material is otherwise equal (both have a queen). Black's queen on d4 is en
    // prise to the e3 pawn, so exd4 wins a whole queen — and is strictly better
    // than the queen trade Qxd4.
    let (mv, score) = search("4k3/8/8/8/3q4/4P3/8/3QK3 w - - 0 1", 8);
    assert_eq!(mv, "e3d4", "should win the queen with the pawn, not trade");
    assert!(
        score > 700,
        "winning a queen should be clearly positive, got {score}"
    );
}

#[test]
fn see_free_pawn_is_positive() {
    let pos = Position::from_fen("4k3/8/8/3p4/4P3/8/8/4K3 w - - 0 1").unwrap();
    let m = pos.parse_uci_move("e4d5").unwrap();
    assert!(
        see::see(&pos, m) > 0,
        "capturing an undefended pawn should be SEE-positive"
    );
}

#[test]
fn see_defended_recapture() {
    // Pawn e4 takes d5, but d5 is defended by the c6 pawn: PxP, PxP is even (0).
    let pos = Position::from_fen("4k3/8/2p5/3p4/4P3/8/8/4K3 w - - 0 1").unwrap();
    let m = pos.parse_uci_move("e4d5").unwrap();
    assert_eq!(see::see(&pos, m), 0, "even pawn trade should be SEE 0");
}

#[test]
fn see_losing_capture_is_negative() {
    // Rook takes a pawn that is defended by another pawn: win 100, lose 500.
    let pos = Position::from_fen("4k3/8/8/2p5/3p4/3R4/8/4K3 w - - 0 1").unwrap();
    let m = pos.parse_uci_move("d3d4").unwrap();
    assert!(
        see::see(&pos, m) < 0,
        "RxP defended by pawn should be SEE-negative"
    );
}
