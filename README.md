# Beluga

**2880 ± 40 Elo based on local evaluations**

UCI chess engine written in Rust.

Beluga is a bitboard chess engine in a Cargo workspace. The `engine-core` library holds the board, move generation, search, and evaluation; the `beluga` binary speaks UCI. It runs in Arena, Cutechess, and other standard GUIs.

The search uses iterative deepening PVS with a transposition table, common pruning techniques, and Lazy SMP threading. Evaluation is a tapered handcrafted function built on PeSTO piece-square tables. Weights are not fully tuned yet.

---

## Build

Requires Rust 1.74+ (tested on 1.96).

```bash
cargo build --release
```

Binary: `target/release/beluga`

```bash
cargo run --release -p beluga-uci
```

Optional, for the CPU you are running on:

```bash
RUSTFLAGS="-C target-cpu=native" cargo build --release
```

---

## UCI

Standard commands:

```
uci
isready
ucinewgame
position startpos
position startpos moves e2e4 e7e5
position fen <fen> moves ...
go depth 14
go movetime 5000
go wtime 60000 btime 60000 winc 1000 binc 1000 movestogo 40
go infinite
stop
quit
```

Extra commands on stdin (not part of the UCI spec): `d` (print board), `eval` (evaluation trace), `perft <depth>`, `bench [depth]`.

CLI bench:

```bash
cargo run --release -p beluga-uci -- bench 12
```

### Options

| Option | Type | Default | Notes |
|--------|------|---------|-------|
| Hash | spin | 16 MB | 1–65536 |
| Threads | spin | 1 | Lazy SMP, 1–256 |
| Move Overhead | spin | 30 ms | 0–5000; clock safety margin |
| Clear Hash | button | — | Clears the transposition table |

---

## What is implemented

### Board

Twelve piece bitboards, color and occupied masks, and a 64-square mailbox. Squares use LERF indexing (`A1 = 0`, `H8 = 63`). Incremental 64-bit Zobrist hashing on make/unmake. FEN parse and emit with round-trip tests. Repetition detection over the key history within the fifty-move window.

### Attacks and move generation

Pawn, knight, and king attacks from lookup tables. Bishop, rook, and queen from magic bitboards (magics generated at startup with a fixed seed). `between` and `line` tables for pins and check blocking.

Legal moves are generated directly with check and pin masks. Castling verifies empty squares and that the king is not in, through, or into check. En passant uses a simulated occupancy test for pinned and discovered-check cases.

Moves are 16-bit packed (from, to, flags). Move lists are stack buffers of 256 entries with no heap allocation in movegen.

Perft matches published counts on startpos, Kiwipete, and CPW positions 3–6 through depth 5–6.

### Search

Iterative deepening negamax with PVS and aspiration windows. Quiescence search on captures and promotions; full legal generation in qsearch when in check.

Transposition table: lockless entries with XOR key validation, depth-preferred replacement, mate scores normalized by ply. TT move is used for ordering only.

Pruning and reduction: null move, LMR, reverse futility, razoring, futility, late-move pruning, SEE on captures, check extensions, internal iterative reduction, mate-distance pruning.

Move ordering: TT move, SEE/MVV-LVA captures, killers, counter moves, butterfly history, one-ply continuation history.

Draw handling for repetition, fifty-move rule, and insufficient material. Stop flag checked every 2048 nodes. Soft and hard time limits from `go` parameters (30 ms overhead is hardcoded in the time manager).

### Evaluation

Tapered midgame/endgame interpolation from material phase. PeSTO PSTs plus mobility, bishop pair, rook on open or semi-open files, passed pawns, and tempo. The `eval` command prints a short trace.

Static exchange evaluation (SEE) is implemented for capture ordering and pruning.

### Threading

Lazy SMP with a shared transposition table. The main thread reports search info; helper threads search cloned positions until the stop flag is set.

### NNUE (code only)

`engine-core/src/nnue.rs` defines a HalfKP-style layout (128 hidden, Beluga-specific feature indexing), a weight file loader, and incremental accumulator updates. Unit tests verify incremental updates against a full refresh. **Search still uses the handcrafted evaluator.** No trained network is included.

### Tools and scripts

