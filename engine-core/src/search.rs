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

/// Correction-history table size (entries per side, power of two).
const CORR_SIZE: usize = 16384;
/// Correction values are stored scaled by this grain.
const CORR_GRAIN: i32 = 256;
/// Maximum stored correction: ±32 cp.
const CORR_MAX: i32 = 32 * CORR_GRAIN;

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

/// Heuristic tables for move ordering. Owned by the search but transferable
/// across searches (see [`Search::set_heuristics`]) so history persists
/// between moves of the same game; reset on `ucinewgame`.
pub struct Heuristics {
    killers: [[Move; 2]; MAX_PLY],
    history: Box<[[[i32; 64]; 64]; 2]>,
    counters: [[Move; 64]; 12],
    // Continuation history: [prev_piece][prev_to][cur_piece][cur_to], one
    // table per ply offset (1 = counter context, 2 = follow-up context).
    conthist: [Box<[[[[i16; 64]; 12]; 64]; 12]>; 2],
    // Capture history: [moving_piece][to][captured_piece_type].
    capthist: Box<[[[i16; 6]; 64]; 12]>,
    // Static-eval correction by [side][pawn-structure key], in CORR_GRAIN units.
    corrhist: Box<[[i32; CORR_SIZE]; 2]>,
}

impl Heuristics {
    pub fn new() -> Heuristics {
        Heuristics {
            killers: [[Move::NULL; 2]; MAX_PLY],
            history: Box::new([[[0; 64]; 64]; 2]),
            counters: [[Move::NULL; 64]; 12],
            conthist: [
                Box::new([[[[0; 64]; 12]; 64]; 12]),
                Box::new([[[[0; 64]; 12]; 64]; 12]),
            ],
            capthist: Box::new([[[0; 6]; 64]; 12]),
            corrhist: Box::new([[0; CORR_SIZE]; 2]),
        }
    }

    fn clear(&mut self) {
        self.killers = [[Move::NULL; 2]; MAX_PLY];
        *self.history = [[[0; 64]; 64]; 2];
        self.counters = [[Move::NULL; 64]; 12];
        for t in &mut self.conthist {
            **t = [[[[0; 64]; 12]; 64]; 12];
        }
        *self.capthist = [[[0; 6]; 64]; 12];
        *self.corrhist = [[0; CORR_SIZE]; 2];
    }
}

/// Per-ply search stack data.
#[derive(Clone, Copy)]
struct Stack {
    eval: i32,
    moved_piece: u8,
    current_move: Move,
    /// Move excluded at this ply by a singular-extension verification search.
    excluded: Move,
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
    /// True if the current iteration's aspiration window failed low at root.
    iter_fail_low: bool,

    heur: Heuristics,
    stack: [Stack; MAX_PLY + 4],
    pv_table: Box<[[Move; MAX_PLY]; MAX_PLY]>,
    pv_len: [usize; MAX_PLY],

    lmr: [[i32; 64]; 64],

    /// While non-zero, null moves are disabled below this ply (zugzwang
    /// verification re-search after a null-move fail-high).
    nmp_min_ply: i32,

    root_best: Move,
    root_score: i32,
    /// Nodes spent under each root move (indexed from/to), cumulative over
    /// the whole search. The best move's share measures decision "effort":
    /// a dominant share means an easy move that needs less of the budget.
    root_nodes: Box<[[u64; 64]; 64]>,

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
        let fullmove = pos.fullmove_number() as u32;
        let tm = TimeManager::new(&limits, stm, fullmove);
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
            iter_fail_low: false,
            heur: Heuristics::new(),
            stack: [Stack {
                eval: 0,
                moved_piece: 0,
                current_move: Move::NULL,
                excluded: Move::NULL,
            }; MAX_PLY + 4],
            pv_table: Box::new([[Move::NULL; MAX_PLY]; MAX_PLY]),
            pv_len: [0; MAX_PLY],
            lmr,
            nmp_min_ply: 0,
            root_best: Move::NULL,
            root_score: 0,
            root_nodes: Box::new([[0; 64]; 64]),
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
        let single_reply = fallback.len() == 1;

