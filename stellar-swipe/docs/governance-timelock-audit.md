# Governance Timelock Audit (Issue #942)

**Scope:** every state-mutating entry point in
`contracts/governance/src/lib.rs`, checked for whether a malicious admin can
use it to change critical state without waiting out a timelock delay.

**Status:** all category (b) findings are now routed through the admin
timelock. A one-way enforcement latch (`enforce_admin_timelock`) makes the
direct entry points reject calls once it is switched on — intended to be done
before a DAO token launch.

---

## 1. Timelock architecture

The governance contract has two independent timelock layers:

| Layer | Module | Flow |
|---|---|---|
| **Proposal timelock** | `timelock.rs` (`queue_action` / `execute_queued_action`) | A proposal that passes voting is queued, then executed only after the per-`ActionType` delay (`TreasurySpend` 2d, `ParameterChange` 3d, `ContractUpgrade` 5d, `EmergencyPause` 0). |
| **Admin timelock** | `timelock.rs` (`queue_admin_action` / `execute_admin_action`) | An admin-only action that skips the proposal pipeline is queued with a fixed 2-day delay, then executed via its `*_timelocked` entry point. |

### Enforcement latch (added by this issue)

`queue_*` + `*_timelocked` pairs existed already, but the **original direct
entry points were still callable**, so the delay was optional. This PR adds:

- `enforce_admin_timelock(admin)` — one-way latch, admin-only. No disable
  function exists, so a compromised admin cannot re-open the bypass.
- `is_admin_timelock_enforced() -> bool` — read-only status.
- `require_no_timelock_bypass()` — called at the top of every category (b)
  direct entry point. While the latch is on it returns
  `GovernanceError::TimelockBypassBlocked` (alias of `Unauthorized`).

Default is **off** for backward compatibility (pre-launch deployments and the
existing test suite). Operators enable it as the final step of DAO hand-off.

---

## 2. Entry-point inventory

### Category (a) — correctly gated by the proposal timelock

| Entry point | Gate |
|---|---|
| `execute_proposal` | Runs the proposal pipeline; `TreasurySpend` / `ParameterChange` / `ContractUpgrade` proposals must go `queue_action` → wait → `execute_queued_action`. |
| `queue_action` | Requires a `Succeeded` proposal. |
| `execute_queued_action` / `execute_multiple_actions` | Reject before `execution_available`. |
| `cancel_queued_action` | Admin/guardian only; risk-reducing. |
| `emergency_execute` | Guardian only, `EmergencyPause` action type only. |
| `emergency_unblock_action` | Guardian only, only after the stuck-grace period (`execution_available + 1d`). |
| `extend_execution_window` | Admin only; can only lengthen the delay. |

### Category (b) — was ungated, now routed through the admin timelock

Each row has a `queue_<x>` + `<x>_timelocked` pair. The **direct** entry point
in column 1 now calls `require_no_timelock_bypass()` and is rejected with
`TimelockBypassBlocked` once `enforce_admin_timelock` is latched on.

| Direct entry point (guarded) | Queue / execute pair | Critical state it mutates |
|---|---|---|
| `set_treasury_asset` | `queue_set_treasury_asset` / `set_treasury_asset_timelocked` | Raw treasury balances |
| `execute_treasury_spend` | `queue_execute_treasury_spend` / `treasury_spend_timelocked` | Treasury outflows |
| `create_budget` | `queue_create_budget` / `create_budget_timelocked` | Spending budgets |
| `approve_treasury_budget` | `queue_approve_treasury_budget` / `treasury_budget_timelocked` | Budget spend caps |
| `create_recurring_payment` | `queue_create_recurring_payment` / `recurring_payment_timelocked` | Recurring treasury obligations |
| `configure_governance` | `queue_configure_governance` / `configure_governance_timelocked` | Quorum, thresholds, voting periods |
| `set_category_thresholds` | `queue_set_category_thresholds` / `category_thresholds_timelocked` | Per-category quorum / supermajority |
| `create_committee` | `queue_create_committee` / `create_committee_timelocked` | Committee membership & authorities |
| `dissolve_committee` | `queue_dissolve_committee` / `dissolve_committee_timelocked` | Removes committee oversight |
| `override_committee_decision` | `queue_committee_override` / `committee_override_timelocked` | Bypasses a committee vote |
| `set_guardian` | `queue_set_guardian` / `set_guardian_timelocked` | Guardian address (emergency powers) |
| `grant_capability` | `queue_grant_capability` / `grant_capability_timelocked` | Permission escalation |
| `revoke_capability` | `queue_revoke_capability` / `revoke_capability_timelocked` | Permission removal |
| `update_timelock_delay` | `queue_update_timelock_delay` / `update_timelock_delay_timelocked` | The delays themselves |
| `create_vesting_schedule` | `queue_create_vesting_schedule` / `vesting_schedule_timelocked` | Token vesting grants |
| `enter_shadow_mode` | `queue_enter_shadow_mode` / `enter_shadow_mode_timelocked` | Starts a WASM-upgrade trial |
| `promote_from_shadow_mode` | `queue_promote_from_shadow_mode` / `shadow_mode_promote_timelocked` | Finalizes a WASM upgrade |
| `set_rebalance_target` | `queue_set_rebalance_target` / `set_rebalance_target_timelocked` | Treasury allocation targets |

### Category (c) — intentionally ungated (rationale in `SECURITY.md`)

Emergency ops (`set_contract_paused`, `emergency_revoke_admin`,
`register_pause_target`, `unregister_pause_target`), the consent-based key
rotation flow (`propose_key_rotation` / `accept_key_rotation` /
`cancel_key_rotation`), risk-reducing cancels (`cancel_admin_action`,
`cancel_shadow_mode`), one-time setup (`initialize`, `initialize_timelock`),
the enforcement latch itself (`enforce_admin_timelock`), permissionless
housekeeping (`cleanup_proposals`, `reclaim_expired_proposal`,
`migrate_storage`), bounded low-risk parameter setters
(`update_reputation_config`, `set_liquidity_mining_config`,
`set_deposit_config`, `set_conviction_calibration`,
`set_conviction_decay_rate`), execution of already-authorized work
(`process_recurring_payments`, `rebalance_treasury`,
`distribute_reputation_rewards`, `accrue_liquidity_rewards`), election
management (`start_committee_election`, `finalize_committee_election`,
`set_committee_approval_rating`), and user self-service
(`stake`, `unstake`, `cast_vote`, `delegate_voting_power`,
`release_vested_tokens`, conviction/quadratic voting, …).

---

## 3. Tests

`contracts/governance/src/test_admin_timelock.rs`:

- `enforce_admin_timelock_is_latched_and_readable` / `enforce_admin_timelock_rejects_non_admin`
- `direct_set_treasury_asset_allowed_before_enforcement` — latch off ⇒ direct call still works
- `direct_set_treasury_asset_blocked_after_enforcement` — latch on ⇒ `TimelockBypassBlocked`
- `enforcement_blocks_every_category_b_direct_entry_point` — every row of the category (b) table, direct call rejected
- `timelocked_path_still_works_after_enforcement` — `queue_*` + `*_timelocked` unaffected
- `enforcement_leaves_category_c_functions_callable` — pause / pause-target / key rotation still direct

Plus the pre-existing `*_timelocked_rejects_without_queue` /
`*_rejects_before_delay` / `*_succeeds_after_delay` coverage.

---

## 4. Recommendation

Call `enforce_admin_timelock(admin)` as the last step of DAO hand-off, after
the multisig / DAO controls the admin address. Front-ends and off-chain
tooling must target the `*_timelocked` entry points from that point on.
