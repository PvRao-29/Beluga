//! Legal move generation using check and pin masks.
//!
//! The generator produces *fully legal* moves directly (no generate-then-filter
//! for the common case). The single per-move legality test is reserved for en
//! passant, which is the one move whose legality cannot be expressed with simple
//! masks (pinned EP / discovered check across the two vacated squares).

use crate::attacks;
use crate::bitboard::Bitboard;
use crate::chess_move::{Move, MoveFlag, MoveList};
use crate::position::Position;
use crate::types::{Color, PieceType, Square};

const F_QUIET: u16 = MoveFlag::Quiet as u16;
const F_DOUBLE: u16 = MoveFlag::DoublePush as u16;
const F_CAPTURE: u16 = MoveFlag::Capture as u16;
const F_EP: u16 = MoveFlag::EnPassant as u16;
const F_KCASTLE: u16 = MoveFlag::KingCastle as u16;
const F_QCASTLE: u16 = MoveFlag::QueenCastle as u16;
const F_PROMO_QUIET: u16 = MoveFlag::PromoKnight as u16; // base 8 (N,B,R,Q = +0..+3)
const F_PROMO_CAP: u16 = MoveFlag::PromoKnightCapture as u16; // base 12

/// Generate all legal moves.
pub fn generate_legal(pos: &Position, list: &mut MoveList) {
    generate(pos, list, true);
}

/// Generate noisy moves (captures, en passant, promotions). Assumes the side to
/// move is **not** in check (quiescence handles in-check positions with the full
/// generator).
pub fn generate_captures(pos: &Position, list: &mut MoveList) {
    debug_assert!(!pos.in_check());
    generate(pos, list, false);
}

fn generate(pos: &Position, list: &mut MoveList, quiets: bool) {
    let us = pos.side_to_move();
    let them = us.flip();
    let occ = pos.occupied();
    let our = pos.color_pieces(us);
    let enemies = pos.color_pieces(them);
    let ksq = pos.king_square(us);

    let checkers = pos.checkers();
    let num_checkers = checkers.count();

    // Target squares for non-king pieces:
    //  - not in check: every square not occupied by our own pieces (quiets+caps)
    //    or only enemies when `quiets == false`.
    //  - single check: capture the checker or block the ray.
    let targets = if num_checkers >= 1 {
        let checker_sq = checkers.lsb();
        attacks::between(ksq, checker_sq) | checkers
    } else if quiets {
        !our
    } else {
        enemies
    };

    if num_checkers < 2 {
        let pinned = compute_pinned(pos, us, ksq);
        gen_pawns(
            pos,
            list,
            us,
            them,
            ksq,
            occ,
            enemies,
            pinned,
            targets,
            quiets,
            num_checkers,
        );
        gen_knight_slider(pos, list, us, ksq, occ, enemies, pinned, targets);
        if quiets && num_checkers == 0 {
            gen_castling(pos, list, us, them, occ);
        }
    }

    gen_king(pos, list, them, ksq, occ, our, enemies, quiets);
}

/// Bitboard of our pieces pinned to our king.
fn compute_pinned(pos: &Position, us: Color, ksq: Square) -> Bitboard {
    let them = us.flip();
    let our = pos.color_pieces(us);
    let enemy_rq = pos.pieces(them, PieceType::Rook) | pos.pieces(them, PieceType::Queen);
    let enemy_bq = pos.pieces(them, PieceType::Bishop) | pos.pieces(them, PieceType::Queen);
    let snipers = (attacks::rook_attacks(ksq, Bitboard::EMPTY) & enemy_rq)
        | (attacks::bishop_attacks(ksq, Bitboard::EMPTY) & enemy_bq);

    let mut pinned = Bitboard::EMPTY;
    for sniper in snipers {
        let between = attacks::between(ksq, sniper) & pos.occupied();
        if between.count() == 1 && (between & our).any() {
            pinned |= between;
        }
    }
    pinned
}

#[inline]
fn push_piece_moves(list: &mut MoveList, from: Square, mut moves: Bitboard, enemies: Bitboard) {
    while moves.any() {
        let to = moves.pop_lsb();
        let flag = if enemies.contains(to) {
            F_CAPTURE
        } else {
            F_QUIET
        };
        list.push(Move::from_raw_flag(from, to, flag));
    }
}

