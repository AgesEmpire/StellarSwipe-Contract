# Security: Governance Timelock Audit

## Governance Timelock Architecture

The governance contract implements a two-layer timelock system:

1. **Proposal-based timelock** (`timelock.rs`): Proposals that pass voting are queued
   and executed after a configurable delay (TreasurySpend: 2 days, ParameterChange: 3 days,
   ContractUpgrade: 5 days).

2. **Admin action timelock** (`queue_admin_action` / `execute_admin_action`): Critical
   admin-only functions that bypass the proposal pipeline are now routed through a
   mandatory 2-day timelock. Admin queues an intent, waits for the delay, then executes.

---

## Category (a) — Correctly Gated (via Proposal Timelock)

| Function | Gate |
|---|---|
| `execute_proposal` | Proposal pipeline → `queue_action` → `execute_queued_action` |
| `queue_action` | Requires succeeded proposal |
| `execute_queued_action` | Enforces timelock delay |
| `cancel_queued_action` | Admin/guardian only |
| `emergency_execute` | Guardian only, EmergencyPause type only |
| `emergency_unblock_action` | Guardian only, after stuck grace period |
| `execute_multiple_actions` | Enforces timelock delay per action |

## Category (b) — Routed Through Admin Timelock (was ungated)

These functions previously allowed admin to directly mutate critical state.
They now require a two-step flow: `queue_*` then `*_timelocked`.

The original direct entry points still exist for backward compatibility, but
each one now calls `require_no_timelock_bypass()`. Once an operator calls
`enforce_admin_timelock(admin)` — a **one-way latch** with no disable function,
intended to be switched on before a DAO token launch — every direct entry
point below rejects the call with `GovernanceError::TimelockBypassBlocked`, so
the `queue_*` + `*_timelocked` pair becomes the only path. Status is readable
via `is_admin_timelock_enforced()`. Full inventory:
`docs/governance-timelock-audit.md`.

| Queue Function | Execute Function | Action | Rationale |
|---|---|---|---|
| `queue_set_treasury_asset` | `set_treasury_asset_timelocked` | Direct treasury balance manipulation | Prevents sudden drain/inflate of treasury |
| `queue_execute_treasury_spend` | `execute_treasury_spend_timelocked` | Direct treasury spending | Prevents unauthorized fund transfers |
| `queue_configure_governance` | `configure_governance_timelocked` | Governance parameter changes | Prevents governance parameter manipulation |
| `queue_set_category_thresholds` | `set_category_thresholds_timelocked` | Quorum/supermajority threshold changes | Prevents quorum manipulation |
| `queue_create_committee` | `create_committee_timelocked` | Committee creation | Prevents stacking committees |
| `queue_dissolve_committee` | `dissolve_committee_timelocked` | Committee dissolution | Prevents removing oversight |
| `queue_override_committee_decision` | `override_committee_decision_timelocked` | Committee decision override | Prevents bypassing committee governance |
| `queue_set_guardian` | `set_guardian_timelocked` | Guardian address change | Prevents guardian takeover |
| `queue_grant_capability` | `grant_capability_timelocked` | Capability grants | Prevents permission escalation |
| `queue_revoke_capability` | `revoke_capability_timelocked` | Capability revocation | Prevents permission revocation attacks |
| `queue_create_budget` | `create_budget_timelocked` | Budget creation | Prevents unauthorized spending budgets |
| `queue_approve_treasury_budget` | `approve_treasury_budget_timelocked` | Budget approval | Prevents unauthorized budget caps |
| `queue_create_recurring_payment` | `create_recurring_payment_timelocked` | Recurring payment scheduling | Prevents unauthorized recurring drains |
| `queue_enter_shadow_mode` | `enter_shadow_mode_timelocked` | WASM upgrade shadow trial | Prevents unauthorized upgrade initiation |
| `queue_promote_from_shadow_mode` | `promote_from_shadow_mode_timelocked` | WASM upgrade promotion | Prevents unauthorized upgrade finalization |
| `queue_update_timelock_delay` | `update_timelock_delay_timelocked` | Timelock delay configuration | Prevents reducing timelock to bypass delays |
| `queue_create_vesting_schedule` | `create_vesting_schedule_timelocked` | Vesting schedule creation | Prevents unauthorized token vesting |
| `queue_set_rebalance_target` | `set_rebalance_target_timelocked` | Treasury rebalance targets | Prevents manipulation of allocation targets |

