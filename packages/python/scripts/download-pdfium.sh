#!/usr/bin/env bash
#
# Download the prebuilt pdfium binary for a given target triple and stage it
# at `packages/python/liteparse/<dylib>` so maturin's `[tool.maturin] include`
# rule packs it into the wheel (and records it in RECORD) at build time.
#
# Usage: ./scripts/download-pdfium.sh <target-triple>
#
# The release tag is parsed from `crates/pdfium-sys/build.rs` so we only have
# one source of truth for the pdfium version.

set -euo pipefail

TARGET="${1:?usage: download-pdfium.sh <target-triple>}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PKG_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
REPO_ROOT="$(cd "$PKG_DIR/../.." && pwd)"

BUILD_RS="$REPO_ROOT/crates/pdfium-sys/build.rs"
TAG=$(grep -E 'const PDFIUM_RELEASE_TAG' "$BUILD_RS" | sed -E 's/.*"([^"]+)".*/\1/')
[ -n "$TAG" ] || { echo "Could not parse PDFIUM_RELEASE_TAG from $BUILD_RS" >&2; exit 1; }

case "$TARGET" in
  aarch64-apple-darwin)        ASSET="pdfium-mac-arm64";      DYLIB="libpdfium.dylib" ;;
  x86_64-apple-darwin)         ASSET="pdfium-mac-x64";        DYLIB="libpdfium.dylib" ;;
  x86_64-unknown-linux-gnu)    ASSET="pdfium-linux-x64";      DYLIB="libpdfium.so"    ;;
  aarch64-unknown-linux-gnu)   ASSET="pdfium-linux-arm64";    DYLIB="libpdfium.so"    ;;
  x86_64-unknown-linux-musl)   ASSET="pdfium-linux-musl-x64"; DYLIB="libpdfium.so"    ;;
  aarch64-unknown-linux-musl)  ASSET="pdfium-linux-arm64";    DYLIB="libpdfium.so"    ;;
  x86_64-pc-windows-msvc)      ASSET="pdfium-win-x64";        DYLIB="pdfium.dll"      ;;
  aarch64-pc-windows-msvc)     ASSET="pdfium-win-arm64";      DYLIB="pdfium.dll"      ;;
  *) echo "Unsupported target: $TARGET" >&2; exit 1 ;;
esac

URL_TAG="${TAG//\//%2F}"
URL="https://github.com/run-llama/pdfium-binaries/releases/download/${URL_TAG}/${ASSET}.tgz"

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

echo "Downloading $URL"
curl -fsSL "$URL" -o "$TMP/${ASSET}.tgz"
tar -xzf "$TMP/${ASSET}.tgz" -C "$TMP"

# pdfium-binaries layout: lib/<DYLIB> on unix, bin/pdfium.dll on windows
SRC=""
for candidate in "$TMP/lib/$DYLIB" "$TMP/bin/$DYLIB"; do
  if [ -f "$candidate" ]; then
    SRC="$candidate"
    break
  fi
done
[ -n "$SRC" ] || { echo "Could not find $DYLIB in archive" >&2; ls -R "$TMP" >&2; exit 1; }

# Mirror the install-name fix in crates/pdfium-sys/build.rs so the dylib resolves
# via @rpath at runtime. pdfium-binaries ships macOS dylibs with install name
# `./libpdfium.dylib` which won't be found via our rpath.
if [ "$DYLIB" = "libpdfium.dylib" ] && command -v install_name_tool >/dev/null 2>&1; then
  install_name_tool -id "@rpath/libpdfium.dylib" "$SRC"
fi

DEST_DIR="$PKG_DIR/liteparse"
mkdir -p "$DEST_DIR"
cp "$SRC" "$DEST_DIR/$DYLIB"
echo "Staged $DEST_DIR/$DYLIB"
