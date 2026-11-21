# Treasury Budget Caps

This document describes how governance-approved budget caps are configured,
enforced, and queried in the StellarSwipe governance contract.

---

## Overview

The treasury module has two independent layers of spending control:

| Layer | Configured by | Purpose |
|-------|---------------|---------|
| **Budget** (`create_budget`) | Admin | Allocates funds per category and sets a per-transaction spend limit |
| **Approval cap** (`approve_treasury_budget`) | Admin (referencing a passed proposal) | Sets a cumulative governance-approved ceiling for how much can be drawn from that budget |

Both layers must be satisfied before any spend executes. This means even if
a budget has remaining balance, spending is blocked until a governance
proposal has approved a cap for that category.

---

## Data Structures

### `BudgetApproval`

```rust
pub struct BudgetApproval {
    pub proposal_id: u64,    // governance proposal that authorised this cap
    pub category:    String, // budget category this approval applies to
    pub approved_cap: i128,  // maximum cumulative amount allowed under this approval
    pub total_drawn:  i128,  // amount drawn so far against this cap
    pub approved_at:  u64,   // ledger timestamp when approval was recorded
}
```

Stored in `Treasury.approved_budgets: Map<String, BudgetApproval>`, keyed by
category string. One approval record exists per category at any given time.

---

## Workflow

### 1. Create a budget

```
create_budget(admin, category, allocated, spend_limit, period_start, period_end, auto_renew)
```

- `allocated` — total funds reserved for the category this period.
- `spend_limit` — maximum amount allowed in a **single** transaction.
- The budget is active immediately and tracks `spent` / `remaining`.

### 2. Approve a budget cap via governance

A governance proposal of any type must pass the full vote → finalize →
execute cycle (or the admin may use the `approve_treasury_budget` function
directly after confirming a proposal outcome off-chain).

```
approve_treasury_budget(admin, category, proposal_id, approved_cap)
```

- `proposal_id` — the on-chain governance proposal ID that authorised this cap.
- `approved_cap` — must be > 0 and ≤ `budget.allocated`.
- Re-calling for an existing category **replaces** the previous approval and
  resets `total_drawn` to zero.
- Emits a `(treasury, budgapprv)` event with `(category, proposal_id, approved_cap, timestamp)`.

### 3. Execute a treasury spend

```
execute_treasury_spend(admin, recipient, amount, asset, category, purpose, approved_by_proposal)
```

Before funds move, the contract enforces **all** of the following in order:

1. `purpose` is non-empty and `amount > 0`.
2. The category's budget exists and is within its active period (auto-renews
   if `auto_renew = true`).
3. `amount ≤ budget.remaining` **and** `amount ≤ budget.spend_limit`
   (per-transaction limit).
4. A `BudgetApproval` record exists for the category
   (`BudgetApprovalRequired` if missing).
5. `approval.total_drawn + amount ≤ approval.approved_cap`
   (`ApprovedCapExceeded` if exceeded).
6. Treasury has sufficient asset balance.

On success:
- `budget.spent` and `budget.remaining` are updated.
- `approval.total_drawn` is incremented.
- The spend is appended to `treasury.spending_history`.
- A `(treasury, spend)` event is emitted containing:
  `(spend_id, recipient, amount, category, approved_by_proposal, approval_proposal_id, approved_cap, new_total_drawn, executed_at)`.

---

## Cap Renewal

When a budget period ends and `auto_renew = true`, the budget's `spent` /
`remaining` counters reset automatically. However, **the approval cap is NOT
reset automatically** — a new governance proposal must authorise a fresh cap
for the renewed period via `approve_treasury_budget`.

This is intentional: cap renewal requires explicit governance consent each
period.

---

## Recurring Payments

`create_recurring_payment` requires the category to have both a budget **and**
a governance-approved cap before the payment can be scheduled.

During `process_recurring_payments`, if an individual execution would exceed
the approved cap (or the cap is missing), the recurring payment is
**deactivated** (set `active = false`) rather than returning an error, so
other payments in the queue continue processing.

---

## Error Reference

| Error | Trigger |
|-------|---------|
| `BudgetNotFound` | Category has no budget, or `approve_treasury_budget` called for unknown category |
| `BudgetApprovalRequired` | No `BudgetApproval` record exists for the category |
| `ApprovedCapExceeded` | `total_drawn + amount` would exceed `approved_cap` |
| `BudgetExceeded` | `amount > budget.remaining` or `amount > budget.spend_limit`, or approval cap > allocated |
| `BudgetPeriodEnded` | Budget period expired and `auto_renew = false` |
| `InvalidAmount` | `amount ≤ 0` or `approved_cap ≤ 0` |

---

## Events

| Topics | Data |
|--------|------|
| `(treasury, budgapprv)` | `(category, proposal_id, approved_cap, timestamp)` |
| `(treasury, spend)` | `(spend_id, recipient, amount, category, approved_by_proposal, approval_proposal_id, approved_cap, total_drawn, executed_at)` |

---

## Example

```rust
// 1. Create a budget for "engineering" (600 allocated, 300 per-tx limit, 1 year)
client.create_budget(&admin, &"engineering", &600, &300, &0, &(365 * 86_400), &true);

// 2. Governance proposal #42 passes — admin records the approved cap
client.approve_treasury_budget(&admin, &"engineering", &42, &600);

// 3. Execute a spend against the approved cap
client.execute_treasury_spend(
    &admin, &recipient, &250,
    &xlm_asset, &"engineering", &"Q1 audit", &Some(42),
);
// → approved_budgets["engineering"].total_drawn == 250

// 4. Attempt to overspend is rejected
client.execute_treasury_spend(..., &400, ...);
// → Err(ApprovedCapExceeded)  (250 + 400 = 650 > cap 600)

// 5. New period: re-approve with a fresh proposal
client.approve_treasury_budget(&admin, &"engineering", &55, &600);
// → total_drawn reset to 0 under proposal 55
```

---

## Test Coverage

| Test | Location | What it verifies |
|------|----------|-----------------|
| `spend_without_approval_is_rejected` | `treasury.rs` + `test.rs` | `BudgetApprovalRequired` returned when no cap set |
| `spend_exceeding_approved_cap_is_rejected` | `treasury.rs` + `test.rs` | `ApprovedCapExceeded` when cumulative draw > cap |
| `spend_exactly_at_cap_succeeds` | `treasury.rs` | Boundary: draw == cap is allowed |
| `re_approval_resets_drawn_counter` | `treasury.rs` + `test.rs` | New proposal resets `total_drawn` to 0 |
| `approve_budget_for_unknown_category_fails` | `treasury.rs` + `test.rs` | `BudgetNotFound` on non-existent category |
| `approve_budget_cap_exceeding_allocated_fails` | `treasury.rs` + `test.rs` | `BudgetExceeded` when cap > allocated |
| `recurring_payment_deactivated_when_cap_exhausted` | `treasury.rs` + `test.rs` | Recurring payment auto-deactivates on cap exhaustion |
| `non_admin_cannot_approve_budget` | `test.rs` | `Unauthorized` for non-admin caller |
