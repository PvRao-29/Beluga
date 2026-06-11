//! Handcrafted, tapered evaluation.
//!
//! The base is the public-domain **PeSTO** piece-square tables (midgame /
//! endgame), interpolated by a material **phase**. On top we add the classic
//! positional terms from `docs/DESIGN.md` §F: bishop pair, rook files, rook on
//! 7th, safe-square mobility, king safety (attack units + pawn shelter), pawn
//! structure (doubled / isolated / backward), refined passed pawns (blockers,
//! king proximity, connection), knight outposts, threats, space, and tempo.
//! All scores are integer centipawns from the side to move's perspective,
//! which is what negamax expects.
//!
//! All scalar weights live in [`EvalParams`] (`PARAMS`) so a future tuner can
//! lift them without another refactor. The PSTs stay standalone consts.
//!
//! Tables are indexed in human reading order (index 0 = a8). A white piece on
//! square `s` (a1 = 0) reads `TABLE[s ^ 56]`; a black piece reads `TABLE[s]`.

use crate::attacks;
use crate::bitboard::Bitboard;
use crate::position::Position;
use crate::types::{Color, PieceType, Square};

/// Tapered score accumulator: holds separate midgame and endgame components.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
struct Score {
    mg: i32,
    eg: i32,
}

impl Score {
    #[inline(always)]
    fn add(&mut self, mg: i32, eg: i32) {
        self.mg += mg;
        self.eg += eg;
    }
}

/// All scalar evaluation weights, grouped for a future tuner. Magnitudes are
/// conservative, PeSTO/Crafty-scale published values — not tuned for Beluga.
pub struct EvalParams {
    pub bishop_pair: (i32, i32),
    pub rook_open_file: (i32, i32),
    pub rook_semi_open_file: (i32, i32),
    /// Rook on the 7th rank with the enemy king on the 8th or enemy pawns on the 7th.
    pub rook_on_seventh: (i32, i32),
    pub tempo: i32,
    /// Passed pawn bonus by relative rank (owner's perspective, rank 0..7).
    pub passed_pawn: [(i32, i32); 8],
    /// Percent of the rank bonus kept when the stop square is blocked.
    pub passed_blocked_pct: i32,
    /// Passed pawn defended by an own pawn or with a phalanx neighbor.
    pub passed_connected: (i32, i32),
    /// Endgame king-distance weights (own-king penalty, enemy-king bonus) per
    /// Chebyshev distance to the stop square, scaled by relative rank.
    pub passed_king_dist: (i32, i32),
    /// Per-attacked-safe-square bonus, in quarter-centipawns (N, B, R, Q).
    pub mobility: [(i32, i32); 4],
    pub doubled_pawn: (i32, i32),
    pub isolated_pawn: (i32, i32),
    pub backward_pawn: (i32, i32),
    /// Knight on a pawn-defended square that no enemy pawn can ever attack.
    pub knight_outpost: (i32, i32),
    /// King-zone attack units per attacking piece type (N, B, R, Q).
    pub king_attack_weight: [i32; 4],
    /// Midgame king danger = min(units^2 / div, max), needs >= 2 attackers.
    pub king_danger_div: i32,
    pub king_danger_max: i32,
    /// Shelter: per shield file with no own pawn ahead of the king (mg).
    pub shelter_missing: i32,
    /// Shelter: per rank the nearest shield pawn is beyond the king + 1 (mg).
    pub shelter_far: i32,
    /// Extra mg penalty when the king's own file is fully open / semi-open.
    pub king_file_open: i32,
    pub king_file_semi_open: i32,
    /// Pawn attacks an enemy piece, indexed by victim type (P unused).
    pub pawn_threat: [(i32, i32); 5],
    /// Minor attacks an enemy piece not defended by a pawn, by victim type.
    pub minor_threat: [(i32, i32); 5],
    /// Per safe central square on our side of the board (mg only).
    pub space: i32,
}

