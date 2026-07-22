//! Time management: derive soft/hard limits from UCI `go` parameters.
//!
//! * **soft limit** — do not *start* a new iterative-deepening iteration past it.
//! * **hard limit** — abort the search immediately (checked on a node mask).

use crate::types::Color;

#[cfg(target_arch = "wasm32")]
use web_time::{Duration, Instant};
#[cfg(not(target_arch = "wasm32"))]
use std::time::{Duration, Instant};

/// Parsed `go` limits.
#[derive(Clone)]
pub struct Limits {
    pub depth: Option<u32>,
    pub nodes: Option<u64>,
    pub movetime: Option<u64>,
    pub wtime: Option<u64>,
    pub btime: Option<u64>,
    pub winc: Option<u64>,
    pub binc: Option<u64>,
    pub movestogo: Option<u32>,
    pub infinite: bool,
    /// Reserved safety margin (ms) subtracted from the clock budget.
    pub move_overhead_ms: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            depth: None,
            nodes: None,
            movetime: None,
            wtime: None,
            btime: None,
            winc: None,
            binc: None,
            movestogo: None,
            infinite: false,
            move_overhead_ms: 30,
        }
    }
}

pub struct TimeManager {
    start: Instant,
    soft: Option<Duration>,
    hard: Option<Duration>,
}

impl TimeManager {
    /// `fullmove` is the game's fullmove number; sudden-death allocation uses
    /// it to model how many moves are still ahead.
    pub fn new(limits: &Limits, stm: Color, fullmove: u32) -> TimeManager {
        let start = Instant::now();

        if limits.infinite
            || (limits.movetime.is_none() && limits.wtime.is_none() && limits.btime.is_none())
        {
            // Depth/nodes/infinite searches have no wall-clock budget.
            return TimeManager {
                start,
                soft: None,
                hard: None,
            };
        }

        let overhead = limits.move_overhead_ms;

        if let Some(mt) = limits.movetime {
            let budget = mt.saturating_sub(overhead).max(1);
            let d = Duration::from_millis(budget);
            return TimeManager {
                start,
                soft: Some(d),
                hard: Some(d),
            };
        }

        let (time, inc) = match stm {
            Color::White => (limits.wtime.unwrap_or(0), limits.winc.unwrap_or(0)),
            Color::Black => (limits.btime.unwrap_or(0), limits.binc.unwrap_or(0)),
        };

        let total = time.saturating_sub(overhead);
        let (soft, hard) = match limits.movestogo {
            Some(mtg) => {
                // Cap mtg so a distant time control doesn't starve every move,
                // and keep a cushion so the moves just before the control are
                // never played on an empty clock.
                let mtg = (mtg.max(1) as u64).min(40);
                let soft = (total / mtg + inc * 3 / 4).min(total * 8 / 10).max(1);
                let hard = ((soft * 4).min(total * 8 / 10)).max(soft);
                (soft, hard)
            }
            None => {
                // Sudden death / increment: model the remaining game length
                // from the move number — long horizon in the opening (bank
                // time while moves are easy), shorter from the middlegame on
                // where decisions are hardest. The hard ceiling stays wide so
                // critical moves (fail-lows, instability) can run long.
                let horizon = 55u64.saturating_sub(fullmove as u64 * 2).clamp(24, 50);
                let soft = (total / horizon + inc * 3 / 4).min(total * 8 / 10).max(1);
                let hard = ((soft * 5).min(total * 8 / 10)).max(soft);
                (soft, hard)
            }
        };

        TimeManager {
            start,
            soft: Some(Duration::from_millis(soft)),
            hard: Some(Duration::from_millis(hard)),
        }
    }

    /// True when the search runs against a wall clock (movetime or game clock).
    #[inline]
    pub fn has_clock(&self) -> bool {
        self.soft.is_some()
    }

    #[inline]
    pub fn elapsed(&self) -> Duration {
        self.start.elapsed()
    }

    #[inline]
    pub fn elapsed_ms(&self) -> u64 {
        self.start.elapsed().as_millis() as u64
    }

    /// Should we abort right now (hard limit reached)?
    #[inline]
    pub fn hard_expired(&self) -> bool {
        match self.hard {
            Some(h) => self.start.elapsed() >= h,
            None => false,
        }
    }

    /// Should we avoid starting another deepening iteration (soft limit)?
    #[inline]
    pub fn soft_expired(&self) -> bool {
        self.soft_expired_scaled(100)
    }

    /// Soft-limit check with the budget scaled to `pct` percent. The search
    /// shrinks the budget when the best move is stable across iterations and
    /// grows it on aspiration fail-lows / best-move churn.
    #[inline]
    pub fn soft_expired_scaled(&self, pct: u64) -> bool {
        match self.soft {
            Some(s) => self.start.elapsed() >= s * pct as u32 / 100,
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits() -> Limits {
        Limits {
            wtime: Some(60_000),
            winc: Some(1_000),
            ..Default::default()
        }
    }

    #[test]
    fn budget_is_a_sane_fraction_of_the_clock() {
        let tm = TimeManager::new(&limits(), Color::White, 1);
        let soft = tm.soft.unwrap().as_millis() as u64;
        let hard = tm.hard.unwrap().as_millis() as u64;
        assert!(soft >= 1 && soft <= 60_000 * 8 / 10, "soft = {soft}");
        assert!(hard >= soft && hard <= 60_000 * 8 / 10, "hard = {hard}");
    }

    #[test]
    fn movestogo_divides_the_clock() {
        let mut l = limits();
        l.movestogo = Some(10);
        l.winc = Some(0);
        let tm = TimeManager::new(&l, Color::White, 1);
        let soft = tm.soft.unwrap().as_millis() as u64;
        // 60s - 30ms overhead over 10 moves ≈ 6s per move.
        assert!((5_000..=6_500).contains(&soft), "soft = {soft}");
    }

    #[test]
    fn opening_moves_get_less_time_than_middlegame() {
        let mut l = limits();
        l.winc = Some(0);
        let opening = TimeManager::new(&l, Color::White, 1);
        let middlegame = TimeManager::new(&l, Color::White, 25);
        let s_open = opening.soft.unwrap().as_millis() as u64;
        let s_mid = middlegame.soft.unwrap().as_millis() as u64;
        assert!(s_open < s_mid, "open = {s_open}, mid = {s_mid}");
        // Horizon is clamped: very late moves are no greedier than move 25.
        let late = TimeManager::new(&l, Color::White, 60);
        let s_late = late.soft.unwrap().as_millis() as u64;
        assert_eq!(s_mid, s_late);
    }

    #[test]
    fn depth_only_search_has_no_clock() {
        let l = Limits {
            depth: Some(10),
            ..Default::default()
        };
        let tm = TimeManager::new(&l, Color::White, 1);
        assert!(tm.soft.is_none() && tm.hard.is_none());
        assert!(!tm.soft_expired() && !tm.hard_expired());
        assert!(!tm.has_clock());
    }

    #[test]
    fn movetime_has_a_clock() {
        let l = Limits {
            movetime: Some(1_000),
            ..Default::default()
        };
        let tm = TimeManager::new(&l, Color::White, 1);
        assert!(tm.has_clock());
    }
}
