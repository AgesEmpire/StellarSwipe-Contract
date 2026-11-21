# Cross-Contract Call Allowlist

## Overview

The cross-contract call allowlist restricts which contracts can invoke sensitive
entrypoints, reducing the attack surface when privileged logic is exposed to
other contracts on-chain.

Before this feature, any contract could call any entrypoint in StellarSwipe
contracts — including privileged flows like keeper-triggered close operations.
The allowlist narrows this to an explicit set of approved counterpart contracts.

---

## Module

The allowlist is implemented centrally in `contracts/shared/src/allowlist.rs`
and exported through `shared::allowlist`.

```rust
use shared::allowlist::{
    add_allowed_contract, remove_allowed_contract,
    require_allowed_contract, is_contract_allowed,
    get_allowlist, AllowlistError,
};
```

---

## How It Works

Each protected entrypoint is identified by an `Address` (typically
`env.current_contract_address()` or a stable logical identifier). The allowlist
maps that address to a `Vec<Address>` of approved caller contracts, stored in
instance storage.

Before executing privileged logic, the contract calls:

```rust
require_allowed_contract(&env, &entrypoint, &caller)?;
```

This returns `Err(AllowlistError::ContractNotAllowed)` for any caller not on the
list, rejecting the request before any state mutation occurs.

---

## Cross-Contract Messaging Integration

The `shared::cross_contract` module exposes a convenience wrapper:

```rust
use shared::cross_contract::require_sensitive_caller;

require_sensitive_caller(&env, &env.current_contract_address(), &caller)?;
```

This returns `Err(CrossContractError::CallerNotAllowed)` for unapproved callers,
surfacing as a standard `CrossContractError` variant.

---

## Admin Operations

All allowlist mutations require the contract admin to call them explicitly.
They are **not** exposed as public contract entrypoints by default — embed them
inside admin-gated entrypoints in each contract that needs allowlist management.

| Function | Description |
|---|---|
| `add_allowed_contract(env, entrypoint, contract)` | Add a contract to the allowlist. Returns `ContractAlreadyAllowed` if already present. |
| `remove_allowed_contract(env, entrypoint, contract)` | Remove a contract. Returns `ContractNotInAllowlist` if absent. |
| `get_allowlist(env, entrypoint)` | Read the full allowlist for an entrypoint. |
| `is_contract_allowed(env, entrypoint, contract)` | Read-only check, no error. |

### Capacity

Each entrypoint allowlist can hold at most `MAX_ALLOWLIST_SIZE = 50` contracts.
This bounds instance storage growth and keeps lookups O(n) with a small constant.

---

## Deployment Checklist

After deploying contracts that use the allowlist:

1. Identify the caller contract address (e.g. `TradeExecutor`).
2. Call the admin-gated `add_allowed_contract` entrypoint on the callee contract,
   passing the caller's address.
3. Verify with `is_contract_allowed` before going live.
4. Document the expected allowlist in the deployment manifest.

Example (using Stellar CLI):

```sh
stellar contract invoke \
  --id <CALLEE_CONTRACT_ID> \
  -- add_allowed_contract \
  --entrypoint <CALLEE_CONTRACT_ID> \
  --contract <CALLER_CONTRACT_ID>
```

---

## Error Reference

| Variant | Code | When returned |
|---|---|---|
| `ContractNotAllowed` | 1 | Caller is not in the allowlist |
| `ContractAlreadyAllowed` | 2 | Adding a contract already present |
| `ContractNotInAllowlist` | 3 | Removing a contract that is not present |
| `AllowlistFull` | 4 | Adding would exceed `MAX_ALLOWLIST_SIZE` |

---

## Testing

Tests covering trusted and untrusted caller scenarios live in
`contracts/shared/src/allowlist.rs` (`#[cfg(test)] mod tests`).

Key cases covered:

- Approved caller passes `require_allowed_contract`.
- Unapproved caller is rejected before any state change.
- Duplicate adds and missing removes return correct errors.
- Allowlist capacity limit is enforced.
- Multiple entrypoints have independent lists.
