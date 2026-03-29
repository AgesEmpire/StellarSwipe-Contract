#!/usr/bin/env bash
# Verify a StellarSwipe Soroban deployment by calling health_check() on all contracts
# and validating cross-contract references. Exits 0 on full success, 1 on any failure.
#
# Usage: ./scripts/verify_deployment.sh
#
# Required env:
#   STELLAR_SOURCE_ACCOUNT   Signing identity / secret key (or STELLAR_ACCOUNT as fallback)
#
# Optional env:
#   DEPLOY_STATE             Path to deployment state JSON (default: $ROOT/deployments/testnet.json)
#   STELLAR_NETWORK          Network name for --network flag (default: testnet)
#   STELLAR_RPC_URL          Soroban RPC endpoint (default: https://soroban-testnet.stellar.org)
#   STELLAR_NETWORK_PASSPHRASE  Network passphrase (default: Test SDF Network ; September 2015)
#   ROOT                     Workspace root; auto-detected as grandparent of script dir

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="${ROOT:-$(cd "$SCRIPT_DIR/../.." && pwd)}"
STATE="${DEPLOY_STATE:-$ROOT/deployments/testnet.json}"
NET="${STELLAR_NETWORK:-testnet}"
RPC_URL="${STELLAR_RPC_URL:-https://soroban-testnet.stellar.org}"
NETWORK_PASSPHRASE="${STELLAR_NETWORK_PASSPHRASE:-Test SDF Network ; September 2015}"

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

die() { echo "error: $*" >&2; exit 1; }

# ---------------------------------------------------------------------------
# check_env — validate required tools and env vars
# ---------------------------------------------------------------------------

check_env() {
  command -v stellar >/dev/null \
    || die "stellar CLI not found — install stellar-cli and ensure it is on PATH"

  command -v jq >/dev/null \
    || die "jq not found — install jq and ensure it is on PATH"

  [[ -n "${STELLAR_SOURCE_ACCOUNT:-${STELLAR_ACCOUNT:-}}" ]] \
    || die "set STELLAR_SOURCE_ACCOUNT or STELLAR_ACCOUNT (signing key / identity)"

  SOURCE="${STELLAR_SOURCE_ACCOUNT:-${STELLAR_ACCOUNT}}"
  export SOURCE
}

# ---------------------------------------------------------------------------
# load_state — verify state file and extract all five contract IDs
# ---------------------------------------------------------------------------

load_state() {
  [[ -f "$STATE" ]] \
    || die "state file not found: $STATE (run deploy_testnet.sh first or set DEPLOY_STATE)"

  _extract_id() {
    local logical="$1"
    local id
    id="$(jq -r --arg k "$logical" '.contracts[$k].contract_id // empty' "$STATE")"
    [[ -n "$id" ]] \
      || die "contract ID missing or empty for '$logical' in state file: $STATE"
    echo "$id"
  }

  SIGNAL_REGISTRY_ID="$(_extract_id signal_registry)"
  FEE_COLLECTOR_ID="$(_extract_id fee_collector)"
  STAKE_VAULT_ID="$(_extract_id stake_vault)"
  USER_PORTFOLIO_ID="$(_extract_id user_portfolio)"
  TRADE_EXECUTOR_ID="$(_extract_id trade_executor)"

  export SIGNAL_REGISTRY_ID FEE_COLLECTOR_ID STAKE_VAULT_ID USER_PORTFOLIO_ID TRADE_EXECUTOR_ID
}

# ---------------------------------------------------------------------------
# Result tracking — stubs (fully implemented in Task 4.1)
# ---------------------------------------------------------------------------

FAILURES=()
PASS_COUNT=0
TOTAL_COUNT=0

