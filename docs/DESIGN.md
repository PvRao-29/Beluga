# Beluga — Design Review

This document is the pre-implementation design review for **Beluga**, a UCI chess
engine written in Rust. It is deliberately honest about trade-offs and about what
is and is not realistically achievable. Where strength numbers appear they are
*targets with measurement methodology*, never fabricated results.

---

## A. Engine-strength strategy

Modern engine strength comes from the interaction of four multiplicative factors,
not any single trick:

1. **Search efficiency (nodes to depth).** The dominant lever. A strong engine
   reaches far higher *effective* depth than a naive one at equal node counts,
   because of aggressive but *sound* pruning/reductions (null move, LMR, futility,
   SEE-based ordering, transposition table). Two engines at the same raw NPS can
   differ by 600+ Elo purely from search.
2. **Move ordering.** Pruning only pays off when the best move is searched first.
   TT move → winning captures (SEE) → killers → counter/continuation history →
   ordered quiets is the backbone. Good ordering raises the alpha-beta cutoff rate
   toward the theoretical ~`sqrt(b)` branching factor.
3. **Evaluation accuracy.** Even a perfect search of depth N is only as good as the
   leaf eval. We stage this: a strong *tapered handcrafted* eval first (correct,
   debuggable, ~2400-class if tuned), then an **NNUE** architecture for the jump to
   2800+. NNUE wins because it captures non-linear positional knowledge cheaply via
   an incrementally-updated accumulator.
4. **Raw speed (NPS).** Multiplies everything above. Bitboards, magic attack
   tables, packed 16-bit moves, branch-aware hot paths, no per-node heap
   allocation, and Lazy SMP scaling.

**Honest framing.** Stockfish-level strength is the product of a decade of
distributed testing (Fishtest), terabytes of NNUE training data, and continuous
SPRT-gated tuning. Beluga targets *correct, competitive, well-engineered* strength
and a clean path to NNUE. We claim methodology and architecture, and we *measure*
rather than assert.

---

## B. Architecture

Cargo workspace; clean separation of a reusable engine library from the binary.

```
beluga/
├── engine-core/   # library: board, movegen, search, eval, tt, time
├── engine-uci/    # binary: UCI loop, option handling, bench/perft commands
├── engine-tune/   # texel-style tuning + SPSA helpers (uses engine-core)
├── tools/         # perft driver, bench, (future) book + nnue conversion bins
├── scripts/       # Python: self-play (cutechess), SPRT, NNUE data/training
├── docs/          # DESIGN, ARCHITECTURE, TUNING, ROADMAP, SELF_CRITIQUE
└── tests/         # integration: perft suite, FEN, invariants (in engine-core)
```

**Module map (engine-core):**

| Module        | Responsibility |
|---------------|----------------|
| `types`       | Color, Piece, PieceType, Square, CastlingRights |
| `bitboard`    | `Bitboard(u64)` + iteration, shifts, popcount |
| `chess_move`  | packed 16-bit `Move` (from/to/flags) |
| `attacks`     | precomputed leaper + magic slider tables |
| `zobrist`     | Zobrist key tables |
| `position`    | board state, FEN, make/unmake, repetition, key |
| `movegen`     | legal move generation (check/pin aware) |
| `perft`       | correctness driver |
| `eval`        | tapered handcrafted eval (+ NNUE hook) |
| `tt`          | transposition table (lockless shared) |
| `search`      | iterative deepening, PVS, pruning, ordering |
| `timeman`     | soft/hard limits from UCI time controls |
| `see`         | static exchange evaluation |

**Ownership boundaries.** `Position` owns board state and a `Vec<Undo>` history
stack used by make/unmake. `Search` borrows a `Position` mutably and holds search
stacks (killers, continuation history pointers, PV) in fixed-size arrays indexed by
ply. The `TT` is shared read-mostly across threads via a raw slice with atomic-ish
relaxed access (Lazy SMP tolerates races by design). The UCI layer owns the engine
and a worker thread; the stop flag is an `Arc<AtomicBool>`.

---

## C. Board representation

**Choice: bitboards (12 piece bitboards + 2 color occupancies) + a 64-entry mailbox
for fast "what's on square X".** Justification: bitboards give O(1) set operations
for attack/movegen and pair naturally with magic sliders; the mailbox avoids a
loop to identify the piece type on captures. This redundancy is standard (Stockfish
keeps both) and is kept consistent inside make/unmake.

**Exact encodings:**

- **Square**: `0..63`, `A1=0`, `H1=7`, `A8=56`, `H8=63` (little-endian
  rank-file, LERF). `file = sq & 7`, `rank = sq >> 3`.
