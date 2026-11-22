# Contract Upgrade Procedures

This document describes the step-by-step upgrade procedure for all StellarSwipe
contracts.  The on-chain `run_pre_upgrade_checks` / `record_rollback_checkpoint`
entry points (Issue #513) must be called before any WASM replacement.

---

## Overview

StellarSwipe contracts use a strict version-monotonicity invariant: every
upgrade must set `new_version > current_version`.  The `upgrade()` entry point
rejects equal or lower versions with `IncompatibleContractVersion`.

Upgrades go through five phases:

```
1. Pre-upgrade validation (off-chain + on-chain)
2. Rollback checkpoint capture
3. Pre-upgrade check report
4. WASM upload and upgrade
5. Post-upgrade verification
```

---

## Phase 1: Pre-upgrade Validation (off-chain)

Before touching the chain, the release engineer must:

1. **ABI diff**: run `stellar-cli contract diff` between the current and new
   WASM to confirm no breaking changes exist in entry-point signatures.
2. **Storage diff**: compare `storage-baselines/<contract>.json` with the new
   WASM's storage layout.  Every key that existed before must still be
   deserializable.
3. **Testnet dry run**: deploy the new WASM to `testnet` and run the full
   integration test suite (`stellar-swipe/contracts/integration_tests`).

---

## Phase 2: Rollback Checkpoint Capture

Call `record_rollback_checkpoint` with the **current** WASM hash **before**
uploading the new WASM.  This stores the previous version and hash on-chain so
the upgrade can be reversed by governance.

```typescript
await client.record_rollback_checkpoint({
  caller: adminAddress,
  previous_wasm_hash: currentWasmHash,  // 32-byte hex
});
```

---

## Phase 3: Pre-upgrade Check Report

Call `run_pre_upgrade_checks`.  Inspect `report.all_passed`; abort if `false`.

```typescript
const report = await client.run_pre_upgrade_checks({
  caller: adminAddress,
  target_version: newVersion,
  descriptors: [
    { name: "admin",        required: true  },
    { name: "pause_states", required: true  },
    { name: "fee_config",   required: false },
  ],
  min_caller_version: null,
});

if (!report.all_passed) {
  console.error("Pre-upgrade checks failed:", report.results);
  process.exit(1);
}
```

The report is also stored on-chain and retrievable via `get_last_upgrade_report`.

---

## Phase 4: WASM Upload and Upgrade

```typescript
// 1. Upload the new WASM via the Stellar Deployer
const newWasmHash = await deployer.upload_contract_wasm({ wasm: newWasmBytes });

// 2. Call upgrade()
await client.upgrade({
  caller: adminAddress,
  new_wasm_hash: newWasmHash,
  new_version: newVersion,
});
```

The contract emits a `contract_upgraded` event that includes both the previous
and new version numbers.

---

## Phase 5: Post-upgrade Verification

After the upgrade ledger is confirmed:

1. Call `get_contract_version()` and assert it equals `newVersion`.
2. Re-run the integration test suite against the upgraded contract.
3. Verify that deprecated entry-point shims in `compat.rs` still function
   correctly by calling each one and checking for `DeprecatedCall` events.
4. Update `storage-baselines/<contract>.json` to reflect the new storage layout.

---

## Rollback Procedure

If a critical issue is discovered after the upgrade:

1. Retrieve the rollback checkpoint: `get_rollback_checkpoint()`.
2. Re-upload `checkpoint.previous_wasm_hash` via the Deployer.
3. Call `upgrade()` with a new version number **strictly greater** than the
   current (even after rollback the version must increase).
4. Notify all off-chain monitors and indexers.

> **Note:** Because version must always increase, a rollback installs the old
> WASM under a new version number.  The original pre-rollback version number
> is never reused.

---

## State Migration Tests

For upgrades that change storage layout, write a migration test in
`contracts/integration_tests` that:

1. Deploys the **old** WASM and writes representative state.
2. Upgrades to the **new** WASM via `upgrade()`.
3. Reads all previously-written state and asserts it deserializes correctly.
4. Asserts that any new storage keys have sensible default values.

See `contracts/integration_tests/src/` for existing examples.

---

## Version Compatibility Matrix

| Contract          | Current version | Minimum caller version |
|-------------------|-----------------|------------------------|
| `auto_trade`      | see `Cargo.toml`| 1                      |
| `fee_collector`   | see `Cargo.toml`| 1                      |
| `signal_registry` | see `Cargo.toml`| 1                      |
| `stake_vault`     | see `Cargo.toml`| 1                      |
| `trade_executor`  | see `Cargo.toml`| 1                      |

Update this table whenever a breaking change is introduced.