#[allow(clippy::too_many_arguments)]
fn gen_knight_slider(
    pos: &Position,
    list: &mut MoveList,
    us: Color,
    ksq: Square,
    occ: Bitboard,
    enemies: Bitboard,
    pinned: Bitboard,
    targets: Bitboard,
) {
    // Knights (a pinned knight can never move legally).
    let mut knights = pos.pieces(us, PieceType::Knight) & !pinned;
    while knights.any() {
        let from = knights.pop_lsb();
        let moves = attacks::knight_attacks(from) & targets;
        push_piece_moves(list, from, moves, enemies);
    }

    // Bishops + queens (diagonal component).
    let mut diag = pos.pieces(us, PieceType::Bishop) | pos.pieces(us, PieceType::Queen);
    while diag.any() {
        let from = diag.pop_lsb();
        let mut moves = attacks::bishop_attacks(from, occ) & targets;
        if pinned.contains(from) {
            moves &= attacks::line(ksq, from);
        }
        push_piece_moves(list, from, moves, enemies);
    }

    // Rooks + queens (straight component).
    let mut straight = pos.pieces(us, PieceType::Rook) | pos.pieces(us, PieceType::Queen);
    while straight.any() {
        let from = straight.pop_lsb();
        let mut moves = attacks::rook_attacks(from, occ) & targets;
        if pinned.contains(from) {
            moves &= attacks::line(ksq, from);
        }
        push_piece_moves(list, from, moves, enemies);
    }
}

#[allow(clippy::too_many_arguments)]
fn gen_king(
    pos: &Position,
    list: &mut MoveList,
    them: Color,
    ksq: Square,
    occ: Bitboard,
    our: Bitboard,
    enemies: Bitboard,
    quiets: bool,
) {
    // Remove the king from occupancy so sliders x-ray through its square.
    let occ_no_king = occ ^ Bitboard::from_square(ksq);
    let mut moves = attacks::king_attacks(ksq) & !our;
    if !quiets {
        moves &= enemies;
    }
    while moves.any() {
        let to = moves.pop_lsb();
        if !pos.is_attacked(to, them, occ_no_king) {
            let flag = if enemies.contains(to) {
                F_CAPTURE
            } else {
                F_QUIET
            };
            list.push(Move::from_raw_flag(ksq, to, flag));
        }
    }
}

fn gen_castling(pos: &Position, list: &mut MoveList, us: Color, them: Color, occ: Bitboard) {
    let rights = pos.castling_rights();
    if us == Color::White {
        // King side: squares f1,g1 empty; e1,f1,g1 not attacked.
        if rights.has(crate::types::CastlingRights::WHITE_KING)
            && (occ & Bitboard(0x60)).is_empty()
            && !pos.is_attacked(Square(4), them, occ)
            && !pos.is_attacked(Square(5), them, occ)
            && !pos.is_attacked(Square(6), them, occ)
        {
            list.push(Move::from_raw_flag(Square(4), Square(6), F_KCASTLE));
        }
        // Queen side: squares b1,c1,d1 empty; e1,d1,c1 not attacked.
        if rights.has(crate::types::CastlingRights::WHITE_QUEEN)
            && (occ & Bitboard(0x0E)).is_empty()
            && !pos.is_attacked(Square(4), them, occ)
            && !pos.is_attacked(Square(3), them, occ)
            && !pos.is_attacked(Square(2), them, occ)
        {
            list.push(Move::from_raw_flag(Square(4), Square(2), F_QCASTLE));
        }
    } else {
        if rights.has(crate::types::CastlingRights::BLACK_KING)
            && (occ & Bitboard(0x6000_0000_0000_0000)).is_empty()
            && !pos.is_attacked(Square(60), them, occ)
            && !pos.is_attacked(Square(61), them, occ)
            && !pos.is_attacked(Square(62), them, occ)
        {
            list.push(Move::from_raw_flag(Square(60), Square(62), F_KCASTLE));
        }
        if rights.has(crate::types::CastlingRights::BLACK_QUEEN)
            && (occ & Bitboard(0x0E00_0000_0000_0000)).is_empty()
            && !pos.is_attacked(Square(60), them, occ)
            && !pos.is_attacked(Square(59), them, occ)
            && !pos.is_attacked(Square(58), them, occ)
        {
            list.push(Move::from_raw_flag(Square(60), Square(58), F_QCASTLE));
        }
    }
}

#[inline]
fn add_promotions(list: &mut MoveList, from: Square, to: Square, capture: bool) {
    let base = if capture { F_PROMO_CAP } else { F_PROMO_QUIET };
    // Queen, Rook, Bishop, Knight (low 2 bits: N=0,B=1,R=2,Q=3).
    list.push(Move::from_raw_flag(from, to, base + 3)); // queen
    list.push(Move::from_raw_flag(from, to, base + 2)); // rook
    list.push(Move::from_raw_flag(from, to, base + 1)); // bishop
    list.push(Move::from_raw_flag(from, to, base)); // knight
}

