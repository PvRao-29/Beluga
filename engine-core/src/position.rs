//! Board representation, FEN handling, and incremental make/unmake.

use crate::attacks;
use crate::bitboard::Bitboard;
use crate::chess_move::Move;
use crate::types::{CastlingRights, Color, Piece, PieceType, Square};
use crate::zobrist;
use std::fmt;

pub const START_FEN: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

/// Per-ply data needed to undo a move in O(1).
#[derive(Clone, Copy)]
struct Undo {
    captured: u8,
    castling: CastlingRights,
    ep_square: Option<Square>,
    halfmove: u16,
    key: u64,
}

#[derive(Clone)]
pub struct Position {
    piece_bb: [Bitboard; 12],
    color_bb: [Bitboard; 2],
    occupied: Bitboard,
    mailbox: [u8; 64],
    side: Color,
    castling: CastlingRights,
    ep_square: Option<Square>,
    halfmove: u16,
    fullmove: u16,
    key: u64,
    history: Vec<Undo>,
    key_history: Vec<u64>,
    castle_mask: [u8; 64],
}

impl Position {
    fn empty() -> Position {
        let mut castle_mask = [CastlingRights::ALL; 64];
        castle_mask[Square::A1.index()] &= !CastlingRights::WHITE_QUEEN;
        castle_mask[Square::H1.index()] &= !CastlingRights::WHITE_KING;
        castle_mask[Square::E1.index()] &=
            !(CastlingRights::WHITE_KING | CastlingRights::WHITE_QUEEN);
        castle_mask[Square::A8.index()] &= !CastlingRights::BLACK_QUEEN;
        castle_mask[Square::H8.index()] &= !CastlingRights::BLACK_KING;
        castle_mask[Square::E8.index()] &=
            !(CastlingRights::BLACK_KING | CastlingRights::BLACK_QUEEN);
        Position {
            piece_bb: [Bitboard::EMPTY; 12],
            color_bb: [Bitboard::EMPTY; 2],
            occupied: Bitboard::EMPTY,
            mailbox: [Piece::NONE; 64],
            side: Color::White,
            castling: CastlingRights::empty(),
            ep_square: None,
            halfmove: 0,
            fullmove: 1,
            key: 0,
            history: Vec::with_capacity(512),
            key_history: Vec::with_capacity(512),
            castle_mask,
        }
    }

    /// Standard starting position.
    pub fn startpos() -> Position {
        Position::from_fen(START_FEN).expect("valid start FEN")
    }

    // -- accessors -----------------------------------------------------------

    #[inline(always)]
    pub fn side_to_move(&self) -> Color {
        self.side
    }
    #[inline(always)]
    pub fn key(&self) -> u64 {
        self.key
    }
    #[inline(always)]
    pub fn halfmove_clock(&self) -> u16 {
        self.halfmove
    }
    #[inline(always)]
    pub fn fullmove_number(&self) -> u16 {
        self.fullmove
    }
    #[inline(always)]
    pub fn ep_square(&self) -> Option<Square> {
        self.ep_square
    }
    #[inline(always)]
    pub fn castling_rights(&self) -> CastlingRights {
        self.castling
    }
    #[inline(always)]
    pub fn occupied(&self) -> Bitboard {
        self.occupied
    }

    #[inline(always)]
    pub fn pieces(&self, color: Color, pt: PieceType) -> Bitboard {
        self.piece_bb[Piece::new(color, pt).index()]
    }

    #[inline(always)]
    pub fn pieces_by_type(&self, pt: PieceType) -> Bitboard {
        self.piece_bb[Piece::new(Color::White, pt).index()]
            | self.piece_bb[Piece::new(Color::Black, pt).index()]
    }

    #[inline(always)]
    pub fn color_pieces(&self, color: Color) -> Bitboard {
        self.color_bb[color.index()]
    }

    #[inline(always)]
    pub fn piece_on(&self, sq: Square) -> Option<Piece> {
        let p = self.mailbox[sq.index()];
        if p == Piece::NONE {
            None
        } else {
            Some(Piece(p))
        }
    }

    #[inline(always)]
    pub fn king_square(&self, color: Color) -> Square {
        self.pieces(color, PieceType::King).lsb()
    }

    // -- attack queries ------------------------------------------------------

