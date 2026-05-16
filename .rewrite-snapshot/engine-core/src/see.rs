//! Static Exchange Evaluation (SEE) via the swap-list algorithm.
//!
//! X-ray attackers are handled because `attackers_to` is recomputed with the
//! shrinking occupancy each iteration. The king is modelled as an attacker with
//! a very large value, which naturally prevents SEE from "winning" exchanges by
//! capturing into a defended square with the king.

use crate::bitboard::Bitboard;
use crate::chess_move::Move;
use crate::position::Position;
use crate::types::{Color, PieceType};

/// Piece values used for exchange arithmetic (independent of the eval PSTs).
pub const SEE_VALUE: [i32; 6] = [100, 320, 330, 500, 900, 20000];

#[inline]
fn value_on(pos: &Position, sq: crate::types::Square) -> i32 {
    match pos.piece_on(sq) {
        Some(p) => SEE_VALUE[p.piece_type().index()],
        None => 0,
    }
}

/// Least-valuable attacker of `to` for `side`, restricted to pieces still in
/// `occ`. Returns the attacker's square and value.
fn least_valuable_attacker(
    pos: &Position,
    to: crate::types::Square,
    side: Color,
    occ: Bitboard,
    attackers: Bitboard,
) -> Option<(crate::types::Square, i32)> {
    for pt in PieceType::ALL {
        let bb = attackers & pos.pieces(side, pt) & occ;
        if bb.any() {
            return Some((bb.lsb(), SEE_VALUE[pt.index()]));
        }
    }
    let _ = to;
    None
}

/// Static exchange value of `mv` (positive = good for the mover).
pub fn see(pos: &Position, mv: Move) -> i32 {
    let from = mv.from();
    let to = mv.to();
    let us = pos.side_to_move();

    let mut occ = pos.occupied();
    let mut gain = [0i32; 32];

    // Initial captured value.
    let captured = if mv.is_en_passant() {
        SEE_VALUE[PieceType::Pawn.index()]
    } else {
        value_on(pos, to)
    };
    gain[0] = captured;

    // Value of the piece making the first capture (promotions land as the
    // promoted piece for recapture purposes).
    let mut attacker_val = if let Some(promo) = mv.promotion() {
        SEE_VALUE[promo.index()]
    } else {
        value_on(pos, from)
    };

    occ.clear(from);
    if mv.is_en_passant() {
        let cap_sq = crate::types::Square(if us == Color::White {
            to.0 - 8
        } else {
            to.0 + 8
        });
        occ.clear(cap_sq);
    }

    let mut side = us.flip();
    let mut depth = 0usize;

    loop {
        depth += 1;
        if depth >= gain.len() {
            break;
        }
        gain[depth] = attacker_val - gain[depth - 1];

        let attackers = pos.attackers_to(to, occ) & occ;
        let side_attackers = attackers & pos.color_pieces(side);
        if side_attackers.is_empty() {
            break;
        }
        let (sq, val) = match least_valuable_attacker(pos, to, side, occ, attackers) {
            Some(x) => x,
            None => break,
        };
        attacker_val = val;
        occ.clear(sq);
        side = side.flip();

        // If the side just played a king capture and the opponent still has
        // attackers, the king move was illegal in this exchange — stop.
        if attacker_val == SEE_VALUE[PieceType::King.index()] {
            // Allow one more level only if no further recapture exists.
            let next = pos.attackers_to(to, occ) & occ & pos.color_pieces(side);
            if next.any() {
                // King would be captured; this capture is not actually possible.
                depth -= 1;
                break;
            }
        }
    }

    // Negamax the swap list back to the root.
    while depth > 1 {
        depth -= 1;
        gain[depth - 1] = -(-gain[depth - 1]).max(gain[depth]);
    }
    gain[0]
}

/// True if the static exchange value of `mv` is at least `threshold`.
#[inline]
pub fn see_ge(pos: &Position, mv: Move, threshold: i32) -> bool {
    see(pos, mv) >= threshold
}
