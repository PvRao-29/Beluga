#!/usr/bin/env bash
# Build Beluga, install En Croissant locally, register the engine, and launch the GUI.
#
# Usage: scripts/gui.sh [--no-build] [--setup-only]
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BUILD=1
SETUP_ONLY=0

for arg in "$@"; do
  case "$arg" in
    --no-build) BUILD=0 ;;
    --setup-only) SETUP_ONLY=1 ;;
    *)
      echo "usage: scripts/gui.sh [--no-build] [--setup-only]" >&2
      exit 2
      ;;
  esac
done

if [[ "$BUILD" -eq 1 ]]; then
  echo "Building Beluga (release)..."
  cargo build --release -p beluga-uci --manifest-path "$ROOT/Cargo.toml"
fi

"$ROOT/scripts/setup_en_croissant.sh"
python3 "$ROOT/scripts/register_beluga_engine.py"

if [[ "$SETUP_ONLY" -eq 1 ]]; then
  exit 0
fi

OS="$(uname -s)"
case "$OS" in
  Darwin)
    APP="$ROOT/.local/en-croissant.app"
    if [[ ! -d "$APP" ]]; then
      echo "error: $APP not found after setup" >&2
      exit 1
    fi
    open "$APP"
    ;;
  Linux)
    APPIMAGE="$ROOT/.local/en-croissant.AppImage"
    if [[ ! -x "$APPIMAGE" ]]; then
      echo "error: $APPIMAGE not found after setup" >&2
      exit 1
    fi
    exec "$APPIMAGE"
    ;;
  *)
    echo "error: launch not implemented for $OS; run En Croissant manually after setup" >&2
    exit 1
    ;;
esac
