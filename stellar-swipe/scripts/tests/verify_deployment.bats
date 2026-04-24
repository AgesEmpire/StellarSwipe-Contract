#!/usr/bin/env bats
# verify_deployment.bats — unit tests for scripts/verify_deployment.sh
#
# Run with:  bats scripts/tests/verify_deployment.bats

load 'helpers/stub_stellar'

# ---------------------------------------------------------------------------
# Paths
# ---------------------------------------------------------------------------

# Resolve the script under test relative to this file's directory
SCRIPT_DIR="$(cd "$(dirname "$BATS_TEST_FILENAME")" && pwd)"
VERIFY_SCRIPT="$(cd "$SCRIPT_DIR/.." && pwd)/verify_deployment.sh"

# ---------------------------------------------------------------------------
# Healthy HealthStatus JSON used by most tests
# ---------------------------------------------------------------------------

HEALTHY_JSON='{"is_initialized":true,"is_paused":false,"version":"0.1.0","admin":"GADMIN"}'

# ---------------------------------------------------------------------------
# Helper: write a valid state file into $BATS_TEST_TMPDIR
#
# Usage: make_state_file [signal_registry_id] [fee_collector_id] ...
#   Positional args override the default contract IDs.
# ---------------------------------------------------------------------------

make_state_file() {
  local sr_id="${1:-CSIGNAL}"
  local fc_id="${2:-CFEE}"
  local sv_id="${3:-CSTAKE}"
  local up_id="${4:-CPORTFOLIO}"
  local te_id="${5:-CTRADE}"

  local state_dir="$BATS_TEST_TMPDIR/deployments"
  mkdir -p "$state_dir"

  printf '{\n  "contracts": {\n    "signal_registry": { "contract_id": "%s" },\n    "fee_collector":   { "contract_id": "%s" },\n    "stake_vault":     { "contract_id": "%s" },\n    "user_portfolio":  { "contract_id": "%s" },\n    "trade_executor":  { "contract_id": "%s" }\n  }\n}\n' \
    "$sr_id" "$fc_id" "$sv_id" "$up_id" "$te_id" \
    > "$state_dir/testnet.json"

  echo "$state_dir/testnet.json"
}

# ---------------------------------------------------------------------------
# setup / teardown
# ---------------------------------------------------------------------------

setup() {
  setup_stellar_stub

  # Point ROOT at a temp dir so the script never touches real deployments
  export ROOT="$BATS_TEST_TMPDIR"

  # Satisfy the mandatory env-var check
  export STELLAR_SOURCE_ACCOUNT="test-account"

  # Use dummy network values so no real network calls are attempted
  export STELLAR_NETWORK="testnet"
  export STELLAR_RPC_URL="http://localhost:9999"
  export STELLAR_NETWORK_PASSPHRASE="Test Passphrase"

  # Default: stub returns healthy JSON for every contract
  export STELLAR_STUB_OUTPUT="$HEALTHY_JSON"
  export STELLAR_STUB_EXIT_CODE="0"
}

teardown() {
  teardown_stellar_stub
}

# ---------------------------------------------------------------------------
# Helper: configure the cross-ref stub so get_user_portfolio returns the
# user_portfolio contract ID (matching the state file default CPORTFOLIO).
# Keyed on contract_id + function name so it doesn't affect health_check.
# ---------------------------------------------------------------------------

setup_matching_cross_ref() {
  local te_id="${1:-CTRADE}"
  local up_id="${2:-CPORTFOLIO}"
  stub_stellar_set_output "$te_id" "\"$up_id\"" "get_user_portfolio"
}

# ===========================================================================
# Test 1 — Happy path: all 5 contracts healthy, cross-ref matches → exit 0
# ===========================================================================

@test "happy path: all contracts healthy and cross-ref matches exits 0 with 6 PASS lines" {
  make_state_file > /dev/null
  setup_matching_cross_ref

  run bash "$VERIFY_SCRIPT"

  [ "$status" -eq 0 ]

  # Count PASS lines in stdout
  pass_count="$(echo "$output" | grep -c '^\[PASS\]')"
  [ "$pass_count" -eq 6 ]

  # No FAIL lines
  fail_count="$(echo "$output" | grep -c '^\[FAIL\]' || true)"
  [ "$fail_count" -eq 0 ]
}