pub const PARAMS: EvalParams = EvalParams {
    bishop_pair: (22, 38),
    rook_open_file: (28, 12),
    rook_semi_open_file: (12, 8),
    rook_on_seventh: (20, 32),
    tempo: 14,
    passed_pawn: [
        (0, 0),
        (2, 8),
        (6, 14),
        (14, 30),
        (28, 56),
        (52, 98),
        (88, 150),
        (0, 0),
    ],
    passed_blocked_pct: 60,
    passed_connected: (10, 18),
    passed_king_dist: (2, 4),
    mobility: [(8, 4), (8, 6), (4, 8), (2, 10)],
    doubled_pawn: (10, 30),
    isolated_pawn: (10, 18),
    backward_pawn: (9, 22),
    knight_outpost: (24, 14),
    king_attack_weight: [2, 2, 3, 5],
    king_danger_div: 12,
    king_danger_max: 500,
    shelter_missing: 21,
    shelter_far: 7,
    king_file_open: 18,
    king_file_semi_open: 9,
    pawn_threat: [(0, 0), (40, 30), (40, 32), (55, 40), (60, 40)],
    minor_threat: [(0, 0), (12, 18), (14, 18), (38, 24), (42, 28)],
    space: 3,
};

const MG_VALUE: [i32; 6] = [82, 337, 365, 477, 1025, 0];
const EG_VALUE: [i32; 6] = [94, 281, 297, 512, 936, 0];

/// Phase weights per piece type; total over a full board is 24.
const PHASE_WEIGHT: [i32; 6] = [0, 1, 1, 2, 4, 0];
const TOTAL_PHASE: i32 = 24;

#[rustfmt::skip]
const MG_PST: [[i32; 64]; 6] = [
    // Pawn
    [   0,   0,   0,   0,   0,   0,  0,   0,
       98, 134,  61,  95,  68, 126, 34, -11,
       -6,   7,  26,  31,  65,  56, 25, -20,
      -14,  13,   6,  21,  23,  12, 17, -23,
      -27,  -2,  -5,  12,  17,   6, 10, -25,
      -26,  -4,  -4, -10,   3,   3, 33, -12,
      -35,  -1, -20, -23, -15,  24, 38, -22,
        0,   0,   0,   0,   0,   0,  0,   0],
    // Knight
    [-167, -89, -34, -49,  61, -97, -15, -107,
      -73, -41,  72,  36,  23,  62,   7,  -17,
      -47,  60,  37,  65,  84, 129,  73,   44,
       -9,  17,  19,  53,  37,  69,  18,   22,
      -13,   4,  16,  13,  28,  19,  21,   -8,
      -23,  -9,  12,  10,  19,  17,  25,  -16,
      -29, -53, -12,  -3,  -1,  18, -14,  -19,
     -105, -21, -58, -33, -17, -28, -19,  -23],
    // Bishop
    [ -29,   4, -82, -37, -25, -42,   7,  -8,
      -26,  16, -18, -13,  30,  59,  18, -47,
      -16,  37,  43,  40,  35,  50,  37,  -2,
       -4,   5,  19,  50,  37,  37,   7,  -2,
       -6,  13,  13,  26,  34,  12,  10,   4,
        0,  15,  15,  15,  14,  27,  18,  10,
        4,  15,  16,   0,   7,  21,  33,   1,
      -33,  -3, -14, -21, -13, -12, -39, -21],
    // Rook
    [  32,  42,  32,  51, 63,  9,  31,  43,
       27,  32,  58,  62, 80, 67,  26,  44,
       -5,  19,  26,  36, 17, 45,  61,  16,
      -24, -11,   7,  26, 24, 35,  -8, -20,
      -36, -26, -12,  -1,  9, -7,   6, -23,
      -45, -25, -16, -17,  3,  0,  -5, -33,
      -44, -16, -20,  -9, -1, 11,  -6, -71,
      -19, -13,   1,  17, 16,  7, -37, -26],
    // Queen
    [ -28,   0,  29,  12,  59,  44,  43,  45,
      -24, -39,  -5,   1, -16,  57,  28,  54,
      -13, -17,   7,   8,  29,  56,  47,  57,
      -27, -27, -16, -16,  -1,  17,  -2,   1,
       -9, -26,  -9, -10,  -2,  -4,   3,  -3,
      -14,   2, -11,  -2,  -5,   2,  14,   5,
      -35,  -8,  11,   2,   8,  15,  -3,   1,
       -1, -18,  -9,  10, -15, -25, -31, -50],
    // King
    [ -65,  23,  16, -15, -56, -34,   2,  13,
       29,  -1, -20,  -7,  -8,  -4, -38, -29,
       -9,  24,   2, -16, -20,   6,  22, -22,
      -17, -20, -12, -27, -30, -25, -14, -36,
      -49,  -1, -27, -39, -46, -44, -33, -51,
      -14, -14, -22, -46, -44, -30, -15, -27,
        1,   7,  -8, -64, -43, -16,   9,   8,
      -15,  36,  12, -54,   8, -28,  24,  14],
];

