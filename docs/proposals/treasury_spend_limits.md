# Treasury Spend Limits & Budget Enforcement

## Problem
Governance-managed treasury funds should not be distributed without explicit
spending constraints and accounting safeguards.

## Proposed Solution
- Add a per-period budget config: `max_spend_per_epoch`, `epoch_secs`, set
  and updatable only via governance proposal execution.
- Track `spent_this_epoch` in storage, reset when the epoch rolls over.
- On any treasury disbursement call, reject if
  `spent_this_epoch + amount > max_spend_per_epoch`; otherwise increment
  the counter and emit a `TreasurySpend` event (recipient, amount, running
  total, epoch id) for auditability.
- Add an optional per-recipient cap and a global emergency pause flag
  (governance-gated) to halt all disbursements.
- Expose read-only getters (`remaining_budget`, `epoch_id`) for off-chain
  dashboards/auditing.

## Rollout
1. Add config + storage fields and getters (no enforcement yet).
2. Enforce limits on disbursement path, default limit = current unrestricted
   behavior (i.e. very high cap) to avoid surprise breakage.
3. Governance sets real budget values once monitoring confirms events look
   correct.
