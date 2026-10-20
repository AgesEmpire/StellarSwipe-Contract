# StellarSwipe Mainnet Deployment Checklist

> **STOP.** Mainnet deployment is irreversible. Every item below must be checked and
> signed off before proceeding. A single missed step can result in permanent loss of
> funds or an unrecoverable contract state.

This checklist extends the [Testnet Checklist](./testnet_checklist.md). All testnet
checklist items are prerequisites — complete them on testnet first.

---

## Go / No-Go Decision Framework

Before starting deployment, the team must reach a unanimous **Go** decision.
Any single **No-Go** vote blocks the deployment.

| # | Gate | Status | Owner |
|---|------|--------|-------|
| 1 | Security audit completed and sign-off received | ☐ Go / ☐ No-Go | Security Lead |
| 2 | All testnet checklist items passed on a clean testnet run | ☐ Go / ☐ No-Go | Deployer |
| 3 | Multisig admin wallet configured and tested | ☐ Go / ☐ No-Go | Ops Lead |
| 4 | Governance contract initialized and voting tested on testnet | ☐ Go / ☐ No-Go | Governance Lead |
| 5 | Insurance / treasury pool funded | ☐ Go / ☐ No-Go | Finance Lead |
| 6 | Rollback plan documented and reviewed | ☐ Go / ☐ No-Go | Tech Lead |
| 7 | Monitoring and alerting confirmed operational | ☐ Go / ☐ No-Go | Ops Lead |
| 8 | All three required sign-offs obtained (see Sign-Off section) | ☐ Go / ☐ No-Go | All |

**Final decision:** ☐ **GO** — proceed to deployment  /  ☐ **NO-GO** — deployment blocked

---

## Pre-Deployment

### Security Audit

- [ ] External security audit completed for all contracts in scope
- [ ] Audit report received and all Critical/High findings resolved or formally accepted
- [ ] Audit firm sign-off document attached to this PR
- [ ] Internal security review completed (see `docs/security/` for threat models)
- [ ] Fee rounding analysis reviewed: `docs/security/fee_rounding_analysis.md`
- [ ] Flash loan analysis reviewed: `docs/security/flash_loan_analysis.md`
- [ ] Front-running analysis reviewed: `docs/security/front_running_analysis.md`
- [ ] Privilege escalation analysis reviewed: `docs/security/privilege_escalation_analysis.md`

### Multisig Admin Setup

- [ ] Multisig wallet created with threshold ≥ 2-of-3 (or higher per security policy)
- [ ] All multisig signers confirmed and keys verified out-of-band
- [ ] Hardware wallet used for at least one signer (Ledger or equivalent)
- [ ] Test multisig transaction signed and broadcast on testnet to confirm setup
- [ ] `STELLAR_ADMIN_ADDRESS` set to the multisig account address (not a single-key account)
- [ ] No plaintext private keys on the deployment machine

### Environment & Tooling

- [ ] Deployment is NOT running in CI (`CI` env var must not be `true`)
- [ ] Deployment machine is dedicated/clean — not a personal developer laptop
- [ ] All environment variables set and verified:
  - `STELLAR_NETWORK=mainnet`
  - `STELLAR_RPC_URL=https://mainnet.sorobanrpc.com`
  - `STELLAR_NETWORK_PASSPHRASE="Public Global Stellar Network ; September 2015"`
  - `STELLAR_SOURCE_ACCOUNT` — mainnet signer (hardware wallet or offline signer)
  - `STELLAR_ADMIN_ADDRESS` — multisig G... address
- [ ] `config/mainnet.json` reviewed — no `REPLACE_WITH_*` placeholder values
- [ ] `config/mainnet.json` `admin` matches the multisig `STELLAR_ADMIN_ADDRESS`
- [ ] A second team member has independently reviewed `config/mainnet.json`

### Build & Verification

- [ ] All unit tests pass on the exact commit being deployed:
  ```bash
  cd stellar-swipe && cargo test --workspace 2>&1 | tail -20
  ```
- [ ] WASM artifacts built from the tagged release commit (not a dirty working tree):
  ```bash
  git status  # must show "nothing to commit, working tree clean"
  git tag     # confirm release tag is on HEAD
  cd stellar-swipe && cargo build --workspace --target wasm32-unknown-unknown --release
  ```
- [ ] WASM artifacts optimized:
  ```bash
  stellar contract optimize --wasm target/wasm32-unknown-unknown/release/<contract>.wasm
  ```
  (repeat for each contract)
- [ ] WASM checksums recorded and verified by a second team member:

| Contract WASM | SHA-256 | Verified by |
|---------------|---------|-------------|
| stake_vault | | |
| signal_registry | | |
| fee_collector | | |
| user_portfolio | | |
| trade_executor | | |

### Governance & Treasury

