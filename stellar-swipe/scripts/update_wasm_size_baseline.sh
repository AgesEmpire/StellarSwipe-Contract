#!/usr/bin/env bash
# Update the committed WASM size baseline after an intentional change.
#
# Usage:
#   ./scripts/update_wasm_size_baseline.sh [--wasm-dir <path>]
#
# This builds the optimized WASM artifacts, then updates
# baselines/wasm_size_baseline.json with the new measurements.
#
# After running, review the diff and commit:
#   git add baselines/wasm_size_baseline.json
#   git commit -m "chore: update WASM size baseline — <reason>"
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR/.."

WASM_DIR="${1:-target/wasm-optimized}"

if [ ! -d "$WASM_DIR" ]; then
  echo "==> Building optimized WASM artifacts first..."
  chmod +x scripts/build.sh
  ./scripts/build.sh
fi

echo "==> Updating WASM size baseline from $WASM_DIR..."
python3 scripts/check_wasm_size.py --wasm-dir "$WASM_DIR" --update

echo ""
echo "Done. Review the diff and commit baselines/wasm_size_baseline.json."
