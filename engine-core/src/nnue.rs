//! NNUE evaluation architecture (Phase 5 scaffolding).
//!
//! This module implements the *structure* of an efficiently-updatable neural
//! network evaluation — HalfKP-style features, a two-perspective accumulator with
//! incremental add/remove, clipped-ReLU + integer dot-product inference, and file
//! loading — together with a validation test that the incremental accumulator
//! matches a from-scratch refresh.
//!
//! Honesty note: **no trained network ships with this repository.** Without a
//! loaded net, [`evaluate`] returns `None` and the engine falls back to the
//! handcrafted evaluation. The feature indexing here is self-consistent (suitable
//! for nets trained against *this* layout); it is not byte-compatible with
//! Stockfish nets.

use crate::position::Position;
use crate::types::{Color, PieceType};

pub const HIDDEN: usize = 128;
/// 10 non-king (color, type) classes × 64 squares, bucketed by king square.
pub const FEATURES_PER_BUCKET: usize = 10 * 64;
pub const INPUT: usize = 64 * FEATURES_PER_BUCKET;

const QA: i32 = 255; // accumulator clipped-ReLU ceiling
const SCALE: i32 = 400; // output scaling to centipawns
const OUTPUT_SHIFT: i32 = 6; // fixed-point shift for output weights

/// Quantized network parameters.
pub struct Network {
    /// `INPUT × HIDDEN` feature transformer weights (column-major by feature).
    feature_weights: Vec<i16>,
    feature_bias: Vec<i16>,
    /// `2 × HIDDEN` output weights (side-to-move perspective first).
    output_weights: Vec<i16>,
    output_bias: i32,
}

/// Two-perspective accumulator, updated incrementally on make/unmake.
#[derive(Clone)]
pub struct Accumulator {
    pub v: [[i16; HIDDEN]; 2],
}

impl Default for Accumulator {
    fn default() -> Self {
        Accumulator {
            v: [[0; HIDDEN]; 2],
        }
    }
}

#[inline]
fn orient(perspective: Color, sq: usize) -> usize {
    if perspective == Color::White {
        sq
    } else {
        sq ^ 56
    }
}

/// HalfKP-style feature index for `piece` of `color` on `sq`, from the point of
/// view of `perspective` whose king is on `king_sq`. Kings are not encoded as
/// feature pieces (they define the bucket instead).
#[inline]
pub fn feature_index(
    perspective: Color,
    king_sq: usize,
    color: Color,
    pt: PieceType,
    sq: usize,
) -> Option<usize> {
    if pt == PieceType::King {
        return None;
    }
    let rel_color = usize::from(color != perspective);
    let p = rel_color * 5 + pt.index();
    let oksq = orient(perspective, king_sq);
    let osq = orient(perspective, sq);
    Some(oksq * FEATURES_PER_BUCKET + p * 64 + osq)
}

