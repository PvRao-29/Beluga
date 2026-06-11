//! Beluga UCI front-end.
//!
//! Implements the UCI subset required for tournament play plus `bench`, `perft`,
//! `eval`, and `d` helper commands. The search runs on a background thread so
//! `stop` is honored promptly.

use beluga_core::eval;
use beluga_core::perft;
use beluga_core::position::{Position, START_FEN};
use beluga_core::search::{Heuristics, Search, SearchInfo, MATE, MATE_IN_MAX};
use beluga_core::timeman::Limits;
use beluga_core::tt::Tt;
use std::io::{self, BufRead, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;

const NAME: &str = "Beluga";
const VERSION: &str = env!("CARGO_PKG_VERSION");
const AUTHORS: &str = "Beluga authors";

const DEFAULT_HASH_MB: usize = 16;
const MIN_HASH_MB: usize = 1;
const MAX_HASH_MB: usize = 65536;
const DEFAULT_MOVE_OVERHEAD_MS: u64 = 30;

struct Engine {
    pos: Position,
    tt: Arc<Tt>,
    stop: Arc<AtomicBool>,
    /// The manager thread returns the heuristic tables when it finishes so
    /// killers/history persist across moves of the same game.
    worker: Option<JoinHandle<Heuristics>>,
    heur: Option<Heuristics>,
    hash_mb: usize,
    threads: usize,
    move_overhead_ms: u64,
}

impl Engine {
    fn new() -> Engine {
        beluga_core::attacks::init();
        Engine {
            pos: Position::startpos(),
            tt: Arc::new(Tt::new(DEFAULT_HASH_MB)),
            stop: Arc::new(AtomicBool::new(false)),
            worker: None,
            heur: None,
            hash_mb: DEFAULT_HASH_MB,
            threads: 1,
            move_overhead_ms: DEFAULT_MOVE_OVERHEAD_MS,
        }
    }

    fn join_worker(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.worker.take() {
            if let Ok(heur) = h.join() {
                self.heur = Some(heur);
            }
        }
    }

    fn handle(&mut self, line: &str) -> bool {
        let mut it = line.split_whitespace();
        let cmd = match it.next() {
            Some(c) => c,
            None => return true,
        };
        match cmd {
            "uci" => self.cmd_uci(),
            "isready" => println!("readyok"),
            "ucinewgame" => self.cmd_newgame(),
            "setoption" => self.cmd_setoption(line),
            "position" => self.cmd_position(line),
            "go" => self.cmd_go(line),
            "stop" => self.join_worker(),
            "ponderhit" => {}
            "d" | "display" => println!("{}", self.pos),
            "eval" => println!("{}", eval::trace(&self.pos)),
            "perft" => self.cmd_perft(&mut it),
            "bench" => self.cmd_bench(&mut it),
            "quit" => {
                self.join_worker();
                return false;
            }
            _ => {}
        }
        io::stdout().flush().ok();
        true
    }

    fn cmd_uci(&self) {
        println!("id name {NAME} {VERSION}");
        println!("id author {AUTHORS}");
        println!(
            "option name Hash type spin default {DEFAULT_HASH_MB} min {MIN_HASH_MB} max {MAX_HASH_MB}"
        );
        println!("option name Threads type spin default 1 min 1 max 256");
        println!("option name Clear Hash type button");
        println!("option name Move Overhead type spin default {DEFAULT_MOVE_OVERHEAD_MS} min 0 max 5000");
        println!("uciok");
    }

    fn cmd_newgame(&mut self) {
        self.join_worker();
        self.tt.clear();
        self.heur = None;
        self.pos = Position::startpos();
    }

    fn cmd_setoption(&mut self, line: &str) {
        // setoption name <Name...> [value <Value>]
        let lower = line.to_ascii_lowercase();
        let name = extract_between(&lower, "name", "value")
            .unwrap_or_default()
            .trim()
            .to_string();
        let value = extract_after(&lower, "value")
            .unwrap_or_default()
            .trim()
            .to_string();
        match name.as_str() {
            "hash" => {
                if let Ok(mb) = value.parse::<usize>() {
                    self.join_worker();
                    self.hash_mb = mb.clamp(MIN_HASH_MB, MAX_HASH_MB);
                    self.tt = Arc::new(Tt::new(self.hash_mb));
                }
            }
            "threads" => {
                if let Ok(n) = value.parse::<usize>() {
                    self.join_worker();
                    self.threads = n.clamp(1, 256);
                }
            }
            "clear hash" => {
                self.join_worker();
                self.tt.clear();
            }
            "move overhead" => {
                if let Ok(ms) = value.parse::<u64>() {
                    self.move_overhead_ms = ms.clamp(0, 5000);
                }
            }
            _ => {}
        }
    }

    fn cmd_position(&mut self, line: &str) {
        self.join_worker();
        let rest = line.trim_start_matches("position").trim();
        let (fen, moves) = if let Some(after) = rest.strip_prefix("startpos") {
            (START_FEN.to_string(), after.trim())
        } else if let Some(after) = rest.strip_prefix("fen") {
            let after = after.trim();
            // FEN is 6 space-separated fields; "moves" terminates it.
            match after.find(" moves ") {
                Some(idx) => (after[..idx].trim().to_string(), &after[idx + 7..]),
                None => (after.to_string(), ""),
            }
        } else {
            (START_FEN.to_string(), "")
        };

        let mut pos = match Position::from_fen(&fen) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("info string bad fen: {e}");
                return;
            }
        };

        let moves = moves.trim().strip_prefix("moves").unwrap_or(moves).trim();
        for tok in moves.split_whitespace() {
            match pos.parse_uci_move(tok) {
                Some(m) => pos.make_move(m),
                None => {
                    eprintln!("info string illegal move in position: {tok}");
                    break;
                }
            }
        }
        self.pos = pos;
    }

    fn cmd_go(&mut self, line: &str) {
        self.join_worker();
        let mut limits = parse_go(line);
        limits.move_overhead_ms = self.move_overhead_ms;

        self.stop.store(false, Ordering::Relaxed);
        let main_pos = self.pos.clone();
        let tt = Arc::clone(&self.tt);
        let stop = Arc::clone(&self.stop);
        let threads = self.threads;
        let heur = self.heur.take();

        // The "manager" thread owns the main search (which reports and returns
        // the best move) and the Lazy-SMP helper threads. Helpers share the TT
        // and the stop flag; they search silently to deepen the shared table.
        self.worker = Some(std::thread::spawn(move || {
            let mut helpers = Vec::new();
            for _ in 1..threads {
                let mut hpos = main_pos.clone();
                let htt = Arc::clone(&tt);
                let hstop = Arc::clone(&stop);
                let hlimits = limits.clone();
                helpers.push(std::thread::spawn(move || {
                    let tt_ref: &Tt = &htt;
                    let mut s = Search::new(&mut hpos, tt_ref, hstop, hlimits);
                    s.think();
                }));
            }

            let mut pos = main_pos;
            let tt_ref: &Tt = &tt;
            let mut search = Search::new(&mut pos, tt_ref, Arc::clone(&stop), limits);
            if let Some(h) = heur {
                search.set_heuristics(h);
            }
            search.set_info_callback(Box::new(|info: &SearchInfo| {
                print_info(info);
            }));
            let best = search.think();

            // Stop the helpers and collect them before announcing the move.
            stop.store(true, Ordering::Relaxed);
            for h in helpers {
                let _ = h.join();
            }

            println!("bestmove {}", best.to_uci());
            io::stdout().flush().ok();
            search.take_heuristics()
        }));
    }

    fn cmd_perft(&mut self, it: &mut std::str::SplitWhitespace) {
        let depth: u32 = it.next().and_then(|s| s.parse().ok()).unwrap_or(5);
        let start = std::time::Instant::now();
        let nodes = perft::perft(&mut self.pos, depth);
        let secs = start.elapsed().as_secs_f64();
        println!(
            "perft({depth}) = {nodes}  ({:.0} nps)",
            nodes as f64 / secs.max(1e-9)
        );
    }

    fn cmd_bench(&mut self, it: &mut std::str::SplitWhitespace) {
        let depth: u32 = it
            .next()
            .and_then(|s| s.parse().ok())
            .unwrap_or(BENCH_DEPTH);
        run_bench(depth, self.hash_mb, self.move_overhead_ms);
    }
}

