# Next-improvement roadmap

Ordered by expected Elo-per-effort, gated by SPRT.

## Search
- [ ] Singular extensions (with a verification search and `excluded move` infra).
- [ ] Multi-ply continuation history (2- and 4-ply) and capture history.
- [ ] History-based LMR adjustment (reduce less for high-history quiets).
- [ ] Correction history for static eval.
- [ ] Improve null-move (zugzwang verification search at high depth).
- [ ] Probcut.
- [ ] Aggregated SMP node counter for honest multi-thread nps reporting, plus
      per-thread depth/aspiration diversity for better Lazy-SMP scaling.

## Evaluation
- [ ] Texel-tune all handcrafted terms; add king-safety attack tables, threats,
      space, outposts, rook-on-7th, mobility area with king-zone weighting.
- [ ] Pawn-structure hash cache.
- [ ] **NNUE (Phase 5)**: train a net against this layout, wire quantized SIMD
      inference (`std::arch` with scalar fallback), incremental accumulator on
      make/unmake with king-bucket refresh, and a `Use NNUE` UCI option.

## Endgame & openings
- [ ] Syzygy WDL/DTZ probing via Fathom FFI; root-move filtering; 50-move-aware.
- [ ] Polyglot `.bin` book support (requires the published Polyglot Zobrist
      constants) with weighted/deterministic selection and an `OwnBook` option.

## Time & threading
- [ ] Node-based time management and best-move-stability scaling.
- [ ] Ponder support.

## Tooling & QA
- [ ] `cargo bench` (Criterion) micro-benchmarks for movegen/make/eval.
- [ ] miri run over make/unmake in CI; nightly sanitizer job.
- [ ] OpenBench worker config; continuous SPRT.
- [ ] 10k-game illegal-move/crash soak in CI (nightly).
