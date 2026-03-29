# Design Document: Deployment Verification Script

## Overview

`scripts/verify_deployment.sh` is a read-only bash script that verifies a StellarSwipe deployment by calling `health_check()` on all five deployed Soroban contracts and validating cross-contract references. It reads contract IDs from the existing `deployments/testnet.json` state file produced by `deploy_testnet.sh`, so no manual address input is required. The script exits with code 0 on full success and code 1 on any failure, making it suitable as a CI gate after the deploy step.

## Architecture

The script follows the same conventions as the existing `deploy_testnet.sh` and `health_check.sh` scripts:

- Bash with `set -euo pipefail` for safety
- Reads state from `$DEPLOY_STATE` (defaults to `$ROOT/deployments/testnet.json`)
- Uses `jq` to parse JSON state
- Uses the `stellar` CLI for all contract invocations with `--send=no` (read-only)
- Collects failures into an array and reports them all at the end before exiting

```
verify_deployment.sh
  │
  ├── load_state()          — read + validate state file, extract contract IDs
  ├── check_env()           — validate required env vars
  ├── probe_health()        — call health_check() on one contract, parse result
  ├── assert_healthy()      — assert is_initialized=true, is_paused=false
  ├── check_cross_refs()    — call get_user_portfolio on trade_executor, compare
  ├── print_result()        — emit per-check PASS/FAIL line
  └── summarize()           — print final summary, exit 0 or 1
```

The script does **not** exit on the first failure. It accumulates failures and always runs all checks before exiting, so CI logs show the complete picture.

## Components and Interfaces

### Environment Variables

| Variable | Default | Description |
|---|---|---|
| `DEPLOY_STATE` | `$ROOT/deployments/testnet.json` | Path to deployment state JSON |
| `ROOT` | grandparent of script dir | Workspace root |
| `STELLAR_SOURCE_ACCOUNT` / `STELLAR_ACCOUNT` | (required) | Signing identity for CLI |
| `STELLAR_NETWORK` | `testnet` | Network name for `--network` flag |
| `STELLAR_RPC_URL` | `https://soroban-testnet.stellar.org` | Soroban RPC endpoint |
| `STELLAR_NETWORK_PASSPHRASE` | `Test SDF Network ; September 2015` | Network passphrase |

### State File Schema

The script reads from the JSON structure produced by `deploy_testnet.sh`:

```json
{
  "contracts": {
    "signal_registry": { "contract_id": "C..." },
    "fee_collector":   { "contract_id": "C..." },
    "stake_vault":     { "contract_id": "C..." },
    "user_portfolio":  { "contract_id": "C..." },
    "trade_executor":  { "contract_id": "C..." }
  }
}
```

### Stellar CLI Invocations

All invocations use `--send=no` (simulation only, no ledger writes):

```bash
# health_check
stellar contract invoke \
  --id "$CONTRACT_ID" \
  --source-account "$SOURCE" \
  --network "$NET" \
  --rpc-url "$RPC_URL" \
  --network-passphrase "$NETWORK_PASSPHRASE" \
  --send=no \
  -- health_check

# get_user_portfolio (cross-contract reference check)
stellar contract invoke \
  --id "$TRADE_EXECUTOR_ID" \
  --source-account "$SOURCE" \
  --network "$NET" \
  --rpc-url "$RPC_URL" \
  --network-passphrase "$NETWORK_PASSPHRASE" \
  --send=no \
  -- get_user_portfolio
```

### Output Format

Per-check lines (printed as each check completes):

```
[PASS] signal_registry    (C...abc)  is_initialized=true  is_paused=false
[FAIL] trade_executor     (C...xyz)  is_initialized=false
[PASS] cross_ref: trade_executor.get_user_portfolio == user_portfolio
```

Final summary:

```
--- Verification Summary ---
Passed: 5/6
Failed: 1/6
  FAIL: trade_executor health_check: is_initialized=false

Exit code: 1
```

## Data Models

### Internal State (bash variables)

```bash
FAILURES=()          # array of failure description strings
PASS_COUNT=0         # number of checks that passed
TOTAL_COUNT=0        # total checks attempted

# Per-contract
CONTRACT_ID          # C... strkey from state file
HEALTH_OUTPUT        # raw JSON from health_check()
IS_INITIALIZED       # parsed bool
IS_PAUSED            # parsed bool
```

### HealthStatus (returned by contracts)

```json
{
  "is_initialized": true,
  "is_paused": false,
  "version": "0.1.0",
  "admin": "G..."
}
```

The script parses `is_initialized` and `is_paused` using `jq`. The `version` and `admin` fields are printed for informational purposes but not asserted.

## Correctness Properties

*A property is a characteristic or behavior that should hold true across all valid executions of a system — essentially, a formal statement about what the system should do. Properties serve as the bridge between human-readable specifications and machine-verifiable correctness guarantees.*

Property 1: Invalid state input always produces exit code 1 with a descriptive error
*For any* invocation of the script where the state file is missing, unreadable, or missing a required contract ID, the script should exit with code 1 and print a message identifying the specific problem.
**Validates: Requirements 1.2, 1.3**

Property 2: Health check classification is correct for all HealthStatus values
*For any* `HealthStatus` response from a contract, the script should classify it as PASS if and only if `is_initialized` is `true` AND `is_paused` is `false`; any other combination should be classified as FAIL.
**Validates: Requirements 3.1, 3.2, 3.3**