#[rustfmt::skip]
const EG_PST: [[i32; 64]; 6] = [
    // Pawn
    [   0,   0,   0,   0,   0,   0,   0,   0,
      178, 173, 158, 134, 147, 132, 165, 187,
       94, 100,  85,  67,  56,  53,  82,  84,
       32,  24,  13,   5,  -2,   4,  17,  17,
       13,   9,  -3,  -7,  -7,  -8,   3,  -1,
        4,   7,  -6,   1,   0,  -5,  -1,  -8,
       13,   8,   8,  10,  13,   0,   2,  -7,
        0,   0,   0,   0,   0,   0,   0,   0],
    // Knight
    [ -58, -38, -13, -28, -31, -27, -63, -99,
      -25,  -8, -25,  -2,  -9, -25, -24, -52,
      -24, -20,  10,   9,  -1,  -9, -19, -41,
      -17,   3,  22,  22,  22,  11,   8, -18,
      -18,  -6,  16,  25,  16,  17,   4, -18,
      -23,  -3,  -1,  15,  10,  -3, -20, -22,
      -42, -20, -10,  -5,  -2, -20, -23, -44,
      -29, -51, -23, -15, -22, -18, -50, -64],
    // Bishop
    [ -14, -21, -11,  -8,  -7,  -9, -17, -24,
       -8,  -4,   7, -12,  -3, -13,  -4, -14,
        2,  -8,   0,  -1,  -2,   6,   0,   4,
       -3,   9,  12,   9,  14,  10,   3,   2,
       -6,   3,  13,  19,   7,  10,  -3,  -9,
      -12,  -3,   8,  10,  13,   3,  -7, -15,
      -14, -18,  -7,  -1,   4,  -9, -15, -27,
      -23,  -9, -23,  -5,  -9, -16,  -5, -17],
    // Rook
    [  13,  10,  18,  15,  12,  12,   8,   5,
       11,  13,  13,  11,  -3,   3,   8,   3,
        7,   7,   7,   5,   4,  -3,  -5,  -3,
        4,   3,  13,   1,   2,   1,  -1,   2,
        3,   5,   8,   4,  -5,  -6,  -8, -11,
       -4,   0,  -5,  -1,  -7, -12,  -8, -16,
       -6,  -6,   0,   2,  -9,  -9, -11,  -3,
       -9,   2,   3,  -1,  -5, -13,   4, -20],
    // Queen
    [  -9,  22,  22,  27,  27,  19,  10,  20,
      -17,  20,  32,  41,  58,  25,  30,   0,
      -20,   6,   9,  49,  47,  35,  19,   9,
        3,  22,  24,  45,  57,  40,  57,  36,
      -18,  28,  19,  47,  31,  34,  39,  23,
      -16, -27,  15,   6,   9,  17,  10,   5,
      -22, -23, -30, -16, -16, -23, -36, -32,
      -33, -28, -22, -43,  -5, -32, -20, -41],
    // King
    [ -74, -35, -18, -18, -11,  15,   4, -17,
      -12,  17,  14,  17,  17,  38,  23,  11,
       10,  17,  23,  15,  20,  45,  44,  13,
       -8,  22,  24,  27,  26,  33,  26,   3,
      -18,  -4,  21,  24,  27,  23,   9, -11,
      -19,  -3,  11,  21,  23,  16,   7,  -9,
      -27, -11,   4,  13,  14,   4,  -5, -17,
      -53, -34, -21, -11, -28, -14, -24, -43],
];

// -- precomputed masks --------------------------------------------------------

const FILE_MASKS: [u64; 8] = {
    let mut t = [0u64; 8];
    let mut f = 0;
    while f < 8 {
        t[f] = 0x0101_0101_0101_0101u64 << f;
        f += 1;
    }
    t
};

/// Adjacent-file masks (excluding the file itself).
const ADJ_FILES: [u64; 8] = {
    let mut t = [0u64; 8];
    let mut f = 0;
    while f < 8 {
        let mut m = 0u64;
        if f > 0 {
            m |= FILE_MASKS[f - 1];
        }
        if f < 7 {
            m |= FILE_MASKS[f + 1];
        }
        t[f] = m;
        f += 1;
    }
    t
};

