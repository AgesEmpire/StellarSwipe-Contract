# Admin Privilege Escalation Analysis

**Issue:** #267  
**Status:** Audited & Mitigated

---

## 1. Scope

All paths that write to the `Admin` / `AdminStorageKey::Admin` storage key across the five contracts:

| Contract | Admin Key |
|---|---|
| `auto_trade` | `AdminStorageKey::Admin` |
| `signal_registry` | `AdminStorageKey::Admin` |
| `oracle` | `StorageKey::Admin` |
| `governance` | `StorageKey::Admin` |
| `trade_executor` | `StorageKey::Admin` |

---

## 2. Identified Paths That Write the Admin Key

### 2.1 `auto_trade` — `contracts/auto_trade/src/admin.rs`

| Function | Who can call | Guard |
|---|---|---|
| `init_admin` | Anyone (once) | Panics if already set — one-time init only |
| `accept_admin_transfer` | Pending admin only | `caller == pending_admin`, expiry check, `caller.require_auth()` |

**No other function writes `AdminStorageKey::Admin`.**

### 2.2 `signal_registry` — `contracts/signal_registry/src/admin.rs`

| Function | Who can call | Guard |
|---|---|---|
| `init_admin` | Anyone (once) | Returns `AlreadyInitialized` if already set |
| `accept_admin_transfer` | Pending admin only | `caller == pending.pending_admin`, expiry check, `caller.require_auth()` |

**No other function writes `AdminStorageKey::Admin`.**

### 2.3 `oracle` — `contracts/oracle/src/lib.rs` + `admin.rs`

| Function | Who can call | Guard |
|---|---|---|
| `initialize` | Anyone (once) | Panics if `StorageKey::Admin` already set |
| `accept_admin_transfer` | Pending admin only | `caller == pending_admin`, expiry check, `caller.require_auth()` |

**No other function writes `StorageKey::Admin`.**

### 2.4 `governance` — `contracts/governance/src/lib.rs`

| Function | Who can call | Guard |
|---|---|---|
| `initialize` | Anyone (once) | Returns `AlreadyInitialized` if `StorageKey::Initialized` is set |

**Governance has no admin transfer mechanism — admin is immutable post-init.**  
Governance proposals (`execute_proposal_action`) can write `GovernanceParameters`, `GovernanceFeatures`, `GovernanceUpgrades`, and `Treasury` — **but never `StorageKey::Admin`.**

### 2.5 `trade_executor` — `contracts/trade_executor/src/lib.rs`

| Function | Who can call | Guard |
|---|---|---|
| `initialize` | Anyone (once) | Panics if `StorageKey::Admin` already set |

**No admin transfer mechanism. Admin is immutable post-init.**

---

## 3. Governance Proposal Escalation Path — Analysis

`ProposalType::ContractUpgrade` stores a `(contract_name, new_hash)` pair in `GovernanceUpgrades` map. It does **not** call `env.deployer()` or write `StorageKey::Admin`. The upgrade hash is informational only — actual Wasm replacement requires a separate privileged host call that is not wired to any proposal executor.

`ProposalType::ParameterChange` writes to `GovernanceParameters` (a `Map<String, i128>`). The key is a plain string — it cannot alias `StorageKey::Admin` (which is a `contracttype` enum variant, not a string).

`ProposalType::TreasurySpend` calls `add_balance` on a recipient — this writes `StorageKey::Balances`, not `StorageKey::Admin`.

**Conclusion: No governance proposal type can set or overwrite the admin key.**

---

## 4. Timelock Bypass Analysis

`emergency_execute` in `timelock.rs` is restricted to `ActionType::EmergencyPause` only — it panics with `InvalidCommitteeAction` for any other action type. It cannot execute a `ContractUpgrade` or `ParameterChange` without the timelock delay.

`execute_queued_action` enforces `execution_available` timestamp before executing. The guardian can only cancel actions, not execute them early (except `EmergencyPause`).

**Conclusion: No timelock bypass path exists for admin escalation.**

---

## 5. Guardian Role Analysis

The guardian role (present in `auto_trade`, `signal_registry`, `oracle`) can:
- Pause categories (emergency response)
- Cancel queued timelock actions (governance only)

The guardian **cannot**:
- Set or change the admin key
- Propose or accept admin transfers
- Execute proposals

Guardian is set and revoked exclusively by the current admin (`require_admin` guard on both `set_guardian` and `revoke_guardian`).

---

## 6. Multi-Sig Analysis (`signal_registry`)

`enable_multisig` requires existing admin auth. Once enabled, `require_admin` checks `is_multisig_signer`. Adding/removing signers also requires `require_admin`. A signer cannot unilaterally escalate to sole admin — the admin key itself is only changed via the two-step transfer flow.

---

## 7. Mitigations in Place

| Risk | Mitigation |
|---|---|
| Re-initialization | All contracts guard `init_admin` / `initialize` with an "already set" check |
| Unauthorized admin transfer | Two-step transfer: propose (admin only) → accept (pending admin only) with 48h expiry |
| Governance proposal sets admin | `execute_proposal_action` never writes the admin storage key |
| Timelock bypass | `emergency_execute` restricted to `EmergencyPause` action type only |
| Guardian escalation | Guardian cannot propose/accept admin transfers |

---

## 8. Residual Risk

- `governance` has no admin rotation mechanism. If the admin key is compromised, governance admin is permanently lost. **Recommendation:** add a two-step admin transfer to governance (low priority, no current escalation path).
- `trade_executor` has no admin rotation. Same recommendation.
