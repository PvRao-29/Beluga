//! Search: iterative deepening PVS with a transposition table, aspiration
//! windows, and the standard suite of sound reductions/prunings.
//!
//! Score conventions (centipawns, side-to-move relative):
//! * `MATE = 30000`; a mate in `n` plies scores `MATE - n`. TT store/probe
//!   normalize mate scores by `±ply` (see [`score_to_tt`]/[`score_from_tt`]).
//! * `INFINITY = 32000` is the alpha/beta sentinel.

use crate::chess_move::{Move, MoveList};
use crate::eval;
use crate::movegen;
use crate::position::Position;
use crate::see;
use crate::timeman::{Limits, TimeManager};
use crate::tt::{Bound, Tt};
use crate::types::{Color, PieceType};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub const MATE: i32 = 30000;
pub const INFINITY: i32 = 32000;
pub const MAX_PLY: usize = 128;
pub const MATE_IN_MAX: i32 = MATE - MAX_PLY as i32;

const NODES_CHECK_MASK: u64 = 2047;

/// Information reported after each completed iteration.
pub struct SearchInfo {
    pub depth: u32,
    pub seldepth: u32,
    pub score: i32,
    pub nodes: u64,
    pub time_ms: u64,
    pub pv: Vec<Move>,
    pub hashfull: usize,
}

/// Heuristic tables for move ordering (sized to be reusable across a search).
struct Heuristics {
    killers: [[Move; 2]; MAX_PLY],
    history: Box<[[[i32; 64]; 64]; 2]>,
    counters: [[Move; 64]; 12],
    // Continuation history: [prev_piece][prev_to][cur_piece][cur_to].
    conthist: Box<[[[[i16; 64]; 12]; 64]; 12]>,
}

impl Heuristics {
    fn new() -> Heuristics {
        Heuristics {
            killers: [[Move::NULL; 2]; MAX_PLY],
            history: Box::new([[[0; 64]; 64]; 2]),
            counters: [[Move::NULL; 64]; 12],
            conthist: Box::new([[[[0; 64]; 12]; 64]; 12]),
        }
    }

    fn clear(&mut self) {
        self.killers = [[Move::NULL; 2]; MAX_PLY];
        *self.history = [[[0; 64]; 64]; 2];
        self.counters = [[Move::NULL; 64]; 12];
        *self.conthist = [[[[0; 64]; 12]; 64]; 12];
    }
}

/// Per-ply search stack data.
#[derive(Clone, Copy)]
struct Stack {
    eval: i32,
    moved_piece: u8,
    current_move: Move,
}

pub struct Search<'a> {
    pos: &'a mut Position,
    tt: &'a Tt,
    stop: Arc<AtomicBool>,
    tm: TimeManager,
    limits: Limits,

    nodes: u64,
    seldepth: u32,
    stopped: bool,

    heur: Heuristics,
    stack: [Stack; MAX_PLY + 4],
    pv_table: Box<[[Move; MAX_PLY]; MAX_PLY]>,
    pv_len: [usize; MAX_PLY],

    lmr: [[i32; 64]; 64],

    root_best: Move,
    root_score: i32,

    /// Called once per completed depth with progress info.
    on_info: Option<InfoCallback<'a>>,
}

type InfoCallback<'a> = Box<dyn FnMut(&SearchInfo) + 'a>;