# record_fail CONTRACT_NAME REASON [CONTRACT_ID]
# Appends to FAILURES[], increments TOTAL_COUNT, and prints a FAIL line immediately.
# For health checks (id non-empty):  [FAIL] <name>  (<id>)  <reason>
# For cross-ref checks (id empty):   [FAIL] <name>  <reason>
record_fail() {
  local name="$1"
  local reason="$2"
  local id="${3:-}"
  FAILURES+=("FAIL: $name${id:+ ($id)}: $reason")
  (( TOTAL_COUNT++ )) || true
  if [[ -n "$id" ]]; then
    printf '[FAIL] %s  (%s)  %s\n' "$name" "$id" "$reason"
  else
    printf '[FAIL] %s  %s\n' "$name" "$reason"
  fi
}

# record_pass CONTRACT_NAME [CONTRACT_ID]
# Increments PASS_COUNT and TOTAL_COUNT, and prints a PASS line immediately.
# For health checks (id non-empty):  [PASS] <name>  (<id>)  is_initialized=true  is_paused=false
# For cross-ref checks (id empty):   [PASS] <name>
record_pass() {
  local name="$1"
  local id="${2:-}"
  (( PASS_COUNT++ )) || true
  (( TOTAL_COUNT++ )) || true
  if [[ -n "$id" ]]; then
    printf '[PASS] %s  (%s)  is_initialized=true  is_paused=false\n' "$name" "$id"
  else
    printf '[PASS] %s\n' "$name"
  fi
}

# ---------------------------------------------------------------------------
# probe_health CONTRACT_NAME CONTRACT_ID
#
# Invokes `stellar contract invoke --send=no -- health_check` for the given
# contract ID.  On success, prints the raw JSON output to stdout.
# On CLI failure, calls record_fail() and prints nothing (caller should check
# the return value before using the output).
#
# Returns:
#   0  — invocation succeeded; raw JSON is on stdout
#   1  — invocation failed; failure already recorded via record_fail()
# ---------------------------------------------------------------------------

probe_health() {
  local name="$1"
  local contract_id="$2"
  local _outvar="$3"   # name of variable to receive output (avoids subshell)

  local output
  if output="$(stellar contract invoke \
      --id "$contract_id" \
      --source-account "$SOURCE" \
      --network "$NET" \
      --rpc-url "$RPC_URL" \
      --network-passphrase "$NETWORK_PASSPHRASE" \
      --send=no \
      -- health_check 2>&1)"; then
    # Write output into the caller's variable via printf + eval-safe assignment
    printf -v "$_outvar" '%s' "$output"
    return 0
  else
    record_fail "$name" "health_check() CLI invocation failed: $output" "$contract_id"
    return 1
  fi
}

# ---------------------------------------------------------------------------
# assert_healthy CONTRACT_NAME CONTRACT_ID
#
# Calls probe_health() and validates the returned HealthStatus JSON:
#   - is_initialized must be true  (Requirement 3.1)
#   - is_paused must be false      (Requirement 3.2)
# Records a pass only when both conditions hold (Requirement 3.3).
#
# Handles:
#   - probe_health() failure (non-zero return) — returns early; failure already recorded
#   - unparseable JSON — records failure with descriptive message
#   - is_initialized != true — records failure "is_initialized=false"
#   - is_paused == true      — records failure "is_paused=true"
# ---------------------------------------------------------------------------

assert_healthy() {
  local name="$1"
  local contract_id="$2"

  local health_output=""
  probe_health "$name" "$contract_id" health_output || return 0

  # Parse is_initialized
  local is_initialized
  is_initialized="$(printf '%s' "$health_output" | jq -r '.is_initialized // empty' 2>/dev/null)" || true

  # Parse is_paused
  local is_paused
  is_paused="$(printf '%s' "$health_output" | jq -r '.is_paused // empty' 2>/dev/null)" || true

  # Detect unparseable JSON (both fields empty when jq couldn't parse)
  if [[ -z "$is_initialized" && -z "$is_paused" ]]; then
    # Confirm it's truly unparseable by attempting to validate JSON
    if ! printf '%s' "$health_output" | jq empty 2>/dev/null; then
      record_fail "$name" "unparseable health_check output" "$contract_id"
      return 0
    fi
  fi

  local failed=false

  if [[ "$is_initialized" != "true" ]]; then
    record_fail "$name" "is_initialized=false" "$contract_id"
    failed=true
  fi

  if [[ "$is_paused" == "true" ]]; then
    record_fail "$name" "is_paused=true" "$contract_id"
    failed=true
  fi

  if [[ "$failed" == "false" ]]; then
    record_pass "$name" "$contract_id"
  fi
}

