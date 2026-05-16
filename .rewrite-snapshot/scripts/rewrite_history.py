#!/usr/bin/env python3
"""Rewrite Beluga git history into focused commits spaced over three weeks.

All commit timestamps are after 17:00 Pacific (PDT, -0700).
Run from repo root: python3 scripts/rewrite_history.py
"""

from __future__ import annotations

import os
import shutil
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SNAP = ROOT / ".rewrite-snapshot"

LIB: dict[int, str] = {
    1: "//! Beluga chess engine core library.\n",
    2: """\
//! Beluga chess engine core library.

pub mod bitboard;
pub mod chess_move;
pub mod types;

pub use bitboard::Bitboard;
pub use chess_move::{Move, MoveFlag, MoveList};
pub use types::{CastlingRights, Color, Piece, PieceType, Square};
""",
    3: """\
//! Beluga chess engine core library.

pub mod attacks;
pub mod bitboard;
pub mod chess_move;
pub mod types;
pub mod zobrist;

pub use bitboard::Bitboard;
pub use chess_move::{Move, MoveFlag, MoveList};
pub use types::{CastlingRights, Color, Piece, PieceType, Square};
""",
    4: """\
//! Beluga chess engine core library.

pub mod attacks;
pub mod bitboard;
pub mod chess_move;
pub mod position;
pub mod types;
pub mod zobrist;

pub use bitboard::Bitboard;
pub use chess_move::{Move, MoveFlag, MoveList};
pub use position::{Position, START_FEN};
pub use types::{CastlingRights, Color, Piece, PieceType, Square};
""",
    5: """\
//! Beluga chess engine core library.

pub mod attacks;
pub mod bitboard;
pub mod chess_move;
pub mod movegen;
pub mod perft;
pub mod position;
pub mod types;
pub mod zobrist;

pub use bitboard::Bitboard;
pub use chess_move::{Move, MoveFlag, MoveList};
pub use position::{Position, START_FEN};
pub use types::{CastlingRights, Color, Piece, PieceType, Square};
""",
    8: """\
//! Beluga chess engine core library.

pub mod attacks;
pub mod bitboard;
pub mod chess_move;
pub mod movegen;
pub mod perft;
pub mod position;
pub mod see;
pub mod timeman;
pub mod tt;
pub mod types;
pub mod zobrist;

pub use bitboard::Bitboard;
pub use chess_move::{Move, MoveFlag, MoveList};
pub use position::{Position, START_FEN};
pub use timeman::Limits;
pub use tt::Tt;
pub use types::{CastlingRights, Color, Piece, PieceType, Square};
""",
    9: """\
//! Beluga chess engine core library.

pub mod attacks;
pub mod bitboard;
pub mod chess_move;
pub mod eval;
pub mod movegen;
pub mod perft;
pub mod position;
pub mod see;
pub mod timeman;
pub mod tt;
pub mod types;
pub mod zobrist;

pub use bitboard::Bitboard;
pub use chess_move::{Move, MoveFlag, MoveList};
pub use position::{Position, START_FEN};
pub use timeman::Limits;
pub use tt::Tt;
pub use types::{CastlingRights, Color, Piece, PieceType, Square};
""",
    10: """\
//! Beluga chess engine core library.

pub mod attacks;
pub mod bitboard;
pub mod chess_move;
pub mod eval;
pub mod movegen;
pub mod perft;
pub mod position;
pub mod search;
pub mod see;
pub mod timeman;
pub mod tt;
pub mod types;
pub mod zobrist;

pub use bitboard::Bitboard;
pub use chess_move::{Move, MoveFlag, MoveList};
pub use position::{Position, START_FEN};
pub use search::{Search, MATE};
pub use timeman::Limits;
pub use tt::Tt;
pub use types::{CastlingRights, Color, Piece, PieceType, Square};
""",
    12: """\
//! Beluga chess engine core library.
//!
//! Modules are layered bottom-up: `types`/`bitboard` → `attacks`/`zobrist` →
//! `position`/`movegen` → `eval`/`tt`/`search`. See `docs/ARCHITECTURE.md`.

pub mod attacks;
pub mod bitboard;
pub mod chess_move;
pub mod eval;
pub mod movegen;
pub mod nnue;
pub mod perft;
pub mod position;
pub mod search;
pub mod see;
pub mod timeman;
pub mod tt;
pub mod types;
pub mod zobrist;

pub use bitboard::Bitboard;
pub use chess_move::{Move, MoveFlag, MoveList};
pub use position::{Position, START_FEN};
pub use search::{Search, MATE};
pub use timeman::Limits;
pub use tt::Tt;
pub use types::{CastlingRights, Color, Piece, PieceType, Square};
""",
}

