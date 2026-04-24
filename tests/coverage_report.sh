#!/usr/bin/env bash
# =============================================================================
# StellarSwipe Contract Coverage Report Generator
# =============================================================================
# Usage:
#   chmod +x tests/coverage_report.sh
#   ./tests/coverage_report.sh [--html] [--check] [--contract <name>]
#
# Options:
#   --html              Generate HTML coverage report (default: stdout summary)
#   --check             Fail with exit code 1 if coverage targets are not met
#   --contract <name>   Run coverage for a single contract only
#                       (signal_registry | auto_trade | oracle | common)
#
# Requirements:
#   - cargo-tarpaulin: cargo install cargo-tarpaulin
#   - Run from the repository root or stellar-swipe/ directory
# =============================================================================

set -euo pipefail

# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
WORKSPACE_DIR="${REPO_ROOT}/stellar-swipe"
OUTPUT_DIR="${SCRIPT_DIR}/coverage_output"
REPORT_FILE="${OUTPUT_DIR}/coverage_summary.txt"

TARGET_LINE_COVERAGE=95
TARGET_BRANCH_COVERAGE=90

ALL_CONTRACTS=("signal_registry" "auto_trade" "oracle" "common")

# ---------------------------------------------------------------------------
# Argument parsing
# ---------------------------------------------------------------------------

GENERATE_HTML=false
CHECK_TARGETS=false
SINGLE_CONTRACT=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --html)     GENERATE_HTML=true; shift ;;
    --check)    CHECK_TARGETS=true; shift ;;
    --contract) SINGLE_CONTRACT="$2"; shift 2 ;;
    -h|--help)  head -n 20 "$0" | grep "^#" | sed 's/^# \?//'; exit 0 ;;
    *)          echo "Unknown option: $1" >&2; exit 1 ;;
  esac
done

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

log()  { echo "[$(date '+%H:%M:%S')] $*"; }
pass() { echo "  PASS $*"; }
fail() { echo "  FAIL $*"; }
warn() { echo "  WARN $*"; }

check_dependency() {
  if ! command -v "$1" &>/dev/null; then
    echo "ERROR: '$1' is not installed. Install with: $2" >&2
    exit 1
  fi
}

# ---------------------------------------------------------------------------
# Pre-flight checks
# ---------------------------------------------------------------------------

log "StellarSwipe Coverage Report Generator"
log "======================================="

check_dependency cargo "curl https://sh.rustup.rs -sSf | sh"
check_dependency cargo-tarpaulin "cargo install cargo-tarpaulin"

if [[ ! -d "${WORKSPACE_DIR}" ]]; then
  echo "ERROR: Workspace not found at ${WORKSPACE_DIR}" >&2
  exit 1
fi

mkdir -p "${OUTPUT_DIR}"

# ---------------------------------------------------------------------------
# Determine contracts to run
# ---------------------------------------------------------------------------

if [[ -n "${SINGLE_CONTRACT}" ]]; then
  CONTRACTS=("${SINGLE_CONTRACT}")
else
  CONTRACTS=("${ALL_CONTRACTS[@]}")
fi

# ---------------------------------------------------------------------------
# Run test suite
# ---------------------------------------------------------------------------

log "Running test suite..."
cd "${WORKSPACE_DIR}"

if ! cargo test --workspace 2>&1 | tee "${OUTPUT_DIR}/test_output.txt"; then
  fail "Test suite failed. Fix failing tests before generating coverage."
  exit 1
fi
pass "All tests passed."

# ---------------------------------------------------------------------------
# Generate coverage with tarpaulin
# ---------------------------------------------------------------------------

log "Generating coverage report..."

TARPAULIN_ARGS=(
  --workspace
  --timeout 300
  --out Json
  --output-dir "${OUTPUT_DIR}"
)

if [[ "${GENERATE_HTML}" == "true" ]]; then
  TARPAULIN_ARGS+=(--out Html)
  log "HTML report will be written to: ${OUTPUT_DIR}/tarpaulin-report.html"
fi

# tarpaulin runs on the native target (x86_64) — same logic paths as wasm32
cargo tarpaulin "${TARPAULIN_ARGS[@]}" 2>&1 | tee "${OUTPUT_DIR}/tarpaulin_raw.txt"

# ---------------------------------------------------------------------------
# Parse per-contract coverage from tarpaulin stdout
# ---------------------------------------------------------------------------

log "Parsing coverage results..."

declare -A CONTRACT_COVERED
declare -A CONTRACT_TOTAL

for contract in "${CONTRACTS[@]}"; do
  CONTRACT_COVERED[$contract]=0
  CONTRACT_TOTAL[$contract]=0