- **Color**: `White=0`, `Black=1`.
- **PieceType**: `Pawn=0, Knight=1, Bishop=2, Rook=3, Queen=4, King=5`.
- **Piece**: `color*6 + piece_type` (0..11), plus a `None=12` sentinel in mailbox.
- **Move** (packed `u16`): bits `0..6` = from, `6..12` = to, `12..16` = flags.
  Flags enumerate: quiet, double-push, king-castle, queen-castle, capture,
  en-passant, and the four promotions × {quiet, capture}. This is the classic
  "Pradyumna/ chessprogramming" 16-bit layout enabling MVV ordering on flags.
- **CastlingRights**: 4-bit mask `WK=1, WQ=2, BK=4, BQ=8`. Updated by an
  AND-mask table indexed by from/to square (rook capture / king or rook move all
  handled by one table).
- **En passant**: target square (the square *behind* the pushed pawn) or a `None`
  sentinel. Only set when an enemy pawn can actually capture (so Zobrist keys of
  otherwise-equal positions match — important for repetition/TT).
- **Hash**: 64-bit Zobrist = XOR of piece-square keys, side-to-move key, castling
  keys, and en-passant file key. Maintained incrementally in make/unmake.
- **Game-state stack**: `Vec<Undo>` storing, per ply, the captured piece, prior
  castling rights, prior EP square, prior halfmove clock, and prior key, so
  unmake is O(1) and exact.

State also tracks: side to move, halfmove clock (fifty-move), fullmove number, and
a ring/history of keys for repetition detection.

---

## D. Move generation and legality

**Strategy: direct legal generation** using check and pin masks (no
generate-then-filter-by-make for the common case), which is both correct and fast.

Per position we compute:
- `checkers`: bitboard of enemy pieces giving check (via attacks-to-king).
- `pinned`: our pieces pinned to our king, with each pinned piece restricted to the
  ray between king and pinner.
- `check_mask`: if 1 checker, the set of squares that resolve check (capture the
  checker or block the ray); if 0 checkers, all squares; if 2 checkers (double
  check), empty for non-king pieces (king must move).

Generation:
- **King moves**: to squares not attacked by the enemy, computed with the king
  *removed* from the occupancy (so sliding x-rays through the king square are
  respected). Includes castling, which checks (a) rights, (b) empty path between,
  (c) king start/transit/target squares not attacked.
- **Other pieces**: pseudo-attacks ∩ `check_mask`; if the piece is pinned, further
  ∩ its pin ray.
- **Pawns**: pushes, double pushes, captures, promotions (all four), and en
  passant. Promotions emit Q/R/B/N.
- **En passant**: the one case needing special care — *en-passant discovered check
  / pinned EP*. We make the EP capture's occupancy change on a temporary bitboard
  and test whether our king is then attacked along the 5th/4th rank (the classic
  "two pawns removed from one rank exposing a rook/queen" bug). This is the only
  per-move legality test and EP is rare, so cost is negligible.

**Proving perft correctness:** exhaustive node counts against the canonical
positions (startpos, Kiwipete, positions 3–6 from the CPW perft page, plus
promotion/EP-heavy positions) through depth 5–6, compared to published exact
values. Any mismatch localizes a generator/make-unmake bug. We also run a
divide-perft to isolate the offending move subtree. Additionally, a randomized
property test makes random legal moves and asserts make/unmake restores the exact
Zobrist key and board (round-trip invariant), and that FEN→parse→emit round-trips.

---

## E. Search design

Single pipeline, negamax with fail-soft alpha-beta.

```
go → time_man.init
iterative deepening d = 1..max:
    aspiration window around prev score (widen on fail)
    score = negamax(root, d, alpha, beta)   # PV node
    update best move, PV, report info
    stop if time soft-limit / depth / nodes / mate found
negamax(pos, depth, alpha, beta, ply):
    if repetition/50-move/insufficient → draw (0, with small contempt later)
    mate-distance pruning (tighten alpha/beta toward mate bounds)
    TT probe → cutoff if usable (depth & bound), else TT move for ordering
    if depth <= 0 → qsearch
    static eval (TT-corrected); compute improving flag
    [non-PV, not in check] reverse futility (static - margin*depth >= beta → return)
    [non-PV] razoring at low depth (static + margin < alpha → drop to qsearch)
    [non-PV, not in check, has non-pawn material] null-move pruning (R = 3 + depth/3 …)
    internal iterative reduction if no TT move at high depth
    move loop (ordered):
        skip illegal; legality already guaranteed by generator
        [late quiets, non-PV] futility pruning / late-move pruning by move count
        [quiets, low depth] SEE pruning of losing captures
        extensions: check extension; (singular extension — guarded)
        PVS: first move full window; rest null window, re-search on raise
        LMR: reduce late quiets by a depth/movecount log table, re-search if it beats alpha
        make → recurse → unmake
        update alpha / best; on beta cutoff: update killers/history/counter/continuation
    if no legal moves: checkmate (-MATE+ply) or stalemate (0)
    store TT (depth, bound, move, score normalized for mate)
qsearch(pos, alpha, beta, ply):
    stand-pat = eval; if >= beta return (fail-high); alpha = max(alpha, stand_pat)
    generate captures (+ queen promos, + checks at shallow q-depth optionally)
    order by MVV-LVA / SEE; skip SEE-losing captures
    recurse; same cutoff logic
```

