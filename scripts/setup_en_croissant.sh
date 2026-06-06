#!/usr/bin/env bash
# Download En Croissant into .local/ for interactive play and analysis.
#
# Usage: scripts/setup_en_croissant.sh [version]
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LOCAL="$ROOT/.local"
APP="$LOCAL/en-croissant.app"
VERSION="${1:-0.15.0}"
VERSION_FILE="$LOCAL/en-croissant.version"
TMP=""
MOUNT=""

cleanup() {
  if [[ -n "$MOUNT" ]] && mount | grep -q "$MOUNT"; then
    hdiutil detach -quiet "$MOUNT" || true
  fi
  if [[ -n "$TMP" ]]; then
    rm -rf "$TMP"
  fi
}
trap cleanup EXIT

if [[ -d "$APP" ]] && [[ -f "$VERSION_FILE" ]] && [[ "$(cat "$VERSION_FILE")" == "$VERSION" ]]; then
  echo "En Croissant $VERSION already installed at $APP"
  exit 0
fi

OS="$(uname -s)"
ARCH="$(uname -m)"
case "$OS:$ARCH" in
  Darwin:arm64)
    ASSET="en-croissant_${VERSION}_aarch64.dmg"
    ;;
  Darwin:x86_64)
    ASSET="en-croissant_${VERSION}_x64.dmg"
    ;;
  Linux:x86_64)
    ASSET="en-croissant_${VERSION}_amd64.AppImage"
    ;;
  *)
    echo "error: unsupported platform $OS/$ARCH (macOS arm64/x64 and Linux x86_64 only)" >&2
    exit 1
    ;;
esac

URL="https://github.com/franciscoBSalgueiro/en-croissant/releases/download/v${VERSION}/${ASSET}"
TMP="$(mktemp -d)"

mkdir -p "$LOCAL"
echo "Downloading En Croissant v${VERSION} (${ASSET})..."
curl -fsSL "$URL" -o "$TMP/$ASSET"

rm -rf "$APP"

case "$ASSET" in
  *.dmg)
    MOUNT="$TMP/mount"
    mkdir -p "$MOUNT"
    hdiutil attach -nobrowse -quiet -mountpoint "$MOUNT" "$TMP/$ASSET"
    APP_SRC="$(find "$MOUNT" -maxdepth 2 -iname '*.app' -print -quit)"
    if [[ -z "$APP_SRC" ]]; then
      echo "error: .app bundle not found in DMG" >&2
      exit 1
    fi
    ditto "$APP_SRC" "$APP"
    hdiutil detach -quiet "$MOUNT"
    MOUNT=""
    ;;
  *.AppImage)
    cp "$TMP/$ASSET" "$LOCAL/en-croissant.AppImage"
    chmod +x "$LOCAL/en-croissant.AppImage"
    ln -sf "$LOCAL/en-croissant.AppImage" "$LOCAL/en-croissant"
    ;;
esac

echo "$VERSION" >"$VERSION_FILE"
cat >"$LOCAL/en-croissant.readme" <<EOF
En Croissant ${VERSION}
Installed by scripts/setup_en_croissant.sh

macOS app: .local/en-croissant.app
Linux app: .local/en-croissant.AppImage

Upstream: https://github.com/franciscoBSalgueiro/en-croissant
License: GPL-3.0-only
EOF

echo "Installed En Croissant $VERSION at $APP"
