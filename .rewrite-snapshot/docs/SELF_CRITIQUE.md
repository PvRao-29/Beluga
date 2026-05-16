# Self-critique & adversarial review

An honest review of Beluga as implemented, structured around the adversarial
questions posed in the project brief. Where a risk is mitigated, the mitigation
is cited; where it is an accepted current limitation, it is stated plainly.

## What would make this engine play illegal moves?

**Largely impossible by construction.** The search only ever plays moves produced
by `movegen::generate_legal`, which is verified against published perft counts on
all canonical positions (startpos, Kiwipete, CPW 3–6) through depth 5–6, plus a
randomized 400-game legal-move fuzz with make/unmake + Zobrist round-trip
assertions. The TT move is used *only* for ordering and is matched against the
generated legal list before it can be played, so a hash collision cannot inject an
illegal move. The trickiest case — en-passant discovered/pinned-EP — has a
dedicated simulated-occupancy legality test and is covered by perft.

Residual risk: a future change to make/unmake could desync state silently. Caught
by the `strict-asserts` feature (incremental vs recomputed Zobrist) and the
make/unmake invariant test.

## What would make it crash?

- `MoveList` overflow — bounded at 256 (a safe legal-move maximum) with a debug
  assert.
- Search stack overflow — `ply` is clamped to `MAX_PLY`; per-ply arrays are sized
  `MAX_PLY + 4`.
- Malformed UCI / FEN — parsed defensively; the black-box UCI test feeds junk,
  bad FENs, and illegal move tokens and asserts the engine recovers and still
  returns a legal `bestmove`. `stop` mid-search is tested.
- Huge `Hash` — clamped to `[1, 65536]` MB.

Accepted limitation: extremely large hash sizes initialize entries in a per-entry
loop (slow, not unsafe).

## What would make it weak?

This is the honest core limitation. **The evaluation is untuned.** It uses PeSTO
PSTs plus hand-picked positional terms (mobility, bishop pair, rook files, passed
pawns, tempo) with *guessed* weights. Until these are Texel-tuned and the search
constants SPSA-tuned (see `docs/TUNING.md`), strength is left on the table. The
search itself is modern and sound, so most of the deficit is evaluation and
tuning, plus the absence of NNUE.

Other strength gaps: no singular extensions, single-ply continuation history only,
no capture history, no probcut, no pawn/eval caches, no Syzygy in the endgame.

## What would make benchmarks misleading?

- Perft nps is **bulk-counted** (depth-1 returns the move count), which overstates
  raw make/unmake throughput — this is stated wherever the number appears, and
  `perft_full` measures the un-bulked path.
- The reported search nps under multithreading is **main-thread only** (helper
  nodes are not yet aggregated), so it under-reports true SMP throughput. Single-
  thread `bench` is the canonical, deterministic figure.
- All numbers were measured on Apple Silicon (aarch64); the x86 "30M nps perft"
  design target is hardware-dependent and not claimed as achieved here.

## What would make Elo testing invalid?

- Comparing builds on different hardware, flags, or time controls — forbidden by
  the methodology; `scripts/sprt.sh` enforces parity.
- Tuning against an untested baseline — every change is SPRT-gated.
- Drawing conclusions from too few games — SPRT with proper bounds, not fixed-N
  "it looks stronger".

## What would prevent NNUE from working?

- Accumulator drift — mitigated by the `incremental_matches_refresh` test; a
  king-bucket refresh on king moves is specified for the production path.
- Layout mismatch — the feature indexing is self-consistent and documented; nets
  must be trained against *this* layout (not Stockfish-compatible).
- Honest status: no trained net ships, and quantized SIMD inference is not yet
  wired into `eval::evaluate`. `evaluate` falls back to handcrafted unconditionally
  today.

## What would fail under tournament time controls?

- Time forfeit — soft/hard limits, a reserved `Move Overhead` margin, a node-mask
  time check (every 2048 nodes), and "don't start an iteration past the soft
  limit". `go movetime` parity verified (stops before the budget).
- Accepted limitation: bare `go` with no limits is treated as infinite (requires
  `stop`); GUIs always send time controls, so this is low-risk but noted.

## What would break under multithreading?

- TT torn reads — mitigated by lockless `key ^ data` validation; a corrupted entry
  fails the check and is ignored.
- Stop coordination — a single `Arc<AtomicBool>` shared by all threads; the manager
  sets it and joins helpers before announcing the move.
- Runtime thread-count change — `setoption Threads` stops the current search first;
  the pool is only rebuilt between searches.
- Determinism — `Threads 1` is deterministic; SMP is intentionally not (documented).

## Top residual risks (prioritized)

1. Untuned eval/search constants → biggest strength gap. (Tune via SPRT.)
2. SMP nps reporting and scaling are basic. (Add shared node counter + diversity.)
3. No singular extensions / capture history. (Roadmap.)
4. No Syzygy/Polyglot. (Roadmap; documented, not faked.)
5. Draw score carries a tiny ±2 jitter that can be stored in the TT — negligible
   but technically a minor inconsistency.
