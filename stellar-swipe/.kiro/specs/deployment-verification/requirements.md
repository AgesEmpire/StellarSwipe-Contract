# Requirements Document

## Introduction

After deploying StellarSwipe Soroban contracts to testnet or mainnet, there is no automated way to verify that all contracts are correctly initialized and properly wired to each other. A misconfigured deployment (e.g., wrong cross-contract address, uninitialized contract, or paused contract) can silently break user-facing functionality. This feature adds `scripts/verify_deployment.sh` — a read-only verification script that calls `health_check()` on every deployed contract, validates cross-contract references, and exits with a non-zero code on any failure so CI can gate the deploy step.

## Glossary

- **Script**: The `scripts/verify_deployment.sh` bash script being specified.
- **Contract**: A deployed Soroban smart contract on the Stellar network.
- **health_check()**: A read-only Soroban function present on each contract that returns a `HealthStatus` struct containing `is_initialized`, `is_paused`, `version`, and `admin` fields.
- **HealthStatus**: The struct returned by `health_check()` with fields `is_initialized: bool`, `is_paused: bool`, `version: String`, and `admin: Address`.
- **Cross-contract reference**: A stored address inside one contract that points to another contract (e.g., TradeExecutor storing the UserPortfolio address).
- **State file**: The `deployments/testnet.json` (or equivalent) JSON file produced by `deploy_testnet.sh` that records contract IDs for each logical contract name.
- **Logical name**: The key used in the state file to identify a contract (e.g., `signal_registry`, `fee_collector`, `stake_vault`, `user_portfolio`, `trade_executor`).
- **CI**: Continuous integration pipeline that runs the script after the deploy step.
- **STELLAR_SOURCE_ACCOUNT**: The signing identity or secret key used to submit read-only (`--send=no`) invocations via the Stellar CLI.
- **Stellar CLI**: The `stellar` command-line tool used to invoke Soroban contract functions.

## Requirements

### Requirement 1: Load Contract Addresses from State File

**User Story:** As a DevOps engineer, I want the script to automatically read contract addresses from the deployment state file, so that I do not have to manually supply contract IDs after each deploy.

#### Acceptance Criteria

1. WHEN the Script starts, THE Script SHALL read contract IDs from the state file at `$DEPLOY_STATE` (defaulting to `$ROOT/deployments/testnet.json` using the same path logic as `deploy_testnet.sh`).
2. IF the state file does not exist at the resolved path, THEN THE Script SHALL print a descriptive error message identifying the missing file path and exit with code 1.
3. IF a required contract's `contract_id` field is absent or empty in the state file, THEN THE Script SHALL print a descriptive error identifying which logical contract is missing and exit with code 1.
4. THE Script SHALL resolve the `ROOT` variable the same way `deploy_testnet.sh` does: defaulting to the grandparent of the script directory when `ROOT` is not set in the environment.

### Requirement 2: Invoke health_check() on All Contracts

**User Story:** As a DevOps engineer, I want the script to call `health_check()` on every deployed contract, so that I can confirm each contract is reachable and responding on the network.

#### Acceptance Criteria

1. THE Script SHALL invoke `health_check()` on each of the five logical contracts: `signal_registry`, `fee_collector` (oracle package), `stake_vault` (governance package), `user_portfolio` (auto_trade package), and `trade_executor` (bridge package).
2. WHEN invoking `health_check()`, THE Script SHALL pass `--send=no` to the Stellar CLI so that no transaction is submitted to the network.
3. WHEN invoking `health_check()`, THE Script SHALL use the `--rpc-url` and `--network-passphrase` values from environment variables `STELLAR_RPC_URL` and `STELLAR_NETWORK_PASSPHRASE`, falling back to the testnet defaults used by `deploy_testnet.sh`.
4. IF a `health_check()` invocation fails (non-zero CLI exit code or unparseable output), THEN THE Script SHALL record the failure, print a descriptive error identifying the contract name and contract ID, and continue checking remaining contracts before exiting.

### Requirement 3: Assert Initialization and Pause State

**User Story:** As a DevOps engineer, I want the script to assert that every contract reports `is_initialized: true` and `is_paused: false`, so that I can detect contracts that were deployed but not initialized, or contracts that are incorrectly paused.