        let max_depth = self
            .limits
            .depth
            .unwrap_or(MAX_PLY as u32 - 1)
            .min(MAX_PLY as u32 - 1);
        let mut last_score = 0;
        let mut stability = 0u32;

        for depth in 1..=max_depth {
            self.iter_fail_low = false;
            let score = self.aspiration(depth as i32, last_score);
            if self.stopped {
                break;
            }
            let prev_score = last_score;
            last_score = score;
            let prev_best = best_move;
            best_move = self.pv_table[0][0];
            self.root_best = best_move;
            self.root_score = score;

            self.report(depth, score);

            // A forced move needs no deepening on a clock: bank the time.
            if single_reply && self.tm.has_clock() {
                break;
            }

            // Soft-limit budget scaling. Four signals adjust how much of the
            // soft budget this move may spend:
            // * best-move stability across iterations (stable → spend less),
            // * the best move's share of all nodes (a dominant share means
            //   the decision is easy → spend less; a contested root → more),
            // * a score that fell since the previous iteration (→ more),
            // * a root aspiration fail-low this iteration (→ more).
            if best_move == prev_best {
                stability = (stability + 1).min(8);
            } else {
                stability = 0;
            }
            const STABILITY_PCT: [u64; 9] = [140, 120, 110, 100, 95, 90, 85, 82, 80];
            let mut budget_pct = STABILITY_PCT[stability as usize];

            if depth >= 8 && self.nodes > 0 {
                let bn = self.root_nodes[best_move.from().index()][best_move.to().index()];
                let effort_pct = (100 * bn / self.nodes).min(100);
                let node_pct = (130 - effort_pct * 6 / 10).clamp(75, 130);
                budget_pct = budget_pct * node_pct / 100;
            }

            if depth >= 6 && score < prev_score {
                let drop = ((prev_score - score) as u64).min(50);
                budget_pct = budget_pct * (100 + drop) / 100;
            }
            if self.iter_fail_low {
                budget_pct = budget_pct * 130 / 100;
            }
            budget_pct = budget_pct.clamp(40, 300);

            if self.tm.soft_expired_scaled(budget_pct) {
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
                self.iter_fail_low = true;
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

        // Move excluded by a singular verification search at this node; while
        // set, TT cutoffs/stores and whole-node pruning are disabled because
        // the searched move set is not the full move set of the position.
        let excluded = self.stack[ply as usize].excluded;

        let key = self.pos.key();
        let tt_hit = self.tt.probe(key);
        let mut tt_move = Move::NULL;
        let mut tt_eval = None;
        if let Some(h) = tt_hit {
            tt_move = h.mv;
            tt_eval = Some(h.eval);
            let tt_score = score_from_tt(h.score, ply);
            if !is_pv && excluded.is_null() && h.depth >= depth {
                match h.bound {
                    Bound::Exact => return tt_score,
                    Bound::Lower if tt_score >= beta => return tt_score,
                    Bound::Upper if tt_score <= alpha => return tt_score,
                    _ => {}
                }
            }
        }

        // Static eval (skipped while in check). The TT stores the *raw* eval;
        // correction history nudges it toward past search results for the
        // same pawn structure before it feeds any pruning decision.
        let raw_eval = if in_check {
            -INFINITY
        } else {
            tt_eval.unwrap_or_else(|| eval::evaluate(self.pos))
        };
        let static_eval = if in_check {
            raw_eval
        } else {
            (raw_eval + self.correction()).clamp(-MATE_IN_MAX + 1, MATE_IN_MAX - 1)
        };
        self.stack[ply as usize].eval = static_eval;

        let improving = !in_check && ply >= 2 && static_eval > self.stack[(ply - 2) as usize].eval;

        // Whole-node pruning (non-PV, not in check, no immediate mate threat).
        if !is_pv && !in_check && excluded.is_null() && beta.abs() < MATE_IN_MAX {
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
                && (self.nmp_min_ply == 0 || ply >= self.nmp_min_ply)
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
                    let score = if score >= MATE_IN_MAX { beta } else { score };
                    // At high depth, verify the fail-high with a reduced
                    // null-free search to protect against zugzwang.
                    if depth < 10 || self.nmp_min_ply != 0 {
                        return score;
                    }
                    self.nmp_min_ply = ply + 3 * (depth - r) / 4;
                    let v = self.negamax(depth - 1 - r, beta - 1, beta, ply, false);
                    self.nmp_min_ply = 0;
                    if self.stopped {
                        return 0;
                    }
                    if v >= beta {
                        return score;
                    }
                }
            }

            // Probcut: if a shallow search of a strong capture already beats
            // beta by a safety margin, trust the cutoff at this depth.
            let probcut_beta = beta + 200;
            let tt_blocks_probcut = match tt_hit {
                Some(h) => h.depth >= depth - 3 && score_from_tt(h.score, ply) < probcut_beta,
                None => false,
            };
            if depth >= 6 && !tt_blocks_probcut {
                let mut clist = MoveList::new();
                movegen::generate_captures(self.pos, &mut clist);
                self.score_moves_qs(&mut clist, tt_move);
                for i in 0..clist.len() {
                    let m = clist.pick_best(i);
                    // The capture must be able to lift eval above the bar.
                    if !see::see_ge(self.pos, m, probcut_beta - static_eval) {
                        continue;
                    }
                    let moving_piece = self.pos.piece_on(m.from()).map(|p| p.0).unwrap_or(0);
                    self.stack[ply as usize].current_move = m;
                    self.stack[ply as usize].moved_piece = moving_piece;
                    self.pos.make_move(m);
                    let mut s = -self.qsearch(-probcut_beta, -probcut_beta + 1, ply + 1);
                    if s >= probcut_beta {
                        s = -self.negamax(
                            depth - 4,
                            -probcut_beta,
                            -probcut_beta + 1,
                            ply + 1,
                            false,
                        );
                    }
                    self.pos.unmake_move(m);
                    if self.stopped {
                        return 0;
                    }
                    if s >= probcut_beta {
                        self.tt.store(
                            key,
                            m,
                            score_to_tt(s, ply),
                            raw_eval,
                            depth - 3,
                            Bound::Lower,
                        );
                        return s;
                    }
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
        let mut caps: [Move; 32] = [Move::NULL; 32];
        let mut cap_count = 0usize;

        for i in 0..list.len() {
            let m = list.pick_best(i);
            if m == excluded {
                continue;
            }
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

            // Singular extension: if the TT move fails high in a reduced
            // null-window search that *excludes* it, no other move comes
            // close — extend it. If even the rest of the moves beat beta, the
            // node is a multi-cut fail-high and we can return early.
            let mut sing_ext = 0;
            if !root && depth >= 8 && m == tt_move && excluded.is_null() {
                if let Some(h) = tt_hit {
                    let tt_score = score_from_tt(h.score, ply);
                    if h.depth >= depth - 3
                        && matches!(h.bound, Bound::Lower | Bound::Exact)
                        && tt_score.abs() < MATE_IN_MAX
                    {
                        let sing_beta = tt_score - 2 * depth;
                        self.stack[ply as usize].excluded = m;
                        let s = self.negamax((depth - 1) / 2, sing_beta - 1, sing_beta, ply, false);
                        self.stack[ply as usize].excluded = Move::NULL;
                        if self.stopped {
                            return 0;
                        }
                        if s < sing_beta {
                            sing_ext = 1;
                        } else if sing_beta >= beta {
                            return sing_beta;
                        }
                    }
                }
            }

            let moving_piece = self.pos.piece_on(m.from()).map(|p| p.0).unwrap_or(0);
            self.stack[ply as usize].current_move = m;
            self.stack[ply as usize].moved_piece = moving_piece;

            let nodes_before = self.nodes;
            self.pos.make_move(m);
            let gives_check = self.pos.in_check();
            let ext = i32::from(gives_check).max(sing_ext);
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

            if root {
                self.root_nodes[m.from().index()][m.to().index()] += self.nodes - nodes_before;
            }

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
                        self.update_capture_stats(m, depth, &caps[..cap_count]);
                        break;
                    }
                }
            }

            if is_quiet && quiet_count < quiets.len() {
                quiets[quiet_count] = m;
                quiet_count += 1;
            } else if m.is_capture() && cap_count < caps.len() {
                caps[cap_count] = m;
                cap_count += 1;
            }
        }