**Conventions & semantics:**
- **Depth** integer plies; fractional extensions accumulate but search calls use
  integer depth (extensions add a full ply, reductions subtract; clamp ≥ 0).
- **Node types**: Root (PV), PV, NonPV (zero-window). Determined by `alpha+1==beta`.
- **Bounds**: `Exact` (alpha<score<beta), `Lower`/`Beta` (score≥beta, fail-high,
  store best move), `Upper`/`Alpha` (score≤alpha, fail-low).
- **Mate score encoding**: `MATE = 30000`; a mate in `n` plies scores
  `MATE - n`. TT store/probe *normalize* mate scores by `±ply` so a mate found at
  ply 6 is stored relative to the node, not the root (classic TT mate bug if
  omitted).
- **TT replacement**: bucketed; replace if same key, or empty, or
  `entry.depth + (age_diff*2) <= new.depth` (depth-preferred with aging). Each
  search increments a global generation counter for aging.
- **Edge cases**: never play a null move when in check or with no non-pawn material
  (zugzwang); verify null-move fail-highs at high depth (optional zugzwang verify);
  draw scores avoid storing misleading mate bounds; stop flag checked on a node
  counter mask to keep it cheap and time-safe.

---

## F. Evaluation design

**Stage 1 — handcrafted, tapered.** Score interpolates between *midgame* (mg) and
*endgame* (eg) values by a game **phase** computed from remaining non-pawn material
(`phase = sum(piece_phase)`, normalized 0..256). Terms:

- Material (mg/eg).
- Piece-square tables (mg/eg), color-mirrored.
- Mobility (legal-ish attack counts per piece, with safe-square weighting).
- King safety (attacker count/weight to the king zone, pawn shelter, open files).
- Pawn structure (doubled, isolated, backward; pawn hash cache later).
- Passed pawns (rank-scaled, blockers, king proximity in eg).
- Bishop pair, rook on open/semi-open file, rook on 7th, knight outposts.
- Threats (attacks on higher-value pieces), space, tempo.

Output from side-to-move perspective (negamax-friendly). Everything is integer
centipawns. An `eval trace` mode prints each term (mg/eg/phase) for debugging and
tuning.

**Stage 2 — NNUE (architecture now, weights later).** HalfKP-style or
(simpler) HalfKAv2-lite feature set. Design:

- **Features**: `(king_square, piece_square, piece_type, color)` indices, one
  perspective per side; the accumulator is `2 × H` (H hidden, e.g. 256/512).
- **Accumulator update**: incremental on make/unmake — add/sub the feature columns
  for moved/captured pieces; full refresh when the own king moves (king-bucketed
  features). A `Stack<Accumulator>` mirrors the move stack.
- **Inference**: `clipped_relu(accumulator) → int8 dot products → int32 → scale`.
  Quantized (weights int16/int8, accumulator int16) for SIMD; portable scalar
  fallback always present.
- **Scaling**: network output scaled to centipawns and blended with material phase
  to keep behavior sane near tablebase/draw boundaries.
- **Fallback**: if no `.nnue` file loads, use handcrafted eval. Selected at runtime
  via `Use NNUE` UCI option. A validation test compares incremental accumulator vs.
  full recomputation after random move sequences (drift detection).
- **Training hooks**: self-play produces `(fen, eval, game_result)` records in a
  documented format consumed by `scripts/` (PyTorch + python-chess).

---

## G. Performance plan (targets + how measured)

All numbers are *targets on a modern desktop x86-64 with `-C target-cpu=native`*;
the dev machine here is Apple Silicon (aarch64), so absolute NPS will differ and we
report what we actually measure rather than inventing x86 numbers.

| Phase | Target | Measurement |
|-------|--------|-------------|
| 1 legal movegen | ≥ 30M nodes/s perft (x86 desktop; hardware-dependent) | `tools` perft bench, `cargo bench` |
| 2 basic α-β | ≥ 1M search NPS | bench command node count / time |
| 3 optimized search | multi-M NPS, scaling across threads | bench + thread sweep |
| 4 NNUE | competitive NPS retained | bench with/without NNUE |

