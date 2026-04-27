# Reentrancy Analysis

**Issue:** #264  
**Status:** Mitigated  
**Last updated:** 2026-04-26  
**Scope:** `execute_copy_trade` (trade_executor) and `withdraw_stake` (stake_vault)

---

## Background

Soroban executes each transaction atomically and does not support async callbacks in the
EVM sense. However, reentrancy is still theoretically possible when a contract makes a
cross-contract call to a token or external contract that itself calls back into the
originating contract before the first invocation returns. The two highest-value targets
are functions that move tokens or update balances.

---

## 1. `execute_copy_trade` — `trade_executor`

**File:** `contracts/trade_executor/src/lib.rs`

### External call map

| Step | Callee | Call type | Reentrancy vector? |
|---|---|---|---|
| Balance check | SEP-41 token SAC (`token.balance(user)`) | Read-only cross-contract | No — read-only, no state change |
| Position validate + record | `UserPortfolio::validate_and_record` | Cross-contract write | Theoretical: a malicious portfolio contract could call back |
| Reentrancy lock set | Temporary storage | Local | N/A |

### Analysis

The function sets `EXECUTION_LOCK` (`"ExecLock"`) in temporary storage **before** any
cross-contract call. If a malicious `UserPortfolio` contract called back into
`execute_copy_trade` for the same user, the lock check at the top of the function would
detect `true` and return `ContractError::ReentrancyDetected` immediately.

The lock is cleared on both the success path and all error paths (including the
`resolve_trade_amount` and `check_user_balance` error branches).

### Verdict: MITIGATED ✓

The guard was already present prior to this audit (added in Issue #57). No changes
required.

---

## 2. `withdraw_stake` — `stake_vault`

**File:** `contracts/stake_vault/src/lib.rs`

### External call map

| Step | Callee | Call type | Reentrancy vector? |
|---|---|---|---|
| Load stake record | Persistent storage | Local | No |
| Zero balance in storage | Persistent storage | Local | No |
| Token transfer | SEP-41 token SAC (`token.transfer(vault → staker, amount)`) | Cross-contract write | **Yes** — a malicious token could call back before the function returns |

### Analysis

`withdraw_stake` performs a token transfer as its final step. A malicious SEP-41 token
contract could invoke `withdraw_stake` again during the `transfer` callback. Without a
guard, the second call would find the balance already zeroed (checks-effects-interactions
ordering is applied), so a double-spend is not possible in the current implementation.
However, the guard is added as defence-in-depth for two reasons:

1. Future refactors that reorder the balance-zero and transfer steps would inherit
   protection automatically.
2. The lock prevents any unexpected state mutation from a reentrant call, regardless of
   ordering.

### Guard implementation

```rust
const EXECUTION_LOCK: &str = "WithdrawLock";

let lock_key = Symbol::new(&env, EXECUTION_LOCK);
if env.storage().temporary().get::<_, bool>(&lock_key).unwrap_or(false) {
    return Err(StakeVaultError::ReentrancyDetected);
}
env.storage().temporary().set(&lock_key, &true);

let result = Self::do_withdraw(&env, &staker);

env.storage().temporary().remove(&lock_key);
result
```

The lock uses **temporary storage** (same pattern as `execute_copy_trade`) so it is
automatically scoped to the current transaction and cannot persist across ledgers. It is
explicitly removed on both success and error paths.

### Verdict: MITIGATED ✓ (guard added in this PR)

---

## Guard Pattern Summary

Both guarded functions use the same pattern:

1. **Check:** read the lock key from temporary storage; return `ReentrancyDetected` if set.
2. **Set:** write `true` to the lock key before any external call.
3. **Clear:** remove the lock key on all exit paths (success and error).

Temporary storage is used rather than instance/persistent storage because:
- It is automatically scoped to the current transaction.
- It cannot be left set across ledger boundaries by a failed transaction.
- It does not consume persistent storage quota.

| Function | Contract | Lock key | Added |
|---|---|---|---|
| `execute_copy_trade` | `trade_executor` | `"ExecLock"` | Issue #57 |
| `withdraw_stake` | `stake_vault` | `"WithdrawLock"` | Issue #264 |

---

## Unit Tests

Reentrancy tests are co-located with each contract:

| Test | File | What it verifies |
|---|---|---|
| `reentrant_call_returns_reentrancy_detected` | `trade_executor/src/test.rs` | `ReentrantPortfolio` calls back into `execute_copy_trade`; second call returns `ReentrancyDetected` |
| `lock_cleared_after_successful_execution` | `trade_executor/src/test.rs` | Two sequential calls both succeed (lock is cleared between them) |
| `reentrant_withdraw_is_blocked` | `stake_vault/src/tests.rs` | `ReentrantToken::transfer` calls back into `withdraw_stake`; second call returns `ReentrancyDetected` |
| `lock_cleared_after_successful_withdrawal` | `stake_vault/src/tests.rs` | Two sequential withdrawals both succeed |
| `lock_cleared_after_failed_withdrawal` | `stake_vault/src/tests.rs` | Lock is absent after a failed (NoStake) withdrawal |

---

## Residual Risk

- `execute_copy_trade` does not perform a token transfer itself — it validates balance and
  records the position. If a direct transfer is added in the future, the existing guard
  covers it.
- `withdraw_stake` applies checks-effects-interactions ordering (balance zeroed before
  transfer), so even without the guard a double-spend is not possible in the current
  implementation. The guard provides defence-in-depth.