    /// All pieces of both colors that attack `sq` given occupancy `occ`.
    #[inline(always)]
    pub fn attackers_to(&self, sq: Square, occ: Bitboard) -> Bitboard {
        let bishops =
            self.pieces_by_type(PieceType::Bishop) | self.pieces_by_type(PieceType::Queen);
        let rooks = self.pieces_by_type(PieceType::Rook) | self.pieces_by_type(PieceType::Queen);
        (attacks::pawn_attacks(Color::Black, sq) & self.pieces(Color::White, PieceType::Pawn))
            | (attacks::pawn_attacks(Color::White, sq) & self.pieces(Color::Black, PieceType::Pawn))
            | (attacks::knight_attacks(sq) & self.pieces_by_type(PieceType::Knight))
            | (attacks::king_attacks(sq) & self.pieces_by_type(PieceType::King))
            | (attacks::bishop_attacks(sq, occ) & bishops)
            | (attacks::rook_attacks(sq, occ) & rooks)
    }

    /// Is `sq` attacked by any piece of `by`?
    #[inline]
    pub fn is_attacked(&self, sq: Square, by: Color, occ: Bitboard) -> bool {
        let pawns = self.pieces(by, PieceType::Pawn);
        if (attacks::pawn_attacks(by.flip(), sq) & pawns).any() {
            return true;
        }
        if (attacks::knight_attacks(sq) & self.pieces(by, PieceType::Knight)).any() {
            return true;
        }
        if (attacks::king_attacks(sq) & self.pieces(by, PieceType::King)).any() {
            return true;
        }
        let bishops = self.pieces(by, PieceType::Bishop) | self.pieces(by, PieceType::Queen);
        if (attacks::bishop_attacks(sq, occ) & bishops).any() {
            return true;
        }
        let rooks = self.pieces(by, PieceType::Rook) | self.pieces(by, PieceType::Queen);
        if (attacks::rook_attacks(sq, occ) & rooks).any() {
            return true;
        }
        false
    }

    #[inline]
    pub fn in_check(&self) -> bool {
        let ksq = self.king_square(self.side);
        self.is_attacked(ksq, self.side.flip(), self.occupied)
    }

    /// Bitboard of enemy pieces giving check to the side to move.
    #[inline]
    pub fn checkers(&self) -> Bitboard {
        let ksq = self.king_square(self.side);
        self.attackers_to(ksq, self.occupied) & self.color_bb[self.side.flip().index()]
    }

    // -- low-level board mutation (update Zobrist key incrementally) ----------

    #[inline(always)]
    fn add_piece(&mut self, piece: Piece, sq: Square) {
        self.piece_bb[piece.index()].set(sq);
        self.color_bb[piece.color().index()].set(sq);
        self.occupied.set(sq);
        self.mailbox[sq.index()] = piece.0;
        self.key ^= zobrist::piece_key(piece, sq);
    }

    #[inline(always)]
    fn remove_piece(&mut self, sq: Square) {
        let piece = Piece(self.mailbox[sq.index()]);
        self.piece_bb[piece.index()].clear(sq);
        self.color_bb[piece.color().index()].clear(sq);
        self.occupied.clear(sq);
        self.mailbox[sq.index()] = Piece::NONE;
        self.key ^= zobrist::piece_key(piece, sq);
    }

    #[inline(always)]
    fn move_piece(&mut self, from: Square, to: Square) {
        let piece = Piece(self.mailbox[from.index()]);
        let mask = Bitboard::from_square(from) | Bitboard::from_square(to);
        self.piece_bb[piece.index()] ^= mask;
        self.color_bb[piece.color().index()] ^= mask;
        self.occupied ^= mask;
        self.mailbox[from.index()] = Piece::NONE;
        self.mailbox[to.index()] = piece.0;
        self.key ^= zobrist::piece_key(piece, from) ^ zobrist::piece_key(piece, to);
    }

    // -- make / unmake -------------------------------------------------------