**No fabricated results.** The README/benchmark section is populated by actually
running `beluga bench` and `cargo bench` and pasting real output, annotated with the
CPU. Where a target is x86-specific and we only have aarch64, we say so explicitly.

---

## H. Testing methodology

- **Perft suite** — exact counts, standard positions, depths 1–6 (CI runs a fast
  subset; full suite behind `--ignored`/feature).
- **Tactical suite** — WAC/ECM-style EPD `bm` positions; report solve rate at fixed
  time/depth.
- **Mate suite** — forced mates; assert mate score and correct distance.
- **Draw/repetition** — threefold, fifty-move, insufficient material unit tests.
- **FEN parsing** — round-trip + malformed-input rejection.
- **make/unmake invariants** — proptest: random legal sequences restore key+board.
- **Hash consistency** — incremental key equals from-scratch recompute every move.
- **UCI protocol** — drive the binary with scripted command sequences incl.
  malformed input and `stop` mid-search; assert no crash, legal `bestmove`.
- **Self-play** — cutechess-cli scripts; assert zero illegal moves / crashes.
- **SPRT Elo** — `scripts/sprt.sh` runs gauntlets, Ordo/`bayeselo` for ratings.
- **Regression** — keep prior binaries; SPRT each change before merge.
- **Fuzzing** — random legal positions + random UCI command fuzzing.

## I. Benchmarks and success criteria

- Perft: exact match through depth 5–6 on the standard suite. **(Gate.)**
- Tactical: target solve-rate buckets at 1s/position (reported, not asserted hard).
- ≥ 10,000 self-play games with zero illegal moves / crashes.
- No crash under randomized UCI fuzzing.
- Elo targets (CCRL-style, *aspirational and measured by SPRT*): 2200+ after a
  correct handcrafted eval, 2500+ after tuning, 2800+ as an NNUE stretch goal.
- Long-term: approach top open-source *practices*; we explicitly state that
  Stockfish-level strength needs years of community-scale testing and data.

## J. Top failure modes (detection → mitigation)

1. **Silent illegal move (pinned EP)** — perft mismatch on EP positions → special EP legality test; dedicated perft positions. 
2. **Castling through/into check allowed** — perft + unit tests on castle squares.
3. **Castling rights not cleared on rook capture** — perft Kiwipete; rights-mask table covers from/to.
4. **Zobrist desync** — incremental vs recompute assertion every make in debug.
5. **EP key set when no EP capture possible** — repetition/TT false (mis)matches → only set EP square when a capturer exists.
6. **TT mate score not normalized** — wrong mate distances/instability → normalize ±ply on store/probe; mate-suite test.
7. **TT move not validated** — corrupt/colliding entry yields illegal move → verify TT move is in the legal list (or pseudo-legal check) before use.
8. **Null move in zugzwang** — drops won endgames → disable with no non-pawn material; optional verification search.
9. **LMR/futility unsound (prunes in check / captures / PV)** — tactical regressions → guard reductions; never reduce/prune when in check, never futility-prune captures/promotions/PV.
10. **Aspiration window never widens** — infinite re-search / wrong score → exponential widen, fall back to full window.
11. **Time forfeit** — exceed hard limit → soft+hard limits; check stop on node mask; reserve move overhead; never start an iteration unlikely to finish.
12. **Stop ignored mid-search** — GUI desync → atomic stop flag checked cheaply; return immediately, keep best so far.
13. **Repetition with irreversible moves** — false draw across captures/pawn moves → only scan keys back to last irreversible (halfmove clock) ply.
14. **Fifty-move vs mate-on-move** — claim draw when checkmate delivered → check mate before fifty-move return.
15. **Insufficient material false positive (KBPk)** — only declare draw for the strict KvK/KNvK/KBvK(+same-color KBvKB) set.
16. **Move encoding overflow / wrong promo flag** — perft promo positions; unit tests on Move pack/unpack.
17. **MoveList overflow** — fixed 256 cap; debug assert; 256 is a safe legal-move upper bound.
18. **Search stack overflow at high ply** — clamp ply to MAX_PLY; arrays sized MAX_PLY+pad.
19. **Eval not side-relative** — sign bug flips play → eval tests + symmetric-position equality.
20. **Phase miscount → eval discontinuity** — clamp phase 0..max; tapered-eval unit test.
21. **SEE wrong (x-rays/promotions)** — SEE unit tests vs known exchanges; conservative on promotions.
22. **TT race corruption (SMP)** — store key XOR data (lockless validation) so torn reads are detected and discarded.
23. **Thread count change at runtime** — rebuild worker pool safely between searches only.
24. **NNUE accumulator drift** — periodic/asserted full-refresh comparison; refresh on king move.
25. **Benchmark/Elo invalidity** — fixed bench position set & node count; SPRT with proper bounds and time-control parity; never compare across different hardware/flags.