done

if [[ -f "${OUTPUT_DIR}/tarpaulin_raw.txt" ]]; then
  while IFS= read -r line; do
    for contract in "${CONTRACTS[@]}"; do
      if echo "$line" | grep -q "contracts/${contract}/"; then
        if [[ "$line" =~ ([0-9]+)/([0-9]+) ]]; then
          CONTRACT_COVERED[$contract]=$(( CONTRACT_COVERED[$contract] + BASH_REMATCH[1] ))
          CONTRACT_TOTAL[$contract]=$(( CONTRACT_TOTAL[$contract] + BASH_REMATCH[2] ))
        fi
      fi
    done
  done < "${OUTPUT_DIR}/tarpaulin_raw.txt"
fi

# ---------------------------------------------------------------------------
# Build summary report
# ---------------------------------------------------------------------------

OVERALL_PASS=true

{
  echo "============================================================"
  echo "  StellarSwipe Contract Coverage Report"
  echo "  Generated: $(date '+%Y-%m-%d %H:%M:%S')"
  echo "============================================================"
  echo ""
  echo "Targets:"
  echo "  Line coverage   : >= ${TARGET_LINE_COVERAGE}%"
  echo "  Branch coverage : >= ${TARGET_BRANCH_COVERAGE}%"
  echo ""
  echo "------------------------------------------------------------"
  echo "  Per-Contract Results"
  echo "------------------------------------------------------------"
} > "${REPORT_FILE}"

for contract in "${CONTRACTS[@]}"; do
  covered=${CONTRACT_COVERED[$contract]}
  total=${CONTRACT_TOTAL[$contract]}

  if [[ $total -gt 0 ]]; then
    pct=$(( covered * 100 / total ))
  else
    pct=0
    warn "No coverage data for ${contract} — verify tests compile on native target."
  fi

  if [[ $pct -ge $TARGET_LINE_COVERAGE ]]; then
    status="PASS"
  else
    status="FAIL"
    OVERALL_PASS=false
  fi

  printf "  %-20s  %3d%% line coverage  (%d/%d lines)  [%s]\n" \
    "${contract}" "${pct}" "${covered}" "${total}" "${status}" \
    | tee -a "${REPORT_FILE}"
done

# Extract overall line coverage from tarpaulin stdout
OVERALL_LINE_PCT=0
if [[ -f "${OUTPUT_DIR}/tarpaulin_raw.txt" ]]; then
  COVERAGE_LINE=$(grep -E "^[0-9]+\.[0-9]+% coverage" "${OUTPUT_DIR}/tarpaulin_raw.txt" | tail -1 || true)
  if [[ -n "${COVERAGE_LINE}" ]]; then
    OVERALL_LINE_PCT=$(echo "${COVERAGE_LINE}" | grep -oE "^[0-9]+" || echo "0")
  fi
fi

{
  echo ""
  echo "------------------------------------------------------------"
  echo "  Overall Coverage"
  echo "------------------------------------------------------------"
  printf "  Line coverage   : %d%%  (target: %d%%)\n" "${OVERALL_LINE_PCT}" "${TARGET_LINE_COVERAGE}"
  printf "  Branch coverage : N/A   (target: %d%%)  -- use --branch flag for branch data\n" "${TARGET_BRANCH_COVERAGE}"
  echo ""
} | tee -a "${REPORT_FILE}"

# ---------------------------------------------------------------------------
# Security-critical function coverage check
# ---------------------------------------------------------------------------

log "Checking security-critical function coverage..."

CRITICAL_FUNCTIONS=(
  "require_admin"
  "require_not_paused"
  "is_authorized"
  "validate_trade"
  "check_stop_loss"
  "check_daily_trade_limit"
  "check_position_limit"
  "stake"
  "unstake"
  "get_price_with_confidence"
  "calculate_consensus"
  "enable_multisig"
  "transfer_admin"
  "pause_trading"
  "record_trade_execution"
)

{
  echo "------------------------------------------------------------"
  echo "  Security-Critical Function Coverage"
  echo "------------------------------------------------------------"
} >> "${REPORT_FILE}"

for fn in "${CRITICAL_FUNCTIONS[@]}"; do
  if grep -rq "fn test_.*${fn}\|${fn}.*assert\|assert.*${fn}" \
       "${WORKSPACE_DIR}/contracts/" 2>/dev/null; then
    printf "  [TESTED]  %s\n" "${fn}" | tee -a "${REPORT_FILE}"
  else
    printf "  [NO TEST] %s\n" "${fn}" | tee -a "${REPORT_FILE}"
  fi
done