    /// Apply a legal move. Behavior is defined only for legal moves; debug
    /// builds verify key consistency via the `strict-asserts` feature.
    pub fn make_move(&mut self, mv: Move) {
        let us = self.side;
        let them = us.flip();
        let from = mv.from();
        let to = mv.to();
        let moving = Piece(self.mailbox[from.index()]);

        self.history.push(Undo {
            captured: self.mailbox[to.index()],
            castling: self.castling,
            ep_square: self.ep_square,
            halfmove: self.halfmove,
            key: self.key,
        });

        // Clear previous en-passant from the key.
        if let Some(ep) = self.ep_square {
            self.key ^= zobrist::en_passant_key(ep.file());
            self.ep_square = None;
        }

        self.halfmove += 1;

        // Captures (including en passant).
        if mv.is_en_passant() {
            let cap_sq = Square(if us == Color::White {
                to.0 - 8
            } else {
                to.0 + 8
            });
            self.remove_piece(cap_sq);
            self.halfmove = 0;
        } else if self.mailbox[to.index()] != Piece::NONE {
            self.remove_piece(to);
            self.halfmove = 0;
        }

        // Move / promote the piece.
        if let Some(promo) = mv.promotion() {
            self.remove_piece(from);
            self.add_piece(Piece::new(us, promo), to);
            self.halfmove = 0;
        } else {
            self.move_piece(from, to);
            if moving.piece_type() == PieceType::Pawn {
                self.halfmove = 0;
            }
        }

        // Castling: move the rook (king already moved above).
        if mv.is_king_castle() {
            let (rf, rt) = if us == Color::White {
                (Square::H1, Square(5))
            } else {
                (Square::H8, Square(61))
            };
            self.move_piece(rf, rt);
        } else if mv.is_queen_castle() {
            let (rf, rt) = if us == Color::White {
                (Square::A1, Square(3))
            } else {
                (Square::A8, Square(59))
            };
            self.move_piece(rf, rt);
        }

        // Double push: set en passant square only if an enemy pawn can capture.
        if mv.is_double_push() {
            let ep = Square(if us == Color::White {
                to.0 - 8
            } else {
                to.0 + 8
            });
            let capturers = attacks::pawn_attacks(us, ep) & self.pieces(them, PieceType::Pawn);
            if capturers.any() {
                self.ep_square = Some(ep);
                self.key ^= zobrist::en_passant_key(ep.file());
            }
        }

        // Update castling rights via the from/to mask.
        let new_castling = CastlingRights(
            self.castling.0 & self.castle_mask[from.index()] & self.castle_mask[to.index()],
        );
        if new_castling != self.castling {
            self.key ^=
                zobrist::castling_key(self.castling.0) ^ zobrist::castling_key(new_castling.0);
            self.castling = new_castling;
        }

        if us == Color::Black {
            self.fullmove += 1;
        }
        self.side = them;
        self.key ^= zobrist::side_key();

        self.key_history.push(self.key);

        #[cfg(feature = "strict-asserts")]
        debug_assert_eq!(
            self.key,
            self.recompute_key(),
            "zobrist desync after make {mv}"
        );
    }

    /// Undo the most recent [`make_move`].
    pub fn unmake_move(&mut self, mv: Move) {
        let undo = self.history.pop().expect("unmake without matching make");
        self.key_history.pop();

        let them = self.side; // side that just moved is the *other* one now
        let us = them.flip();
        self.side = us;
        self.fullmove -= u16::from(us == Color::Black);

        let from = mv.from();
        let to = mv.to();

        // Reverse rook move for castling first (king reversed below).
        if mv.is_king_castle() {
            let (rf, rt) = if us == Color::White {
                (Square::H1, Square(5))
            } else {
                (Square::H8, Square(61))
            };
            self.move_piece(rt, rf);
        } else if mv.is_queen_castle() {
            let (rf, rt) = if us == Color::White {
                (Square::A1, Square(3))
            } else {
                (Square::A8, Square(59))
            };
            self.move_piece(rt, rf);
        }

        // Reverse the moving piece.
        if mv.is_promotion() {
            self.remove_piece(to);
            self.add_piece(Piece::new(us, PieceType::Pawn), from);
        } else {
            self.move_piece(to, from);
        }

        // Restore captured material.
        if mv.is_en_passant() {
            let cap_sq = Square(if us == Color::White {
                to.0 - 8
            } else {
                to.0 + 8
            });
            self.add_piece(Piece::new(them, PieceType::Pawn), cap_sq);
        } else if undo.captured != Piece::NONE {
            self.add_piece(Piece(undo.captured), to);
        }

        // Restore scalar state and key exactly (key was mangled by helpers above).
        self.castling = undo.castling;
        self.ep_square = undo.ep_square;
        self.halfmove = undo.halfmove;
        self.key = undo.key;
    }