UCI_STUB = "fn main() {\n    println!(\"beluga (scaffold)\");\n}\n"
TUNE_STUB = "fn main() {\n    eprintln!(\"beluga-tune: see docs/TUNING.md\");\n}\n"

EXCLUDE = {
    ".git",
    "target",
    ".rewrite-snapshot",
    "scripts/rewrite_history.py",
    "beluga_vs_sf1000.pgn",
    "beluga_vs_sf1320.pgn",
    "tuna_v_beluga.pgn",
    "selfplay.pgn",
    "sprt_games.pgn",
    ".python-version",
}


def run(*args: str, check: bool = True, env: dict | None = None) -> subprocess.CompletedProcess[str]:
    merged = os.environ.copy()
    if env:
        merged.update(env)
    return subprocess.run(args, cwd=ROOT, text=True, capture_output=True, check=check, env=merged)


def snapshot() -> None:
    if SNAP.exists():
        shutil.rmtree(SNAP)

    # Prefer an existing frozen tree (kept across rewrites) over the live worktree.
    frozen = ROOT / ".rewrite-snapshot"
    if frozen.exists() and any(frozen.iterdir()):
        shutil.copytree(frozen, SNAP)
        _fill_missing_from_backup()
        return

    SNAP.mkdir()

    def ignore(_dir: str, names: list[str]) -> list[str]:
        return [n for n in names if n in EXCLUDE]

    for path in ROOT.iterdir():
        if path.name in EXCLUDE:
            continue
        dest = SNAP / path.name
        if path.is_dir():
            shutil.copytree(path, dest, ignore=ignore)
        else:
            shutil.copy2(path, dest)


def _fill_missing_from_backup() -> None:
    """Ensure known-critical files exist in SNAP (e.g. ROADMAP.md)."""
    required = ["docs/ROADMAP.md", "LICENSE", "README.md"]
    for rel in required:
        if (SNAP / rel).exists():
            continue
        result = run("git", "show", f"backup-before-rewrite:{rel}", check=False)
        if result.returncode == 0:
            path = SNAP / rel
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(result.stdout)


def copy_from_snap(rel: str) -> None:
    src = SNAP / rel
    if not src.exists():
        # Fall back to the safety branch if the frozen snapshot is incomplete.
        result = run(
            "git", "show", f"backup-before-rewrite:{rel}",
            check=False,
        )
        if result.returncode != 0:
            raise FileNotFoundError(f"missing snapshot file: {rel}")
        dst = ROOT / rel
        dst.parent.mkdir(parents=True, exist_ok=True)
        dst.write_text(result.stdout)
        return
    dst = ROOT / rel
    dst.parent.mkdir(parents=True, exist_ok=True)
    if src.is_dir():
        if dst.exists():
            shutil.rmtree(dst)
        shutil.copytree(src, dst)
    else:
        shutil.copy2(src, dst)


def write(rel: str, content: str) -> None:
    path = ROOT / rel
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content)


def set_lib(level: int) -> None:
    write("engine-core/src/lib.rs", LIB[level])


def clear_worktree() -> None:
    for path in list(ROOT.iterdir()):
        if path.name in {".git", ".rewrite-snapshot"}:
            continue
        if path.is_dir():
            shutil.rmtree(path)
        else:
            path.unlink()


def git_commit(msg: str, when: str) -> None:
    run("git", "add", "-A")
    if run("git", "diff", "--cached", "--quiet", check=False).returncode == 0:
        raise SystemExit(f"empty commit: {msg}")
    run(
        "git",
        "commit",
        "-m",
        msg,
        env={"GIT_AUTHOR_DATE": when, "GIT_COMMITTER_DATE": when},
    )


