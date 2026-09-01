#!/usr/bin/env bash
# Build the classic-gfx web Basis Universal transcoder wasm.
#
# Reproducible build of `basis_transcoder.wasm` from BinomialLLC's
# basis_universal transcoder (Apache 2.0), transcoder-only (no encoder).
# The source revision is pinned so this script always reproduces the committed
# wasm byte-for-byte:
#
#   basis_universal @ ad9386a4a1cf2a248f7bbd45f543a7448db15267
#   (v1.16.4-16-gad9386a — the exact tree vendored by the native
#    `basis-universal` 0.3.1 crate, so web and native transcode identically)
#
# Usage (needs Emscripten — `nix develop` in classic-wgl provides `emcc`,
# currently 6.0.2 via the flake's pinned nixpkgs):
#   ./build.sh [--source-dir /path/to/basis_universal]
#
# If `--source-dir` is omitted, the script clones the pinned revision into a
# temporary directory (requires network + git).

set -euo pipefail

SOURCE_DIR=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --source-dir) SOURCE_DIR="$2"; shift 2 ;;
    *) echo "unknown arg: $1" >&2; exit 2 ;;
  esac
done

PIN="ad9386a4a1cf2a248f7bbd45f543a7448db15267"
HERE="$(cd "$(dirname "$0")" && pwd)"

if [[ -z "$SOURCE_DIR" ]]; then
  TMP="$(mktemp -d)"
  trap 'rm -rf "$TMP"' EXIT
  git clone --filter=blob:none https://github.com/BinomialLLC/basis_universal.git "$TMP"
  git -C "$TMP" checkout "$PIN"
  SOURCE_DIR="$TMP"
fi

if ! emcc --version >/dev/null 2>&1; then
  echo "emcc not found on PATH (run `nix develop` in classic-wgl first)" >&2
  exit 1
fi

emcc -O3 -std=c++11 \
  -s STANDALONE_WASM=1 \
  -s DEFAULT_TO_CXX=1 \
  --no-entry \
  -s EXPORTED_FUNCTIONS='["_malloc","_free","_classic_initialize","_classic_transcode"]' \
  -s ALLOW_MEMORY_GROWTH=1 \
  -s INITIAL_MEMORY=16777216 \
  -I "$SOURCE_DIR/transcoder" \
  -o "$HERE/basis_transcoder.wasm" \
  "$SOURCE_DIR/transcoder/basisu_transcoder.cpp" \
  "$HERE/transcoder_wrapper.cpp"

echo "built $HERE/basis_transcoder.wasm"
