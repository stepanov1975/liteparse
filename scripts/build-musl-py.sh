#!/bin/sh
set -eux

# Mirrors scripts/build-musl-node.sh but for the Python wheel.
#
# Why a hand-rolled Alpine container instead of maturin-action's
# musllinux_1_2 image:
#   - musllinux_1_2 is Alpine-based but ships a musl-cross GCC whose default
#     specs define _FORTIFY_SOURCE=2. tesseract's C++ then references glibc
#     fortify wrappers (__printf_chk, __fprintf_chk, ...) that don't exist in
#     musl libc, so linking fails.
#   - tesseract-rs hard-codes clang++ + libc++ for its CMake build, so we want
#     the real Alpine clang/libc++ toolchain, not a musl-cross-gcc.
#   - tesseract-rs's build-deps (reqwest → native-tls → openssl-sys) get
#     compiled for the host. In the musllinux_1_2 cross container the host
#     openssl install is half-broken; native Alpine clang against system
#     openssl-dev just works.
#
# So we replicate the node build container pattern: a vanilla python:3-alpine,
# install clang/libc++/tesseract dev libs from apk, then run maturin against
# a self-contained venv.

apk add --no-cache \
  build-base cmake git curl pkgconf perl \
  clang libc++-dev llvm-libunwind-dev \
  tesseract-ocr-dev leptonica-dev \
  openssl-dev openssl-libs-static zlib-static \
  patchelf

curl --proto "=https" --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable -t x86_64-unknown-linux-musl
. /root/.cargo/env

python3 -m venv /tmp/venv
. /tmp/venv/bin/activate
pip install --upgrade pip maturin

# cdylib MUST be dynamically linked against libc.
export RUSTFLAGS="-C target-feature=-crt-static"

cd packages/python
rm -rf dist

# pyproject.toml's [tool.maturin] include rule expects liteparse/libpdfium.so
# to exist at build time so it gets packed into the wheel. pdfium-sys's build
# script downloads pdfium into ~/.cache/pdfium-rs; we need to compile once
# (which triggers the download) and then copy the .so into place before the
# real maturin build picks it up via include. Easiest: just run
# scripts/copy-pdfium.sh after a probe build, OR run copy-pdfium.sh directly
# pointing at the cache. The script auto-detects the cache path.
#
# Order: trigger the pdfium download via a cargo metadata-only compile of
# pdfium-sys (cheap), then copy, then real maturin build.
( cd ../../crates/pdfium-sys && cargo build --release --target x86_64-unknown-linux-musl )
sh scripts/copy-pdfium.sh

# --find-interpreter would try to build for every python on PATH; we only
# have the container's python (3.12 for python:3.12-alpine, etc.). Letting
# maturin auto-detect produces a single wheel for that interpreter, which is
# what we want for the musl matrix entry.
maturin build --release --out dist --target x86_64-unknown-linux-musl

WHEEL=$(ls dist/*.whl | head -n1)
echo "Built wheel: $WHEEL"

# The extension module lives at liteparse/_liteparse*.so inside the wheel.
# Inspect its DT_NEEDED entries (clang + libc++ runtime) and bundle anything
# non-system next to it, same logic as build-musl-node.sh. We use a scratch
# dir to unpack/repack the wheel.
WORK=$(mktemp -d)
unzip -q "$WHEEL" -d "$WORK"

EXT=$(find "$WORK/liteparse" -maxdepth 1 -name '_liteparse*.so' | head -n1)
[ -n "$EXT" ] || { echo "ERROR: could not find _liteparse*.so in wheel"; exit 1; }
echo "Extension module: $EXT"
echo "DT_NEEDED for extension:"
ldd "$EXT" || true

DEST=$(dirname "$EXT")
for needed in $(ldd "$EXT" 2>/dev/null | awk '{print $1}' | grep -E '^lib'); do
  case "$needed" in
    libc.musl-*.so.* | ld-musl-*.so.* | libc.so | libdl.so* | libm.so* | libpthread.so* | librt.so*)
      continue
      ;;
    # libpdfium is already bundled by maturin via pyproject.toml include rules.
    libpdfium.so)
      continue
      ;;
  esac
  if [ -f "$DEST/$needed" ]; then
    echo "Already bundled: $needed"
    continue
  fi
  src=$(find /usr/lib /usr/lib64 -maxdepth 3 -name "$needed" 2>/dev/null | head -n1)
  if [ -z "$src" ]; then
    echo "WARN: could not locate $needed under /usr/lib, skipping"
    continue
  fi
  src=$(readlink -f "$src")
  [ -f "$src" ] || { echo "ERROR: $src does not resolve to a file"; exit 1; }
  cp -v "$src" "$DEST/$needed"
done

echo "Bundled libs next to extension:"
ls -la "$DEST"

# Repack the wheel. Use python's zipfile to preserve wheel layout / metadata.
python3 - <<PY
import os, zipfile, sys
src = "$WORK"
out = "$WHEEL"
os.remove(out)
with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as z:
    for root, _, files in os.walk(src):
        for f in files:
            full = os.path.join(root, f)
            rel = os.path.relpath(full, src)
            z.write(full, rel)
print(f"Repacked {out}")
PY

echo "Final wheel contents:"
unzip -l "$WHEEL"
