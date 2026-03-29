#!/usr/bin/env bash
# stub_stellar.bash — bats helper that installs a fake `stellar` executable.
#
# The stub:
#   - Records every invocation's arguments to $BATS_TEST_TMPDIR/stellar_calls
#   - Returns output from $STELLAR_STUB_OUTPUT (env var) by default
#   - Supports per-contract-ID responses via files named
#       $BATS_TEST_TMPDIR/stellar_out_<contract_id>
#     (the stub scans its own args for a value following --id and uses that file
#      if it exists, falling back to $STELLAR_STUB_OUTPUT)
#   - Exits with $STELLAR_STUB_EXIT_CODE (default 0), or a per-contract-ID
#     exit code from $BATS_TEST_TMPDIR/stellar_exit_<contract_id> if present
#
# Usage in a bats test file:
#   load 'helpers/stub_stellar'
#
#   setup() {
#     setup_stellar_stub
#   }
#
#   teardown() {
#     teardown_stellar_stub
#   }

# Directory where the fake `stellar` binary lives (set by setup_stellar_stub)
_STUB_STELLAR_BIN_DIR=""

# ---------------------------------------------------------------------------
# setup_stellar_stub
#
# Creates a temp directory, writes the fake `stellar` script into it, and
# prepends that directory to PATH so it shadows the real stellar binary.
# ---------------------------------------------------------------------------
setup_stellar_stub() {
  _STUB_STELLAR_BIN_DIR="$(mktemp -d)"

  cat > "$_STUB_STELLAR_BIN_DIR/stellar" <<'STUB_EOF'
#!/usr/bin/env bash
# Fake stellar binary — records calls and returns configurable output.

CALLS_FILE="${BATS_TEST_TMPDIR}/stellar_calls"

# Append the full argument list (one line per invocation)
printf '%s\n' "$*" >> "$CALLS_FILE"

# Determine the contract ID from --id <value> in the argument list
contract_id=""
prev=""
for arg in "$@"; do
  if [[ "$prev" == "--id" ]]; then
    contract_id="$arg"
    break
  fi
  prev="$arg"
done

# Determine the function name: the last argument (after --)
func_name=""
found_sep=false
for arg in "$@"; do
  if [[ "$found_sep" == "true" ]]; then
    func_name="$arg"
    break
  fi
  if [[ "$arg" == "--" ]]; then
    found_sep=true
  fi
done

# Determine exit code: per-contract-function > per-contract > env var > default 0
exit_code_file_cf="${BATS_TEST_TMPDIR}/stellar_exit_${contract_id}_${func_name}"
exit_code_file_c="${BATS_TEST_TMPDIR}/stellar_exit_${contract_id}"
if [[ -n "$contract_id" && -n "$func_name" && -f "$exit_code_file_cf" ]]; then
  exit_code="$(cat "$exit_code_file_cf")"
elif [[ -n "$contract_id" && -f "$exit_code_file_c" ]]; then
  exit_code="$(cat "$exit_code_file_c")"
else
  exit_code="${STELLAR_STUB_EXIT_CODE:-0}"
fi

# Determine output: per-contract-function > per-contract > env var > empty
output_file_cf="${BATS_TEST_TMPDIR}/stellar_out_${contract_id}_${func_name}"
output_file_c="${BATS_TEST_TMPDIR}/stellar_out_${contract_id}"
if [[ -n "$contract_id" && -n "$func_name" && -f "$output_file_cf" ]]; then
  cat "$output_file_cf"
elif [[ -n "$contract_id" && -f "$output_file_c" ]]; then
  cat "$output_file_c"
elif [[ -n "${STELLAR_STUB_OUTPUT:-}" ]]; then
  printf '%s' "$STELLAR_STUB_OUTPUT"
fi

exit "$exit_code"
STUB_EOF

  chmod +x "$_STUB_STELLAR_BIN_DIR/stellar"

  # Prepend stub dir to PATH so it shadows the real stellar
  export PATH="$_STUB_STELLAR_BIN_DIR:$PATH"
}

# ---------------------------------------------------------------------------
# teardown_stellar_stub
#
# Removes the temp directory containing the fake stellar binary.
# PATH is not restored here because bats runs each test in a subshell;
# the modified PATH is discarded automatically after the test.
# ---------------------------------------------------------------------------
teardown_stellar_stub() {
  if [[ -n "$_STUB_STELLAR_BIN_DIR" && -d "$_STUB_STELLAR_BIN_DIR" ]]; then
    rm -rf "$_STUB_STELLAR_BIN_DIR"
  fi
  _STUB_STELLAR_BIN_DIR=""
}

# ---------------------------------------------------------------------------
# stub_stellar_set_output CONTRACT_ID OUTPUT [FUNC_NAME]
#
# Convenience function: writes OUTPUT to the per-contract (optionally
# per-function) output file so the stub returns it for that contract.
# If FUNC_NAME is provided, the key is CONTRACT_ID_FUNC_NAME.
# ---------------------------------------------------------------------------
stub_stellar_set_output() {
  local contract_id="$1"
  local output="$2"
  local func_name="${3:-}"
  if [[ -n "$func_name" ]]; then
    printf '%s' "$output" > "${BATS_TEST_TMPDIR}/stellar_out_${contract_id}_${func_name}"
  else
    printf '%s' "$output" > "${BATS_TEST_TMPDIR}/stellar_out_${contract_id}"
  fi
}

# ---------------------------------------------------------------------------
# stub_stellar_set_exit CONTRACT_ID EXIT_CODE [FUNC_NAME]
#
# Convenience function: writes EXIT_CODE to the per-contract (optionally
# per-function) exit-code file.
# ---------------------------------------------------------------------------
stub_stellar_set_exit() {
  local contract_id="$1"
  local code="$2"
  local func_name="${3:-}"
  if [[ -n "$func_name" ]]; then
    printf '%s' "$code" > "${BATS_TEST_TMPDIR}/stellar_exit_${contract_id}_${func_name}"
  else
    printf '%s' "$code" > "${BATS_TEST_TMPDIR}/stellar_exit_${contract_id}"
  fi
}

# ---------------------------------------------------------------------------
# stub_stellar_call_count
#
# Prints the number of times the stub was invoked in the current test.
# ---------------------------------------------------------------------------
stub_stellar_call_count() {
  local calls_file="${BATS_TEST_TMPDIR}/stellar_calls"
  if [[ -f "$calls_file" ]]; then
    wc -l < "$calls_file"
  else
    echo 0
  fi
}