fn print_info(info: &SearchInfo) {
    let nps = (info.nodes * 1000)
        .checked_div(info.time_ms)
        .unwrap_or(info.nodes);
    let score = format_score(info.score);
    let pv: Vec<String> = info.pv.iter().map(|m| m.to_uci()).collect();
    println!(
        "info depth {} seldepth {} score {} nodes {} nps {} time {} hashfull {} pv {}",
        info.depth,
        info.seldepth,
        score,
        info.nodes,
        nps,
        info.time_ms,
        info.hashfull,
        pv.join(" ")
    );
    io::stdout().flush().ok();
}

fn format_score(score: i32) -> String {
    if score >= MATE_IN_MAX {
        format!("mate {}", (MATE - score + 1) / 2)
    } else if score <= -MATE_IN_MAX {
        format!("mate {}", -((MATE + score + 1) / 2))
    } else {
        format!("cp {score}")
    }
}

fn parse_go(line: &str) -> Limits {
    let mut l = Limits::default();
    let toks: Vec<&str> = line.split_whitespace().collect();
    let mut i = 1; // skip "go"
    while i < toks.len() {
        let next = toks.get(i + 1).copied();
        match toks[i] {
            "depth" => {
                l.depth = next.and_then(|s| s.parse().ok());
                i += 1;
            }
            "nodes" => {
                l.nodes = next.and_then(|s| s.parse().ok());
                i += 1;
            }
            "movetime" => {
                l.movetime = next.and_then(|s| s.parse().ok());
                i += 1;
            }
            "wtime" => {
                l.wtime = next.and_then(|s| s.parse().ok());
                i += 1;
            }
            "btime" => {
                l.btime = next.and_then(|s| s.parse().ok());
                i += 1;
            }
            "winc" => {
                l.winc = next.and_then(|s| s.parse().ok());
                i += 1;
            }
            "binc" => {
                l.binc = next.and_then(|s| s.parse().ok());
                i += 1;
            }
            "movestogo" => {
                l.movestogo = next.and_then(|s| s.parse().ok());
                i += 1;
            }
            "infinite" => l.infinite = true,
            _ => {}
        }
        i += 1;
    }
    l
}

