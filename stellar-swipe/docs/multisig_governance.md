# Multisig Governance Procedures

## Overview

StellarSwipe's `signal_registry` contract supports **M-of-N multisignature approval** for critical admin operations. When enabled, direct calls to gated functions return `RequiresMultisigApproval (26)` and must flow through the proposal → approval → timelock → execution pipeline.

Guardian emergency pause remains a **single-signer fast path** and does not require multisig approval.

---

## Enabling Multisig

1. Initialize the contract with a bootstrap admin (`initialize(admin)`).
2. Call `enable_multisig(admin, signers, threshold)` where:
   - `signers` is a unique list of authorized addresses (max 20)
   - `threshold` is M in M-of-N (must satisfy `1 <= M <= N`)
3. After enablement, the bootstrap admin address is superseded by the signer set for authorization checks.

Optional: configure per-action timelock delays via `set_multisig_timelock_config`.

---

## Default Timelock Delays

| Action type | Default delay |
|---|---|
| Fee changes | 3 days |
| Parameter updates | 3 days |
| Admin-initiated pause | 0 (immediate after approval) |
| Unpause | 1 day |
| Guardian assignment | 2 days |
| Admin transfer proposal | 2 days |
| Config changes (fee collection pause/resume) | 2 days |

Delays are configurable by any authorized signer when multisig is disabled, or by signers via `set_multisig_timelock_config` when enabled.

---

## Critical Operations Requiring Approval

When multisig is enabled, these operations **cannot** be called directly:

| Operation | Payload variant |
|---|---|
| `set_trade_fee` | `SetTradeFee(bps)` |
| `set_min_stake` | `SetMinStake(amount)` |
| `set_risk_defaults` | `SetRiskDefaults(stop_loss, position_limit)` |
| `set_tier_signal_limits` | `SetTierSignalLimits(bronze, silver, gold)` |
| Admin pause | `PauseCategory(category, duration, reason)` |
| Unpause | `UnpauseCategory(category)` |
| `set_guardian` | `SetGuardian(address)` |
| `propose_admin_transfer` | `ProposeAdminTransfer(new_admin)` |
| `pause_fee_collection` | `PauseFeeCollection` |
| `resume_fee_collection` | `ResumeFeeCollection` |

---

## Standard Operating Procedure

### 1. Propose

Any signer calls `propose_critical_action(caller, payload)`. The proposer's approval is counted automatically.

**Event:** `multisig_proposal_created(proposal_id, proposer, action_type)`

### 2. Approve

Additional signers call `approve_proposal(caller, proposal_id)` until the threshold is met.

**Event:** `multisig_approval_recorded(proposal_id, approver, approval_count, threshold)`

When M approvals are collected:

**Event:** `multisig_proposal_approved(proposal_id, executable_at)`

### 3. Wait for timelock

Monitor `executable_at` on the proposal (via `get_approval_proposal`). Execution is blocked until `ledger.timestamp >= executable_at`.

### 4. Execute

Any signer calls `execute_proposal(caller, proposal_id)` after the timelock elapses.

**Event:** `multisig_proposal_executed(proposal_id, executor)`

### 5. Cancel (optional)

Any signer may cancel a pending or approved-but-not-executed proposal via `cancel_proposal`.

**Event:** `multisig_proposal_cancelled(proposal_id, cancelled_by)`

---

## Example: 2-of-3 Fee Change

```
Signer A: propose_critical_action(SetTradeFee(25))  → proposal_id = 1
Signer B: approve_proposal(1)                        → status = Approved
[wait 3 days]
Signer C: execute_proposal(1)                      → trade_fee = 25 bps
```

For testing or emergency config, set `fee_change_delay = 0` via `set_multisig_timelock_config`.

---

## Signer Roster Management

| Action | Auth | Notes |
|---|---|---|
| `add_multisig_signer` | Any signer | Adds to roster |
| `remove_multisig_signer` | Any signer | Blocked if `N - 1 < M` |
| `disable_multisig` | Any signer | Reverts to single stored admin |

Roster changes themselves are **not** gated by the approval workflow (they require only one signer today). For production, use a Stellar account-level multisig as the bootstrap admin before enabling on-chain multisig.

---

## Monitoring

Index these events for governance dashboards:

- `multisig_proposal_created`
- `multisig_approval_recorded`
- `multisig_proposal_approved`
- `multisig_proposal_executed`
- `multisig_proposal_cancelled`
- `multisig_timelock_updated`
- `multisig_signer_added` / `multisig_signer_removed`

Query proposal state: `get_approval_proposal(proposal_id)`

Query timelock config: `get_multisig_timelock_config()`

---

## Emergency Response

| Scenario | Procedure |
|---|---|
| Active exploit | Guardian calls `pause_trading` or `pause_category` directly (no multisig) |
| Post-incident unpause | Multisig proposal with `UnpauseCategory` (1-day default timelock) |
| Fee exploit | Multisig proposal to reduce fee; 3-day timelock by default |

---

## Integration with Protocol Governance

The on-chain DAO (`governance` contract) operates independently. For unified protocol-wide changes (fee_collector, auto_trade, oracle), either:

1. Deploy each contract's admin as a Stellar multisig account, or
2. Extend governance `execute_proposal_action` to invoke sibling contracts (future work).

This multisig module secures **signal_registry** critical operations at the contract level.
