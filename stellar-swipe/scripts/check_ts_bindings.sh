#!/usr/bin/env bash
# check_ts_bindings.sh — issue #582
#
# CI gate: generate TypeScript client bindings from the current compiled
# contract WASMs and fail if the generated output differs from the bindings
# that are checked into the frontend.
#
# Usage:
#   ./scripts/check_ts_bindings.sh [--update]
#
# Options:
#   --update   Regenerate and overwrite the checked-in bindings (run locally
#              after an intentional contract interface change, then commit the
#              updated files).
#
# Pre-requisites:
#   - stellar CLI installed and in PATH
#   - Contract WASMs built in target/wasm-optimized/ (run scripts/build.sh first)
#   - frontend/ directory exists at ../../frontend (or FRONTEND_DIR env var)
#
# How to update after an intentional contract change:
#   1. Rebuild WASMs:    ./scripts/build.sh
#   2. Regenerate:       ./scripts/check_ts_bindings.sh --update
#   3. Commit the diff in frontend/src/contracts/generated/
#   4. Push and open your PR — CI should now pass.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
WASM_DIR="$WORKSPACE_DIR/target/wasm-optimized"
FRONTEND_DIR="${FRONTEND_DIR:-$WORKSPACE_DIR/../../frontend}"
GENERATED_DIR="$FRONTEND_DIR/src/contracts/generated"
TMP_DIR="$(mktemp -d)"
UPDATE_MODE=false

if [[ "${1:-}" == "--update" ]]; then
    UPDATE_MODE=true
fi

cleanup() { rm -rf "$TMP_DIR"; }
trap cleanup EXIT

# ── Validate pre-requisites ───────────────────────────────────────────────────

if ! command -v stellar &> /dev/null; then
    echo "ERROR: stellar CLI not found. Install it first:"
    echo "  https://developers.stellar.org/docs/tools/stellar-cli"
    exit 1
fi

if [[ ! -d "$WASM_DIR" ]]; then
    echo "ERROR: WASM directory not found: $WASM_DIR"
    echo "Run ./scripts/build.sh first to compile optimised WASMs."
    exit 1
fi

# ── Generate TypeScript bindings from each WASM ──────────────────────────────

echo "Generating TypeScript bindings from WASMs in: $WASM_DIR"

CONTRACTS=()
for wasm in "$WASM_DIR"/*.wasm; do
    [[ -f "$wasm" ]] || continue
    name=$(basename "$wasm" .wasm | sed 's/_optimized$//')
    CONTRACTS+=("$name")

    output_dir="$TMP_DIR/$name"
    mkdir -p "$output_dir"

    stellar contract bindings typescript \
        --wasm "$wasm" \
        --output-dir "$output_dir" \
        --overwrite \
        2>/dev/null || {
            echo "WARNING: could not generate bindings for $name — skipping"
            continue
        }

    echo "  Generated: $name"
done

if [[ ${#CONTRACTS[@]} -eq 0 ]]; then
    echo "ERROR: No WASM files found in $WASM_DIR"
    exit 1
fi

# ── Compare or update ─────────────────────────────────────────────────────────

if $UPDATE_MODE; then
    echo ""
    echo "UPDATE MODE: copying generated bindings to $GENERATED_DIR"
    mkdir -p "$GENERATED_DIR"
    cp -r "$TMP_DIR"/. "$GENERATED_DIR/"
    echo "Done. Commit the changes in $GENERATED_DIR before pushing."
    exit 0
fi

# Check mode: compare generated vs checked-in.
DIFFS=0
if [[ ! -d "$GENERATED_DIR" ]]; then
    echo "ERROR: Frontend generated-bindings directory not found: $GENERATED_DIR"
    echo "Run with --update to generate them for the first time."
    exit 1
fi

for name in "${CONTRACTS[@]}"; do
    generated="$TMP_DIR/$name"
    checked_in="$GENERATED_DIR/$name"

    if [[ ! -d "$checked_in" ]]; then
        echo "MISSING: checked-in bindings not found for contract '$name'"
        echo "  Expected at: $checked_in"
        DIFFS=$((DIFFS + 1))
        continue
    fi

    if ! diff -rq "$generated" "$checked_in" > /dev/null 2>&1; then
        echo "DRIFT: bindings for '$name' differ from checked-in version"
        diff -r "$generated" "$checked_in" || true
        DIFFS=$((DIFFS + 1))
    fi
done

echo ""
if [[ $DIFFS -gt 0 ]]; then
    echo "ERROR: $DIFFS contract binding(s) have drifted."
    echo ""
    echo "To regenerate and update the checked-in bindings:"
    echo "  1. Build WASMs:  ./scripts/build.sh"
    echo "  2. Update:       ./scripts/check_ts_bindings.sh --update"
    echo "  3. Commit the changes in frontend/src/contracts/generated/"
    exit 1
else
    echo "OK: all TypeScript bindings match the compiled contracts."
    exit 0
fi
