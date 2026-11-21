# Shared Module — Version Upgrade Rules

## Overview

Every contract in `stellar-swipe` stores a monotonically increasing `u32`
version in instance storage (set during `initialize` via
`shared::version::set_contract_version`). Before making a cross-contract call,
the caller fetches the callee's version and asserts compatibility using
`check_compatible` or `require_compatible`.

## Version constants (current)

| Contract         | Constant                    | Value |
|------------------|-----------------------------|-------|
| `signal_registry`| `SIGNAL_REGISTRY_VERSION`   | 2     |
| `auto_trade`     | `AUTO_TRADE_VERSION`        | 2     |
| `oracle`         | `ORACLE_VERSION`            | 2     |
| `stake_vault`    | `STAKE_VAULT_VERSION`       | 2     |
| `fee_collector`  | `FEE_COLLECTOR_VERSION`     | 2     |

## Minimum acceptable callee versions

Defined in `shared::version::min_version_for(ContractKind)`:

| Callee kind       | Min acceptable version |
|-------------------|------------------------|
| `SignalRegistry`  | 2                      |
| `AutoTrade`       | 2                      |
| `Oracle`          | 2                      |
| `StakeVault`      | 2                      |
| `FeeCollector`    | 2                      |

A callee whose stored version is **below** these values will be rejected with
`VersionError::IncompatibleContractVersion` (error code `1`).

## How to bump a version

1. Increment the relevant `*_VERSION` constant in `shared/src/version.rs`.
2. Update `min_version_for` if older callees should now be considered
   incompatible (breaking change). Leave it unchanged for backward-compatible
   changes.
3. Callers re-deploy first, then callees. This way callers always see an
   up-to-date callee.
4. Add a changelog entry describing what changed in the new version.

## Rules for backward-compatible changes (no min-version bump needed)

- Adding new read-only query methods.
- Extending a response type with optional trailing fields.
- Internal refactors that don't alter on-chain storage layout.

## Rules requiring a min-version bump (breaking changes)

- Removing or renaming an existing method.
- Changing storage key layout in a way that breaks old readers.
- Changing the semantics of an existing method parameter.
- Removing a `#[contracttype]` variant used by callers.

## Using the API

```rust
// In a contract method that calls into `oracle`:
use shared::version::{check_compatible, ContractKind, get_contract_version};
use shared::cross_contract::validate_callee_version;

// Option A — propagate the error:
validate_callee_version(&env, &oracle_address, ContractKind::Oracle)?;

// Option B — panic immediately (preferred inside #[contractimpl]):
require_compatible(&env, oracle_version, ContractKind::Oracle);
```

## Error handling

`VersionError::IncompatibleContractVersion` (SDK error code `1`) is surfaced as
a Soroban invocation failure. Callers should catch it and surface it to users as
a "please upgrade the target contract" message.