    /// Make a null move (search only — never a real game move).
    pub fn make_null_move(&mut self) {
        self.history.push(Undo {
            captured: Piece::NONE,
            castling: self.castling,
            ep_square: self.ep_square,
            halfmove: self.halfmove,
            key: self.key,
        });
        if let Some(ep) = self.ep_square {
            self.key ^= zobrist::en_passant_key(ep.file());
            self.ep_square = None;
        }
        self.halfmove += 1;
        self.side = self.side.flip();
        self.key ^= zobrist::side_key();
        self.key_history.push(self.key);
    }

    pub fn unmake_null_move(&mut self) {
        let undo = self.history.pop().expect("unmake null without make");
        self.key_history.pop();
        self.side = self.side.flip();
        self.castling = undo.castling;
        self.ep_square = undo.ep_square;
        self.halfmove = undo.halfmove;
        self.key = undo.key;
    }

    // -- draw detection ------------------------------------------------------

    /// True if the current position has repeated within the irreversible window.
    /// In search we treat the first repetition as a draw (standard, avoids
    /// search blindness to cycles).
    pub fn is_repetition(&self) -> bool {
        let n = self.key_history.len();
        if n < 5 {
            return false;
        }
        let limit = (self.halfmove as usize).min(n - 1);
        // Same side to move recurs every 2 plies.
        let mut i = 4;
        while i <= limit {
            if self.key_history[n - 1 - i] == self.key {
                return true;
            }
            i += 2;
        }
        false
    }

    pub fn is_fifty_move(&self) -> bool {
        self.halfmove >= 100
    }

    /// Draw by insufficient mating material (strict set only).
    pub fn is_insufficient_material(&self) -> bool {
        if self.pieces_by_type(PieceType::Pawn).any()
            || self.pieces_by_type(PieceType::Rook).any()
            || self.pieces_by_type(PieceType::Queen).any()
        {
            return false;
        }
        let knights = self.pieces_by_type(PieceType::Knight).count();
        let bishops = self.pieces_by_type(PieceType::Bishop).count();
        let minors = knights + bishops;
        // KvK, KNvK, KBvK. (KNNvK is not a forced mate but we conservatively do
        // not claim it as a draw to avoid mis-adjudicating won-by-blunder lines.)
        minors <= 1
    }

    // -- FEN -----------------------------------------------------------------

    pub fn from_fen(fen: &str) -> Result<Position, String> {
        let mut pos = Position::empty();
        let mut parts = fen.split_whitespace();

        let board = parts.next().ok_or("FEN: missing board")?;
        let mut rank: i32 = 7;
        let mut file: i32 = 0;
        for c in board.chars() {
            match c {
                '/' => {
                    if file != 8 {
                        return Err(format!("FEN: rank {rank} wrong length"));
                    }
                    rank -= 1;
                    file = 0;
                }
                '1'..='8' => {
                    file += c.to_digit(10).unwrap() as i32;
                    if file > 8 {
                        return Err("FEN: file overflow".into());
                    }
                }
                _ => {
                    let piece = Piece::from_char(c).ok_or(format!("FEN: bad piece '{c}'"))?;
                    if !(0..8).contains(&file) || !(0..8).contains(&rank) {
                        return Err("FEN: piece out of board".into());
                    }
                    pos.add_piece(piece, Square::from_file_rank(file as u8, rank as u8));
                    file += 1;
                }
            }
        }
        if rank != 0 || file != 8 {
            return Err("FEN: board not 8x8".into());
        }

        match parts.next() {
            Some("w") => pos.side = Color::White,
            Some("b") => pos.side = Color::Black,
            _ => return Err("FEN: bad side to move".into()),
        }

        let castle = parts.next().unwrap_or("-");
        if castle != "-" {
            for c in castle.chars() {
                match c {
                    'K' => pos.castling.add(CastlingRights::WHITE_KING),
                    'Q' => pos.castling.add(CastlingRights::WHITE_QUEEN),
                    'k' => pos.castling.add(CastlingRights::BLACK_KING),
                    'q' => pos.castling.add(CastlingRights::BLACK_QUEEN),
                    _ => return Err(format!("FEN: bad castling '{c}'")),
                }
            }
        }

        let ep = parts.next().unwrap_or("-");
        if ep != "-" {
            let sq = Square::from_uci(ep).ok_or("FEN: bad ep square")?;
            // Only honor EP if a capturer actually exists (keeps keys canonical).
            let capturers =
                attacks::pawn_attacks(pos.side, sq) & pos.pieces(pos.side.flip(), PieceType::Pawn);
            if capturers.any() {
                pos.ep_square = Some(sq);
            }
        }

        pos.halfmove = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
        pos.fullmove = parts.next().and_then(|s| s.parse().ok()).unwrap_or(1);

        // Sanity: exactly one king per side.
        if pos.pieces(Color::White, PieceType::King).count() != 1
            || pos.pieces(Color::Black, PieceType::King).count() != 1
        {
            return Err("FEN: must have exactly one king per side".into());
        }
        // Side not to move must not be in check.
        let opp = pos.side.flip();
        if pos.is_attacked(pos.king_square(opp), pos.side, pos.occupied) {
            return Err("FEN: side not to move is in check".into());
        }

        pos.key = pos.recompute_key();
        pos.key_history.clear();
        pos.key_history.push(pos.key);
        Ok(pos)
    }

