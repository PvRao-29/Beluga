//! Time management: derive soft/hard limits from UCI `go` parameters.
//!
//! * **soft limit** — do not *start* a new iterative-deepening iteration past it.
//! * **hard limit** — abort the search immediately (checked on a node mask).

use crate::types::Color;
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
    pub fn new(limits: &Limits, stm: Color) -> TimeManager {
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
        let mtg = limits.movestogo.map(|m| m.max(1)).unwrap_or(30) as u64;

        let soft = (total / mtg + inc * 3 / 4).min(total * 8 / 10).max(1);
        let hard = ((soft * 4).min(total * 8 / 10)).max(soft);

        TimeManager {
            start,
            soft: Some(Duration::from_millis(soft)),
            hard: Some(Duration::from_millis(hard)),
        }
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
        match self.soft {
            Some(s) => self.start.elapsed() >= s,
            None => false,
        }
    }
}
