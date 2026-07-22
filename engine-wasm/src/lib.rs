//! Thin WASM API over `beluga-core` for browser play.
//!
//! Single-threaded only (no Lazy SMP). Prefer depth-limited `go` for predictable
//! latency; `go_movetime` works when the host provides wall-clock time.

use beluga_core::attacks;
use beluga_core::chess_move::MoveList;
use beluga_core::movegen;
use beluga_core::position::{Position, START_FEN};
use beluga_core::search::{Heuristics, Search};
use beluga_core::timeman::Limits;
use beluga_core::tt::Tt;
use beluga_core::types::Color;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct BelugaEngine {
    pos: Position,
    tt: Tt,
    heur: Option<Heuristics>,
    stop: Arc<AtomicBool>,
}

#[wasm_bindgen]
impl BelugaEngine {
    #[wasm_bindgen(constructor)]
    pub fn new(hash_mb: usize) -> BelugaEngine {
        console_error_panic_hook::set_once();
        attacks::init();
        let mb = hash_mb.clamp(1, 64);
        BelugaEngine {
            pos: Position::from_fen(START_FEN).expect("start fen"),
            tt: Tt::new(mb),
            heur: Some(Heuristics::new()),
            stop: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Reset to the standard starting position and clear search state.
    pub fn new_game(&mut self) {
        self.pos = Position::from_fen(START_FEN).expect("start fen");
        self.tt.clear();
        self.heur = Some(Heuristics::new());
        self.stop.store(false, Ordering::Relaxed);
    }

    /// Set position from a FEN string. Returns an error message on failure.
    pub fn set_fen(&mut self, fen: &str) -> Result<(), JsValue> {
        match Position::from_fen(fen) {
            Ok(pos) => {
                self.pos = pos;
                Ok(())
            }
            Err(e) => Err(JsValue::from_str(&e)),
        }
    }

    pub fn fen(&self) -> String {
        self.pos.to_fen()
    }

    /// `"w"` or `"b"`.
    pub fn side_to_move(&self) -> String {
        match self.pos.side_to_move() {
            Color::White => "w".into(),
            Color::Black => "b".into(),
        }
    }

    /// Space-separated UCI moves that are legal in the current position.
    pub fn legal_moves(&self) -> String {
        let mut list = MoveList::new();
        movegen::generate_legal(&self.pos, &mut list);
        let mut out = String::new();
        for (i, mv) in list.as_slice().iter().enumerate() {
            if i > 0 {
                out.push(' ');
            }
            out.push_str(&mv.to_uci());
        }
        out
    }

    /// Apply a UCI move. Returns false if illegal.
    pub fn make_move(&mut self, uci: &str) -> bool {
        let Some(mv) = self.pos.parse_uci_move(uci) else {
            return false;
        };
        let mut legal = MoveList::new();
        movegen::generate_legal(&self.pos, &mut legal);
        if !legal.contains(mv) {
            return false;
        }
        self.pos.make_move(mv);
        true
    }

    pub fn is_check(&self) -> bool {
        self.pos.in_check()
    }

    pub fn is_game_over(&self) -> bool {
        !self.result().is_empty()
    }

    /// `"checkmate" | "stalemate" | "draw" | ""` (empty if play continues).
    pub fn result(&self) -> String {
        let mut legal = MoveList::new();
        movegen::generate_legal(&self.pos, &mut legal);
        if legal.is_empty() {
            if self.pos.in_check() {
                return "checkmate".into();
            }
            return "stalemate".into();
        }
        if self.pos.is_repetition()
            || self.pos.is_fifty_move()
            || self.pos.is_insufficient_material()
        {
            return "draw".into();
        }
        String::new()
    }

    /// Search to a fixed depth (recommended for WASM). Returns best move in UCI.
    pub fn go_depth(&mut self, depth: u32) -> String {
        let mut limits = Limits::default();
        limits.depth = Some(depth.clamp(1, 64));
        self.search(limits)
    }

    /// Search for about `ms` milliseconds. Returns best move in UCI.
    pub fn go_movetime(&mut self, ms: u64) -> String {
        let mut limits = Limits::default();
        limits.movetime = Some(ms.max(50));
        limits.move_overhead_ms = 20;
        self.search(limits)
    }

    fn search(&mut self, limits: Limits) -> String {
        self.stop.store(false, Ordering::Relaxed);
        let mut pos = self.pos.clone();
        let mut search = Search::new(&mut pos, &self.tt, Arc::clone(&self.stop), limits);
        if let Some(h) = self.heur.take() {
            search.set_heuristics(h);
        }
        let best = search.think();
        self.heur = Some(search.take_heuristics());
        best.to_uci()
    }
}
