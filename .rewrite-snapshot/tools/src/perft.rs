//! Perft driver: `perft [depth] [fen]` or `perft divide [depth] [fen]`.

use beluga_core::perft;
use beluga_core::position::Position;
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    beluga_core::attacks::init();

    let (divide, rest) = match args.first().map(String::as_str) {
        Some("divide") => (true, &args[1..]),
        _ => (false, &args[..]),
    };

    let depth: u32 = rest.first().and_then(|s| s.parse().ok()).unwrap_or(6);
    let fen = if rest.len() > 1 {
        rest[1..].join(" ")
    } else {
        beluga_core::position::START_FEN.to_string()
    };

    let mut pos = match Position::from_fen(&fen) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("bad FEN: {e}");
            std::process::exit(1);
        }
    };

    let start = Instant::now();
    if divide {
        let results = perft::perft_divide(&mut pos, depth);
        let mut total = 0u64;
        for (m, n) in &results {
            println!("{m}: {n}");
            total += n;
        }
        println!("\nNodes: {total}");
        report(total, start.elapsed().as_secs_f64());
    } else {
        let nodes = perft::perft(&mut pos, depth);
        println!("perft({depth}) = {nodes}");
        report(nodes, start.elapsed().as_secs_f64());
    }
}

fn report(nodes: u64, secs: f64) {
    let nps = nodes as f64 / secs.max(1e-9);
    println!("time: {secs:.3}s  nps: {:.2}M", nps / 1e6);
}
