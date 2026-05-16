//! Precomputed attack tables.
//!
//! * Leapers (pawn / knight / king) are simple lookup tables.
//! * Sliders (bishop / rook / queen) use **magic bitboards**. The magic numbers
//!   are found deterministically at first use with a fixed-seed PRNG, so builds
//!   are reproducible and no external data file is required.
//! * `between` / `line` tables support pin and check-resolution masks in movegen.

use crate::bitboard::Bitboard;
use crate::types::{Color, Square};
use std::sync::OnceLock;

/// Rook movement deltas as (file, rank) steps.
const ROOK_DIRS: [(i8, i8); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];
/// Bishop movement deltas as (file, rank) steps.
const BISHOP_DIRS: [(i8, i8); 4] = [(1, 1), (-1, 1), (1, -1), (-1, -1)];

struct Magic {
    mask: u64,
    magic: u64,
    shift: u32,
    offset: usize,
}

pub struct Tables {
    pawn: [[u64; 64]; 2],
    knight: [u64; 64],
    king: [u64; 64],
    between: [[u64; 64]; 64],
    line: [[u64; 64]; 64],
    bishop_magics: Vec<Magic>,
    rook_magics: Vec<Magic>,
    bishop_table: Vec<u64>,
    rook_table: Vec<u64>,
}

static TABLES: OnceLock<Tables> = OnceLock::new();

#[inline(always)]
fn tables() -> &'static Tables {
    TABLES.get_or_init(Tables::build)
}

/// Force initialization (useful for benchmarking init cost out of hot loops).
pub fn init() {
    let _ = tables();
}

#[inline(always)]
pub fn pawn_attacks(color: Color, sq: Square) -> Bitboard {
    Bitboard(tables().pawn[color.index()][sq.index()])
}

#[inline(always)]
pub fn knight_attacks(sq: Square) -> Bitboard {
    Bitboard(tables().knight[sq.index()])
}

#[inline(always)]
pub fn king_attacks(sq: Square) -> Bitboard {
    Bitboard(tables().king[sq.index()])
}

#[inline(always)]
pub fn bishop_attacks(sq: Square, occ: Bitboard) -> Bitboard {
    let t = tables();
    let m = &t.bishop_magics[sq.index()];
    let idx = (((occ.0 & m.mask).wrapping_mul(m.magic)) >> m.shift) as usize;
    Bitboard(t.bishop_table[m.offset + idx])
}

#[inline(always)]
pub fn rook_attacks(sq: Square, occ: Bitboard) -> Bitboard {
    let t = tables();
    let m = &t.rook_magics[sq.index()];
    let idx = (((occ.0 & m.mask).wrapping_mul(m.magic)) >> m.shift) as usize;
    Bitboard(t.rook_table[m.offset + idx])
}

#[inline(always)]
pub fn queen_attacks(sq: Square, occ: Bitboard) -> Bitboard {
    bishop_attacks(sq, occ) | rook_attacks(sq, occ)
}

/// Squares strictly between `a` and `b` along a shared rank/file/diagonal,
/// or empty if they are not aligned.
#[inline(always)]
pub fn between(a: Square, b: Square) -> Bitboard {
    Bitboard(tables().between[a.index()][b.index()])
}

/// The full line (rank/file/diagonal) through `a` and `b`, including both
/// endpoints, or empty if not aligned. Used to detect pin rays.
#[inline(always)]
pub fn line(a: Square, b: Square) -> Bitboard {
    Bitboard(tables().line[a.index()][b.index()])
}

// ---------------------------------------------------------------------------
// Construction
// ---------------------------------------------------------------------------

fn on_board(file: i8, rank: i8) -> bool {
    (0..8).contains(&file) && (0..8).contains(&rank)
}

/// Sliding attacks computed by ray-walking, used to *build* the magic tables.
fn sliding_attacks(sq: Square, occ: u64, dirs: &[(i8, i8); 4]) -> u64 {
    let mut attacks = 0u64;
    let (sf, sr) = (sq.file() as i8, sq.rank() as i8);
    for &(df, dr) in dirs {
        let (mut f, mut r) = (sf + df, sr + dr);
        while on_board(f, r) {
            let bit = 1u64 << (r * 8 + f);
            attacks |= bit;
            if occ & bit != 0 {
                break;
            }
            f += df;
            r += dr;
        }
    }
    attacks
}

/// Relevant-occupancy mask: ray squares excluding the edges (edges never change
/// the set of attacked squares for magic indexing).
fn relevant_mask(sq: Square, dirs: &[(i8, i8); 4]) -> u64 {
    let mut mask = 0u64;
    let (sf, sr) = (sq.file() as i8, sq.rank() as i8);
    for &(df, dr) in dirs {
        let (mut f, mut r) = (sf + df, sr + dr);
        while on_board(f + df, r + dr) {
            mask |= 1u64 << (r * 8 + f);
            f += df;
            r += dr;
        }
    }
    mask
}

/// Enumerate the n-th subset of the bits in `mask` (carry-rippler order).
fn index_to_subset(index: usize, mask: u64) -> u64 {
    let mut subset = 0u64;
    let mut m = mask;
    let mut i = index;
    while m != 0 {
        let bit = m & m.wrapping_neg();
        m &= m - 1;
        if i & 1 != 0 {
            subset |= bit;
        }
        i >>= 1;
    }
    subset
}

/// Small deterministic xorshift PRNG for magic search.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    /// Sparse candidate (AND of three draws) — far more likely to be a valid magic.
    fn sparse(&mut self) -> u64 {
        self.next() & self.next() & self.next()
    }
}

