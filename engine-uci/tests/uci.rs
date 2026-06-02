//! Black-box UCI protocol tests: drive the real binary over stdin/stdout,
//! including malformed input and `stop` during search.

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Duration;

fn run(input: &str, wait: Duration) -> String {
    let mut child = Command::new(env!("CARGO_BIN_EXE_beluga"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn beluga");

    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    // Give the engine time to work before stdin closes (EOF -> exit).
    std::thread::sleep(wait);
    drop(child.stdin.take());

    let out = child.wait_with_output().expect("wait");
    String::from_utf8_lossy(&out.stdout).to_string()
}

#[test]
fn handshake_and_search() {
    let out = run(
        "uci\nisready\nposition startpos\ngo depth 8\n",
        Duration::from_millis(1500),
    );
    assert!(out.contains("id name Beluga"), "missing id: {out}");
    assert!(out.contains("uciok"), "missing uciok");
    assert!(out.contains("readyok"), "missing readyok");
    assert!(out.contains("bestmove "), "missing bestmove: {out}");
}

#[test]
fn malformed_input_does_not_crash() {
    let out = run(
        "garbage\nposition fen not a real fen\ngo\nsetoption name Nonexistent value 5\n\
         position startpos moves e2e4 notamove\nposition startpos\ngo depth 6\n",
        Duration::from_millis(1500),
    );
    // The engine must survive junk and still answer a legal search.
    assert!(
        out.contains("bestmove "),
        "engine should recover and move: {out}"
    );
}

#[test]
fn stop_during_search_returns_move() {
    // Infinite search interrupted by stop must still yield a bestmove promptly.
    let out = run(
        "position startpos\ngo infinite\nstop\n",
        Duration::from_millis(1200),
    );
    assert!(
        out.contains("bestmove "),
        "stop should produce a bestmove: {out}"
    );
}

#[test]
fn fen_position_with_moves() {
    let out = run(
        "position fen rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1 moves e2e4 e7e5\n\
         go depth 6\n",
        Duration::from_millis(1200),
    );
    assert!(out.contains("bestmove "), "fen+moves search failed: {out}");
}