## Category (c) — Intentionally Ungated (with Rationale)

These functions are deliberately not routed through the admin timelock.

### Emergency Operations

| Function | Rationale |
|---|---|
| `set_contract_paused` | Emergency pause must be instant. Capability-gated (`Capability::Pause`). Propagates to downstream contracts (Issue #865). |
| `emergency_revoke_admin` | Guardian emergency action to revoke compromised admin. Requires guardian auth. Time-critical by nature. |
| `register_pause_target` / `unregister_pause_target` | Pause infrastructure configuration. Capability-gated (`Capability::Pause`). Must be operational before pause is needed. |

### Admin Key Rotation

| Function | Rationale |
|---|---|
| `propose_key_rotation` | Proposes a new admin but does not execute the change. Low risk — the rotation requires `accept_key_rotation` by the new admin. |
| `accept_key_rotation` | Only callable by the pending new admin. Inherently safe because the new admin must consent. |
| `cancel_key_rotation` | Allows current admin to cancel a pending rotation. No state mutation beyond clearing `PendingAdmin`. |

### Timelock Enforcement

| Function | Rationale |
|---|---|
| `enforce_admin_timelock` | One-way latch that *tightens* security by disabling the category (b) direct entry points. There is deliberately no disable function, so timelocking it would only delay a purely risk-reducing change. Admin-only. |
| `cancel_admin_action` / `cancel_queued_action` | Cancel a pending timelock action before it executes. Risk-reducing — a delay would only make a queued malicious action harder to stop. |
| `cancel_shadow_mode` | Aborts a pending WASM-upgrade trial without promoting it. Risk-reducing. |

### Operational / Low-Risk

| Function | Rationale |
|---|---|
| `update_reputation_config` | Adjusts reputation decay/penalty parameters. Low immediate risk; affects scoring only, not token balances. |
| `set_liquidity_mining_config` | Adjusts reward rates. Admin-gated but amounts are bounded by `reward_bps` (basis points) and `min_claim_threshold`. |
| `set_deposit_config` | Adjusts proposal spam-deposit requirements. Low systemic risk. |
| `set_conviction_calibration` | Adjusts conviction voting calibration. Low immediate risk. |
| `set_conviction_decay_rate` | Adjusts conviction decay. Bounded by MIN/MAX_DECAY_RATE. |
| `create_conviction_pool` | Creates a new conviction voting pool. Admin-gated, funds from existing treasury allocation. |
| `process_recurring_payments` | Processes already-scheduled payments. Does not create new obligations. |
| `rebalance_treasury` | Rebalances according to pre-set targets. Admin-gated, bounded by existing targets. |
| `set_vote_lock` | Administrative utility for vote locking. Admin-gated. |
| `accrue_liquidity_rewards` | Calculates rewards based on trading volume. No new token minting. |
| `distribute_reputation_rewards` | Distributes reputation rewards. Admin-gated, amounts specified per call. |
| `start_committee_election` / `finalize_committee_election` | Election management. Admin-gated, governed by election rules (duration, quorum). |
| `set_committee_approval_rating` | Sets community approval rating. Read-only metric, does not trigger actions. |

### Read-Only / No State Mutation

All getter/query functions (e.g., `treasury`, `balance`, `proposal`, `committees`,
`health_check`, `analytics`, etc.) are excluded by design — they do not modify state.

---

## Security Notes

- The admin timelock delay is fixed at **2 days** (172,800 seconds) as a baseline.
- The existing proposal timelock delays (2-5 days) remain unchanged.
- `cancel_admin_action` is admin-only and can cancel any pending admin action before execution.
- The `emergency_execute` path for proposal-based timelock remains guardian-only and is
  limited to `EmergencyPause` action types with a 1-day stuck grace period.
- The original (non-timelocked) versions of category (b) functions remain callable **only
  while `enforce_admin_timelock` has not been latched on** (the default). Operators MUST call
  `enforce_admin_timelock(admin)` as the final step of DAO hand-off; from then on the direct
  entry points revert with `TimelockBypassBlocked` and the `queue_*` + `*_timelocked` pair is
  the only path. Front-ends and off-chain tooling should target the `*_timelocked` variants.
- `TimelockBypassBlocked` is an alias of `Unauthorized` (the `GovernanceError` enum is at the
  50-variant XDR cap), so on-chain it surfaces as error code 3.
