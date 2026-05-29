//! Draw rules, stalemate/checkmate detection, and edge-case legality.

use beluga_core::chess_move::MoveList;
use beluga_core::movegen;
use beluga_core::position::Position;

fn legal_count(fen: &str) -> usize {
    let pos = Position::from_fen(fen).unwrap();
    let mut list = MoveList::new();
    movegen::generate_legal(&pos, &mut list);
    list.len()
}

#[test]
fn stalemate_has_no_moves_and_not_in_check() {
    // Classic stalemate: black to move, king a8 boxed in, not in check.
    let fen = "k7/8/1Q6/2K5/8/8/8/8 b - - 0 1";
    let pos = Position::from_fen(fen).unwrap();
    assert!(!pos.in_check());
    assert_eq!(legal_count(fen), 0, "should be stalemate");
}

#[test]
fn checkmate_has_no_moves_and_in_check() {
    // Fool's mate position after 1.f3 e5 2.g4 Qh4#.
    let fen = "rnb1kbnr/pppp1ppp/8/4p3/6Pq/5P2/PPPPP2P/RNBQKBNR w KQkq - 1 3";
    let pos = Position::from_fen(fen).unwrap();
    assert!(pos.in_check());
    assert_eq!(legal_count(fen), 0, "should be checkmate");
}

#[test]
fn insufficient_material_set() {
    assert!(Position::from_fen("8/8/4k3/8/8/4K3/8/8 w - - 0 1")
        .unwrap()
        .is_insufficient_material()); // KvK
    assert!(Position::from_fen("8/8/4k3/8/8/4K3/8/4N3 w - - 0 1")
        .unwrap()
        .is_insufficient_material()); // KNvK
    assert!(Position::from_fen("8/8/4k3/8/8/4K3/8/4B3 w - - 0 1")
        .unwrap()
        .is_insufficient_material()); // KBvK
    assert!(!Position::from_fen("8/8/4k3/8/8/4K3/8/3PN3 w - - 0 1")
        .unwrap()
        .is_insufficient_material()); // pawn present
    assert!(!Position::from_fen("8/8/4k3/8/8/4K3/8/3RN3 w - - 0 1")
        .unwrap()
        .is_insufficient_material()); // rook present
}

#[test]
fn fifty_move_detected_from_fen() {
    let pos = Position::from_fen("8/8/4k3/8/8/4K3/8/3RN3 w - - 100 80").unwrap();
    assert!(pos.is_fifty_move());
    let pos2 = Position::from_fen("8/8/4k3/8/8/4K3/8/3RN3 w - - 99 80").unwrap();
    assert!(!pos2.is_fifty_move());
}

#[test]
fn repetition_detected() {
    let mut pos = Position::startpos();
    let seq = [
        "g1f3", "g8f6", "f3g1", "f6g8", "g1f3", "g8f6", "f3g1", "f6g8",
    ];
    for (i, mv) in seq.iter().enumerate() {
        let m = pos.parse_uci_move(mv).unwrap();
        pos.make_move(m);
        if i == seq.len() - 1 {
            // After the second full cycle the start position has recurred.
            assert!(pos.is_repetition(), "threefold/repetition not detected");
        }
    }
}

#[test]
fn castling_through_check_is_illegal() {
    // White can castle king-side normally...
    let fen_ok = "r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1";
    let pos = Position::from_fen(fen_ok).unwrap();
    let mut list = MoveList::new();
    movegen::generate_legal(&pos, &mut list);
    assert!(list.as_slice().iter().any(|m| m.to_uci() == "e1g1"));

    // ...but not when f1 is attacked by a rook on f8 (king passes through check).
    let fen_bad = "r4rk1/8/8/8/8/8/8/R3K2R w KQ - 0 1";
    let pos = Position::from_fen(fen_bad).unwrap();
    let mut list = MoveList::new();
    movegen::generate_legal(&pos, &mut list);
    assert!(
        !list.as_slice().iter().any(|m| m.to_uci() == "e1g1"),
        "king-side castling through f1 attack must be illegal"
    );
}