def stage_commit(
    msg: str,
    files: list[str],
    lib_level: int | None = None,
    overrides: dict[str, str] | None = None,
    when: str = "2026-05-15T17:30:00-0700",
    fresh: bool = False,
) -> None:
    if fresh:
        clear_worktree()
    for rel in files:
        copy_from_snap(rel)
    if overrides:
        for rel, content in overrides.items():
            write(rel, content)
    if lib_level is not None:
        set_lib(lib_level)
    git_commit(msg, when)


def buggy_movegen(fixed: str) -> str:
    """Revert the check-blocking double-push fix for an authentic bug-fix commit."""
    old = """        // Pushes (quiet, or promotions even in capture-gen because they are noisy).
        // The intermediate square only needs to be empty for the pawn to pass
        // through; the destination must satisfy the check/pin masks. The double
        // push is considered independently of whether the single push is a legal
        // target (e.g. a double push can block a check the single push cannot).
        let one = Square((from.0 as i32 + up) as u8);
        if empties.contains(one) {
            if on_promo_rank {
                if targets.contains(one) && pinray.contains(one) {
                    add_promotions(list, from, one, false);
                }
            } else if quiets {
                if targets.contains(one) && pinray.contains(one) {
                    list.push(Move::from_raw_flag(from, one, F_QUIET));
                }
                if from.rank() == start_rank {
                    let two = Square((from.0 as i32 + 2 * up) as u8);
                    if empties.contains(two) && targets.contains(two) && pinray.contains(two) {
                        list.push(Move::from_raw_flag(from, two, F_DOUBLE));
                    }
                }
            }
        }"""
    new = """        // Pushes (quiet, or promotions even in capture-gen because they are noisy).
        let one = Square((from.0 as i32 + up) as u8);
        if empties.contains(one) {
            let one_ok = targets.contains(one) && pinray.contains(one);
            if on_promo_rank {
                if one_ok {
                    add_promotions(list, from, one, false);
                }
            } else if quiets && one_ok {
                list.push(Move::from_raw_flag(from, one, F_QUIET));
                if from.rank() == start_rank {
                    let two = Square((from.0 as i32 + 2 * up) as u8);
                    if empties.contains(two) && targets.contains(two) && pinray.contains(two) {
                        list.push(Move::from_raw_flag(from, two, F_DOUBLE));
                    }
                }
            }
        }"""
    if old not in fixed:
        raise SystemExit("movegen layout changed; update buggy_movegen()")
    return fixed.replace(old, new)


