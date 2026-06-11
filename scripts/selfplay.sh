#!/usr/bin/env bash
# Self-play gauntlet vs. a baseline engine, then rate with Ordo if available.
#
# Usage: scripts/selfplay.sh <beluga_binary> <opponent_binary> [games] [book.pgn]
#
# Time control: set TC (cutechess format), default 8+0.08
#   TC=10+0.1 scripts/selfplay.sh ./target/release/beluga engines/Stockfish
#   TC=60+0.6 scripts/selfplay.sh ./target/release/beluga engines/Tuna 500
#
# PGN output: games/YYYY.MM.DD_Beluga - <Opponent>.pgn
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
GAMES_DIR="$ROOT/games"

ENGINE="${1:?path to beluga binary}"
OPP="${2:?path to opponent binary}"
GAMES="${3:-1000}"
BOOK="${4:-}"

TC="${TC:-8+0.08}"
CONCURRENCY="${CONCURRENCY:-$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 4)}"

OPP_NAME="$(basename "$OPP")"
OPP_NAME="${OPP_NAME%.*}"
DATE="$(date +%Y.%m.%d)"
mkdir -p "$GAMES_DIR"
PGN="$GAMES_DIR/${DATE}_Beluga - ${OPP_NAME}.pgn"
if [[ -e "$PGN" ]]; then
  n=2
  while [[ -e "$GAMES_DIR/${DATE}_Beluga - ${OPP_NAME} (${n}).pgn" ]]; do
    n=$((n + 1))
  done
  PGN="$GAMES_DIR/${DATE}_Beluga - ${OPP_NAME} (${n}).pgn"
fi

command -v cutechess-cli >/dev/null 2>&1 || {
  echo "error: cutechess-cli not found on PATH" >&2
  exit 127
}

BOOK_ARGS=()
if [[ -n "$BOOK" ]]; then
  BOOK_ARGS=(-openings "file=$BOOK" format=pgn order=random -repeat)
fi

cutechess-cli \
  -engine name=beluga cmd="$ENGINE" proto=uci \
  -engine name=opp cmd="$OPP" proto=uci \
  -each tc="$TC" \
  -concurrency "$CONCURRENCY" \
  -games 2 -rounds "$((GAMES / 2))" \
  "${BOOK_ARGS[@]+"${BOOK_ARGS[@]}"}" \
  -pgnout "$PGN"

echo "Games written to $PGN (tc=$TC)"
if command -v ordo >/dev/null 2>&1; then
  ordo -Q -D -a 0 -A beluga -W -n "$CONCURRENCY" -p "$PGN"
else
  echo "(install 'ordo' or 'bayeselo' to compute Elo from $PGN)"
fi
