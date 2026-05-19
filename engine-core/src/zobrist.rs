//! Zobrist hashing keys, generated deterministically from a fixed seed.

use crate::types::{Color, Piece, Square};
use std::sync::OnceLock;

pub struct Zobrist {
    pub pieces: [[u64; 64]; 12],
    pub castling: [u64; 16],
    pub en_passant: [u64; 8],
    pub side: u64,
}

static ZOBRIST: OnceLock<Zobrist> = OnceLock::new();

#[inline(always)]
fn z() -> &'static Zobrist {
    ZOBRIST.get_or_init(Zobrist::build)
}

impl Zobrist {
    fn build() -> Zobrist {
        // splitmix64 stream for high-quality, reproducible keys.
        let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut next = || {
            state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        };

        let mut pieces = [[0u64; 64]; 12];
        for piece in pieces.iter_mut() {
            for sq in piece.iter_mut() {
                *sq = next();
            }
        }
        let mut castling = [0u64; 16];
        for c in castling.iter_mut() {
            *c = next();
        }
        let mut en_passant = [0u64; 8];
        for e in en_passant.iter_mut() {
            *e = next();
        }
        let side = next();

        Zobrist {
            pieces,
            castling,
            en_passant,
            side,
        }
    }
}

#[inline(always)]
pub fn piece_key(piece: Piece, sq: Square) -> u64 {
    z().pieces[piece.index()][sq.index()]
}

#[inline(always)]
pub fn castling_key(rights: u8) -> u64 {
    z().castling[rights as usize]
}

#[inline(always)]
pub fn en_passant_key(file: u8) -> u64 {
    z().en_passant[file as usize]
}

#[inline(always)]
pub fn side_key() -> u64 {
    z().side
}

/// Convenience for the side-to-move toggle (XOR `side` key when it is Black's
/// move). We fold the side key in only for Black so White-to-move keys are the
/// base; this is a free choice as long as it is consistent.
#[inline(always)]
pub fn color_key(color: Color) -> u64 {
    if color == Color::Black {
        z().side
    } else {
        0
    }
}