# ---------------------------------------------------------------------------
# check_cross_refs
#
# Invokes `get_user_portfolio` on the trade_executor contract and compares
# the returned address to the user_portfolio contract ID from the state file.
#
# Requirements: 4.1, 4.2, 4.3, 4.4
# ---------------------------------------------------------------------------

check_cross_refs() {
  local output
  if ! output="$(stellar contract invoke \
      --id "$TRADE_EXECUTOR_ID" \
      --source-account "$SOURCE" \
      --network "$NET" \
      --rpc-url "$RPC_URL" \
      --network-passphrase "$NETWORK_PASSPHRASE" \
      --send=no \
      -- get_user_portfolio 2>&1)"; then
    record_fail "cross_ref: trade_executor.get_user_portfolio" \
      "get_user_portfolio CLI invocation failed: $output"
    return 0
  fi

  # Strip surrounding quotes that the Stellar CLI may emit for string values
  local actual
  actual="$(printf '%s' "$output" | tr -d '"' | xargs)"

  if [[ -z "$actual" || "$actual" == "null" ]]; then
    record_fail "cross_ref: trade_executor.get_user_portfolio" \
      "UserPortfolio reference not configured in TradeExecutor"
    return 0
  fi

  if [[ "$actual" != "$USER_PORTFOLIO_ID" ]]; then
    record_fail "cross_ref: trade_executor.get_user_portfolio" \
      "address mismatch  expected: $USER_PORTFOLIO_ID  actual: $actual"
    return 0
  fi

  record_pass "cross_ref: trade_executor.get_user_portfolio == user_portfolio"
}

# ---------------------------------------------------------------------------
# summarize
#
# Prints the final verification summary and exits with the appropriate code.
#
# Output format:
#   --- Verification Summary ---
#   Passed: 5/6
#   Failed: 1/6
#     FAIL: trade_executor health_check: is_initialized=false
#
#   Exit code: 1
#
# Requirements: 5.2, 5.3, 6.1, 6.2
# ---------------------------------------------------------------------------

summarize() {
  local fail_count="${#FAILURES[@]}"
  local exit_code=0
  [[ "$fail_count" -eq 0 ]] || exit_code=1

  echo "--- Verification Summary ---"
  printf 'Passed: %s/%s\n' "$PASS_COUNT" "$TOTAL_COUNT"
  printf 'Failed: %s/%s\n' "$fail_count" "$TOTAL_COUNT"

  if [[ "$fail_count" -gt 0 ]]; then
    for failure in "${FAILURES[@]}"; do
      printf '  %s\n' "$failure"
    done
  fi

  printf '\nExit code: %s\n' "$exit_code"
  exit "$exit_code"
}

# ---------------------------------------------------------------------------
# Main execution block
#
# Requirements: 2.1, 6.3
# Each check is guarded with `|| true` so that `set -e` never aborts the
# script mid-run.  assert_healthy and check_cross_refs handle their own
# failures internally; the `|| true` here ensures the outer pipefail/errexit
# cannot short-circuit the remaining checks.
# ---------------------------------------------------------------------------

check_env
load_state

assert_healthy "signal_registry"  "$SIGNAL_REGISTRY_ID"  || true
assert_healthy "fee_collector"    "$FEE_COLLECTOR_ID"     || true
assert_healthy "stake_vault"      "$STAKE_VAULT_ID"       || true
assert_healthy "user_portfolio"   "$USER_PORTFOLIO_ID"    || true
assert_healthy "trade_executor"   "$TRADE_EXECUTOR_ID"    || true

check_cross_refs || true

summarize