#[allow(clippy::too_many_arguments)]
fn gen_pawns(
    pos: &Position,
    list: &mut MoveList,
    us: Color,
    them: Color,
    ksq: Square,
    occ: Bitboard,
    enemies: Bitboard,
    pinned: Bitboard,
    targets: Bitboard,
    quiets: bool,
    num_checkers: u32,
) {
    let mut pawns = pos.pieces(us, PieceType::Pawn);
    let up: i32 = if us == Color::White { 8 } else { -8 };
    let start_rank = if us == Color::White { 1 } else { 6 };
    let promo_rank = if us == Color::White { 6 } else { 1 };
    let empties = !occ;

    while pawns.any() {
        let from = pawns.pop_lsb();
        let pinray = if pinned.contains(from) {
            attacks::line(ksq, from)
        } else {
            Bitboard::FULL
        };
        let on_promo_rank = from.rank() == promo_rank;

        // Pushes (quiet, or promotions even in capture-gen because they are noisy).
        // The intermediate square only needs to be empty for the pawn to pass
        // through; the destination must satisfy the check/pin masks. The double
        // push is considered independently of whether the single push is a legal
        // target (e.g. a double push can block a check the single push cannot).
        let one = Square((from.0 as i32 + up) as u8);
        if empties.contains(one) {
            if on_promo_rank {
                if targets.contains(one) && pinray.contains(one) {
                    add_promotions(list, from, one, false);
                }
            } else if quiets {
                if targets.contains(one) && pinray.contains(one) {
                    list.push(Move::from_raw_flag(from, one, F_QUIET));
                }
                if from.rank() == start_rank {
                    let two = Square((from.0 as i32 + 2 * up) as u8);
                    if empties.contains(two) && targets.contains(two) && pinray.contains(two) {
                        list.push(Move::from_raw_flag(from, two, F_DOUBLE));
                    }
                }
            }
        }

        // Captures.
        let mut caps = attacks::pawn_attacks(us, from) & enemies & targets & pinray;
        while caps.any() {
            let to = caps.pop_lsb();
            if on_promo_rank {
                add_promotions(list, from, to, true);
            } else {
                list.push(Move::from_raw_flag(from, to, F_CAPTURE));
            }
        }

        // En passant — full simulated legality test (handles pinned/discovered).
        if let Some(ep) = pos.ep_square() {
            if (attacks::pawn_attacks(us, from) & Bitboard::from_square(ep)).any()
                && ep_is_legal(pos, from, ep, us, them, ksq, occ, num_checkers, targets)
            {
                list.push(Move::from_raw_flag(from, ep, F_EP));
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn ep_is_legal(
    pos: &Position,
    from: Square,
    ep: Square,
    us: Color,
    them: Color,
    ksq: Square,
    occ: Bitboard,
    num_checkers: u32,
    targets: Bitboard,
) -> bool {
    let cap_sq = Square(if us == Color::White {
        ep.0 - 8
    } else {
        ep.0 + 8
    });

    // If in check, EP is only relevant if it captures the checker (cap_sq in
    // targets) or blocks (ep square in targets). The full king-safety test below
    // still guards correctness, but this prunes obviously-irrelevant EP in check.
    if num_checkers == 1 && !targets.contains(cap_sq) && !targets.contains(ep) {
        return false;
    }

    let new_occ = (occ ^ Bitboard::from_square(from) ^ Bitboard::from_square(cap_sq))
        | Bitboard::from_square(ep);

    let enemy_bq = pos.pieces(them, PieceType::Bishop) | pos.pieces(them, PieceType::Queen);
    let enemy_rq = pos.pieces(them, PieceType::Rook) | pos.pieces(them, PieceType::Queen);
    if (attacks::bishop_attacks(ksq, new_occ) & enemy_bq).any() {
        return false;
    }
    if (attacks::rook_attacks(ksq, new_occ) & enemy_rq).any() {
        return false;
    }
    // Remaining attacker types (captured pawn already removed from consideration).
    let enemy_pawns = pos.pieces(them, PieceType::Pawn) & !Bitboard::from_square(cap_sq);
    if (attacks::pawn_attacks(us, ksq) & enemy_pawns).any() {
        return false;
    }
    if (attacks::knight_attacks(ksq) & pos.pieces(them, PieceType::Knight)).any() {
        return false;
    }
    if (attacks::king_attacks(ksq) & pos.pieces(them, PieceType::King)).any() {
        return false;
    }
    true
}

/// Count legal moves without materializing them (used by perft leaf optimization
/// and tests).
pub fn legal_move_count(pos: &Position) -> usize {
    let mut list = MoveList::new();
    generate_legal(pos, &mut list);
    list.len()
}