impl Network {
    /// Load a quantized network from a flat little-endian file laid out as
    /// `feature_weights | feature_bias | output_weights | output_bias`.
    pub fn load(path: &str) -> std::io::Result<Network> {
        let bytes = std::fs::read(path)?;
        Self::from_bytes(&bytes).ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "nnue: bad file size/format",
            )
        })
    }

    pub fn from_bytes(bytes: &[u8]) -> Option<Network> {
        let n_fw = INPUT * HIDDEN;
        let n_fb = HIDDEN;
        let n_ow = 2 * HIDDEN;
        let expected = (n_fw + n_fb + n_ow) * 2 + 4;
        if bytes.len() != expected {
            return None;
        }
        let mut off = 0;
        let read_i16 = |b: &[u8], o: &mut usize| -> i16 {
            let v = i16::from_le_bytes([b[*o], b[*o + 1]]);
            *o += 2;
            v
        };
        let mut feature_weights = vec![0i16; n_fw];
        for w in feature_weights.iter_mut() {
            *w = read_i16(bytes, &mut off);
        }
        let mut feature_bias = vec![0i16; n_fb];
        for w in feature_bias.iter_mut() {
            *w = read_i16(bytes, &mut off);
        }
        let mut output_weights = vec![0i16; n_ow];
        for w in output_weights.iter_mut() {
            *w = read_i16(bytes, &mut off);
        }
        let output_bias =
            i32::from_le_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]]);
        Some(Network {
            feature_weights,
            feature_bias,
            output_weights,
            output_bias,
        })
    }

    #[inline]
    fn feature_col(&self, idx: usize) -> &[i16] {
        &self.feature_weights[idx * HIDDEN..(idx + 1) * HIDDEN]
    }

    /// Recompute both perspectives from scratch.
    pub fn refresh(&self, pos: &Position, acc: &mut Accumulator) {
        for p in 0..2 {
            acc.v[p].copy_from_slice(&self.feature_bias[..]);
        }
        let wk = pos.king_square(Color::White).index();
        let bk = pos.king_square(Color::Black).index();
        for color in [Color::White, Color::Black] {
            for pt in PieceType::ALL {
                let mut bb = pos.pieces(color, pt);
                while bb.any() {
                    let sq = bb.pop_lsb().index();
                    if let Some(i) = feature_index(Color::White, wk, color, pt, sq) {
                        add_col(&mut acc.v[0], self.feature_col(i));
                    }
                    if let Some(i) = feature_index(Color::Black, bk, color, pt, sq) {
                        add_col(&mut acc.v[1], self.feature_col(i));
                    }
                }
            }
        }
    }

    /// Incrementally add a piece's contribution to both perspectives.
    pub fn add_piece(
        &self,
        acc: &mut Accumulator,
        wk: usize,
        bk: usize,
        color: Color,
        pt: PieceType,
        sq: usize,
    ) {
        if let Some(i) = feature_index(Color::White, wk, color, pt, sq) {
            add_col(&mut acc.v[0], self.feature_col(i));
        }
        if let Some(i) = feature_index(Color::Black, bk, color, pt, sq) {
            add_col(&mut acc.v[1], self.feature_col(i));
        }
    }

    /// Incrementally remove a piece's contribution from both perspectives.
    pub fn remove_piece(
        &self,
        acc: &mut Accumulator,
        wk: usize,
        bk: usize,
        color: Color,
        pt: PieceType,
        sq: usize,
    ) {
        if let Some(i) = feature_index(Color::White, wk, color, pt, sq) {
            sub_col(&mut acc.v[0], self.feature_col(i));
        }
        if let Some(i) = feature_index(Color::Black, bk, color, pt, sq) {
            sub_col(&mut acc.v[1], self.feature_col(i));
        }
    }

    /// Forward pass to a centipawn score from `stm`'s perspective.
    pub fn forward(&self, acc: &Accumulator, stm: Color) -> i32 {
        let (us, them) = (stm.index(), stm.flip().index());
        let mut sum: i32 = 0;
        for i in 0..HIDDEN {
            sum += crelu(acc.v[us][i]) * self.output_weights[i] as i32;
            sum += crelu(acc.v[them][i]) * self.output_weights[HIDDEN + i] as i32;
        }
        ((sum >> OUTPUT_SHIFT) + self.output_bias) * SCALE / (QA * 64)
    }
}

#[inline]
fn crelu(x: i16) -> i32 {
    (x as i32).clamp(0, QA)
}

#[inline]
fn add_col(acc: &mut [i16; HIDDEN], col: &[i16]) {
    for i in 0..HIDDEN {
        acc[i] = acc[i].wrapping_add(col[i]);
    }
}

#[inline]
fn sub_col(acc: &mut [i16; HIDDEN], col: &[i16]) {
    for i in 0..HIDDEN {
        acc[i] = acc[i].wrapping_sub(col[i]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a deterministic pseudo-random network purely to validate the
    /// accumulator math (NOT a trained net — values are meaningless for strength).
    fn synthetic() -> Network {
        let mut state = 0x1234_5678u64;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 48) as i16 / 64
        };
        let n_fw = INPUT * HIDDEN;
        Network {
            feature_weights: (0..n_fw).map(|_| next()).collect(),
            feature_bias: (0..HIDDEN).map(|_| next()).collect(),
            output_weights: (0..2 * HIDDEN).map(|_| next()).collect(),
            output_bias: 0,
        }
    }

    #[test]
    fn incremental_matches_refresh() {
        crate::attacks::init();
        let net = synthetic();

        // Start position, refreshed.
        let mut pos = Position::startpos();
        let mut inc = Accumulator::default();
        net.refresh(&pos, &mut inc);

        // Apply a non-king move incrementally and compare to a full refresh.
        let m = pos.parse_uci_move("e2e4").unwrap();
        let wk = pos.king_square(Color::White).index();
        let bk = pos.king_square(Color::Black).index();
        // Pawn e2 -> e4: remove from e2, add at e4.
        net.remove_piece(&mut inc, wk, bk, Color::White, PieceType::Pawn, 12);
        net.add_piece(&mut inc, wk, bk, Color::White, PieceType::Pawn, 28);

        pos.make_move(m);
        let mut full = Accumulator::default();
        net.refresh(&pos, &mut full);

        assert_eq!(
            inc.v, full.v,
            "incremental accumulator must equal full refresh"
        );
    }

    #[test]
    fn load_rejects_wrong_size() {
        assert!(Network::from_bytes(&[0u8; 10]).is_none());
    }
}