/// All squares on ranks strictly ahead of `sq` from each color's perspective,
/// across the full board width. Indexed `[color][square]`.
const AHEAD: [[u64; 64]; 2] = {
    let mut t = [[0u64; 64]; 2];
    let mut s = 0;
    while s < 64 {
        let r = s >> 3;
        let mut white = 0u64;
        let mut rr = r + 1;
        while rr < 8 {
            white |= 0xFFu64 << (rr * 8);
            rr += 1;
        }
        let mut black = 0u64;
        let mut rr = 0;
        while rr < r {
            black |= 0xFFu64 << (rr * 8);
            rr += 1;
        }
        t[0][s] = white;
        t[1][s] = black;
        s += 1;
    }
    t
};

const CENTER_FILES: u64 = FILE_MASKS[2] | FILE_MASKS[3] | FILE_MASKS[4] | FILE_MASKS[5];
/// Relative ranks 2-4 for each color (the "own half" space area).
const SPACE_RANKS: [u64; 2] = [0x0000_0000_FFFF_FF00, 0x00FF_FFFF_0000_0000];

// -- term bookkeeping (shared by evaluate and trace) ---------------------------

const NTERMS: usize = 11;
const TERM_NAMES: [&str; NTERMS] = [
    "material+pst",
    "bishop pair",
    "rook files",
    "rook on 7th",
    "mobility",
    "king safety",
    "pawn structure",
    "passed pawns",
    "outposts",
    "threats",
    "space",
];
const T_MATERIAL: usize = 0;
const T_BISHOP_PAIR: usize = 1;
const T_ROOK_FILES: usize = 2;
const T_ROOK_7TH: usize = 3;
const T_MOBILITY: usize = 4;
const T_KING_SAFETY: usize = 5;
const T_PAWN_STRUCT: usize = 6;
const T_PASSED: usize = 7;
const T_OUTPOST: usize = 8;
const T_THREATS: usize = 9;
const T_SPACE: usize = 10;

/// Per-term, per-color tapered scores plus the material phase.
#[derive(Default)]
struct Terms {
    v: [[Score; 2]; NTERMS],
    phase: i32,
}

impl Terms {
    /// Final centipawn score from `stm`'s perspective (taper + tempo).
    fn score(&self, stm: Color) -> i32 {
        let mut total = Score::default();
        for t in &self.v {
            total.add(t[0].mg - t[1].mg, t[0].eg - t[1].eg);
        }
        let mg_phase = self.phase.min(TOTAL_PHASE);
        let eg_phase = TOTAL_PHASE - mg_phase;
        let mut score = (total.mg * mg_phase + total.eg * eg_phase) / TOTAL_PHASE;
        score += if stm == Color::White {
            PARAMS.tempo
        } else {
            -PARAMS.tempo
        };
        if stm == Color::White {
            score
        } else {
            -score
        }
    }
}

#[inline(always)]
fn pst_index(color: Color, sq: usize) -> usize {
    if color == Color::White {
        sq ^ 56
    } else {
        sq
    }
}

#[inline(always)]
fn file_bb(file: u8) -> Bitboard {
    Bitboard(FILE_MASKS[file as usize])
}

#[inline(always)]
fn ahead_bb(color: Color, sq: Square) -> Bitboard {
    Bitboard(AHEAD[color.index()][sq.index()])
}

/// Attack set of an entire pawn bitboard.
#[inline(always)]
fn pawn_attacks_bb(color: Color, pawns: Bitboard) -> Bitboard {
    match color {
        Color::White => pawns.shift_north().shift_east() | pawns.shift_north().shift_west(),
        Color::Black => pawns.shift_south().shift_east() | pawns.shift_south().shift_west(),
    }
}

#[inline(always)]
fn forward_sq(color: Color, sq: Square) -> Square {
    Square(if color == Color::White {
        sq.0 + 8
    } else {
        sq.0 - 8
    })
}

#[inline(always)]
fn rel_rank(color: Color, sq: Square) -> usize {
    if color == Color::White {
        sq.rank() as usize
    } else {
        7 - sq.rank() as usize
    }
}

#[inline(always)]
fn chebyshev(a: Square, b: Square) -> i32 {
    let df = (a.file() as i32 - b.file() as i32).abs();
    let dr = (a.rank() as i32 - b.rank() as i32).abs();
    df.max(dr)
}

