#!/usr/bin/env bash
# Full local CI sequence: format, lint, test, perft, bench.
set -euo pipefail
cd "$(dirname "$0")/.."

echo "== rustfmt =="
cargo fmt --all -- --check

echo "== clippy (deny warnings) =="
cargo clippy --workspace --all-targets -- -D warnings

echo "== build (release) =="
cargo build --workspace --release

echo "== tests (release) =="
cargo test --workspace --release

echo "== perft smoke (startpos d5) =="
cargo run --release -p beluga-tools --bin perft -- 5

echo "== bench =="
cargo run --release -p beluga-uci -- bench 12

echo "All checks passed."
