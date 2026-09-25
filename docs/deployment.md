# Deployment Guide

This guide covers deploying StellarSwipe contracts to testnet and mainnet.

## Prerequisites

Install required tools:

```bash
rustup target add wasm32-unknown-unknown
cargo --version
stellar --version
jq --version
```

Prepare funded keypairs:

- **Testnet:** funded account for `STELLAR_SOURCE_ACCOUNT` and matching `STELLAR_ADMIN_ADDRESS`
- **Mainnet:** production signer(s), funded account, and approved admin address

From repository root, build contracts:

```bash
cd stellar-swipe
cargo build --workspace --target wasm32-unknown-unknown --release
```

## WASM Reproducibility Check

Contract releases must be reproducible: building the same source with the same
toolchain and locked dependencies must produce byte-identical WASM hashes. The
release check builds each contract WASM twice and fails if the hashes differ.

Run the check locally before tagging a release:

```bash
./scripts/check_wasm_reproducibility.sh
```

The script captures the build context so a failing run is actionable:

- **Toolchain:** `rustc --version` and the active `rust-toolchain.toml` channel
- **Target:** `wasm32-unknown-unknown`
- **Lock state:** `Cargo.lock` hash (dependencies must be committed and locked)

On success it prints the matching hash for each contract. On failure it exits
non-zero and prints the two differing hashes, the artifact paths, and the
toolchain/lock info above so CI fails with a clear diagnostic.

### Investigating Hash Differences

If the reproducibility check reports differing hashes:

1) Confirm the toolchain matches the release toolchain:

```bash
rustc --version
cat rust-toolchain.toml
```

2) Confirm dependencies are locked and unchanged:

```bash
git status --porcelain Cargo.lock
sha256sum Cargo.lock
```

3) Rebuild cleanly and compare hashes manually:

```bash
cargo clean
cargo build --workspace --target wasm32-unknown-unknown --release
sha256sum target/wasm32-unknown-unknown/release/*.wasm
```

4) Common causes of non-reproducible output:

- A different `rustc`/toolchain version than the release toolchain.
- An unlocked or locally modified `Cargo.lock`.
- Absolute paths or timestamps embedded by a build script.
- Non-deterministic codegen from an unpinned dependency.

Fix the cause, re-run `./scripts/check_wasm_reproducibility.sh`, and only then
proceed with deployment.

## Testnet Deployment (Step-by-Step)

1) Set deployment environment variables:

```bash
export STELLAR_NETWORK=testnet
export STELLAR_RPC_URL="https://soroban-testnet.stellar.org"
export STELLAR_NETWORK_PASSPHRASE="Test SDF Network ; September 2015"
export STELLAR_SOURCE_ACCOUNT="YOUR_TESTNET_SECRET_OR_IDENTITY"
export STELLAR_ADMIN_ADDRESS="YOUR_TESTNET_G_ADDRESS"
```

2) Run deployment script and capture logs:

```bash
./scripts/deploy_testnet.sh 2>&1 | tee ../deployments/testnet-deploy.log
```

3) Confirm state file exists and includes contract IDs:

```bash
cat ../deployments/testnet.json
```

4) Verify deployment health and cross-contract wiring:

```bash
./scripts/verify_deployment.sh
```

Expected result:

- Script prints pass/fail summary
- Final exit code is `0`

## Mainnet Deployment

Mainnet follows the same deployment flow, with additional controls.

### Security Warnings (Required)

- Use a hardware wallet or offline signer for the primary admin key.
- Require multisig authorization for production admin actions.
- Never deploy from a personal developer machine with long-lived plaintext secrets.
- Require a second reviewer to validate contract IDs, addresses, and logs before activation.

### Mainnet Steps

1) Set mainnet environment variables:

```bash
export STELLAR_NETWORK=mainnet
export STELLAR_RPC_URL="https://mainnet.sorobanrpc.com"
export STELLAR_NETWORK_PASSPHRASE="Public Global Stellar Network ; September 2015"
export STELLAR_SOURCE_ACCOUNT="YOUR_MAINNET_SIGNER_OR_IDENTITY"
export STELLAR_ADMIN_ADDRESS="YOUR_MAINNET_G_ADDRESS"
```

2) Deploy using the same script (writes state + contract IDs):

```bash
./scripts/deploy_testnet.sh 2>&1 | tee ../deployments/mainnet-deploy.log
```

3) Verify mainnet deployment using the verification script:

```bash
DEPLOY_STATE="../deployments/mainnet.json" \
STELLAR_NETWORK="mainnet" \
STELLAR_RPC_URL="https://mainnet.sorobanrpc.com" \
STELLAR_NETWORK_PASSPHRASE="Public Global Stellar Network ; September 2015" \
./scripts/verify_deployment.sh
```

## Verification

Use `verify_deployment.sh` after every deployment:

- Default behavior verifies `deployments/testnet.json`
- Override with `DEPLOY_STATE` for other environments
- Exit code `0` = all checks passed; exit code `1` = at least one check failed

Example:

```bash
STELLAR_SOURCE_ACCOUNT="YOUR_SIGNER" ./scripts/verify_deployment.sh
```

## Rollback Procedure

If deployment fails:

1) Stop further deploy or initialize steps immediately.
2) Preserve all logs:
   - `deployments/testnet-deploy.log` or `deployments/mainnet-deploy.log`
3) Run verification script to identify exact failing contract/check.
4) If only a subset deployed, redeploy idempotently using the same state file after fixing the issue.
5) If a bad mainnet deployment is active:
   - Pause affected contracts (if pause controls are available).
   - Rotate compromised keys.
   - Prepare and execute a multisig-approved redeploy/migration plan.

## Additional Resources

- [Testnet Deployment Checklist](./testnet_checklist.md) — step-by-step checklist for testnet deployments
- [Mainnet Deployment Checklist](./mainnet_checklist.md) — enhanced checklist with security sign-offs for mainnet

## Release Checklist (Done Criteria)

Before merge/release, confirm:

- [ ] A new contributor successfully deployed to testnet using only this guide.
- [ ] Mainnet section security warnings were reviewed and acknowledged.
- [ ] Test deployment feedback is included in the PR description/comments.
- [ ] `./scripts/check_wasm_reproducibility.sh` passes with identical hashes.
