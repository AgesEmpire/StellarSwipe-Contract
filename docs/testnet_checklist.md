# StellarSwipe Testnet Deployment Checklist

Use this checklist for every testnet deployment. Mark each item `[x]` as you complete it.
A deployment is only considered successful when every item passes.

---

## Pre-Deployment

### Environment & Tooling

- [ ] `rustup target add wasm32-unknown-unknown` — Rust WASM target installed
- [ ] `stellar --version` — Stellar CLI present and ≥ required version
- [ ] `jq --version` — `jq` present
- [ ] `ts-node --version` — ts-node present (for TypeScript scripts)
- [ ] All environment variables set and non-empty:
  - `STELLAR_NETWORK=testnet`
  - `STELLAR_RPC_URL=https://soroban-testnet.stellar.org`
  - `STELLAR_NETWORK_PASSPHRASE="Test SDF Network ; September 2015"`
  - `STELLAR_SOURCE_ACCOUNT` — testnet secret key (S...) or named identity
  - `STELLAR_ADMIN_ADDRESS` — matching G... public key

### Keypairs & Funding

- [ ] Source account is funded on testnet (minimum 10 XLM recommended)
  - Verify: `stellar account show --account $STELLAR_ADMIN_ADDRESS --network testnet`
- [ ] Admin address matches the public key of `STELLAR_SOURCE_ACCOUNT`
- [ ] No plaintext secrets committed to git (`git log --oneline -5` shows no key material)

### Build

- [ ] All unit tests pass locally:
  ```bash
  cd stellar-swipe && cargo test --workspace 2>&1 | tail -20
  ```
  Pass criterion: `test result: ok` for every crate, zero failures.
- [ ] WASM artifacts built (release):
  ```bash
  cd stellar-swipe && cargo build --workspace --target wasm32-unknown-unknown --release
  ```
  Pass criterion: all `.wasm` files present in `target/wasm32-unknown-unknown/release/`.
- [ ] (Optional) WASM optimized with `stellar contract optimize` for each artifact
- [ ] `config/testnet.json` reviewed — no `REPLACE_WITH_*` placeholder values remain

### Config Review

- [ ] `config/testnet.json` `oracle_address` is a valid testnet contract address
- [ ] `config/testnet.json` `admin` matches `STELLAR_ADMIN_ADDRESS`
- [ ] `min_stake` and `max_fee_rate` are appropriate for testnet testing

---

## Deployment

### Deploy Contracts (in dependency order)

Run the deployment script and capture logs:

```bash
./scripts/deploy_testnet.sh 2>&1 | tee deployments/testnet-deploy.log
```

Verify each contract was deployed in the correct order:

- [ ] `stake_vault` (governance) deployed — contract ID written to `deployments/testnet.json`
- [ ] `signal_registry` deployed — contract ID written to `deployments/testnet.json`
- [ ] `fee_collector` (oracle) deployed — contract ID written to `deployments/testnet.json`
- [ ] `user_portfolio` (auto_trade) deployed — contract ID written to `deployments/testnet.json`
- [ ] `trade_executor` (bridge) deployed — contract ID written to `deployments/testnet.json`
  - Skip with `DEPLOY_TRADE_EXECUTOR=0` only if bridge is intentionally excluded

Pass criterion: `deployments/testnet.json` contains a `contract_id` for each expected contract.

### Initialize Contracts

The deploy script initializes contracts automatically. Confirm each was initialized:

- [ ] `stake_vault` initialized — `initialized: true` in `deployments/testnet.json`
- [ ] `signal_registry` initialized — `initialized: true` in `deployments/testnet.json`
- [ ] `fee_collector` initialized — `initialized: true` in `deployments/testnet.json`
- [ ] `user_portfolio` initialized — `initialized: true` in `deployments/testnet.json`
- [ ] `trade_executor` initialized (or skipped intentionally)

Pass criterion: `jq '.contracts | to_entries[] | select(.value.initialized != true)' deployments/testnet.json` returns empty.

### Cross-Contract References

- [ ] `trade_executor` references the correct `user_portfolio` contract ID
- [ ] Oracle address in `config/testnet.json` matches the deployed `fee_collector` contract ID (if self-hosted)

---

## Post-Deployment

### Automated Verification

```bash
STELLAR_SOURCE_ACCOUNT=$STELLAR_SOURCE_ACCOUNT ./scripts/verify_deployment.sh
```

- [ ] Script exits with code `0`
- [ ] All contracts report `is_initialized=true`
- [ ] All contracts report `is_paused=false`
- [ ] Cross-contract reference check passes

Pass criterion: final line of output is `Exit code: 0`.

### Manual Function Testing

Test each contract with at least one read and one write call:

- [ ] `signal_registry` — submit a test signal, query it back
- [ ] `fee_collector` — query current fee rate
- [ ] `stake_vault` — query governance token supply
- [ ] `user_portfolio` — query portfolio for admin address
- [ ] `trade_executor` — invoke `health_check`

Pass criterion: each call returns expected data without error.

### Testnet Faucet / Onboarding

- [ ] Run `npx ts-node scripts/testnet_utils.ts fund <new-test-address>` to confirm faucet integration works
- [ ] Funded account shows balance on testnet explorer

### Monitoring

- [ ] Start monitoring script: `npx ts-node scripts/monitor.ts`
- [ ] Confirm at least one poll cycle completes without error (check stdout for `[POLL]` lines)

### 24-Hour Observation

- [ ] No unexpected contract errors in the first hour after deployment
- [ ] Oracle heartbeat alerts are not firing (oracle is live)
- [ ] Fee spike alerts are not firing
- [ ] Monitoring script still running after 24 hours with no memory growth

---

## Sign-Off

| Role | Name | Date | Signature |
|------|------|------|-----------|
| Deployer | | | |
| Reviewer | | | |

---

## Completed Checklist (Test Deployment Evidence)

> Fill this section in after running the actual testnet deployment and attach to the PR.

- Deployment date/time (UTC): _______________
- Deployer account (G...): _______________
- `deployments/testnet.json` contract IDs:

| Contract | Contract ID |
|----------|-------------|
| stake_vault | |
| signal_registry | |
| fee_collector | |
| user_portfolio | |
| trade_executor | |

- Verification script output (paste last 20 lines of `testnet-deploy.log`):

```
(paste here)
```

- All checklist items passed: [ ] Yes / [ ] No — if No, describe blockers below:

```
(describe any failures or deviations)
```
