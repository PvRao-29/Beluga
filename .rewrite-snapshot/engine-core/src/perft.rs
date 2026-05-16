//! Perft: exhaustive legal-move node counting for correctness verification.

use crate::chess_move::{Move, MoveList};
use crate::movegen;
use crate::position::Position;

/// Count leaf nodes at the given depth.
pub fn perft(pos: &mut Position, depth: u32) -> u64 {
    if depth == 0 {
        return 1;
    }
    let mut list = MoveList::new();
    movegen::generate_legal(pos, &mut list);

    // Bulk counting: at depth 1 the number of legal moves *is* the node count.
    if depth == 1 {
        return list.len() as u64;
    }

    let mut nodes = 0u64;
    for &m in list.as_slice() {
        pos.make_move(m);
        nodes += perft(pos, depth - 1);
        pos.unmake_move(m);
    }
    nodes
}

/// Perft without bulk counting (every node visited). Useful for stress-testing
/// make/unmake at depth 1.
pub fn perft_full(pos: &mut Position, depth: u32) -> u64 {
    if depth == 0 {
        return 1;
    }
    let mut list = MoveList::new();
    movegen::generate_legal(pos, &mut list);
    let mut nodes = 0u64;
    for &m in list.as_slice() {
        pos.make_move(m);
        nodes += perft_full(pos, depth - 1);
        pos.unmake_move(m);
    }
    nodes
}

/// "Divide" perft: per-root-move node counts, for localizing bugs.
pub fn perft_divide(pos: &mut Position, depth: u32) -> Vec<(Move, u64)> {
    let mut list = MoveList::new();
    movegen::generate_legal(pos, &mut list);
    let mut out = Vec::with_capacity(list.len());
    for &m in list.as_slice() {
        pos.make_move(m);
        let n = if depth <= 1 { 1 } else { perft(pos, depth - 1) };
        pos.unmake_move(m);
        out.push((m, n));
    }
    out
}
