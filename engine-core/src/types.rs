//! Core enumerations and small value types shared across the engine.
//!
//! Square indexing is Little-Endian Rank-File (LERF): `A1 = 0`, `H1 = 7`,
//! `A8 = 56`, `H8 = 63`. `file = sq & 7`, `rank = sq >> 3`.

use std::fmt;

/// Side to move / piece owner.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
#[repr(u8)]
pub enum Color {
    White = 0,
    Black = 1,
}

impl Color {
    #[inline(always)]
    pub const fn flip(self) -> Color {
        match self {
            Color::White => Color::Black,
            Color::Black => Color::White,
        }
    }

    #[inline(always)]
    pub const fn index(self) -> usize {
        self as usize
    }

    #[inline(always)]
    pub const fn from_index(i: usize) -> Color {
        if i == 0 {
            Color::White
        } else {
            Color::Black
        }
    }
}

/// Piece kind, independent of color.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
#[repr(u8)]
pub enum PieceType {
    Pawn = 0,
    Knight = 1,
    Bishop = 2,
    Rook = 3,
    Queen = 4,
    King = 5,
}

impl PieceType {
    pub const ALL: [PieceType; 6] = [
        PieceType::Pawn,
        PieceType::Knight,
        PieceType::Bishop,
        PieceType::Rook,
        PieceType::Queen,
        PieceType::King,
    ];

    #[inline(always)]
    pub const fn index(self) -> usize {
        self as usize
    }

    #[inline(always)]
    pub const fn from_index(i: usize) -> PieceType {
        match i {
            0 => PieceType::Pawn,
            1 => PieceType::Knight,
            2 => PieceType::Bishop,
            3 => PieceType::Rook,
            4 => PieceType::Queen,
            _ => PieceType::King,
        }
    }
}

/// A colored piece, encoded as `color * 6 + piece_type` (0..=11).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
#[repr(transparent)]
pub struct Piece(pub u8);

impl Piece {
    pub const NONE: u8 = 12;

    #[inline(always)]
    pub const fn new(color: Color, pt: PieceType) -> Piece {
        Piece(color as u8 * 6 + pt as u8)
    }

    #[inline(always)]
    pub const fn color(self) -> Color {
        if self.0 < 6 {
            Color::White
        } else {
            Color::Black
        }
    }

    #[inline(always)]
    pub const fn piece_type(self) -> PieceType {
        PieceType::from_index((self.0 % 6) as usize)
    }

    #[inline(always)]
    pub const fn index(self) -> usize {
        self.0 as usize
    }

    pub fn from_char(c: char) -> Option<Piece> {
        let color = if c.is_ascii_uppercase() {
            Color::White
        } else {
            Color::Black
        };
        let pt = match c.to_ascii_lowercase() {
            'p' => PieceType::Pawn,
            'n' => PieceType::Knight,
            'b' => PieceType::Bishop,
            'r' => PieceType::Rook,
            'q' => PieceType::Queen,
            'k' => PieceType::King,
            _ => return None,
        };
        Some(Piece::new(color, pt))
    }

    pub fn to_char(self) -> char {
        let c = match self.piece_type() {
            PieceType::Pawn => 'p',
            PieceType::Knight => 'n',
            PieceType::Bishop => 'b',
            PieceType::Rook => 'r',
            PieceType::Queen => 'q',
            PieceType::King => 'k',
        };
        if self.color() == Color::White {
            c.to_ascii_uppercase()
        } else {
            c
        }
    }
}

/// A board square in `0..64`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Square(pub u8);

impl Square {
    pub const A1: Square = Square(0);
    pub const E1: Square = Square(4);
    pub const H1: Square = Square(7);
    pub const A8: Square = Square(56);
    pub const E8: Square = Square(60);
    pub const H8: Square = Square(63);

    #[inline(always)]
    pub const fn new(index: u8) -> Square {
        Square(index)
    }

    #[inline(always)]
    pub const fn from_file_rank(file: u8, rank: u8) -> Square {
        Square(rank * 8 + file)
    }

    #[inline(always)]
    pub const fn index(self) -> usize {
        self.0 as usize
    }

    #[inline(always)]
    pub const fn file(self) -> u8 {
        self.0 & 7
    }

    #[inline(always)]
    pub const fn rank(self) -> u8 {
        self.0 >> 3
    }

    /// Vertical flip (used for color-mirrored PSTs and Black perspective).
    #[inline(always)]
    pub const fn flip_vertical(self) -> Square {
        Square(self.0 ^ 56)
    }

    pub fn from_uci(s: &str) -> Option<Square> {
        let b = s.as_bytes();
        if b.len() != 2 {
            return None;
        }
        let file = b[0].wrapping_sub(b'a');
        let rank = b[1].wrapping_sub(b'1');
        if file > 7 || rank > 7 {
            return None;
        }
        Some(Square::from_file_rank(file, rank))
    }
}

impl fmt::Display for Square {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let file = (b'a' + self.file()) as char;
        let rank = (b'1' + self.rank()) as char;
        write!(f, "{file}{rank}")
    }
}

impl fmt::Debug for Square {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self}")
    }
}

/// Castling rights as a 4-bit mask.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Hash)]
pub struct CastlingRights(pub u8);

impl CastlingRights {
    pub const WHITE_KING: u8 = 1;
    pub const WHITE_QUEEN: u8 = 2;
    pub const BLACK_KING: u8 = 4;
    pub const BLACK_QUEEN: u8 = 8;
    pub const ALL: u8 = 15;

    #[inline(always)]
    pub const fn empty() -> CastlingRights {
        CastlingRights(0)
    }

    #[inline(always)]
    pub const fn has(self, flag: u8) -> bool {
        self.0 & flag != 0
    }

    #[inline(always)]
    pub fn add(&mut self, flag: u8) {
        self.0 |= flag;
    }

    /// Apply the AND-mask for a square touched by a move (king/rook move or
    /// rook capture). See [`crate::position`] for the precomputed table.
    #[inline(always)]
    pub fn apply_mask(&mut self, mask: u8) {
        self.0 &= mask;
    }
}

impl fmt::Display for CastlingRights {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0 == 0 {
            return write!(f, "-");
        }
        if self.has(Self::WHITE_KING) {
            write!(f, "K")?;
        }
        if self.has(Self::WHITE_QUEEN) {
            write!(f, "Q")?;
        }
        if self.has(Self::BLACK_KING) {
            write!(f, "k")?;
        }
        if self.has(Self::BLACK_QUEEN) {
            write!(f, "q")?;
        }
        Ok(())
    }
}
