//! Standalone search-bench driver: fixed depth over a fixed position set,
//! reporting a deterministic node count and nps (single thread).

use beluga_core::position::Position;
use beluga_core::search::Search;
use beluga_core::timeman::Limits;
use beluga_core::tt::Tt;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Instant;

const FENS: &[&str] = &[
    "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
    "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
    "8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1",
    "r4rk1/1pp1qppp/p1np1n2/2b1p1B1/2B1P1b1/P1NP1N2/1PP1QPPP/R4RK1 w - - 0 10",
    "2rqkb1r/ppp2p2/2npb1p1/1N1Nn2p/2P1PP2/8/PP2B1PP/R1BQK2R b KQ - 0 11",
    "r1bqk2r/pppp1ppp/2n2n2/2b1p3/2B1P3/3P1N2/PPP2PPP/RNBQK2R w KQkq - 0 1",
];

fn main() {
    beluga_core::attacks::init();
    let depth: u32 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(12);

    let tt = Tt::new(16);
    let stop = Arc::new(AtomicBool::new(false));
    let mut nodes = 0u64;
    let start = Instant::now();
    for fen in FENS {
        tt.clear();
        let mut pos = Position::from_fen(fen).expect("valid fen");
        let limits = Limits {
            depth: Some(depth),
            ..Default::default()
        };
        let mut s = Search::new(&mut pos, &tt, Arc::clone(&stop), limits);
        s.think();
        nodes += s.nodes();
    }
    let secs = start.elapsed().as_secs_f64();
    println!(
        "bench depth {depth}: {nodes} nodes in {secs:.3}s ({:.2}M nps)",
        nodes as f64 / secs / 1e6
    );
}
