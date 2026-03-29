#!/usr/bin/env bash
# Build Soroban contract WASM with the workspace release profile, then shrink
# each artifact with `stellar contract optimize` (wasm-opt pipeline).
#
# Run from the workspace root (this directory's parent):
#   cd stellar-swipe && ./scripts/build.sh
#
# Options:
#   ./scripts/build.sh           Release build + optimize → target/wasm-optimized/
#   ./scripts/build.sh --compare Also build debug WASM and print a size table
#                                (for PR before/after notes).
#
# Requires:
#   - rustup target wasm32-unknown-unknown
#   - stellar CLI on PATH (e.g. cargo install stellar-cli --locked)
#
# Output:
#   - target/wasm32-unknown-unknown/release/*.wasm   (cargo --release)
#   - target/wasm-optimized/*.wasm                  (stellar contract optimize)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$ROOT"

TARGET="wasm32-unknown-unknown"
RELEASE_DIR="target/$TARGET/release"
DEBUG_DIR="target/$TARGET/debug"
OPT_DIR="${OPT_DIR:-target/wasm-optimized}"

compare=false
if [[ "${1:-}" == "--compare" ]]; then
  compare=true
fi

need_stellar() {
  if ! command -v stellar >/dev/null 2>&1; then
    echo "error: stellar CLI not found. Install e.g.: cargo install stellar-cli --locked" >&2
    exit 1
  fi
}

file_size() {
  wc -c <"$1" | tr -d ' '
}

if [[ "$compare" == true ]]; then
  echo "==> Building debug WASM (for size comparison)..."
  cargo build --workspace --target "$TARGET"
fi

echo "==> Building release WASM (workspace [profile.release]: opt-level=z, lto, strip, codegen-units=1)..."
cargo build --workspace --target "$TARGET" --release

need_stellar
mkdir -p "$OPT_DIR"
echo "==> stellar contract optimize → $OPT_DIR/"
shopt -s nullglob
release_wasm=( "$RELEASE_DIR"/*.wasm )
if [[ ${#release_wasm[@]} -eq 0 ]]; then
  echo "warning: no *.wasm in $RELEASE_DIR (cdylib members may have failed to build)" >&2
  exit 1
fi
for wasm in "${release_wasm[@]}"; do
  base=$(basename "$wasm")
  stellar contract optimize --wasm "$wasm" --wasm-out "$OPT_DIR/$base"
done

if [[ "$compare" == true ]]; then
  echo ""
  echo "| wasm | debug (bytes) | release (bytes) | optimized (bytes) | vs debug |"
  echo "|------|--------------:|----------------:|------------------:|---------:|"
  for opt in "$OPT_DIR"/*.wasm; do
    base=$(basename "$opt")
    dbg="$DEBUG_DIR/$base"
    rel="$RELEASE_DIR/$base"
    os=$(file_size "$opt")
    if [[ -f "$rel" ]]; then
      rs=$(file_size "$rel")
    else
      rs="—"
    fi
    if [[ -f "$dbg" ]]; then
      ds=$(file_size "$dbg")
      pct=$(( (ds - os) * 100 / ds ))
      echo "| $base | $ds | $rs | $os | ${pct}% smaller |"
    else
      echo "| $base | — | $rs | $os | — |"
    fi
  done
  echo ""
  echo "Paste the table above into your PR description. Target: optimized ≥30% smaller than debug."
fi

echo "Done. Optimized WASM: $OPT_DIR/"
