//! Shared transposition table.
//!
//! Entries are two `u64`s accessed atomically (relaxed). We store `key ^ data`
//! rather than the raw key so that a torn read under Lazy SMP fails validation
//! and is simply ignored — the classic lockless XOR trick. This keeps the table
//! correct under concurrent access without per-entry locking.

use crate::chess_move::Move;
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Bound {
    None = 0,
    Lower = 1, // fail-high: score is a lower bound (>= beta)
    Upper = 2, // fail-low: score is an upper bound (<= alpha)
    Exact = 3,
}

impl Bound {
    #[inline]
    fn from_bits(b: u8) -> Bound {
        match b & 3 {
            1 => Bound::Lower,
            2 => Bound::Upper,
            3 => Bound::Exact,
            _ => Bound::None,
        }
    }
}

#[derive(Clone, Copy)]
pub struct TtHit {
    pub mv: Move,
    pub score: i32,
    pub eval: i32,
    pub depth: i32,
    pub bound: Bound,
}

struct Entry {
    key: AtomicU64,
    data: AtomicU64,
}

impl Entry {
    const fn empty() -> Entry {
        Entry {
            key: AtomicU64::new(0),
            data: AtomicU64::new(0),
        }
    }
}

pub struct Tt {
    entries: Vec<Entry>,
    mask: usize,
    generation: AtomicU8,
}

#[inline]
fn pack(mv: Move, score: i16, eval: i16, depth: u8, bound: Bound, gen: u8) -> u64 {
    (mv.0 as u64)
        | ((score as u16 as u64) << 16)
        | ((eval as u16 as u64) << 32)
        | ((depth as u64) << 48)
        | (((bound as u64) & 3) << 56)
        | (((gen as u64) & 0x3f) << 58)
}

#[inline]
fn unpack_move(data: u64) -> Move {
    Move(data as u16)
}
#[inline]
fn unpack_score(data: u64) -> i32 {
    (data >> 16) as u16 as i16 as i32
}
#[inline]
fn unpack_eval(data: u64) -> i32 {
    (data >> 32) as u16 as i16 as i32
}
#[inline]
fn unpack_depth(data: u64) -> i32 {
    ((data >> 48) & 0xff) as i32
}
#[inline]
fn unpack_bound(data: u64) -> Bound {
    Bound::from_bits((data >> 56) as u8)
}
#[inline]
fn unpack_gen(data: u64) -> u8 {
    ((data >> 58) & 0x3f) as u8
}

impl Tt {
    /// Create a table of approximately `mb` megabytes (rounded down to a power
    /// of two number of 16-byte entries).
    pub fn new(mb: usize) -> Tt {
        let mut tt = Tt {
            entries: Vec::new(),
            mask: 0,
            generation: AtomicU8::new(0),
        };
        tt.resize(mb);
        tt
    }

    pub fn resize(&mut self, mb: usize) {
        let bytes = mb.max(1) * 1024 * 1024;
        let mut n = bytes / std::mem::size_of::<Entry>();
        if n < 1024 {
            n = 1024;
        }
        n = n.next_power_of_two() / 2; // floor to power of two
        if n < 1 {
            n = 1;
        }
        let mut v = Vec::with_capacity(n);
        for _ in 0..n {
            v.push(Entry::empty());
        }
        self.entries = v;
        self.mask = n - 1;
    }

    pub fn clear(&self) {
        for e in &self.entries {
            e.key.store(0, Ordering::Relaxed);
            e.data.store(0, Ordering::Relaxed);
        }
    }

    /// Begin a new search generation (for aging-based replacement).
    pub fn new_generation(&self) {
        self.generation.fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    fn index(&self, key: u64) -> usize {
        (key as usize) & self.mask
    }

    pub fn probe(&self, key: u64) -> Option<TtHit> {
        let e = &self.entries[self.index(key)];
        let data = e.data.load(Ordering::Relaxed);
        let xkey = e.key.load(Ordering::Relaxed);
        if xkey ^ data != key {
            return None;
        }
        Some(TtHit {
            mv: unpack_move(data),
            score: unpack_score(data),
            eval: unpack_eval(data),
            depth: unpack_depth(data),
            bound: unpack_bound(data),
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn store(&self, key: u64, mv: Move, score: i32, eval: i32, depth: i32, bound: Bound) {
        let e = &self.entries[self.index(key)];
        let gen = self.generation.load(Ordering::Relaxed);

        let old_data = e.data.load(Ordering::Relaxed);
        let old_xkey = e.key.load(Ordering::Relaxed);
        let old_valid = old_xkey ^ old_data == key;

        // Depth-preferred with aging. Always keep a move if we have none.
        let mut mv = mv;
        if old_valid && mv.is_null() {
            mv = unpack_move(old_data);
        }

        if old_valid {
            let old_depth = unpack_depth(old_data);
            let same_pos = true; // validated above
            let _ = same_pos;
            // Replace if deeper, exact, or the entry is from an older search.
            let replace =
                bound == Bound::Exact || depth + 2 >= old_depth || unpack_gen(old_data) != gen;
            if !replace {
                return;
            }
        } else {
            // Different/empty slot: only protect very recent, deep entries.
            let occupied = old_data != 0;
            if occupied {
                let old_depth = unpack_depth(old_data);
                if unpack_gen(old_data) == gen && old_depth > depth + 4 {
                    return;
                }
            }
        }

        let s = score.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        let ev = eval.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        let d = depth.clamp(0, 255) as u8;
        let data = pack(mv, s, ev, d, bound, gen);
        e.key.store(key ^ data, Ordering::Relaxed);
        e.data.store(data, Ordering::Relaxed);
    }

    /// Approximate fill permille (sampled), for UCI `hashfull`.
    pub fn hashfull(&self) -> usize {
        let sample = 1000.min(self.entries.len());
        let mut used = 0usize;
        for e in self.entries.iter().take(sample) {
            if e.data.load(Ordering::Relaxed) != 0 {
                used += 1;
            }
        }
        (used * 1000).checked_div(sample).unwrap_or(0)
    }
}