        // With an excluded move, the move set can be empty without it being
        // mate/stalemate; report a fail-low to the verification search.
        if move_count == 0 {
            return alpha;
        }

        let bound = if best >= beta {
            Bound::Lower
        } else if best > orig_alpha {
            Bound::Exact
        } else {
            Bound::Upper
        };

        // Correction history: learn how far the raw static eval was off for
        // this pawn structure. Only when the result is usable as an eval
        // proxy: not in check, not a mate score, quiet (or no) best move, and
        // the bound does not contradict the direction of the adjustment.
        if !in_check
            && excluded.is_null()
            && best.abs() < MATE_IN_MAX
            && (best_move.is_null() || (!best_move.is_capture() && !best_move.is_promotion()))
            && !(bound == Bound::Lower && best <= static_eval)
            && !(bound == Bound::Upper && best >= static_eval)
        {
            self.update_correction(depth, best - raw_eval);
        }

        if excluded.is_null() {
            self.tt.store(
                key,
                best_move,
                score_to_tt(best, ply),
                raw_eval,
                depth,
                bound,
            );
        }

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

        // TT probe: qsearch entries are stored at depth 0, so any entry
        // satisfies the depth requirement; reuse score, eval, and move.
        let key = self.pos.key();
        let mut tt_move = Move::NULL;
        let mut tt_eval = None;
        if let Some(h) = self.tt.probe(key) {
            tt_move = h.mv;
            if !in_check {
                tt_eval = Some(h.eval);
            }
            let tt_score = score_from_tt(h.score, ply);
            match h.bound {
                Bound::Exact => return tt_score,
                Bound::Lower if tt_score >= beta => return tt_score,
                Bound::Upper if tt_score <= alpha => return tt_score,
                _ => {}
            }
        }