Property 3: Cross-contract reference check correctly classifies address match vs mismatch
*For any* pair of addresses (stored address from `get_user_portfolio`, expected address from state file), the script should record PASS if and only if the two addresses are equal; any non-equal or empty/null result should be recorded as FAIL.
**Validates: Requirements 4.2, 4.3, 4.4**

Property 4: Script always completes all checks before exiting
*For any* failure in any subset of checks, the script should still invoke all remaining checks and collect all results before printing the summary and exiting.
**Validates: Requirements 2.4, 6.3**

Property 5: Exit code matches failure presence
*For any* run of the script, the exit code should be 0 if and only if the FAILURES array is empty after all checks complete.
**Validates: Requirements 5.2, 5.3, 6.1, 6.2**

Property 6: Environment variable propagation
*For any* values of `STELLAR_RPC_URL` and `STELLAR_NETWORK_PASSPHRASE` set in the environment, every Stellar CLI invocation made by the script should include those exact values as `--rpc-url` and `--network-passphrase` arguments.
**Validates: Requirements 2.3, 7.2, 7.3**

## Error Handling

| Condition | Behavior |
|---|---|
| State file missing | Print path, exit 1 immediately (before any checks) |
| Contract ID missing from state | Print logical name, exit 1 immediately |
| `STELLAR_SOURCE_ACCOUNT` and `STELLAR_ACCOUNT` both unset | Print error, exit 1 immediately |
| `stellar` CLI not found | Print install hint, exit 1 immediately |
| `jq` not found | Print install hint, exit 1 immediately |
| `health_check()` CLI invocation fails | Record failure, continue to next contract |
| `health_check()` output unparseable | Record failure with raw output, continue |
| `get_user_portfolio` returns empty/null | Record failure as "not configured", continue |
| Address mismatch in cross-ref check | Record failure with expected vs actual, continue |

Early exits (before checks begin) use `die()` consistent with `deploy_testnet.sh`. Failures during checks are accumulated in `FAILURES[]` and never cause early exit.

## Testing Strategy

### Dual Testing Approach

Both unit tests and property-based tests are used. Unit tests cover specific examples and edge cases (e.g., exact output format, specific error messages). Property-based tests verify universal correctness properties across many generated inputs.

### Unit Tests (bats — Bash Automated Testing System)

Unit tests use [bats-core](https://github.com/bats-core/bats-core) to test the script in isolation by mocking the `stellar` CLI and `jq` with stub functions.

Test cases:
- Happy path: all 5 contracts healthy, cross-ref matches → exit 0
- Missing state file → exit 1 with path in error message
- State file missing one contract ID → exit 1 with contract name in error
- One contract returns `is_initialized: false` → exit 1, others still checked
- One contract returns `is_paused: true` → exit 1, others still checked
- `get_user_portfolio` returns wrong address → exit 1 with expected/actual shown
- `get_user_portfolio` returns empty → exit 1 with "not configured" message
- `health_check()` CLI call fails → exit 1, remaining contracts still checked
- Missing `STELLAR_SOURCE_ACCOUNT` → exit 1 before any invocations
- `STELLAR_ACCOUNT` fallback works when `STELLAR_SOURCE_ACCOUNT` unset
- All output goes to stdout (not stderr)

### Property-Based Tests

Property-based tests use [bats-core](https://github.com/bats-core/bats-core) with a simple generator helper that produces random HealthStatus JSON values and random address strings. Each property test runs a minimum of 100 iterations.

**Property 1 test** — `Feature: deployment-verification, Property 1: invalid state input always produces exit code 1`
Generate random combinations of: missing file, file with missing contract IDs (any subset of the 5). Assert exit code 1 and non-empty error output for all.

**Property 2 test** — `Feature: deployment-verification, Property 2: health check classification is correct for all HealthStatus values`
Generate random `HealthStatus` JSON with all four combinations of `is_initialized` (true/false) × `is_paused` (true/false). Assert PASS iff both `is_initialized=true` and `is_paused=false`.

**Property 3 test** — `Feature: deployment-verification, Property 3: cross-contract reference check correctly classifies address match vs mismatch`
Generate random pairs of Stellar contract addresses (C... strkeys). Assert PASS iff the two addresses are equal; FAIL otherwise. Also generate empty/null values and assert FAIL.

**Property 4 test** — `Feature: deployment-verification, Property 4: script always completes all checks before exiting`
Generate random subsets of the 5 contracts to fail. Assert that the number of check result lines in stdout always equals 6 (5 health checks + 1 cross-ref check), regardless of which contracts fail.

**Property 5 test** — `Feature: deployment-verification, Property 5: exit code matches failure presence`
Generate random pass/fail outcomes for each of the 6 checks. Assert exit code 0 iff all 6 pass; exit code 1 otherwise.

**Property 6 test** — `Feature: deployment-verification, Property 6: environment variable propagation`
Generate random valid RPC URLs and network passphrases. Assert every captured CLI invocation contains the generated values as `--rpc-url` and `--network-passphrase` arguments.

### Property Test Configuration

- Minimum 100 iterations per property test
- Each test tagged with: `Feature: deployment-verification, Property N: <property_text>`
- Tests located in `scripts/tests/verify_deployment.bats`
- Run with: `bats scripts/tests/verify_deployment.bats`
