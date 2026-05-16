//! Texel-style evaluation tuning harness (objective + K fitting).
//!
//! This computes the Texel objective for Beluga's current static evaluation over
//! a labeled dataset and fits the sigmoid scaling constant `K`. It validates the
//! tuning data pipeline (the format produced by `scripts/gen_training_data.py`)
//! and the objective. Parameter-mutation/descent hooks are the documented next
//! step (the eval terms must be lifted from `const` to a tunable struct first;
//! see `docs/TUNING.md` and `docs/ROADMAP.md`).
//!
//! Usage: beluga-tune <data.jsonl>
//!   where each line is {"fen": "...", "result": 0.0|0.5|1.0, ...}

use beluga_core::eval;
use beluga_core::position::Position;
use beluga_core::types::Color;

struct Sample {
    eval_white_cp: i32,
    result: f64,
}

fn parse_field<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let pat = format!("\"{key}\"");
    let i = line.find(&pat)? + pat.len();
    let rest = line[i..].trim_start_matches([':', ' ']);
    Some(rest)
}

fn load(path: &str) -> Vec<Sample> {
    beluga_core::attacks::init();
    let text = std::fs::read_to_string(path).expect("read dataset");
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let fen = match parse_field(line, "fen") {
            Some(s) => s.trim_start_matches('"').split('"').next().unwrap_or(""),
            None => continue,
        };
        let result: f64 = match parse_field(line, "result") {
            Some(s) => s
                .split([',', '}'])
                .next()
                .and_then(|v| v.trim().parse().ok())
                .unwrap_or(0.5),
            None => 0.5,
        };
        if let Ok(pos) = Position::from_fen(fen) {
            // Convert side-to-move eval to White's perspective.
            let stm_eval = eval::evaluate(&pos);
            let white_cp = if pos.side_to_move() == Color::White {
                stm_eval
            } else {
                -stm_eval
            };
            out.push(Sample {
                eval_white_cp: white_cp,
                result,
            });
        }
    }
    out
}

fn error(samples: &[Sample], k: f64) -> f64 {
    let mut sum = 0.0;
    for s in samples {
        let sig = 1.0 / (1.0 + 10f64.powf(-k * s.eval_white_cp as f64 / 400.0));
        let d = s.result - sig;
        sum += d * d;
    }
    sum / samples.len().max(1) as f64
}

/// Ternary-search the K that minimizes the Texel error (the objective is unimodal
/// in K for fixed eval).
fn fit_k(samples: &[Sample]) -> (f64, f64) {
    let (mut lo, mut hi) = (0.0f64, 4.0f64);
    for _ in 0..60 {
        let m1 = lo + (hi - lo) / 3.0;
        let m2 = hi - (hi - lo) / 3.0;
        if error(samples, m1) < error(samples, m2) {
            hi = m2;
        } else {
            lo = m1;
        }
    }
    let k = (lo + hi) / 2.0;
    (k, error(samples, k))
}

fn main() {
    let path = match std::env::args().nth(1) {
        Some(p) => p,
        None => {
            eprintln!("usage: beluga-tune <data.jsonl>  (see docs/TUNING.md)");
            std::process::exit(2);
        }
    };
    let samples = load(&path);
    if samples.is_empty() {
        eprintln!("no usable samples in {path}");
        std::process::exit(1);
    }
    let baseline = error(&samples, 1.0);
    let (k, err) = fit_k(&samples);
    println!("samples      : {}", samples.len());
    println!("error @ K=1.0: {baseline:.6}");
    println!("best K       : {k:.4}");
    println!("error @ bestK: {err:.6}");
    println!("(lower error = eval better predicts results; tune terms to reduce it)");
}