fn build_magics(dirs: &[(i8, i8); 4], table: &mut Vec<u64>, rng: &mut Rng) -> Vec<Magic> {
    let mut magics = Vec::with_capacity(64);
    for s in 0..64u8 {
        let sq = Square(s);
        let mask = relevant_mask(sq, dirs);
        let bits = mask.count_ones();
        let size = 1usize << bits;
        let shift = 64 - bits;

        // Precompute (subset, attacks) reference pairs.
        let mut subsets = vec![0u64; size];
        let mut references = vec![0u64; size];
        for (i, slot) in subsets.iter_mut().enumerate() {
            *slot = index_to_subset(i, mask);
            references[i] = sliding_attacks(sq, *slot, dirs);
        }

        let offset = table.len();
        table.resize(offset + size, 0);

        // Search for a magic that yields a collision-free (or constructively
        // consistent) mapping.
        let magic = loop {
            let candidate = rng.sparse();
            // Heuristic reject: needs enough high bits set after multiply.
            if (mask.wrapping_mul(candidate) >> 56).count_ones() < 6 {
                continue;
            }
            let mut used = vec![u64::MAX; size];
            let mut ok = true;
            for i in 0..size {
                let idx = ((subsets[i].wrapping_mul(candidate)) >> shift) as usize;
                if used[idx] == u64::MAX {
                    used[idx] = references[i];
                } else if used[idx] != references[i] {
                    ok = false;
                    break;
                }
            }
            if ok {
                for (i, slot) in used.iter().enumerate() {
                    table[offset + i] = if *slot == u64::MAX { 0 } else { *slot };
                }
                break candidate;
            }
        };

        magics.push(Magic {
            mask,
            magic,
            shift,
            offset,
        });
    }
    magics
}

impl Tables {
    fn build() -> Tables {
        let mut pawn = [[0u64; 64]; 2];
        let mut knight = [0u64; 64];
        let mut king = [0u64; 64];

        for s in 0..64u8 {
            let sq = Square(s);
            let bb = Bitboard::from_square(sq);
            // Pawn attacks.
            pawn[Color::White.index()][s as usize] =
                (bb.shift_north().shift_east() | bb.shift_north().shift_west()).0;
            pawn[Color::Black.index()][s as usize] =
                (bb.shift_south().shift_east() | bb.shift_south().shift_west()).0;

            // Knight attacks.
            let (f, r) = (sq.file() as i8, sq.rank() as i8);
            let kn = [
                (1, 2),
                (2, 1),
                (2, -1),
                (1, -2),
                (-1, -2),
                (-2, -1),
                (-2, 1),
                (-1, 2),
            ];
            let mut kb = 0u64;
            for (df, dr) in kn {
                if on_board(f + df, r + dr) {
                    kb |= 1u64 << ((r + dr) * 8 + (f + df));
                }
            }
            knight[s as usize] = kb;

            // King attacks.
            let mut kgb = 0u64;
            for df in -1..=1 {
                for dr in -1..=1 {
                    if (df != 0 || dr != 0) && on_board(f + df, r + dr) {
                        kgb |= 1u64 << ((r + dr) * 8 + (f + df));
                    }
                }
            }
            king[s as usize] = kgb;
        }

        let mut rng = Rng(0x00C0_FFEE_D00D_1234);
        let mut bishop_table = Vec::new();
        let mut rook_table = Vec::new();
        let bishop_magics = build_magics(&BISHOP_DIRS, &mut bishop_table, &mut rng);
        let rook_magics = build_magics(&ROOK_DIRS, &mut rook_table, &mut rng);

        // between / line tables, computed by explicit colinear stepping.
        let mut between = [[0u64; 64]; 64];
        let mut line = [[0u64; 64]; 64];
        for a in 0..64usize {
            let (af, ar) = ((a & 7) as i8, (a >> 3) as i8);
            for b in 0..64usize {
                if a == b {
                    continue;
                }
                let (bf, br) = ((b & 7) as i8, (b >> 3) as i8);
                let (ddf, ddr) = (bf - af, br - ar);
                // Aligned iff same file, same rank, or same diagonal.
                let aligned = ddf == 0 || ddr == 0 || ddf.abs() == ddr.abs();
                if !aligned {
                    continue;
                }
                let step_f = ddf.signum();
                let step_r = ddr.signum();

                // Squares strictly between a and b.
                let mut bt = 0u64;
                let (mut f, mut r) = (af + step_f, ar + step_r);
                while (f, r) != (bf, br) {
                    bt |= 1u64 << (r * 8 + f);
                    f += step_f;
                    r += step_r;
                }
                between[a][b] = bt;

                // Full board line through a and b (both endpoints included).
                let mut ln = (1u64 << a) | (1u64 << b);
                let (mut f, mut r) = (af + step_f, ar + step_r);
                while on_board(f, r) {
                    ln |= 1u64 << (r * 8 + f);
                    f += step_f;
                    r += step_r;
                }
                let (mut f, mut r) = (af - step_f, ar - step_r);
                while on_board(f, r) {
                    ln |= 1u64 << (r * 8 + f);
                    f -= step_f;
                    r -= step_r;
                }
                line[a][b] = ln;
            }
        }

        Tables {
            pawn,
            knight,
            king,
            between,
            line,
            bishop_magics,
            rook_magics,
            bishop_table,
            rook_table,
        }
    }
}