// -- bench -------------------------------------------------------------------

const BENCH_DEPTH: u32 = 11;

const BENCH_FENS: &[&str] = &[
    "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
    "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
    "8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1",
    "r4rk1/1pp1qppp/p1np1n2/2b1p1B1/2B1P1b1/P1NP1N2/1PP1QPPP/R4RK1 w - - 0 10",
    "4rrk1/pp1n3p/3q2pQ/2p1pb2/2PP4/2P3N1/P2B2PP/4RRK1 b - - 7 19",
    "r3r1k1/2p2ppp/p1p1bn2/8/1q2P3/2NPQN2/PPP3PP/R4RK1 b - - 2 15",
    "2rqkb1r/ppp2p2/2npb1p1/1N1Nn2p/2P1PP2/8/PP2B1PP/R1BQK2R b KQ - 0 11",
    "8/8/4k3/8/4P3/4K3/8/8 w - - 0 1",
    "6k1/5ppp/8/8/8/8/5PPP/3R2K1 w - - 0 1",
    "r1bqk2r/pppp1ppp/2n2n2/2b1p3/2B1P3/3P1N2/PPP2PPP/RNBQK2R w KQkq - 0 1",
];

fn run_bench(depth: u32, hash_mb: usize, move_overhead_ms: u64) {
    beluga_core::attacks::init();
    let tt = Tt::new(hash_mb);
    let stop = Arc::new(AtomicBool::new(false));
    let mut total_nodes = 0u64;
    let start = std::time::Instant::now();

    for fen in BENCH_FENS {
        tt.clear();
        let mut pos = Position::from_fen(fen).expect("valid bench fen");
        let limits = Limits {
            depth: Some(depth),
            move_overhead_ms,
            ..Default::default()
        };
        let mut search = Search::new(&mut pos, &tt, Arc::clone(&stop), limits);
        search.think();
        total_nodes += search.nodes();
    }

    let secs = start.elapsed().as_secs_f64();
    let nps = (total_nodes as f64 / secs.max(1e-9)) as u64;
    println!("===========================");
    println!("Total time (ms) : {}", (secs * 1000.0) as u64);
    println!("Nodes searched  : {total_nodes}");
    println!("Nodes/second    : {nps}");
}

// -- helpers -----------------------------------------------------------------

fn extract_between(s: &str, start: &str, end: &str) -> Option<String> {
    let si = s.find(start)? + start.len();
    let tail = &s[si..];
    let ei = tail.find(end).unwrap_or(tail.len());
    Some(tail[..ei].to_string())
}

fn extract_after(s: &str, key: &str) -> Option<String> {
    let si = s.find(key)? + key.len();
    Some(s[si..].to_string())
}

fn main() {
    // Allow `beluga bench [depth]` as a CLI invocation (used by CI and OpenBench).
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("bench") {
        let depth = args
            .get(1)
            .and_then(|s| s.parse().ok())
            .unwrap_or(BENCH_DEPTH);
        run_bench(depth, DEFAULT_HASH_MB, DEFAULT_MOVE_OVERHEAD_MS);
        return;
    }

    let mut engine = Engine::new();
    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        if !engine.handle(line.trim()) {
            break;
        }
    }
}