/// Most significant set square. Caller must ensure non-empty.
#[inline(always)]
fn msb(bb: Bitboard) -> Square {
    Square(63 - bb.0.leading_zeros() as u8)
}

/// Full static evaluation in centipawns, relative to the side to move.
pub fn evaluate(pos: &Position) -> i32 {
    eval_terms(pos).score(pos.side_to_move())
}

fn eval_terms(pos: &Position) -> Terms {
    let mut t = Terms::default();
    let occ = pos.occupied();

    let pawns = [
        pos.pieces(Color::White, PieceType::Pawn),
        pos.pieces(Color::Black, PieceType::Pawn),
    ];
    let pawn_att = [
        pawn_attacks_bb(Color::White, pawns[0]),
        pawn_attacks_bb(Color::Black, pawns[1]),
    ];
    let ksq = [
        pos.king_square(Color::White),
        pos.king_square(Color::Black),
    ];
    // King zone: king + its ring, extended one rank toward the enemy.
    let king_zone = {
        let wz = attacks::king_attacks(ksq[0]) | Bitboard::from_square(ksq[0]);
        let bz = attacks::king_attacks(ksq[1]) | Bitboard::from_square(ksq[1]);
        [wz | wz.shift_north(), bz | bz.shift_south()]
    };

    // Attack units accumulated against each color's king, and minor-piece
    // attack maps, filled during the per-color piece loop below.
    let mut king_units = [0i32; 2];
    let mut king_attackers = [0i32; 2];
    let mut minor_att = [Bitboard::EMPTY; 2];

    for color in [Color::White, Color::Black] {
        let us = color.index();
        let them = color.flip().index();
        let us_pawns = pawns[us];
        let them_pawns = pawns[them];
        let own = pos.color_pieces(color);
        // Squares that count for mobility: not own pieces, not pawn-controlled.
        let mob_area = !own & !pawn_att[them];

        for pt in PieceType::ALL {
            let pti = pt.index();
            let mut bb = pos.pieces(color, pt);
            while bb.any() {
                let sq = bb.pop_lsb();
                let idx = pst_index(color, sq.index());
                t.v[T_MATERIAL][us].add(
                    MG_VALUE[pti] + MG_PST[pti][idx],
                    EG_VALUE[pti] + EG_PST[pti][idx],
                );
                t.phase += PHASE_WEIGHT[pti];

                let att = match pt {
                    PieceType::Knight => attacks::knight_attacks(sq),
                    PieceType::Bishop => attacks::bishop_attacks(sq, occ),
                    PieceType::Rook => attacks::rook_attacks(sq, occ),
                    PieceType::Queen => attacks::queen_attacks(sq, occ),
                    _ => continue,
                };

                // Mobility (quarter-centipawn weights over safe squares).
                let mob = (att & mob_area).count() as i32;
                let (wm, we) = PARAMS.mobility[pti - 1];
                t.v[T_MOBILITY][us].add(wm * mob / 4, we * mob / 4);

                // King-zone attack units against the enemy king.
                let zone_hits = (att & king_zone[them]).count() as i32;
                if zone_hits > 0 {
                    king_units[them] += PARAMS.king_attack_weight[pti - 1] * zone_hits;
                    king_attackers[them] += 1;
                }

                if pt == PieceType::Knight || pt == PieceType::Bishop {
                    minor_att[us] |= att;
                }

                match pt {
                    PieceType::Knight => {
                        // Outpost: pawn-defended, on ranks 4-6, and no enemy
                        // pawn on an adjacent file can ever attack the square.
                        let rr = rel_rank(color, sq);
                        if (3..=5).contains(&rr)
                            && pawn_att[us].contains(sq)
                            && (Bitboard(ADJ_FILES[sq.file() as usize])
                                & ahead_bb(color, sq)
                                & them_pawns)
                                .is_empty()
                        {
                            t.v[T_OUTPOST][us]
                                .add(PARAMS.knight_outpost.0, PARAMS.knight_outpost.1);
                        }
                    }
                    PieceType::Rook => {
                        let fbb = file_bb(sq.file());
                        if (fbb & (us_pawns | them_pawns)).is_empty() {
                            t.v[T_ROOK_FILES][us]
                                .add(PARAMS.rook_open_file.0, PARAMS.rook_open_file.1);
                        } else if (fbb & us_pawns).is_empty() {
                            t.v[T_ROOK_FILES][us]
                                .add(PARAMS.rook_semi_open_file.0, PARAMS.rook_semi_open_file.1);
                        }
                        // Rook on the 7th, with targets there or a trapped king.
                        if rel_rank(color, sq) == 6 {
                            let seventh =
                                Bitboard(0xFFu64 << (if color == Color::White { 48 } else { 8 }));
                            if rel_rank(color, ksq[them]) == 7 || (them_pawns & seventh).any() {
                                t.v[T_ROOK_7TH][us]
                                    .add(PARAMS.rook_on_seventh.0, PARAMS.rook_on_seventh.1);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        // Bishop pair.
        if pos.pieces(color, PieceType::Bishop).count() >= 2 {
            t.v[T_BISHOP_PAIR][us].add(PARAMS.bishop_pair.0, PARAMS.bishop_pair.1);
        }

        // Pawn structure + passed pawns.
        for f in 0..8u8 {
            let on_file = (file_bb(f) & us_pawns).count() as i32;
            if on_file > 1 {
                t.v[T_PAWN_STRUCT][us].add(
                    -PARAMS.doubled_pawn.0 * (on_file - 1),
                    -PARAMS.doubled_pawn.1 * (on_file - 1),
                );
            }
        }
        let mut p = us_pawns;
        while p.any() {
            let sq = p.pop_lsb();
            let f = sq.file() as usize;
            let neighbors = Bitboard(ADJ_FILES[f]) & us_pawns;
            let ahead = ahead_bb(color, sq);

            if neighbors.is_empty() {
                t.v[T_PAWN_STRUCT][us].add(-PARAMS.isolated_pawn.0, -PARAMS.isolated_pawn.1);
            } else if (neighbors & !ahead).is_empty() {
                // All neighbors are ahead: backward if the stop square is
                // controlled by an enemy pawn.
                let stop = forward_sq(color, sq);
                if pawn_att[them].contains(stop) {
                    t.v[T_PAWN_STRUCT][us].add(-PARAMS.backward_pawn.0, -PARAMS.backward_pawn.1);
                }
            }

            // Passed: no enemy pawn ahead on this or adjacent files.
            if ((Bitboard(FILE_MASKS[f] | ADJ_FILES[f]) & ahead) & them_pawns).is_empty() {
                let rr = rel_rank(color, sq);
                let (mut mg, mut eg) = PARAMS.passed_pawn[rr];
                let stop = forward_sq(color, sq);
                if pos.piece_on(stop).is_some() {
                    mg = mg * PARAMS.passed_blocked_pct / 100;
                    eg = eg * PARAMS.passed_blocked_pct / 100;
                }
                // King proximity matters more the further the pawn is along.
                let (own_w, enemy_w) = PARAMS.passed_king_dist;
                eg += (enemy_w * chebyshev(ksq[them], stop) - own_w * chebyshev(ksq[us], stop))
                    * rr as i32
                    / 2;
                t.v[T_PASSED][us].add(mg, eg);
                // Connected: defended by a pawn or with a phalanx neighbor.
                let bb = Bitboard::from_square(sq);
                if pawn_att[us].contains(sq)
                    || ((bb.shift_east() | bb.shift_west()) & us_pawns).any()
                {
                    t.v[T_PASSED][us].add(PARAMS.passed_connected.0, PARAMS.passed_connected.1);
                }
            }
        }

        // Pawn shelter + file openness in front of our king (mg only).
        let kf = ksq[us].file();
        let lo = kf.saturating_sub(1);
        let hi = (kf + 1).min(7);
        let mut shelter = 0i32;
        for f in lo..=hi {
            let shield = file_bb(f) & us_pawns & ahead_bb(color, ksq[us]);
            if shield.is_empty() {
                shelter -= PARAMS.shelter_missing;
            } else {
                let nearest = if color == Color::White {
                    shield.lsb()
                } else {
                    msb(shield)
                };
                let dist = (nearest.rank() as i32 - ksq[us].rank() as i32).abs();
                shelter -= (dist - 1) * PARAMS.shelter_far;
            }
        }
        let kfile = file_bb(kf);
        if (kfile & us_pawns).is_empty() {
            shelter -= if (kfile & them_pawns).is_empty() {
                PARAMS.king_file_open
            } else {
                PARAMS.king_file_semi_open
            };
        }
        t.v[T_KING_SAFETY][us].add(shelter, 0);

        // Space: safe central squares on our half (mg only).
        let space_area = Bitboard(CENTER_FILES & SPACE_RANKS[us]) & !us_pawns & !pawn_att[them];
        t.v[T_SPACE][us].add(PARAMS.space * space_area.count() as i32, 0);
    }

    // King danger from accumulated attack units (quadratic, mg-weighted).
    for color in [Color::White, Color::Black] {
        let us = color.index();
        let mut units = king_units[us];
        if king_attackers[us] >= 2 && units > 0 {
            // Without a queen the attack is far less dangerous.
            if pos.pieces(color.flip(), PieceType::Queen).is_empty() {
                units /= 2;
            }
            let danger = (units * units / PARAMS.king_danger_div).min(PARAMS.king_danger_max);
            t.v[T_KING_SAFETY][us].add(-danger, -danger / 4);
        }
    }

    // Threats (need both sides' attack maps).
    for color in [Color::White, Color::Black] {
        let us = color.index();
        let them_color = color.flip();
        for pt in [
            PieceType::Knight,
            PieceType::Bishop,
            PieceType::Rook,
            PieceType::Queen,
        ] {
            let pti = pt.index();
            let targets = pos.pieces(them_color, pt);
            let by_pawn = (targets & pawn_att[us]).count() as i32;
            if by_pawn > 0 {
                let (mg, eg) = PARAMS.pawn_threat[pti];
                t.v[T_THREATS][us].add(mg * by_pawn, eg * by_pawn);
            }
            // Minor threats only count targets not protected by a pawn.
            let by_minor =
                (targets & minor_att[us] & !pawn_att[them_color.index()]).count() as i32;
            if by_minor > 0 {
                let (mg, eg) = PARAMS.minor_threat[pti];
                t.v[T_THREATS][us].add(mg * by_minor, eg * by_minor);
            }
        }
    }

    t
}

/// Print a per-term breakdown for debugging/tuning (`eval` UCI command).
pub fn trace(pos: &Position) -> String {
    let t = eval_terms(pos);
    let total = t.score(pos.side_to_move());

    let mut out = String::new();
    out.push_str(&format!(
        "{:<16} | {:>6} {:>6} | {:>6} {:>6} | {:>6} {:>6}\n",
        "term", "w.mg", "w.eg", "b.mg", "b.eg", "mg", "eg"
    ));
    out.push_str(&"-".repeat(66));
    out.push('\n');
    for (i, name) in TERM_NAMES.iter().enumerate() {
        let w = t.v[i][0];
        let b = t.v[i][1];
        out.push_str(&format!(
            "{:<16} | {:>6} {:>6} | {:>6} {:>6} | {:>6} {:>6}\n",
            name,
            w.mg,
            w.eg,
            b.mg,
            b.eg,
            w.mg - b.mg,
            w.eg - b.eg
        ));
    }
    out.push_str(&format!(
        "phase = {}/{} (mg-heavy = high)\ntempo = {} (stm)\nside to move = {:?}\neval (stm) = {} cp",
        t.phase.min(TOTAL_PHASE),
        TOTAL_PHASE,
        PARAMS.tempo,
        pos.side_to_move(),
        total
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn terms(fen: &str) -> Terms {
        crate::attacks::init();
        let pos = Position::from_fen(fen).expect("valid fen");
        eval_terms(&pos)
    }

    fn term_net(t: &Terms, idx: usize, color: Color) -> Score {
        t.v[idx][color.index()]
    }

    #[test]
    fn startpos_is_tempo_only() {
        crate::attacks::init();
        let pos = Position::startpos();
        assert_eq!(evaluate(&pos), PARAMS.tempo);
    }

    #[test]
    fn doubled_and_isolated_pawns_penalized() {
        // White: a2, a3 doubled+isolated; Black: healthy b7, c7.
        let t = terms("4k3/1pp5/8/8/8/P7/P7/4K3 w - - 0 1");
        let w = term_net(&t, T_PAWN_STRUCT, Color::White);
        let b = term_net(&t, T_PAWN_STRUCT, Color::Black);
        assert!(w.mg < 0 && w.eg < 0, "white structure should be penalized: {w:?}");
        assert_eq!(b, Score::default(), "black structure is healthy: {b:?}");
    }

    #[test]
    fn backward_pawn_detected() {
        // White d3 is backward: c4/e4-style neighbors ahead and the stop
        // square d4 is covered by black's pawn on c5 (no e-file neighbor here:
        // use pawns c4 and d3 with black c5 guarding d4... construct directly).
        // White pawns: c4, d3. Black pawn: e5 (attacks d4). d3's only neighbor
        // (c4) is ahead, and d4 is enemy-pawn-controlled => backward.
        let t = terms("4k3/8/8/4p3/2P5/3P4/8/4K3 w - - 0 1");
        let w = term_net(&t, T_PAWN_STRUCT, Color::White);
        assert!(
            w.mg <= -PARAMS.backward_pawn.0,
            "d3 should be flagged backward: {w:?}"
        );
    }

    #[test]
    fn knight_outpost_detected() {
        // White Nd5 supported by pawn e4; black has no c- or e-pawns to evict it.
        let t = terms("4k3/8/8/3N4/4P3/8/8/4K3 w - - 0 1");
        let w = term_net(&t, T_OUTPOST, Color::White);
        assert_eq!(w.mg, PARAMS.knight_outpost.0, "Nd5 is an outpost: {w:?}");

        // Same but black pawn on c7 can eventually attack d5: no outpost.
        let t2 = terms("4k3/2p5/8/3N4/4P3/8/8/4K3 w - - 0 1");
        let w2 = term_net(&t2, T_OUTPOST, Color::White);
        assert_eq!(w2.mg, 0, "c7 pawn denies the outpost: {w2:?}");
    }

    #[test]
    fn rook_on_seventh_detected() {
        // White Ra7, black king on e8 (8th rank) => bonus.
        let t = terms("4k3/R7/8/8/8/8/8/4K3 w - - 0 1");
        let w = term_net(&t, T_ROOK_7TH, Color::White);
        assert_eq!(w.mg, PARAMS.rook_on_seventh.0);

        // Black king off the back rank and no pawns on the 7th => no bonus.
        let t2 = terms("8/R7/4k3/8/8/8/8/4K3 w - - 0 1");
        let w2 = term_net(&t2, T_ROOK_7TH, Color::White);
        assert_eq!(w2.mg, 0);
    }

    #[test]
    fn passed_pawn_blocked_worth_less() {
        // White passed pawn on e5; in the second position a black knight
        // blockades e6. Material differs, so compare the passed-pawn term only.
        let open = terms("4k3/8/8/4P3/8/8/8/4K3 w - - 0 1");
        let blocked = terms("4k3/8/4n3/4P3/8/8/8/4K3 w - - 0 1");
        let wo = term_net(&open, T_PASSED, Color::White);
        let wb = term_net(&blocked, T_PASSED, Color::White);
        assert!(
            wb.eg < wo.eg,
            "blockaded passer should be worth less: open {wo:?} blocked {wb:?}"
        );
    }

    #[test]
    fn shattered_king_shelter_penalized() {
        // Castled king with an intact f2/g2/h2 shield...
        let intact = terms("6k1/5ppp/8/8/8/8/5PPP/6K1 w - - 0 1");
        // ...vs the same with white's g/h pawns ripped away.
        let stripped = terms("6k1/5ppp/8/8/8/8/5P2/6K1 w - - 0 1");
        let wi = term_net(&intact, T_KING_SAFETY, Color::White);
        let ws = term_net(&stripped, T_KING_SAFETY, Color::White);
        assert!(
            ws.mg < wi.mg,
            "stripped shelter must score worse: intact {wi:?} stripped {ws:?}"
        );
    }

    #[test]
    fn pawn_threat_on_piece_detected() {
        // White pawn e4 attacks black knight d5.
        let t = terms("4k3/8/8/3n4/4P3/8/8/4K3 w - - 0 1");
        let w = term_net(&t, T_THREATS, Color::White);
        assert_eq!(w.mg, PARAMS.pawn_threat[PieceType::Knight.index()].0);
    }

    #[test]
    fn eval_is_color_symmetric() {
        crate::attacks::init();
        // A middlegame position and its exact color-mirror must evaluate
        // identically from the mover's perspective.
        let a = Position::from_fen(
            "r1bqk2r/pppp1ppp/2n2n2/2b1p3/2B1P3/3P1N2/PPP2PPP/RNBQK2R w KQkq - 0 1",
        )
        .unwrap();
        let b = Position::from_fen(
            "rnbqk2r/ppp2ppp/3p1n2/2b1p3/2B1P3/2N2N2/PPPP1PPP/R1BQK2R b KQkq - 0 1",
        )
        .unwrap();
        assert_eq!(evaluate(&a), evaluate(&b));
    }
}