impl<'a> Search<'a> {
    pub fn new(
        pos: &'a mut Position,
        tt: &'a Tt,
        stop: Arc<AtomicBool>,
        limits: Limits,
    ) -> Search<'a> {
        let stm = pos.side_to_move();
        let tm = TimeManager::new(&limits, stm);
        let mut lmr = [[0i32; 64]; 64];
        for (d, row) in lmr.iter_mut().enumerate().skip(1) {
            for (m, slot) in row.iter_mut().enumerate().skip(1) {
                *slot = (0.75 + (d as f64).ln() * (m as f64).ln() / 2.25) as i32;
            }
        }
        Search {
            pos,
            tt,
            stop,
            tm,
            limits,
            nodes: 0,
            seldepth: 0,
            stopped: false,
            heur: Heuristics::new(),
            stack: [Stack {
                eval: 0,
                moved_piece: 0,
                current_move: Move::NULL,
            }; MAX_PLY + 4],
            pv_table: Box::new([[Move::NULL; MAX_PLY]; MAX_PLY]),
            pv_len: [0; MAX_PLY],
            lmr,
            root_best: Move::NULL,
            root_score: 0,
            on_info: None,
        }
    }

    /// Score (cp, side-to-move relative) of the last completed iteration.
    pub fn root_score(&self) -> i32 {
        self.root_score
    }

    pub fn set_info_callback(&mut self, cb: Box<dyn FnMut(&SearchInfo) + 'a>) {
        self.on_info = Some(cb);
    }

    pub fn nodes(&self) -> u64 {
        self.nodes
    }

    /// Run iterative deepening and return the best move found.
    pub fn think(&mut self) -> Move {
        self.tt.new_generation();

        // Fallback: guarantee a legal move even if we are stopped immediately.
        let mut fallback = MoveList::new();
        movegen::generate_legal(self.pos, &mut fallback);
        if fallback.is_empty() {
            return Move::NULL;
        }
        let mut best_move = fallback.get(0);

        let max_depth = self
            .limits
            .depth
            .unwrap_or(MAX_PLY as u32 - 1)
            .min(MAX_PLY as u32 - 1);
        let mut last_score = 0;

        for depth in 1..=max_depth {
            let score = self.aspiration(depth as i32, last_score);
            if self.stopped {
                break;
            }
            last_score = score;
            best_move = self.pv_table[0][0];
            self.root_best = best_move;
            self.root_score = score;

            self.report(depth, score);

            // Soft time and mate-found early exit.
            if self.tm.soft_expired() {
                break;
            }
            if score.abs() >= MATE_IN_MAX && depth >= 4 {
                break;
            }
            if let Some(n) = self.limits.nodes {
                if self.nodes >= n {
                    break;
                }
            }
        }

        if best_move.is_null() {
            best_move = fallback.get(0);
        }
        best_move
    }

    fn report(&mut self, depth: u32, score: i32) {
        let mut pv = Vec::new();
        for i in 0..self.pv_len[0] {
            pv.push(self.pv_table[0][i]);
        }
        if pv.is_empty() {
            pv.push(self.root_best);
        }
        let info = SearchInfo {
            depth,
            seldepth: self.seldepth,
            score,
            nodes: self.nodes,
            time_ms: self.tm.elapsed_ms(),
            pv,
            hashfull: self.tt.hashfull(),
        };
        if let Some(cb) = self.on_info.as_mut() {
            cb(&info);
        }
    }

    fn aspiration(&mut self, depth: i32, prev: i32) -> i32 {
        if depth <= 4 {
            return self.negamax(depth, -INFINITY, INFINITY, 0, true);
        }
        let mut delta = 18;
        let mut alpha = (prev - delta).max(-INFINITY);
        let mut beta = (prev + delta).min(INFINITY);
        loop {
            let score = self.negamax(depth, alpha, beta, 0, true);
            if self.stopped {
                return score;
            }
            if score <= alpha {
                beta = (alpha + beta) / 2;
                alpha = (score - delta).max(-INFINITY);
            } else if score >= beta {
                beta = (score + delta).min(INFINITY);
            } else {
                return score;
            }
            delta += delta / 2;
        }
    }

    #[inline]
    fn check_time(&mut self) {
        if self.stop.load(Ordering::Relaxed) || self.tm.hard_expired() {
            self.stopped = true;
        }
        if let Some(n) = self.limits.nodes {
            if self.nodes >= n {
                self.stopped = true;
            }
        }
    }

    fn negamax(
        &mut self,
        mut depth: i32,
        mut alpha: i32,
        mut beta: i32,
        ply: i32,
        is_pv: bool,
    ) -> i32 {
        self.pv_len[ply as usize] = 0;

        if depth <= 0 {
            return self.qsearch(alpha, beta, ply);
        }

        self.nodes += 1;
        if self.nodes & NODES_CHECK_MASK == 0 {
            self.check_time();
        }
        if self.stopped {
            return 0;
        }
        if ply as usize >= MAX_PLY - 1 {
            return eval::evaluate(self.pos);
        }
        self.seldepth = self.seldepth.max(ply as u32);

        let root = ply == 0;
        let in_check = self.pos.in_check();

        if !root {
            // Draw detection.
            if self.pos.is_repetition()
                || self.pos.is_fifty_move()
                || self.pos.is_insufficient_material()
            {
                return draw_score(self.nodes);
            }
            // Mate-distance pruning.
            alpha = alpha.max(-MATE + ply);
            beta = beta.min(MATE - ply - 1);
            if alpha >= beta {
                return alpha;
            }
        }

        let key = self.pos.key();
        let tt_hit = self.tt.probe(key);
        let mut tt_move = Move::NULL;
        let mut tt_eval = None;
        if let Some(h) = tt_hit {
            tt_move = h.mv;
            tt_eval = Some(h.eval);
            let tt_score = score_from_tt(h.score, ply);
            if !is_pv && h.depth >= depth {
                match h.bound {
                    Bound::Exact => return tt_score,
                    Bound::Lower if tt_score >= beta => return tt_score,
                    Bound::Upper if tt_score <= alpha => return tt_score,
                    _ => {}
                }
            }
        }

        // Static eval (skipped while in check).
        let static_eval = if in_check {
            -INFINITY
        } else {
            tt_eval.unwrap_or_else(|| eval::evaluate(self.pos))
        };
        self.stack[ply as usize].eval = static_eval;

        let improving = !in_check && ply >= 2 && static_eval > self.stack[(ply - 2) as usize].eval;

        // Whole-node pruning (non-PV, not in check, no immediate mate threat).
        if !is_pv && !in_check && beta.abs() < MATE_IN_MAX {
            // Reverse futility / static null move.
            if depth <= 8 && static_eval - 80 * depth >= beta {
                return static_eval;
            }
            // Razoring: hopeless node, verify with qsearch.
            if depth <= 3 && static_eval + 200 * depth < alpha {
                let v = self.qsearch(alpha, beta, ply);
                if v < alpha {
                    return v;
                }
            }
            // Null-move pruning (guarded against zugzwang).
            if depth >= 3
                && static_eval >= beta
                && self.pos.has_non_pawn_material(self.pos.side_to_move())
                && self.stack[(ply - 1).max(0) as usize].current_move != Move::NULL
            {
                let r = 3 + depth / 3 + ((static_eval - beta) / 200).min(3);
                self.stack[ply as usize].current_move = Move::NULL;
                self.pos.make_null_move();
                let score = -self.negamax(depth - 1 - r, -beta, -beta + 1, ply + 1, false);
                self.pos.unmake_null_move();
                if self.stopped {
                    return 0;
                }
                if score >= beta {
                    return if score >= MATE_IN_MAX { beta } else { score };
                }
            }
        }

        // Internal iterative reduction: no TT move at high depth → search shallower.
        if depth >= 4 && tt_move.is_null() {
            depth -= 1;
        }

        let mut list = MoveList::new();
        movegen::generate_legal(self.pos, &mut list);

        if list.is_empty() {
            return if in_check {
                -MATE + ply
            } else {
                draw_score(self.nodes)
            };
        }

        self.score_moves(&mut list, tt_move, ply);

        let orig_alpha = alpha;
        let mut best = -INFINITY;
        let mut best_move = Move::NULL;
        let mut move_count = 0i32;
        let mut quiets: [Move; 64] = [Move::NULL; 64];
        let mut quiet_count = 0usize;

        for i in 0..list.len() {
            let m = list.pick_best(i);
            let is_quiet = !m.is_capture() && !m.is_promotion();
            move_count += 1;

            // Late-move and futility pruning of quiet moves (sound: never in
            // check, never on PV first moves, never when a mate score is at risk).
            if !root && !is_pv && !in_check && best > -MATE_IN_MAX {
                if is_quiet {
                    let lmp = if improving {
                        4 + depth * depth
                    } else {
                        (4 + depth * depth) / 2
                    };
                    if depth <= 8 && move_count >= lmp {
                        continue;
                    }
                    if depth <= 6 && static_eval + 100 + 90 * depth <= alpha {
                        continue;
                    }
                } else if depth <= 6 && !see::see_ge(self.pos, m, -90 * depth) {
                    // Prune clearly losing captures at low depth.
                    continue;
                }
            }

            let moving_piece = self.pos.piece_on(m.from()).map(|p| p.0).unwrap_or(0);
            self.stack[ply as usize].current_move = m;
            self.stack[ply as usize].moved_piece = moving_piece;

            self.pos.make_move(m);
            let gives_check = self.pos.in_check();
            let ext = i32::from(gives_check);
            let new_depth = depth - 1 + ext;

            let score;
            if move_count == 1 {
                score = -self.negamax(new_depth, -beta, -alpha, ply + 1, is_pv);
            } else {
                let mut r = 0;
                if depth >= 3 && is_quiet && !gives_check {
                    r = self.lmr[(depth as usize).min(63)][(move_count as usize).min(63)];
                    if is_pv {
                        r -= 1;
                    }
                    if improving {
                        r -= 1;
                    }
                    r = r.clamp(0, new_depth.max(1) - 1);
                }
                let mut s = -self.negamax(new_depth - r, -alpha - 1, -alpha, ply + 1, false);
                if s > alpha && r > 0 {
                    s = -self.negamax(new_depth, -alpha - 1, -alpha, ply + 1, false);
                }
                if s > alpha && s < beta {
                    s = -self.negamax(new_depth, -beta, -alpha, ply + 1, true);
                }
                score = s;
            }

            self.pos.unmake_move(m);

            if self.stopped {
                return 0;
            }

            if score > best {
                best = score;
                best_move = m;
                if score > alpha {
                    alpha = score;
                    self.update_pv(ply, m);
                    if alpha >= beta {
                        if is_quiet {
                            self.update_quiet_stats(m, depth, ply, &quiets[..quiet_count]);
                        }
                        break;
                    }
                }
            }

            if is_quiet && quiet_count < quiets.len() {
                quiets[quiet_count] = m;
                quiet_count += 1;
            }
        }

        let bound = if best >= beta {
            Bound::Lower
        } else if best > orig_alpha {
            Bound::Exact
        } else {
            Bound::Upper
        };
        self.tt.store(
            key,
            best_move,
            score_to_tt(best, ply),
            static_eval,
            depth,
            bound,
        );

        best
    }

    fn qsearch(&mut self, mut alpha: i32, beta: i32, ply: i32) -> i32 {
        self.nodes += 1;
        if self.nodes & NODES_CHECK_MASK == 0 {
            self.check_time();
        }
        if self.stopped {
            return 0;
        }
        self.pv_len[ply as usize] = 0;
        self.seldepth = self.seldepth.max(ply as u32);

        if ply as usize >= MAX_PLY - 1 {
            return eval::evaluate(self.pos);
        }
        if self.pos.is_repetition()
            || self.pos.is_fifty_move()
            || self.pos.is_insufficient_material()
        {
            return draw_score(self.nodes);
        }

        let in_check = self.pos.in_check();
        let mut best;
        if in_check {
            best = -INFINITY;
        } else {
            best = eval::evaluate(self.pos);
            if best >= beta {
                return best;
            }
            if best > alpha {
                alpha = best;
            }
        }

        let mut list = MoveList::new();
        if in_check {
            movegen::generate_legal(self.pos, &mut list);
        } else {
            movegen::generate_captures(self.pos, &mut list);
        }
        self.score_moves_qs(&mut list);

        let mut any = false;
        for i in 0..list.len() {
            let m = list.pick_best(i);
            any = true;
            // Skip clearly losing captures when not in check.
            if !in_check && !m.is_promotion() && !see::see_ge(self.pos, m, 0) {
                continue;
            }
            self.pos.make_move(m);
            let score = -self.qsearch(-beta, -alpha, ply + 1);
            self.pos.unmake_move(m);
            if self.stopped {
                return 0;
            }
            if score > best {
                best = score;
                if score > alpha {
                    alpha = score;
                    self.update_pv(ply, m);
                    if alpha >= beta {
                        break;
                    }
                }
            }
        }

        if in_check && !any {
            return -MATE + ply;
        }
        best
    }

    #[inline]
    fn update_pv(&mut self, ply: i32, m: Move) {
        let p = ply as usize;
        self.pv_table[p][0] = m;
        let child = self.pv_len[p + 1];
        for i in 0..child {
            self.pv_table[p][i + 1] = self.pv_table[p + 1][i];
        }
        self.pv_len[p] = child + 1;
    }

    fn score_moves(&mut self, list: &mut MoveList, tt_move: Move, ply: i32) {
        let stm = self.pos.side_to_move();
        let counter = self.counter_move(ply);
        let killer0 = self.heur.killers[ply as usize][0];
        let killer1 = self.heur.killers[ply as usize][1];

        for i in 0..list.len() {
            let m = list.get(i);
            let s = if m == tt_move {
                1 << 24
            } else if m.is_capture() || m.is_promotion() {
                let mvvlva = self.mvv_lva(m);
                if see::see_ge(self.pos, m, 0) {
                    (1 << 20) + mvvlva
                } else {
                    mvvlva // bad capture: deferred below killers
                }
            } else if m == killer0 {
                1 << 19
            } else if m == killer1 {
                (1 << 19) - 1
            } else if m == counter {
                1 << 18
            } else {
                self.quiet_history(stm, ply, m)
            };
            list.set_score(i, s);
        }
    }

    fn score_moves_qs(&mut self, list: &mut MoveList) {
        for i in 0..list.len() {
            let m = list.get(i);
            let s = if m.is_capture() || m.is_promotion() {
                self.mvv_lva(m)
            } else {
                0
            };
            list.set_score(i, s);
        }
    }

    #[inline]
    fn mvv_lva(&self, m: Move) -> i32 {
        let victim = if m.is_en_passant() {
            PieceType::Pawn.index()
        } else {
            self.pos
                .piece_on(m.to())
                .map(|p| p.piece_type().index())
                .unwrap_or(0)
        };
        let attacker = self
            .pos
            .piece_on(m.from())
            .map(|p| p.piece_type().index())
            .unwrap_or(0);
        let promo = m
            .promotion()
            .map(|pt| see::SEE_VALUE[pt.index()])
            .unwrap_or(0);
        see::SEE_VALUE[victim] * 16 - see::SEE_VALUE[attacker] + promo
    }

    #[inline]
    fn quiet_history(&self, stm: Color, ply: i32, m: Move) -> i32 {
        let mut h = self.heur.history[stm.index()][m.from().index()][m.to().index()];
        if ply >= 1 {
            let pe = self.stack[(ply - 1) as usize];
            if !pe.current_move.is_null() {
                let prev_pt = pe.moved_piece as usize;
                let prev_to = pe.current_move.to().index();
                let cur_pt = self
                    .pos
                    .piece_on(m.from())
                    .map(|p| p.0 as usize)
                    .unwrap_or(0);
                h += self.heur.conthist[prev_pt][prev_to][cur_pt][m.to().index()] as i32;
            }
        }
        h
    }

    #[inline]
    fn counter_move(&self, ply: i32) -> Move {
        if ply >= 1 {
            let pe = self.stack[(ply - 1) as usize];
            if !pe.current_move.is_null() {
                return self.heur.counters[pe.moved_piece as usize][pe.current_move.to().index()];
            }
        }
        Move::NULL
    }

    fn update_quiet_stats(&mut self, best: Move, depth: i32, ply: i32, tried: &[Move]) {
        let stm = self.pos.side_to_move();
        let p = ply as usize;

        // Killers.
        if self.heur.killers[p][0] != best {
            self.heur.killers[p][1] = self.heur.killers[p][0];
            self.heur.killers[p][0] = best;
        }

        // Counter move.
        if ply >= 1 {
            let pe = self.stack[(ply - 1) as usize];
            if !pe.current_move.is_null() {
                self.heur.counters[pe.moved_piece as usize][pe.current_move.to().index()] = best;
            }
        }

        let bonus = (depth * depth).min(1600);

        // Reward the cutoff move, penalize the quiets that failed to cut.
        self.update_history(stm, ply, best, bonus);
        for &q in tried {
            if q != best {
                self.update_history(stm, ply, q, -bonus);
            }
        }
    }

    fn update_history(&mut self, stm: Color, ply: i32, m: Move, bonus: i32) {
        let from = m.from().index();
        let to = m.to().index();
        let e = &mut self.heur.history[stm.index()][from][to];
        *e += bonus - *e * bonus.abs() / 16384;

        if ply >= 1 {
            let pe = self.stack[(ply - 1) as usize];
            if !pe.current_move.is_null() {
                let prev_pt = pe.moved_piece as usize;
                let prev_to = pe.current_move.to().index();
                let cur_pt = self
                    .pos
                    .piece_on(m.from())
                    .map(|p| p.0 as usize)
                    .unwrap_or(0);
                let ce = &mut self.heur.conthist[prev_pt][prev_to][cur_pt][to];
                let cur = *ce as i32;
                let nv = cur + bonus - cur * bonus.abs() / 16384;
                *ce = nv.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
            }
        }
    }

    /// Reset heuristics between games (`ucinewgame`).
    pub fn clear_heuristics(&mut self) {
        self.heur.clear();
    }
}

#[inline]
fn score_to_tt(score: i32, ply: i32) -> i32 {
    if score >= MATE_IN_MAX {
        score + ply
    } else if score <= -MATE_IN_MAX {
        score - ply
    } else {
        score
    }
}

#[inline]
fn score_from_tt(score: i32, ply: i32) -> i32 {
    if score >= MATE_IN_MAX {
        score - ply
    } else if score <= -MATE_IN_MAX {
        score + ply
    } else {
        score
    }
}

/// Small randomized draw value near zero to discourage shuffling into draws and
/// to break ties between equal draw lines.
#[inline]
fn draw_score(nodes: u64) -> i32 {
    2 - (nodes & 3) as i32
}