| Path | Purpose |
|------|---------|
| `tools/` `perft`, `bench` | Standalone perft and search bench |
| `engine-tune/` | Texel loss and K fitting over a JSONL dataset |
| `scripts/gen_training_data.py` | Self-play data: `fen`, `eval_cp`, `result` |
| `scripts/selfplay.sh` | cutechess-cli match vs another engine |
| `scripts/sprt.sh` | SPRT comparison between two builds |
| `scripts/run_tests.sh` | fmt, clippy, tests, perft smoke, bench |
| `scripts/gui.sh` | Build Beluga, install En Croissant, register engine, launch GUI |
| `scripts/setup_en_croissant.sh` | Download En Croissant into `.local/` |
| `scripts/register_beluga_engine.py` | Add/update Beluga in En Croissant `engines.json` |
| `.local/bin/cutechess-cli` | Static macOS arm64 binary (optional) |
| `.local/en-croissant.app` | En Croissant GUI (downloaded, gitignored) |

Match scripts need `cutechess-cli` on `PATH` (see `.local/bin/` or build from [cutechess](https://github.com/cutechess/cutechess)).

### GUI (En Croissant)

Beluga has no built-in board UI. For interactive play and analysis, use [En Croissant](https://github.com/franciscoBSalgueiro/en-croissant):


```bash
./scripts/gui.sh
```

This builds `target/release/beluga`, downloads En Croissant v0.15.0 into `.local/`, registers Beluga in En Croissant’s engine list, and opens the app. Re-run after rebuilding the engine to refresh the binary path and version.

Options:

```bash
./scripts/gui.sh --no-build      # skip cargo build
./scripts/gui.sh --setup-only    # install + register, do not launch
```

---

## Repository layout

```
engine-core/     board, movegen, search, eval, tt, see, nnue, timeman
engine-uci/      beluga binary
engine-wasm/     WebAssembly bindings for browser play (`wasm-pack build`)
engine-tune/     Texel K-fit harness
tools/           perft, bench
scripts/         testing and match helpers
docs/            design notes
```

`engine-core` has no runtime dependencies. Release profile uses thin LTO and one codegen unit.

---

## Testing

```bash
cargo test --workspace --release
./scripts/run_tests.sh
```

Tests cover perft, make/unmake and Zobrist consistency, FEN round-trip, draw rules, castling legality, SEE, mate-in-one, tactical positions, and UCI protocol behavior (including malformed input and `stop` during search).

GitHub Actions runs `cargo fmt`, clippy (`-D warnings`), release build, full tests, perft depth 5, and `bench 11`.

---

## Performance

Measured on Apple Silicon (aarch64), Rust 1.96, `--release`. Numbers will differ on other hardware.

| Test | Result |
|------|--------|
| Perft startpos depth 6 | 119,060,324 nodes (correct) |
| Perft Kiwipete depth 5 | 193,690,690 nodes (correct) |
| `beluga bench 12` (1 thread, 10 positions) | ~1.65M nodes, ~5.7M nps |
| `beluga bench 13` (1 thread) | ~3.23M nodes, ~5.4M nps |

Perft uses bulk counting at depth 1, so perft nps is not comparable to search nps. Multithreaded `info nps` counts main-thread nodes only.

---

## Documentation

- [`docs/DESIGN.md`](docs/DESIGN.md)
- [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md)
- [`docs/TUNING.md`](docs/TUNING.md)
- [`docs/SELF_CRITIQUE.md`](docs/SELF_CRITIQUE.md)

---


## Acknowledgements

I would like to thank my friend [Andy Xia](https://github.com/Andrew-Y-Xia) for inspiring me to embark on this journey.

I would also like to thank the chess programming community that has provided invaluable resources that made this project possible:

- The [Chess Programming Wiki](https://www.chessprogramming.org/) for comprehensive technical documentation on search algorithms, move generation, and evaluation techniques
- [Stockfish](https://stockfishchess.org) and [Crafty](https://craftychess.com) for demonstrating advanced engine architecture and optimization techniques
- The broader open-source chess engine community for sharing knowledge on NNUE networks, tuning methodologies, and performance analysis
---

## Next steps

**Evaluation**

- Texel-tune handcrafted PST and term weights (`beluga-tune` currently fits K only)
- Wire NNUE into search: accumulator on the move stack, UCI option for network file, fallback to handcrafted eval
- Train a network: PyTorch trainer matching `nnue.rs` feature layout, quantize/export to the existing binary format
- Generate training data with a strong teacher (Stockfish at high depth), not just self-play with the current build

**Search**

- Singular extensions, capture history, correction history, probcut
- SPSA tuning of reduction and pruning margins, gated by SPRT
- Aggregate Lazy SMP node counts in `info` output

**Endgame and openings**

- Syzygy tablebase probing
- Polyglot opening book

**Engine polish**

- Ponder support
- Criterion micro-benchmarks; miri/sanitizer CI jobs
- Formal strength testing: large SPRT gauntlets, calibrated opponent pool