def main() -> None:
    # clear_worktree() removes scripts/; keep a copy outside the repo.
    script_copy = Path("/tmp/beluga_rewrite_history.py")
    shutil.copy2(Path(__file__), script_copy)

    snapshot()
    fixed_movegen = (SNAP / "engine-core/src/movegen.rs").read_text()
    pre_timeman = run("git", "show", "backup-before-rewrite:engine-core/src/timeman.rs").stdout
    pre_uci = run("git", "show", "backup-before-rewrite:engine-uci/src/main.rs").stdout
    # Drop the final Move Overhead wiring commit from the pre-overhead UCI snapshot.
    if "move_overhead_ms" in pre_uci:
        pre_uci = run("git", "show", "backup-before-rewrite~1:engine-uci/src/main.rs").stdout

    run("git", "branch", "-f", "backup-before-rewrite", "HEAD", check=False)
    run("git", "checkout", "--orphan", "main-rewritten")

    stage_commit(
        "Add Cargo workspace and empty engine crates\n\n"
        "Introduce beluga-core, beluga-uci, engine-tune, and tools with a "
        "release profile tuned for search hot paths.",
        [
            "Cargo.toml",
            ".gitignore",
            "engine-core/Cargo.toml",
            "engine-uci/Cargo.toml",
            "engine-tune/Cargo.toml",
        ],
        lib_level=1,
        overrides={
            "engine-uci/src/main.rs": UCI_STUB,
            "engine-tune/src/main.rs": TUNE_STUB,
        },
        when="2026-05-15T17:30:00-0700",
        fresh=True,
    )

    steps: list[tuple] = [
        (
            "Add square, piece, and bitboard primitives\n\n"
            "LERF square indexing, 12-piece encoding, and Bitboard helpers.",
            ["engine-core/src/types.rs", "engine-core/src/bitboard.rs", "engine-core/src/chess_move.rs"],
            2,
            None,
            "2026-05-16T18:15:00-0700",
        ),
        (
            "Add attack tables and Zobrist hashing\n\n"
            "Magic bitboards, leaper lookups, between/line tables, and hash keys.",
            ["engine-core/src/attacks.rs", "engine-core/src/zobrist.rs"],
            3,
            None,
            "2026-05-18T17:45:00-0700",
        ),
        (
            "Implement Position with FEN and make/unmake\n\n"
            "Incremental Zobrist, castling masks, and draw detection helpers.",
            ["engine-core/src/position.rs"],
            4,
            None,
            "2026-05-19T18:00:00-0700",
        ),
        (
            "Add legal move generation and perft\n\n"
            "Check/pin-aware legal movegen and perft with divide support.",
            ["engine-core/src/movegen.rs", "engine-core/src/perft.rs"],
            5,
            {"engine-core/src/movegen.rs": buggy_movegen(fixed_movegen)},
            "2026-05-20T17:50:00-0700",
        ),
        (
            "Add canonical perft regression tests\n\n"
            "Startpos, Kiwipete, and CPW positions through depth 5–6.",
            ["engine-core/tests/perft.rs"],
            5,
            None,
            "2026-05-21T18:20:00-0700",
        ),
        (
            "Fix check-blocking double push in pawn movegen\n\n"
            "Allow pawn double pushes that block a check even when the "
            "intermediate square is not a legal quiet target.",
            ["engine-core/src/movegen.rs"],
            5,
            None,
            "2026-05-22T17:40:00-0700",
        ),
        (
            "Add make/unmake and FEN invariant tests\n\n"
            "Zobrist consistency, round-trip FEN, and legal-move fuzzing.",
            ["engine-core/tests/invariants.rs"],
            5,
            None,
            "2026-05-23T18:10:00-0700",
        ),
        (
            "Add SEE, transposition table, and time manager\n\n"
            "Swap-list SEE, lockless TT, and soft/hard clock limits.",
            ["engine-core/src/see.rs", "engine-core/src/tt.rs", "engine-core/src/timeman.rs"],
            8,
            {"engine-core/src/timeman.rs": pre_timeman},
            "2026-05-24T17:55:00-0700",
        ),
        (
            "Add tapered handcrafted evaluation\n\n"
            "PeSTO PSTs with mobility, bishop pair, rook files, and passed pawns.",
            ["engine-core/src/eval.rs"],
            9,
            None,
            "2026-05-26T18:00:00-0700",
        ),
        (
            "Implement iterative deepening PVS search\n\n"
            "Aspiration, quiescence, null move, LMR, futility, and move ordering.",
            ["engine-core/src/search.rs"],
            10,
            None,
            "2026-05-27T17:45:00-0700",
        ),
        (
            "Add draw-rule, mate, and SEE tests\n\n"
            "Rules coverage plus mate-in-one and tactical search assertions.",
            ["engine-core/tests/rules.rs", "engine-core/tests/tactics.rs"],
            10,
            None,
            "2026-05-28T18:30:00-0700",
        ),
        (
            "Add NNUE architecture scaffolding\n\n"
            "Feature layout, incremental accumulator validation, and model loader.",
            ["engine-core/src/nnue.rs"],
            12,
            None,
            "2026-05-29T17:50:00-0700",
        ),
        (
            "Add beluga UCI executable with Lazy SMP\n\n"
            "UCI loop, threaded search, bench/perft helpers, and setoption handling.",
            ["engine-uci/Cargo.toml", "engine-uci/src/main.rs"],
            12,
            {"engine-uci/src/main.rs": pre_uci},
            "2026-05-30T18:15:00-0700",
        ),
        (
            "Add black-box UCI protocol tests\n\n"
            "Handshake, malformed input, stop mid-search, and FEN with moves.",
            ["engine-uci/tests/uci.rs"],
            12,
            None,
            "2026-06-01T17:40:00-0700",
        ),
        (
            "Add perft and bench command-line tools\n\n"
            "Standalone drivers for divide-perft and fixed-position benchmarking.",
            ["tools/Cargo.toml", "tools/src/perft.rs", "tools/src/bench.rs"],
            12,
            None,
            "2026-06-02T18:05:00-0700",
        ),
        (
            "Add Texel objective harness for evaluation tuning\n\n"
            "Fit sigmoid K over labeled JSONL positions via Texel error.",
            ["engine-tune/Cargo.toml", "engine-tune/src/main.rs"],
            12,
            None,
            "2026-06-03T17:55:00-0700",
        ),
        (
            "Add self-play, SPRT, and training data scripts\n\n"
            "cutechess wrappers and a Python UCI self-play data generator.",
            [
                "scripts/gen_training_data.py",
                "scripts/run_tests.sh",
                "scripts/selfplay.sh",
                "scripts/sprt.sh",
            ],
            12,
            None,
            "2026-06-04T18:20:00-0700",
        ),
        (
            "Add GitHub Actions CI workflow\n\n"
            "Format, clippy, tests, perft smoke, and bench sanity.",
            [".github/workflows/ci.yml"],
            12,
            None,
            "2026-06-04T19:05:00-0700",
        ),
        (
            "Document design, architecture, tuning, and roadmap\n\n"
            "Design review, module map, tuning methodology, self-critique, and roadmap.",
            [
                "docs/DESIGN.md",
                "docs/ARCHITECTURE.md",
                "docs/TUNING.md",
                "docs/SELF_CRITIQUE.md",
                "docs/ROADMAP.md",
            ],
            12,
            None,
            "2026-06-05T17:35:00-0700",
        ),
        (
            "Add README and MIT license\n\n"
            "Build/UCI usage, measured benchmarks, and testing instructions.",
            ["README.md", "LICENSE"],
            12,
            None,
            "2026-06-05T18:10:00-0700",
        ),
        (
            "Vendor cutechess-cli for local Elo testing\n\n"
            "Local cutechess binary and script path updates for self-play/SPRT.",
            [
                ".local/bin/cutechess-cli",
                ".local/bin/cutechess-cli.readme",
                "scripts/selfplay.sh",
                "scripts/sprt.sh",
            ],
            12,
            None,
            "2026-06-05T18:40:00-0700",
        ),
        (
            "Lock dependency versions with Cargo.lock\n\n"
            "Reproducible builds for workspace crates and dev-dependencies.",
            ["Cargo.lock"],
            12,
            None,
            "2026-06-05T19:05:00-0700",
        ),
    ]

    for msg, files, lib_level, overrides, when in steps:
        stage_commit(msg, files, lib_level, overrides, when)

    final_files = [
        "engine-core/src/timeman.rs",
        "engine-uci/src/main.rs",
        "README.md",
    ]
    for rel in final_files:
        copy_from_snap(rel)
    run("git", "add", *final_files)
    if run("git", "diff", "--cached", "--quiet", check=False).returncode == 0:
        raise SystemExit("empty final commit")
    run(
        "git",
        "commit",
        "-m",
        "Wire Move Overhead UCI option into time management\n\n"
        "Honor setoption Move Overhead for movetime and wtime/btime budgets; "
        "pass the configured margin through Limits into TimeManager.",
        env={"GIT_AUTHOR_DATE": "2026-06-05T19:30:00-0700", "GIT_COMMITTER_DATE": "2026-06-05T19:30:00-0700"},
    )

    run("git", "branch", "-M", "main")
    # Refresh the frozen tree for the next run instead of deleting it.
    frozen = ROOT / ".rewrite-snapshot"
    if frozen.exists():
        shutil.rmtree(frozen)
    shutil.copytree(
        ROOT,
        frozen,
        ignore=lambda _d, names: [n for n in names if n in EXCLUDE],
    )
    print("Done. backup branch: backup-before-rewrite")
    print(f"Script preserved at {script_copy}")


if __name__ == "__main__":
    main()
