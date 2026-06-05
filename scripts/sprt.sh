#!/usr/bin/env bash
# SPRT regression gate between two engine builds using cutechess-cli.
#
# Usage: scripts/sprt.sh <new_binary> <base_binary> [book.pgn]
#
# Exits non-zero if the new build is not proven non-regressing. Requires
# cutechess-cli on PATH. Bounds are set for an incremental change (elo0=0,elo1=3).
set -euo pipefail

NEW="${1:?path to new engine binary}"
BASE="${2:?path to base engine binary}"
BOOK="${3:-}"

TC="10+0.1"          # 10s + 0.1s increment
CONCURRENCY="${CONCURRENCY:-$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 4)}"
ELO0=0
ELO1=3
ALPHA=0.05
BETA=0.05

BOOK_ARGS=()
if [[ -n "$BOOK" ]]; then
  BOOK_ARGS=(-openings "file=$BOOK" format=pgn order=random -repeat)
fi

command -v cutechess-cli >/dev/null 2>&1 || {
  echo "error: cutechess-cli not found on PATH" >&2
  exit 127
}

cutechess-cli \
  -engine name=new cmd="$NEW" proto=uci \
  -engine name=base cmd="$BASE" proto=uci \
  -each tc="$TC" \
  -concurrency "$CONCURRENCY" \
  -games 2 -rounds 5000 \
  -sprt elo0=$ELO0 elo1=$ELO1 alpha=$ALPHA beta=$BETA \
  -ratinginterval 20 \
  "${BOOK_ARGS[@]+"${BOOK_ARGS[@]}"}" \
  -pgnout sprt_games.pgn