# ===========================================================================
# Test 2 — Missing state file → exit 1, error contains file path
# ===========================================================================

@test "missing state file exits 1 with path in error message" {
  # Do NOT create a state file; point DEPLOY_STATE at a nonexistent path
  export DEPLOY_STATE="$BATS_TEST_TMPDIR/does_not_exist/testnet.json"

  run bash "$VERIFY_SCRIPT"

  [ "$status" -eq 1 ]
  # die() writes to stderr; bats captures combined output in $output when
  # using plain `run`, but stderr is separate. Use run with redirect.
  [[ "$output" == *"$DEPLOY_STATE"* ]] || [[ "${stderr:-}" == *"$DEPLOY_STATE"* ]]
}

@test "missing state file error message appears on stderr" {
  export DEPLOY_STATE="$BATS_TEST_TMPDIR/no_such_dir/testnet.json"

  # Capture stderr explicitly
  run bash -c "bash '$VERIFY_SCRIPT' 2>&1 1>/dev/null"

  [ "$status" -eq 1 ]
  [[ "$output" == *"$DEPLOY_STATE"* ]]
}

# ===========================================================================
# Test 3 — State file missing one contract ID → exit 1, error contains name
# ===========================================================================

@test "state file missing fee_collector contract_id exits 1 with contract name in error" {
  local state_dir="$BATS_TEST_TMPDIR/deployments"
  mkdir -p "$state_dir"

  # Write state file without fee_collector using printf to avoid heredoc parse issues
  printf '{\n  "contracts": {\n    "signal_registry": { "contract_id": "CSIGNAL" },\n    "stake_vault":     { "contract_id": "CSTAKE" },\n    "user_portfolio":  { "contract_id": "CPORTFOLIO" },\n    "trade_executor":  { "contract_id": "CTRADE" }\n  }\n}\n' \
    > "$state_dir/testnet.json"

  run bash -c "bash '$VERIFY_SCRIPT' 2>&1"

  [ "$status" -eq 1 ]
  [[ "$output" == *"fee_collector"* ]]
}

# ===========================================================================
# Test 4 — One contract returns is_initialized: false → exit 1, others checked
# ===========================================================================

@test "contract with is_initialized false causes exit 1 and others are still checked" {
  make_state_file > /dev/null
  setup_matching_cross_ref

  # Make stake_vault return is_initialized=false
  stub_stellar_set_output "CSTAKE" '{"is_initialized":false,"is_paused":false,"version":"0.1.0","admin":"GADMIN"}'

  run bash "$VERIFY_SCRIPT"

  [ "$status" -eq 1 ]

  # stake_vault should FAIL
  [[ "$output" == *"[FAIL]"*"stake_vault"* ]]

  # Other contracts should still be checked (appear in output)
  [[ "$output" == *"signal_registry"* ]]
  [[ "$output" == *"fee_collector"* ]]
  [[ "$output" == *"user_portfolio"* ]]
  [[ "$output" == *"trade_executor"* ]]
}

# ===========================================================================
# Test 5 — One contract returns is_paused: true → exit 1, others still checked
# ===========================================================================

@test "contract with is_paused true causes exit 1 and others are still checked" {
  make_state_file > /dev/null
  setup_matching_cross_ref

  # Make fee_collector return is_paused=true
  stub_stellar_set_output "CFEE" '{"is_initialized":true,"is_paused":true,"version":"0.1.0","admin":"GADMIN"}'

  run bash "$VERIFY_SCRIPT"

  [ "$status" -eq 1 ]

  [[ "$output" == *"[FAIL]"*"fee_collector"* ]]
  [[ "$output" == *"is_paused=true"* ]]

  # Other contracts still appear
  [[ "$output" == *"signal_registry"* ]]
  [[ "$output" == *"stake_vault"* ]]
}

# ===========================================================================
# Test 6 — get_user_portfolio returns wrong address → exit 1 with expected/actual
# ===========================================================================

@test "get_user_portfolio wrong address exits 1 with expected and actual shown" {
  make_state_file > /dev/null

  # Return a different address than CPORTFOLIO, scoped to get_user_portfolio
  stub_stellar_set_output "CTRADE" '"CWRONG_ADDRESS"' "get_user_portfolio"

  run bash "$VERIFY_SCRIPT"

  [ "$status" -eq 1 ]

  [[ "$output" == *"expected"* ]]
  [[ "$output" == *"actual"* ]]
  [[ "$output" == *"CPORTFOLIO"* ]]
  [[ "$output" == *"CWRONG_ADDRESS"* ]]
}

