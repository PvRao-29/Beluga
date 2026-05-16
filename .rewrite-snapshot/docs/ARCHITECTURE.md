# Architecture

## Crate graph

```
beluga-core (lib)
├── used by beluga-uci   (the `beluga` executable + UCI/bench/perft)
├── used by beluga-tune  (eval tuning harness)
└── used by beluga-tools (perft / bench drivers)
```

`beluga-core` has no third-party runtime dependencies; `proptest` is a dev
dependency only.

## Module layering (bottom-up)

```
types, bitboard            value types, 64-bit board sets
   │
attacks, zobrist           magic/leaper tables, hash keys (OnceLock, built once)
   │
chess_move                 packed 16-bit Move + stack MoveList(256)
   │
position                   board, FEN, make/unmake, repetition, Zobrist (incremental)
   │
movegen, see               legal move generation; static exchange evaluation
   │
eval, nnue                 tapered handcrafted eval; NNUE scaffolding (fallback)
   │
tt, timeman                lockless shared TT; soft/hard time limits
   │
search                     iterative deepening PVS + pruning suite
```

## Hot paths and data flow

A `go` command (UCI thread) spawns a *manager* thread which:

1. spawns `Threads-1` Lazy-SMP helper searches (each owns a `Position` clone,
   shares `Arc<Tt>` and the `Arc<AtomicBool>` stop flag);
2. runs the main `Search`, which reports `info` lines via a callback and returns
   the best move;
3. sets the stop flag, joins helpers, prints `bestmove`.

Per search node (`search::negamax`):

```
draw checks → mate-distance pruning → TT probe → static eval
   → reverse-futility / razoring / null-move (non-PV)
   → internal iterative reduction
   → generate_legal → score_moves (TT/SEE/killers/counter/history/conthist)
   → move loop: LMP/futility/SEE pruning → make → (PVS + LMR) → unmake
   → on cutoff: update killers/counter/history/conthist
   → TT store (mate-normalized)
```

The move loop never allocates: `MoveList` is a fixed 256-entry stack buffer and
`pick_best` lazily selects the next-best move (selection sort over the unsorted
tail), so only visited moves pay ordering cost.

## Memory ownership

- `Position` owns the board plus a `Vec<Undo>` and `Vec<u64>` key history for O(1)
  unmake and repetition detection.
- `Search` owns fixed-size per-ply stacks (eval/move/piece), a boxed triangular PV
  table, and boxed heuristic tables (history, continuation history). These are
  allocated once per `Search` instance.
- `Tt` owns a `Vec<Entry>` of `AtomicU64` pairs; shared read-mostly via `Arc`.
  Lockless XOR validation (`key ^ data`) makes torn SMP reads self-detecting.

## Correctness invariants (asserted in debug / tests)

- Incremental Zobrist key equals from-scratch recompute after every move.
- make → unmake restores the exact board, key, and FEN.
- Legal move generation matches published perft counts (depths 5–6).
- NNUE incremental accumulator equals a full refresh.
