#!/usr/bin/env bash
# Generate frontend-ready TypeScript bindings and ABI JSON from compiled
# Soroban contract WASM artifacts.
#
# Typical usage:
#   cd stellar-swipe
#   ./scripts/build.sh
#   ./scripts/generate_abi.sh
#
# Environment overrides:
#   WASM_DIR=target/wasm-optimized
#   BINDINGS_DIR=bindings
#   STELLAR_BIN=stellar

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$ROOT"

WASM_DIR="${WASM_DIR:-target/wasm-optimized}"
BINDINGS_DIR="${BINDINGS_DIR:-bindings}"
STELLAR_BIN="${STELLAR_BIN:-stellar}"
WRITE_ABI_JSON=true

usage() {
  cat <<'USAGE'
Usage: ./scripts/generate_abi.sh [options]

Options:
  --wasm-dir <dir>      Directory containing compiled .wasm files
                        (default: target/wasm-optimized)
  --output-dir <dir>    Directory for generated bindings
                        (default: bindings)
  --no-abi-json         Skip abi.json generation next to each binding package
  -h, --help            Show this help

The script runs:
  stellar contract bindings typescript --wasm <wasm> --output-dir <dir> --overwrite

It also writes abi.json using:
  stellar contract info interface --wasm <wasm> --output json-formatted
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --wasm-dir)
      WASM_DIR="${2:?missing value for --wasm-dir}"
      shift 2
      ;;
    --output-dir)
      BINDINGS_DIR="${2:?missing value for --output-dir}"
      shift 2
      ;;
    --no-abi-json)
      WRITE_ABI_JSON=false
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "error: unknown option: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

if ! command -v "$STELLAR_BIN" >/dev/null 2>&1; then
  echo "error: stellar CLI not found. Install e.g.: cargo install stellar-cli --locked --version 25.2.0" >&2
  exit 1
fi

if [[ ! -d "$WASM_DIR" ]]; then
  fallback="target/wasm32-unknown-unknown/release"
  if [[ -d "$fallback" ]]; then
    echo "warning: $WASM_DIR not found; falling back to $fallback" >&2
    WASM_DIR="$fallback"
  else
    echo "error: wasm directory not found: $WASM_DIR" >&2
    echo "hint: run ./scripts/build.sh first, or pass --wasm-dir" >&2
    exit 1
  fi
fi

mapfile -t wasm_files < <(find "$WASM_DIR" -maxdepth 1 -type f -name '*.wasm' | sort)
if [[ ${#wasm_files[@]} -eq 0 ]]; then
  echo "error: no .wasm artifacts found in $WASM_DIR" >&2
  exit 1
fi

mkdir -p "$BINDINGS_DIR"

echo "==> Generating TypeScript bindings from $WASM_DIR"
for wasm in "${wasm_files[@]}"; do
  base="$(basename "$wasm")"
  contract="${base%.wasm}"
  contract="${contract%.optimized}"
  out_dir="$BINDINGS_DIR/$contract"
  abi_tmp="$(mktemp)"

  "$STELLAR_BIN" contract info interface \
    --wasm "$wasm" \
    --output json-formatted > "$abi_tmp"

  if [[ ! -s "$abi_tmp" ]]; then
    echo "    $contract -> skipped (no Soroban contract interface)"
    rm -f "$abi_tmp"
    continue
  fi

  echo "    $contract -> $out_dir"
  "$STELLAR_BIN" contract bindings typescript \
    --wasm "$wasm" \
    --output-dir "$out_dir" \
    --overwrite

  if [[ "$WRITE_ABI_JSON" == true ]]; then
    cp "$abi_tmp" "$out_dir/abi.json"
  fi
  rm -f "$abi_tmp"
done

echo "Done. Contract bindings: $BINDINGS_DIR/"
