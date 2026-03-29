# Implementation Plan: Deployment Verification Script

## Overview

Implement `scripts/verify_deployment.sh` as a read-only bash script that calls `health_check()` on all five deployed Soroban contracts, validates the TradeExecutor cross-contract reference, and exits non-zero on any failure. Tests use bats-core.

## Tasks

- [x] 1. Scaffold the verification script with environment setup and state file loading
  - Create `scripts/verify_deployment.sh` with shebang, `set -euo pipefail`, and the same `SCRIPT_DIR`/`ROOT`/`STATE` resolution logic as `deploy_testnet.sh`
  - Implement `check_env()`: validate `stellar` and `jq` are on PATH; validate `STELLAR_SOURCE_ACCOUNT` or `STELLAR_ACCOUNT` is set; print descriptive errors and exit 1 if not
  - Implement `load_state()`: verify state file exists; extract all five contract IDs (`signal_registry`, `fee_collector`, `stake_vault`, `user_portfolio`, `trade_executor`) using `jq`; exit 1 with contract name if any ID is missing or empty
  - Accept all env vars from Requirement 7: `DEPLOY_STATE`, `STELLAR_RPC_URL`, `STELLAR_NETWORK_PASSPHRASE`, `STELLAR_NETWORK`, with the same defaults as `deploy_testnet.sh`
  - _Requirements: 1.1, 1.2, 1.3, 1.4, 7.1, 7.2, 7.3, 7.4, 7.5, 7.6_

- [x] 2. Implement health check probing and assertion logic
  - [x] 2.1 Implement `probe_health()`: invoke `stellar contract invoke --send=no -- health_check` for a given contract ID; capture output; return raw JSON or record failure on CLI error
    - Use `--rpc-url`, `--network-passphrase`, `--network`, `--source-account` flags from env vars
    - On CLI failure: call `record_fail()` with contract name and error, do not exit
    - _Requirements: 2.1, 2.2, 2.3, 2.4_

  - [x] 2.2 Implement `assert_healthy()`: parse `is_initialized` and `is_paused` from `probe_health()` output using `jq`; call `record_fail()` if `is_initialized != true` or `is_paused == true`; call `record_pass()` otherwise
    - _Requirements: 3.1, 3.2, 3.3_

  - [ ]* 2.3 Write property test for health check classification (Property 2)
    - **Property 2: Health check classification is correct for all HealthStatus values**
    - **Validates: Requirements 3.1, 3.2, 3.3**
    - Generate all four combinations of `is_initialized` × `is_paused`; assert PASS iff both true/false

- [x] 3. Implement cross-contract reference validation
  - [x] 3.1 Implement `check_cross_refs()`: invoke `get_user_portfolio` on `trade_executor` with `--send=no`; compare returned address to `user_portfolio` contract ID from state file
    - On empty/null result: record failure "UserPortfolio reference not configured in TradeExecutor"
    - On mismatch: record failure showing expected vs actual addresses
    - On match: record pass
    - _Requirements: 4.1, 4.2, 4.3, 4.4_

  - [ ]* 3.2 Write property test for cross-contract reference check (Property 3)
    - **Property 3: Cross-contract reference check correctly classifies address match vs mismatch**
    - **Validates: Requirements 4.2, 4.3, 4.4**
    - Generate random address pairs (equal, unequal, empty); assert correct PASS/FAIL classification

- [x] 4. Implement result tracking and output formatting
  - [x] 4.1 Implement `record_pass()` and `record_fail()`: maintain `FAILURES[]` array and `PASS_COUNT`/`TOTAL_COUNT` counters; print per-check result line to stdout immediately (logical name, contract ID, PASS/FAIL, reason)
    - Format: `[PASS] <name>  (<id>)  is_initialized=true  is_paused=false`
    - Format: `[FAIL] <name>  (<id>)  <reason>`
    - _Requirements: 5.1, 5.4_

  - [x] 4.2 Implement `summarize()`: print `--- Verification Summary ---`, passed/failed counts, list of all failed check names; exit 0 if `FAILURES` is empty, exit 1 otherwise
    - _Requirements: 5.2, 5.3, 6.1, 6.2_

  - [ ]* 4.3 Write property test for exit code correctness (Property 5)
    - **Property 5: Exit code matches failure presence**
    - **Validates: Requirements 5.2, 5.3, 6.1, 6.2**
    - Generate random pass/fail outcomes for all 6 checks; assert exit 0 iff all pass

- [x] 5. Wire all checks together and ensure continue-on-failure behavior
  - [x] 5.1 Write the main execution block: call `check_env`, `load_state`, then call `assert_healthy` for each of the five contracts in order, then call `check_cross_refs`, then call `summarize`
    - Use subshell or conditional execution so a single check failure does not abort the script (do not rely on `set -e` for check functions)
    - _Requirements: 2.1, 6.3_

  - [ ]* 5.2 Write property test for complete-all-checks behavior (Property 4)
    - **Property 4: Script always completes all checks before exiting**
    - **Validates: Requirements 2.4, 6.3**
    - Generate random subsets of contracts to fail; assert stdout always contains exactly 6 result lines

- [x] 6. Set up bats test infrastructure and write unit tests
  - [x] 6.1 Create `scripts/tests/` directory with a `stub_stellar` helper that records invocations and returns configurable mock output; create `verify_deployment.bats` test file
    - _Requirements: all_

  - [ ]* 6.2 Write unit tests for happy path and error conditions
    - Happy path: all 5 contracts healthy, cross-ref matches → exit 0, output contains 6 PASS lines
    - Missing state file → exit 1, error contains file path
    - State file missing one contract ID → exit 1, error contains contract name
    - `health_check()` CLI failure → exit 1, remaining contracts still checked
    - Missing `STELLAR_SOURCE_ACCOUNT` → exit 1 before any CLI invocations
    - `STELLAR_ACCOUNT` fallback works
    - All output goes to stdout (not stderr)
    - _Requirements: 1.2, 1.3, 2.4, 5.4, 6.3, 7.6_

  - [ ]* 6.3 Write property test for invalid state input (Property 1)
    - **Property 1: Invalid state input always produces exit code 1 with a descriptive error**
    - **Validates: Requirements 1.2, 1.3**
    - Generate random combinations of missing file / missing contract IDs; assert exit 1 and non-empty error output

  - [ ]* 6.4 Write property test for environment variable propagation (Property 6)
    - **Property 6: Environment variable propagation**
    - **Validates: Requirements 2.3, 7.2, 7.3**
    - Generate random RPC URLs and passphrases; assert every captured CLI invocation contains them

- [x] 7. Checkpoint — Ensure all tests pass
  - Make the script executable (`chmod +x scripts/verify_deployment.sh`)
  - Run `bats scripts/tests/verify_deployment.bats` and confirm all tests pass
  - Ensure all tests pass, ask the user if questions arise.

## Notes

- Tasks marked with `*` are optional and can be skipped for a faster MVP
- The script must be idempotent and read-only — it never writes to the state file
- All CLI invocations use `--send=no` to avoid submitting transactions
- The script follows the same conventions as `deploy_testnet.sh` for env vars and path resolution
- Property tests run a minimum of 100 iterations each