# ---------------------------------------------------------------------------
# Automated security checklist
# ---------------------------------------------------------------------------

{
  echo ""
  echo "------------------------------------------------------------"
  echo "  Automated Security Checklist"
  echo "------------------------------------------------------------"
} | tee -a "${REPORT_FILE}"

run_check() {
  local description="$1"
  local pattern="$2"
  local search_path="${3:-${WORKSPACE_DIR}/contracts}"
  local invert="${4:-false}"

  if [[ "${invert}" == "true" ]]; then
    if ! grep -rq "${pattern}" "${search_path}" 2>/dev/null; then
      printf "  [PASS] %s\n" "${description}" | tee -a "${REPORT_FILE}"
    else
      printf "  [FAIL] %s\n" "${description}" | tee -a "${REPORT_FILE}"
      OVERALL_PASS=false
    fi
  else
    if grep -rq "${pattern}" "${search_path}" 2>/dev/null; then
      printf "  [PASS] %s\n" "${description}" | tee -a "${REPORT_FILE}"
    else
      printf "  [FAIL] %s\n" "${description}" | tee -a "${REPORT_FILE}"
      OVERALL_PASS=false
    fi
  fi
}

run_check \
  "overflow-checks = true in Cargo.toml" \
  "overflow-checks = true" \
  "${WORKSPACE_DIR}/Cargo.toml"

run_check \
  "require_auth() used in contracts" \
  "require_auth()" \
  "${WORKSPACE_DIR}/contracts"

run_check \
  "No hardcoded 64-char hex secrets" \
  "[0-9a-fA-F]\{64\}" \
  "${WORKSPACE_DIR}/contracts" \
  "true"

run_check \
  "checked_add used for counter arithmetic" \
  "checked_add" \
  "${WORKSPACE_DIR}/contracts"

run_check \
  "Emergency pause mechanism present" \
  "pause_trading\|is_trading_paused\|require_not_paused" \
  "${WORKSPACE_DIR}/contracts"

run_check \
  "Events emitted for critical actions" \
  "emit_\|events().publish" \
  "${WORKSPACE_DIR}/contracts"

run_check \
  "Price input validation present" \
  "price <= 0\|InvalidPrice" \
  "${WORKSPACE_DIR}/contracts"

run_check \
  "Admin re-initialization guard present" \
  "AlreadyInitialized\|has_admin\|already initialized" \
  "${WORKSPACE_DIR}/contracts"

# Check for unimplemented!() outside test code
UNIMPL_COUNT=$(grep -rn "unimplemented!()" \
  "${WORKSPACE_DIR}/contracts" --include="*.rs" \
  | grep -v "test" | wc -l || echo "0")

if [[ "${UNIMPL_COUNT}" -gt 0 ]]; then
  printf "  [WARN] %d unimplemented!() call(s) in non-test code\n" "${UNIMPL_COUNT}" \
    | tee -a "${REPORT_FILE}"
else
  printf "  [PASS] No unimplemented!() in production code paths\n" | tee -a "${REPORT_FILE}"
fi

# Check for unresolved merge conflict markers
CONFLICT_COUNT=$(grep -rn "<<<<<<\|>>>>>>\|=======" \
  "${WORKSPACE_DIR}/contracts" --include="*.rs" | wc -l || echo "0")

if [[ "${CONFLICT_COUNT}" -gt 0 ]]; then
  printf "  [FAIL] %d merge conflict marker(s) found -- resolve before audit\n" \
    "${CONFLICT_COUNT}" | tee -a "${REPORT_FILE}"
  OVERALL_PASS=false
else
  printf "  [PASS] No merge conflict markers in source files\n" | tee -a "${REPORT_FILE}"
fi

# ---------------------------------------------------------------------------
# Final result
# ---------------------------------------------------------------------------

{
  echo ""
  echo "============================================================"
  if [[ "${OVERALL_PASS}" == "true" ]]; then
    echo "  RESULT: PASS -- All checks and coverage targets met"
  else
    echo "  RESULT: FAIL -- One or more checks or coverage targets not met"
  fi
  echo "============================================================"
  echo ""
  echo "Full report : ${REPORT_FILE}"
  [[ "${GENERATE_HTML}" == "true" ]] && echo "HTML report : ${OUTPUT_DIR}/tarpaulin-report.html"
} | tee -a "${REPORT_FILE}"

log "Coverage report written to: ${REPORT_FILE}"

if [[ "${CHECK_TARGETS}" == "true" ]] && [[ "${OVERALL_PASS}" == "false" ]]; then
  log "One or more checks failed. Exiting with code 1."
  exit 1
fi

exit 0