- [ ] Governance contract parameters reviewed and approved by governance team
- [ ] Token distribution recipients (`RECIPIENT_*`) set to correct mainnet addresses (not admin fallback)
- [ ] Treasury address is a multisig or DAO-controlled account
- [ ] Insurance pool funded with agreed minimum amount
- [ ] Total supply and distribution percentages match the approved tokenomics

---

## Deployment

> Deploy one contract at a time. Pause after each and verify before continuing.
> Do not use automated scripts without a second reviewer watching in real time.

### Deploy Contracts (in dependency order)

```bash
./scripts/deploy_testnet.sh 2>&1 | tee deployments/mainnet-deploy.log
```

After each deploy step, verify the contract ID with a second reviewer before proceeding:

- [ ] `stake_vault` deployed — contract ID confirmed by second reviewer
- [ ] `signal_registry` deployed — contract ID confirmed by second reviewer
- [ ] `fee_collector` deployed — contract ID confirmed by second reviewer
- [ ] `user_portfolio` deployed — contract ID confirmed by second reviewer
- [ ] `trade_executor` deployed — contract ID confirmed by second reviewer

Pass criterion: `deployments/mainnet.json` contains a `contract_id` for each contract,
and each ID has been independently verified.

### Initialize Contracts

- [ ] `stake_vault` initialized with correct multisig admin and mainnet recipients
- [ ] `signal_registry` initialized
- [ ] `fee_collector` initialized with correct base currency
- [ ] `user_portfolio` initialized
- [ ] `trade_executor` initialized / health-checked

Pass criterion: `jq '.contracts | to_entries[] | select(.value.initialized != true)' deployments/mainnet.json` returns empty.

### Cross-Contract References

- [ ] `trade_executor` references the correct mainnet `user_portfolio` contract ID
- [ ] Oracle address in `config/mainnet.json` matches the deployed `fee_collector` contract ID
- [ ] All cross-contract references verified by second reviewer

---

## Post-Deployment

### Automated Verification

```bash
DEPLOY_STATE="deployments/mainnet.json" \
STELLAR_NETWORK="mainnet" \
STELLAR_RPC_URL="https://mainnet.sorobanrpc.com" \
STELLAR_NETWORK_PASSPHRASE="Public Global Stellar Network ; September 2015" \
./scripts/verify_deployment.sh
```

- [ ] Script exits with code `0`
- [ ] All contracts report `is_initialized=true`
- [ ] All contracts report `is_paused=false`
- [ ] Cross-contract reference check passes

### Manual Function Testing

- [ ] `signal_registry` — submit a real signal, query it back
- [ ] `fee_collector` — query current fee rate, confirm it matches config
- [ ] `stake_vault` — query governance token supply, confirm it matches tokenomics
- [ ] `user_portfolio` — query portfolio for admin address
- [ ] `trade_executor` — invoke `health_check`, confirm healthy response

### Monitoring

- [ ] Monitoring script started: `npx ts-node scripts/monitor.ts`
- [ ] Slack webhook configured and test alert received
- [ ] Oracle heartbeat monitoring confirmed active
- [ ] At least one full poll cycle completed without error

### 24-Hour Observation

- [ ] No unexpected contract errors in the first hour
- [ ] Oracle heartbeat alerts not firing
- [ ] Fee spike alerts not firing
- [ ] Monitoring script running after 24 hours

---

## Required Sign-Offs

All three sign-offs are required before this PR can be merged and before deployment proceeds.

| Role | Name | Date (UTC) | Signature / Approval |
|------|------|------------|----------------------|
| Tech Lead | | | |
| Security Lead | | | |
| Ops Lead | | | |

> By signing off, each reviewer confirms they have personally reviewed this checklist,
> the deployment logs, and the contract IDs, and that they vote **Go** for mainnet deployment.

---

## Rollback Plan

If any post-deployment check fails:

1. Pause affected contracts immediately (if pause controls are available).
2. Preserve all logs: `deployments/mainnet-deploy.log`.
3. Rotate any compromised keys via multisig.
4. Convene the team within 1 hour to assess severity.
5. Prepare a multisig-approved redeploy or migration plan before any further action.
6. Do not attempt a hotfix without repeating this full checklist.

---

## Completed Checklist (Mainnet Deployment Evidence)

> Fill this section in after deployment and attach to the PR.

- Deployment date/time (UTC): _______________
- Deployer account (multisig G...): _______________
- Release tag / commit SHA: _______________
- `deployments/mainnet.json` contract IDs:

| Contract | Contract ID | Verified by |
|----------|-------------|-------------|
| stake_vault | | |
| signal_registry | | |
| fee_collector | | |
| user_portfolio | | |
| trade_executor | | |

- Verification script output (paste last 20 lines of `mainnet-deploy.log`):

```
(paste here)
```

- All checklist items passed: [ ] Yes / [ ] No — if No, describe blockers:

```
(describe any failures or deviations)
```