#### Acceptance Criteria

1. WHEN the `HealthStatus` returned by `health_check()` contains `is_initialized` equal to `false`, THE Script SHALL record a failure for that contract with a message stating the contract is not initialized.
2. WHEN the `HealthStatus` returned by `health_check()` contains `is_paused` equal to `true`, THE Script SHALL record a failure for that contract with a message stating the contract is paused.
3. WHILE all five contracts return `is_initialized: true` and `is_paused: false`, THE Script SHALL record each as passing.

### Requirement 4: Validate Cross-Contract References in TradeExecutor

**User Story:** As a DevOps engineer, I want the script to verify that TradeExecutor has the correct UserPortfolio address configured, so that I can detect address mismatches before users encounter cross-contract call failures.

#### Acceptance Criteria

1. THE Script SHALL invoke `get_user_portfolio` on the `trade_executor` contract to retrieve the stored UserPortfolio address.
2. WHEN the address returned by `get_user_portfolio` does not match the `user_portfolio` contract ID from the state file, THE Script SHALL record a failure with a message showing both the expected and actual addresses.
3. WHEN the address returned by `get_user_portfolio` matches the `user_portfolio` contract ID from the state file, THE Script SHALL record this cross-contract reference check as passing.
4. IF `get_user_portfolio` returns an empty or null value, THEN THE Script SHALL record a failure stating that the UserPortfolio reference is not configured in TradeExecutor.

### Requirement 5: Output Pass/Fail Summary with Contract Addresses

**User Story:** As a DevOps engineer, I want the script to print a structured summary of all checks with contract addresses, so that I can quickly identify which contracts passed or failed verification.

#### Acceptance Criteria

1. THE Script SHALL print each check result as it completes, showing the logical contract name, contract ID, and PASS or FAIL status.
2. WHEN all checks pass, THE Script SHALL print a final summary line indicating the total number of checks passed and exit with code 0.
3. WHEN one or more checks fail, THE Script SHALL print a final summary line indicating how many checks failed out of the total, followed by a list of all failed check names, and exit with code 1.
4. THE Script SHALL print all output to stdout so that CI log capture works correctly with `tee`.

### Requirement 6: Non-Zero Exit on Any Failure for CI Use

**User Story:** As a CI engineer, I want the script to exit with a non-zero code whenever any check fails, so that the CI pipeline can automatically block a bad deployment from proceeding.

#### Acceptance Criteria

1. WHEN every health check and cross-contract reference check passes, THE Script SHALL exit with code 0.
2. WHEN any single check fails for any reason (unreachable contract, assertion failure, missing address, CLI error), THE Script SHALL exit with code 1 after completing all remaining checks.
3. THE Script SHALL NOT exit immediately on the first failure; THE Script SHALL complete all checks before exiting so that the full failure report is available in CI logs.
4. THE Script SHALL be executable as a standalone command and produce a non-zero exit code that CI systems can detect without additional wrapper logic.

### Requirement 7: Environment Variable Configuration

**User Story:** As a DevOps engineer, I want the script to be configurable via environment variables consistent with the existing deploy script, so that I can reuse the same CI environment setup for both deploying and verifying.

#### Acceptance Criteria

1. THE Script SHALL accept `DEPLOY_STATE` to override the path to the state file, using the same default resolution logic as `deploy_testnet.sh`.
2. THE Script SHALL accept `STELLAR_RPC_URL` to override the Soroban RPC endpoint, defaulting to `https://soroban-testnet.stellar.org`.
3. THE Script SHALL accept `STELLAR_NETWORK_PASSPHRASE` to override the network passphrase, defaulting to `Test SDF Network ; September 2015`.
4. THE Script SHALL accept `STELLAR_SOURCE_ACCOUNT` (or `STELLAR_ACCOUNT` as a fallback) as the signing identity for read-only CLI invocations.
5. THE Script SHALL accept `STELLAR_NETWORK` to set the `--network` flag on CLI invocations, defaulting to `testnet`.
6. IF `STELLAR_SOURCE_ACCOUNT` and `STELLAR_ACCOUNT` are both unset, THEN THE Script SHALL print a descriptive error and exit with code 1 before attempting any contract invocations.
