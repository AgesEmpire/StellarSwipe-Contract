# Governance Pause Semantics

## Overview

The governance contract exposes a single admin-controlled boolean flag —
`ContractPaused` — that acts as a global circuit-breaker for all
state-mutating governance actions.  When the flag is `true`, every
operation that changes on-chain state returns
`GovernanceError::ContractPaused (code 55)` before touching any storage.

---

## Setting and Reading the Pause State

| Entry-point | Auth required | Effect |
|---|---|---|
| `set_contract_paused(admin, true)` | Admin | Activates the pause |
| `set_contract_paused(admin, false)` | Admin | Lifts the pause |
| `health_check()` | None (read-only) | Returns `HealthStatus { is_paused, … }` |

`health_check` is **never** gated by the pause flag; monitoring tooling can
always query the live status without special privileges.

---

## Affected Actions

The following entry-points are blocked while `ContractPaused = true`:

### Proposal lifecycle

| Entry-point | Why it is blocked |
|---|---|
| `create_proposal` | Starting new governance work during a pause could produce proposals that execute under abnormal contract state. |
| `cast_vote` | Votes on proposals created or active before the pause are frozen until the issue is resolved. |
| `finalize_proposal` | Prevents irreversible status transitions while state may be inconsistent. |
| `execute_proposal` | Direct execution of approved proposals is stopped; the admin must explicitly unpause first. |

### Staking

| Entry-point | Why it is blocked |
|---|---|
| `stake` | New staking commits tokens to governance at a time when the contract is considered unsafe. |
| `unstake` | Preventing unstaking during a pause protects against vote-power drain attacks that exploit the paused window. |

### Timelock operations

| Entry-point | Why it is blocked |
|---|---|
| `queue_action` | Queuing actions during a pause would schedule executions without the normal governance oversight. |
| `execute_queued_action` | A paused contract should not execute timelocked actions; they remain queued for execution after the pause is lifted. |
| `execute_multiple_actions` | Batch variant of the above. |

---

## Actions NOT Blocked by the Pause

The following remain fully available during a pause to support monitoring,
emergency investigation, and safe read access:

- `health_check` — always readable
- `balance`, `staked_balance`, `voting_power`, `governance_config` — reads
- `proposal`, `proposals` — read existing proposal state
- `timelock_analytics` — read timelock metrics
- `cancel_queued_action` — guardian or admin can **cancel** queued actions even while paused (this is intentional: the guardian must be able to abort dangerous actions during an emergency)
- `cancel_proposal` — admin / guardian / proposer can cancel even while paused
- `treasury`, `treasury_report` — financial reads
- `committees`, `committee`, `committee_report` — committee reads
- `governance_reputation`, `calculate_reputation_score` — reputation reads

---

## Emergency Execute (Guardian Bypass)

`emergency_execute(action_id, guardian)` is a **special-case** that exists
entirely within the timelock module.  It allows the guardian to immediately
execute a queued action of type `ActionType::EmergencyPause` (delay = 0)
**even when the contract is paused**, because the purpose of that action
type is specifically to enact or respond to an emergency.

All other `ActionType` variants still require the contract to be unpaused
before their timelock can be executed.

---

## Cross-Contract Propagation Model

The pause flag lives in the governance contract's own instance storage.
It does **not** automatically push a pause signal to sibling contracts
(signal_registry, auto_trade, oracle, fee_collector).  Each of those
contracts has its own independent category-based pause system
(`pause_category` / `unpause_category`).

When a governance admin wishes to halt the entire platform:

1. Call `governance.set_contract_paused(admin, true)` — blocks all
   governance actions immediately.
2. Call `signal_registry.pause_category(admin, "all", …)` — blocks
   signal submission.
3. Call `auto_trade.pause_category(admin, "trading", …)` — blocks trade
   execution.
4. Call `oracle.pause_category(admin, "all", …)` — blocks price submission.

This explicit multi-step model is intentional: it allows partial pauses
(e.g. pause governance only while trading continues) and avoids hidden
cross-contract dependencies that are difficult to audit.

---

## Error Reference

| Code | Variant | Description |
|---|---|---|
| 55 | `ContractPaused` | The governance contract is administratively paused. Lift the pause with `set_contract_paused(admin, false)` before retrying. |

---

## Test Coverage

`contracts/governance/src/test_pause_propagation.rs` contains 11 tests
covering the full propagation surface:

| Test | Modules covered |
|---|---|
| `paused_blocks_create_proposal` | Proposals |
| `paused_blocks_cast_vote` | Proposals + Voting |
| `paused_blocks_finalize_proposal` | Proposals |
| `paused_blocks_execute_proposal` | Proposals |
| `paused_blocks_stake` | Staking |
| `paused_blocks_unstake` | Staking |
| `paused_blocks_queue_action` | Timelock |
| `paused_blocks_execute_queued_action` | Timelock |
| `paused_blocks_execute_multiple_actions` | Timelock |
| `paused_does_not_block_reads` | Read-only ops |
| `unpause_restores_proposal_and_staking` | Proposals + Staking (round-trip) |
| `unpause_restores_timelock_queue` | Timelock (round-trip) |