    pub fn to_fen(&self) -> String {
        let mut s = String::new();
        for rank in (0..8).rev() {
            let mut empty = 0;
            for file in 0..8 {
                let sq = Square::from_file_rank(file, rank);
                match self.piece_on(sq) {
                    Some(p) => {
                        if empty > 0 {
                            s.push_str(&empty.to_string());
                            empty = 0;
                        }
                        s.push(p.to_char());
                    }
                    None => empty += 1,
                }
            }
            if empty > 0 {
                s.push_str(&empty.to_string());
            }
            if rank > 0 {
                s.push('/');
            }
        }
        s.push(' ');
        s.push(if self.side == Color::White { 'w' } else { 'b' });
        s.push(' ');
        s.push_str(&self.castling.to_string());
        s.push(' ');
        match self.ep_square {
            Some(sq) => s.push_str(&sq.to_string()),
            None => s.push('-'),
        }
        s.push(' ');
        s.push_str(&self.halfmove.to_string());
        s.push(' ');
        s.push_str(&self.fullmove.to_string());
        s
    }

    /// Recompute the Zobrist key from scratch (used by FEN load and asserts).
    pub fn recompute_key(&self) -> u64 {
        let mut key = 0u64;
        for sq in 0..64u8 {
            let p = self.mailbox[sq as usize];
            if p != Piece::NONE {
                key ^= zobrist::piece_key(Piece(p), Square(sq));
            }
        }
        key ^= zobrist::castling_key(self.castling.0);
        if let Some(ep) = self.ep_square {
            key ^= zobrist::en_passant_key(ep.file());
        }
        key ^= zobrist::color_key(self.side);
        key
    }

    /// True if the position has any non-pawn, non-king material for the side to
    /// move (used to gate null-move pruning against zugzwang).
    #[inline]
    pub fn has_non_pawn_material(&self, color: Color) -> bool {
        (self.color_bb[color.index()]
            & !(self.pieces(color, PieceType::Pawn) | self.pieces(color, PieceType::King)))
        .any()
    }

    /// Find a legal move matching the given UCI string in the current position.
    pub fn parse_uci_move(&self, s: &str) -> Option<Move> {
        let mut list = crate::chess_move::MoveList::new();
        crate::movegen::generate_legal(self, &mut list);
        list.as_slice().iter().find(|&&m| m.to_uci() == s).copied()
    }
}

impl fmt::Display for Position {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for rank in (0..8).rev() {
            write!(f, "{} ", rank + 1)?;
            for file in 0..8 {
                let sq = Square::from_file_rank(file, rank);
                let c = self.piece_on(sq).map(|p| p.to_char()).unwrap_or('.');
                write!(f, "{c} ")?;
            }
            writeln!(f)?;
        }
        writeln!(f, "  a b c d e f g h")?;
        writeln!(f, "fen: {}", self.to_fen())?;
        write!(f, "key: {:016x}", self.key)
    }
}
