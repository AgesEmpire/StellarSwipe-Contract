# Address Validation Audit (issue #1171)

This document lists every public entrypoint in the `contracts/` crates
(`StakeVault`, `FeeCollector`, `SignalRegistry`) that takes an `Address`, and
the validation rule applied to each address parameter.

## Rejection policy

Soroban has no literal "null" address, but some values are never a legitimate
party:

| Sentinel | Strkey | Why it is rejected |
|----------|--------|--------------------|
| All-zero account | `GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF` | ed25519 key of 32 zero bytes. Nobody holds its private key, so it can never authorize anything, and value sent there is unrecoverable. |
| All-zero contract | `CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABSC4` | Contract id of 32 zero bytes. The conventional placeholder for "unset". |
| The contract itself | `env.current_contract_address()` | A contract named as its own admin, provider, source or destination creates self-referential state and confuses authorization. |

Rules:

1. **State-changing entrypoints reject sentinels.** Every `Address` argument
   of an entrypoint that can write storage or emit an event is checked first,
   before `require_auth` and before any storage read or write. A sentinel
   returns `InvalidAddress`, writes nothing and emits no event.
2. **The error is the same in every contract.** Each contract's error enum has
   `InvalidAddress = 100`. Clients can map contract error code `100` to "invalid
   address" no matter which contract they called.
3. **Read-only queries never fail on a sentinel.** Rule 1 means a sentinel can
   never have stored state, so queries return the documented "absent" value
   (`0`, `None`, `Unknown`, `known == false`). Their return types and ABI are
   unchanged.
4. **Batch maintenance inputs are ignored, not rejected.** Admin-only cleanup
   batches (`purge_expired_tombstones`, `purge_expired_idempotency_keys`) take
   lists of addresses that identify existing records. Rule 1 means no record
   can exist for a sentinel, so such entries are skipped like any other address
   with no record. A single stray entry therefore cannot abort an otherwise
   valid cleanup batch.

Each contract implements the check in a private `require_valid_address` helper
and exports the two strkeys as `ZERO_ACCOUNT_STRKEY` and `ZERO_CONTRACT_STRKEY`.

## Inventory

### StakeVault (`contracts/stake_vault`)

| Entrypoint | Parameter | Rule | Previously |
|------------|-----------|------|------------|
| `initialize` | `admin` | Reject sentinel | No check |
| `delegate` | `provider` | Reject sentinel | No check |
| `withdraw` | `provider` | Reject sentinel (before `require_auth`) | No check |
| `provider_stake` | `provider` | Read-only; sentinel returns `0` | — |

`set_max_provider_share_bps`, `set_paused`, `is_paused`, and `total_stake`
take no address arguments.

### FeeCollector (`contracts/fee_collector`)

| Entrypoint | Parameter | Rule | Previously |
|------------|-----------|------|------------|
| `initialize` | `admin` | Reject sentinel | No check |
| `initialize` | `fee_asset` | Reject sentinel | New parameter (#1247) |
| `set_admin` | `caller` | Reject sentinel | No check |
| `set_admin` | `new_admin` | Reject sentinel | No check |
| `collect` | `source` | Reject sentinel | New parameter (#1247) |
| `set_collection_paused` | `caller` | Reject sentinel | New entrypoint (#1248) |
| `sweep_dust` | `caller` | Reject sentinel | No check |
| `sweep_dust` | `destination` | Reject sentinel; also `InvalidDestination` when it equals `caller` | Documented as "must be non-zero" but not enforced |

`balance`, `fee_asset`, and `is_collection_paused` take no address arguments.

### SignalRegistry (`contracts/signal_registry`)

| Entrypoint | Parameter | Rule | Previously |
|------------|-----------|------|------------|
| `initialize` | `admin` | Reject sentinel | No check |
| `propose_admin` | `new_admin` | Reject sentinel | No check |
| `set_provider_metadata` | `provider` | Reject sentinel | No check |
| `set_provider_metadata_checked` | `provider` | Reject sentinel | No check |
| `remove_provider_metadata` | `provider` | Reject sentinel | No check |
| `update_reputation` | `provider` | Reject sentinel | No check |
| `get_provider_metadata` | `provider` | Read-only; sentinel returns `None` | — |
| `get_versioned_metadata` | `provider` | Read-only; sentinel returns `None` | — |
| `get_record_state` | `provider` | Read-only; sentinel returns `Unknown` | — |
| `get_reputation` | `provider` | Read-only; sentinel returns the default score `0` | — |
| `get_reputations` | `providers[]` | Read-only; sentinel entries return `known == false` | — |
| `purge_expired_tombstones` | `providers[]` | Admin batch; sentinel entries have no record and are skipped | — |
| `purge_expired_idempotency_keys` | `entries[].caller` | Admin batch; sentinel entries have no record and are skipped | — |

`accept_admin`, `get_pending_admin`, `set_decay_schedule`, and
`get_decay_schedule` take no address arguments. `accept_admin` can only promote
the stored pending admin, and `propose_admin` already rejected a sentinel
there.

## Regression tests

| Contract | Tests |
|----------|-------|
| StakeVault | `initialize_rejects_sentinel_admin`, `stake_entrypoints_reject_sentinel_providers_without_state_change` |
| FeeCollector | `initialize_rejects_sentinel_addresses`, `state_changing_entrypoints_reject_sentinel_addresses`, `sentinel_strkeys_are_well_formed` |
| SignalRegistry | `initialize_rejects_sentinel_admin`, `state_changing_entrypoints_reject_sentinel_addresses` |

Each test runs every audited entrypoint with each of the three sentinels. It
checks the `InvalidAddress` error and confirms that no state changed and no
success event was emitted. It also runs the read-only queries with each
sentinel and checks the documented absent value.

## Adding a new entrypoint

When adding an entrypoint that takes an `Address`:

1. If it can write state or emit an event, call `require_valid_address` on
   each address argument as its first statement.
2. Add a row to the inventory above.
3. Add the entrypoint to that contract's sentinel regression test.