# ===========================================================================
# Test 7 — get_user_portfolio returns empty → exit 1 with "not configured"
# ===========================================================================

@test "get_user_portfolio empty output exits 1 with not configured message" {
  make_state_file > /dev/null

  # Return empty string for get_user_portfolio on trade_executor
  stub_stellar_set_output "CTRADE" "" "get_user_portfolio"

  run bash "$VERIFY_SCRIPT"

  [ "$status" -eq 1 ]
  [[ "$output" == *"not configured"* ]]
}

# ===========================================================================
# Test 8 — health_check() CLI call fails → exit 1, remaining contracts checked
# ===========================================================================

@test "health_check CLI failure exits 1 and remaining contracts are still checked" {
  make_state_file > /dev/null
  setup_matching_cross_ref

  # Make signal_registry health_check CLI invocation fail
  stub_stellar_set_exit "CSIGNAL" "1" "health_check"
  stub_stellar_set_output "CSIGNAL" "connection refused" "health_check"

  run bash "$VERIFY_SCRIPT"

  [ "$status" -eq 1 ]

  # signal_registry should be marked FAIL
  [[ "$output" == *"[FAIL]"*"signal_registry"* ]]

  # Remaining contracts should still be checked
  [[ "$output" == *"fee_collector"* ]]
  [[ "$output" == *"stake_vault"* ]]
  [[ "$output" == *"user_portfolio"* ]]
  [[ "$output" == *"trade_executor"* ]]
}

# ===========================================================================
# Test 9 — Missing STELLAR_SOURCE_ACCOUNT → exit 1 before any invocations
# ===========================================================================

@test "missing STELLAR_SOURCE_ACCOUNT exits 1 before any stellar invocations" {
  make_state_file > /dev/null

  unset STELLAR_SOURCE_ACCOUNT
  unset STELLAR_ACCOUNT

  run bash -c "unset STELLAR_SOURCE_ACCOUNT; unset STELLAR_ACCOUNT; bash '$VERIFY_SCRIPT' 2>&1"

  [ "$status" -eq 1 ]

  # No stellar invocations should have been made
  call_count="$(stub_stellar_call_count)"
  [ "$call_count" -eq 0 ]
}

# ===========================================================================
# Test 10 — STELLAR_ACCOUNT fallback works when STELLAR_SOURCE_ACCOUNT unset
# ===========================================================================

@test "STELLAR_ACCOUNT fallback is used when STELLAR_SOURCE_ACCOUNT is unset" {
  make_state_file > /dev/null
  setup_matching_cross_ref

  unset STELLAR_SOURCE_ACCOUNT
  export STELLAR_ACCOUNT="fallback-account"

  run bash "$VERIFY_SCRIPT"

  [ "$status" -eq 0 ]

  # Verify the fallback account was passed to the stub
  calls_file="$BATS_TEST_TMPDIR/stellar_calls"
  [[ -f "$calls_file" ]]
  grep -q "fallback-account" "$calls_file"
}

# ===========================================================================
# Test 11 — All output goes to stdout (not stderr)
# ===========================================================================

@test "all check result output goes to stdout not stderr" {
  make_state_file > /dev/null
  setup_matching_cross_ref

  # Capture stdout and stderr separately
  stdout_file="$BATS_TEST_TMPDIR/stdout.txt"
  stderr_file="$BATS_TEST_TMPDIR/stderr.txt"

  bash "$VERIFY_SCRIPT" > "$stdout_file" 2> "$stderr_file"
  exit_code=$?

  [ "$exit_code" -eq 0 ]

  # PASS lines must appear on stdout
  pass_count="$(grep -c '^\[PASS\]' "$stdout_file" || true)"
  [ "$pass_count" -eq 6 ]

  # PASS/FAIL lines must NOT appear on stderr
  fail_on_stderr="$(grep -c '^\[PASS\]\|^\[FAIL\]' "$stderr_file" || true)"
  [ "$fail_on_stderr" -eq 0 ]

  # Summary must be on stdout
  grep -q "Verification Summary" "$stdout_file"
}
