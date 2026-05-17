//! Packed 16-bit move encoding.
//!
//! Layout: bits `0..6` = from square, `6..12` = to square, `12..16` = flags.
//! The flag values follow the common chessprogramming convention so that the
//! high bit of the flag marks captures and the next bit marks promotions.

use crate::types::{PieceType, Square};
use std::fmt;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Move(pub u16);

/// Move flag values (4 bits).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u16)]
pub enum MoveFlag {
    Quiet = 0,
    DoublePush = 1,
    KingCastle = 2,
    QueenCastle = 3,
    Capture = 4,
    EnPassant = 5,
    PromoKnight = 8,
    PromoBishop = 9,
    PromoRook = 10,
    PromoQueen = 11,
    PromoKnightCapture = 12,
    PromoBishopCapture = 13,
    PromoRookCapture = 14,
    PromoQueenCapture = 15,
}

const FLAG_CAPTURE: u16 = 4;
const FLAG_PROMO: u16 = 8;

impl Move {
    pub const NULL: Move = Move(0);

    #[inline(always)]
    pub const fn new(from: Square, to: Square, flag: MoveFlag) -> Move {
        Move((from.0 as u16) | ((to.0 as u16) << 6) | ((flag as u16) << 12))
    }

    #[inline(always)]
    pub const fn from_raw_flag(from: Square, to: Square, flag: u16) -> Move {
        Move((from.0 as u16) | ((to.0 as u16) << 6) | (flag << 12))
    }

    #[inline(always)]
    pub const fn from(self) -> Square {
        Square((self.0 & 0x3f) as u8)
    }

    #[inline(always)]
    pub const fn to(self) -> Square {
        Square(((self.0 >> 6) & 0x3f) as u8)
    }

    #[inline(always)]
    pub const fn flag_bits(self) -> u16 {
        self.0 >> 12
    }

    #[inline(always)]
    pub const fn is_null(self) -> bool {
        self.0 == 0
    }

    #[inline(always)]
    pub const fn is_capture(self) -> bool {
        self.flag_bits() & FLAG_CAPTURE != 0
    }

    #[inline(always)]
    pub const fn is_promotion(self) -> bool {
        self.flag_bits() & FLAG_PROMO != 0
    }

    #[inline(always)]
    pub const fn is_en_passant(self) -> bool {
        self.flag_bits() == MoveFlag::EnPassant as u16
    }

    #[inline(always)]
    pub const fn is_double_push(self) -> bool {
        self.flag_bits() == MoveFlag::DoublePush as u16
    }

    #[inline(always)]
    pub const fn is_king_castle(self) -> bool {
        self.flag_bits() == MoveFlag::KingCastle as u16
    }

    #[inline(always)]
    pub const fn is_queen_castle(self) -> bool {
        self.flag_bits() == MoveFlag::QueenCastle as u16
    }

    #[inline(always)]
    pub const fn is_castle(self) -> bool {
        self.is_king_castle() || self.is_queen_castle()
    }

    /// The promotion target piece type, if this is a promotion.
    #[inline(always)]
    pub const fn promotion(self) -> Option<PieceType> {
        if !self.is_promotion() {
            return None;
        }
        // Low 2 bits of the flag select Knight..Queen.
        Some(match self.flag_bits() & 0b11 {
            0 => PieceType::Knight,
            1 => PieceType::Bishop,
            2 => PieceType::Rook,
            _ => PieceType::Queen,
        })
    }

    /// UCI long-algebraic string, e.g. `e2e4`, `e7e8q`.
    pub fn to_uci(self) -> String {
        if self.is_null() {
            return "0000".to_string();
        }
        let mut s = format!("{}{}", self.from(), self.to());
        if let Some(pt) = self.promotion() {
            s.push(match pt {
                PieceType::Knight => 'n',
                PieceType::Bishop => 'b',
                PieceType::Rook => 'r',
                _ => 'q',
            });
        }
        s
    }
}

impl fmt::Debug for Move {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_uci())
    }
}

impl fmt::Display for Move {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_uci())
    }
}

/// A fixed-capacity, stack-allocated move list. 256 is a safe upper bound on the
/// number of legal moves in any reachable chess position.
pub struct MoveList {
    moves: [Move; 256],
    scores: [i32; 256],
    len: usize,
}

impl MoveList {
    #[inline(always)]
    pub fn new() -> MoveList {
        MoveList {
            moves: [Move::NULL; 256],
            scores: [0; 256],
            len: 0,
        }
    }

    #[inline(always)]
    pub fn len(&self) -> usize {
        self.len
    }

    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    #[inline(always)]
    pub fn push(&mut self, m: Move) {
        debug_assert!(self.len < 256, "move list overflow");
        // Safe in release because legal move count never exceeds 256.
        self.moves[self.len] = m;
        self.len += 1;
    }

    #[inline(always)]
    pub fn get(&self, i: usize) -> Move {
        self.moves[i]
    }

    #[inline(always)]
    pub fn score(&self, i: usize) -> i32 {
        self.scores[i]
    }

    #[inline(always)]
    pub fn set_score(&mut self, i: usize, s: i32) {
        self.scores[i] = s;
    }

    #[inline(always)]
    pub fn as_slice(&self) -> &[Move] {
        &self.moves[..self.len]
    }

    /// Selection-sort style: bring the highest-scored remaining move to `idx`
    /// and return it. This is the standard "pick best, lazily" pattern used in
    /// the search move loop so we only sort what we actually visit.
    #[inline]
    pub fn pick_best(&mut self, idx: usize) -> Move {
        let mut best = idx;
        let mut best_score = self.scores[idx];
        for j in (idx + 1)..self.len {
            if self.scores[j] > best_score {
                best = j;
                best_score = self.scores[j];
            }
        }
        if best != idx {
            self.moves.swap(idx, best);
            self.scores.swap(idx, best);
        }
        self.moves[idx]
    }

    pub fn contains(&self, m: Move) -> bool {
        self.as_slice().contains(&m)
    }
}

impl Default for MoveList {
    fn default() -> Self {
        Self::new()
    }
}