        let raw_eval;
        let mut best;
        if in_check {
            raw_eval = -INFINITY;
            best = -INFINITY;
        } else {
            raw_eval = tt_eval.unwrap_or_else(|| eval::evaluate(self.pos));
            best = (raw_eval + self.correction()).clamp(-MATE_IN_MAX + 1, MATE_IN_MAX - 1);
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
        self.score_moves_qs(&mut list, tt_move);

        let mut any = false;
        let mut best_move = Move::NULL;
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
                best_move = m;
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

        let bound = if best >= beta {
            Bound::Lower
        } else {
            Bound::Upper
        };
        self.tt
            .store(key, best_move, score_to_tt(best, ply), raw_eval, 0, bound);
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
                // Capture history refines MVV-LVA within a victim class (the
                // /8 keeps it from overriding the victim-value ordering).
                let mvvlva = self.mvv_lva(m) + self.capture_history(m) / 8;
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

    fn score_moves_qs(&mut self, list: &mut MoveList, tt_move: Move) {
        for i in 0..list.len() {
            let m = list.get(i);
            let s = if m == tt_move {
                1 << 24
            } else if m.is_capture() || m.is_promotion() {
                self.mvv_lva(m) + self.capture_history(m) / 8
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
        let cur_pt = self
            .pos
            .piece_on(m.from())
            .map(|p| p.0 as usize)
            .unwrap_or(0);
        for (i, off) in [1i32, 2].into_iter().enumerate() {
            if ply >= off {
                let pe = self.stack[(ply - off) as usize];
                if !pe.current_move.is_null() {
                    let prev_pt = pe.moved_piece as usize;
                    let prev_to = pe.current_move.to().index();
                    h += self.heur.conthist[i][prev_pt][prev_to][cur_pt][m.to().index()] as i32;
                }
            }
        }
        h
    }

    /// Current static-eval correction (cp) for the side to move, keyed by
    /// pawn structure.
    #[inline]
    fn correction(&self) -> i32 {
        let stm = self.pos.side_to_move().index();
        let idx = (self.pos.pawn_key() as usize) & (CORR_SIZE - 1);
        self.heur.corrhist[stm][idx] / CORR_GRAIN
    }

    fn update_correction(&mut self, depth: i32, diff: i32) {
        let stm = self.pos.side_to_move().index();
        let idx = (self.pos.pawn_key() as usize) & (CORR_SIZE - 1);
        let e = &mut self.heur.corrhist[stm][idx];
        let w = (depth + 1).min(16);
        let v = (*e * (CORR_GRAIN - w) + diff * CORR_GRAIN * w) / CORR_GRAIN;
        *e = v.clamp(-CORR_MAX, CORR_MAX);
    }

    /// The captured piece type for capture-history indexing (EP = pawn).
    #[inline]
    fn captured_type(&self, m: Move) -> usize {
        if m.is_en_passant() {
            PieceType::Pawn.index()
        } else {
            self.pos
                .piece_on(m.to())
                .map(|p| p.piece_type().index())
                .unwrap_or(PieceType::Pawn.index())
        }
    }

    #[inline]
    fn capture_history(&self, m: Move) -> i32 {
        let piece = self
            .pos
            .piece_on(m.from())
            .map(|p| p.index())
            .unwrap_or(0);
        self.heur.capthist[piece][m.to().index()][self.captured_type(m)] as i32
    }

    fn update_capture_stats(&mut self, best: Move, depth: i32, tried: &[Move]) {
        let bonus = (depth * depth).min(1600);
        if best.is_capture() {
            self.update_capture_history(best, bonus);
        }
        for &c in tried {
            if c != best {
                self.update_capture_history(c, -bonus);
            }
        }
    }

    fn update_capture_history(&mut self, m: Move, bonus: i32) {
        let piece = self
            .pos
            .piece_on(m.from())
            .map(|p| p.index())
            .unwrap_or(0);
        let e = &mut self.heur.capthist[piece][m.to().index()][if m.is_en_passant() {
            PieceType::Pawn.index()
        } else {
            // The board state is pre-move here, so the victim is still on `to`.
            match self.pos.piece_on(m.to()) {
                Some(p) => p.piece_type().index(),
                None => PieceType::Pawn.index(),
            }
        }];
        let cur = *e as i32;
        let nv = cur + bonus - cur * bonus.abs() / 16384;
        *e = nv.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
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

        let cur_pt = self
            .pos
            .piece_on(m.from())
            .map(|p| p.0 as usize)
            .unwrap_or(0);
        for (i, off) in [1i32, 2].into_iter().enumerate() {
            if ply >= off {
                let pe = self.stack[(ply - off) as usize];
                if !pe.current_move.is_null() {
                    let prev_pt = pe.moved_piece as usize;
                    let prev_to = pe.current_move.to().index();
                    let ce = &mut self.heur.conthist[i][prev_pt][prev_to][cur_pt][to];
                    let cur = *ce as i32;
                    let nv = cur + bonus - cur * bonus.abs() / 16384;
                    *ce = nv.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
                }
            }
        }
    }

    /// Reset heuristics between games (`ucinewgame`).
    pub fn clear_heuristics(&mut self) {
        self.heur.clear();
    }

    /// Adopt heuristic tables carried over from a previous search of the same
    /// game, so killers/history/counters keep working across moves.
    pub fn set_heuristics(&mut self, heur: Heuristics) {
        self.heur = heur;
    }

    /// Hand the heuristic tables back to the caller for the next search.
    pub fn take_heuristics(self) -> Heuristics {
        self.heur
    }
}

impl Default for Heuristics {
    fn default() -> Self {
        Heuristics::new()
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
