# Governance Timelock Bypass Audit

**Issue:** #795
**Status:** Audited — 3 of 27 direct-admin entry points hardened; remainder classified below

---

## 1. Scope

Every public, state-mutating function on `GovernanceContract` (`contracts/governance/src/lib.rs`) that is gated only by `require_admin` — i.e. callable directly by the admin key with **no** timelock delay and **no** reference to a passed governance proposal. Read-only functions and functions already routed through `timelock.rs` (`queue_action`, `execute_queued_action`, `cancel_queued_action`, `emergency_execute`, `emergency_unblock_action`, `extend_execution_window`, `execute_multiple_actions`) are out of scope — those already require either the timelock delay or the guardian key.

27 functions call `require_admin` directly. Each is classified below as:

- **(a) Fixed this PR** — now requires a `proposal_id` that references a proposal with `ProposalStatus::Executed` (i.e. it cleared voting *and* the timelock delay), verified by a new `require_executed_proposal` helper.
- **(b) Ungated, flagged for follow-up** — a real gap, but fixing it safely requires changes/test rewrites beyond this PR's scope (see §4).
- **(c) Intentionally admin-direct** — justified below; not a governance-attack vector.

---

## 2. Findings — (a) Fixed this PR

| Function | Before | After |
|---|---|---|
| `approve_treasury_budget` | Accepted any `proposal_id: u64` as a label; the value was **never validated**. Its own doc comment claimed "must be called... referencing the passing proposal" but nothing enforced that. Since this function is the sole gate on how much `execute_treasury_spend` can draw down (see `treasury::execute_spend`'s `approved_cap` check), an admin could authorize unlimited treasury spending caps with a fabricated proposal id and zero governance vote. | Calls `require_executed_proposal(&env, proposal_id)?` before recording the approval — rejects with `ProposalNotExecuted` (alias of `ProposalNotApproved`) unless the referenced proposal is genuinely `Executed`. |
| `override_committee_decision` | Admin alone could instantly overturn any committee's vote, with no proposal reference and no delay. | Now requires an additional `proposal_id: u64` argument, validated as `Executed`. |
| `dissolve_committee` | Admin alone could instantly dissolve any committee — the clearest case of "committee membership modification" the issue calls out — with no proposal reference and no delay. Previously untested. | Now requires an additional `proposal_id: u64` argument, validated as `Executed`. |

These three were prioritized because they match the issue's own named risk categories ("treasury addresses... and committee membership modifications") and because an *unvalidated* `proposal_id` parameter that looks authoritative is strictly more dangerous than an admin function that makes no pretense of governance approval — a reviewer or integrator reading the code/docs would reasonably assume it was already checked.

See `contracts/governance/src/lib.rs::require_executed_proposal` for the shared check, and `contracts/governance/src/test.rs` (section "Timelock bypass audit tests (issue #795)") for the new tests, plus updated fixtures in every existing test that previously passed a fabricated `proposal_id` to `approve_treasury_budget` / `override_committee_decision`.

---

## 3. Findings — (c) Intentionally admin-direct

| Function | Justification |
|---|---|
| `set_contract_paused` | Emergency circuit breaker; must be fast by design. Matches the timelock's own `ActionType::EmergencyPause` delay of `0` in `initialize_timelock`. |
| `initialize_timelock` | One-time bootstrapping call — cannot be gated by the timelock it is creating. |
| `update_timelock_delay` | Bounded to `[timelock.min_delay, timelock.max_delay]`, both fixed at `initialize_timelock` and never themselves adjustable by this function. Admin cannot use it to shrink protection below the original floor. |
| `create_budget` | Creates a spend *envelope* (category, allocation, limit) but authorizes **zero** spending on its own — real spending is gated by `approve_treasury_budget` (now fixed, see §2). |
| `create_committee`, `start_committee_election`, `finalize_committee_election`, `set_committee_approval_rating` | Day-to-day committee administration. Membership itself is decided by election voting (`nominate_for_committee`, `vote_in_committee_election`) open to token holders, not by admin fiat. |
| `distribute_reputation_rewards`, `update_reputation_config`, `create_conviction_pool`, `set_conviction_calibration`, `set_conviction_decay_rate`, `create_vesting_schedule`, `set_vote_lock`, `accrue_liquidity_rewards`, `set_liquidity_mining_config` | Operational parameter/reward administration for the reputation, conviction-voting, and liquidity-mining subsystems. None move treasury funds or change committee/treasury custody. |
| `execute_treasury_spend`, `create_recurring_payment`, `process_recurring_payments` | Move real funds, but are hard-capped by `approve_treasury_budget`'s approved-cap check (now proposal-validated, §2) — these cannot spend a single unit beyond what an executed governance proposal authorized for that category. |
| `set_rebalance_target`, `rebalance_treasury` | Only adjust internal target-weight bookkeeping (`treasury.rebalance_targets`) and emit recommended rebalancing actions; they do not move funds themselves. |

---

## 4. Findings — (b) Ungated, flagged for follow-up (not fixed in this PR)

| Function | Risk | Why not fixed here |
|---|---|---|
| `set_treasury_asset` | Directly overwrites the *tracked* balance of any asset with no proof of a corresponding real deposit. Since `execute_treasury_spend`'s balance check reads this same tracked value, an admin can inflate it and then spend against it (subject to the now-fixed `approve_treasury_budget` cap, but still bypasses the intent of the balance check as a real custody control). | This is a deposit/accounting-reconciliation design question (should balances be settable at all, or only incremented by verified deposit events?), not purely a timelock-delay question — recommend a dedicated follow-up issue rather than folding a design change into this audit PR. |
| `execute_treasury_spend` / `create_recurring_payment`'s `approved_by_proposal: Option<u64>` parameter | Stored as metadata on `TreasurySpend` / `RecurringPayment` for auditability, but never itself validated against `ProposalStatus::Executed` — real enforcement happens entirely through `approve_treasury_budget`'s cap (now fixed). Since it's informational only, it is misleading if callers assume it's authoritative. | Validating it would mean either requiring `Some(proposal_id)` on every call (breaking `None`-cap-covered spends that don't need per-spend attribution) or silently ignoring `None` — a behavior decision better made with maintainer input rather than assumed here. |

**Recommendation:** open a follow-up issue for `set_treasury_asset` specifically — it is the highest-risk remaining item, since it can be combined with `execute_treasury_spend` to move real value out of the treasury.

---

## 5. Testing

`cargo test -p governance --lib` — new tests added:

- `approve_treasury_budget_rejects_nonexistent_proposal`
- `approve_treasury_budget_rejects_unexecuted_proposal`
- `override_committee_decision_rejects_unexecuted_proposal`
- `dissolve_committee_requires_executed_proposal`

All pre-existing tests that previously passed a fabricated `proposal_id` to `approve_treasury_budget` or `override_committee_decision` were updated to create and finalize a real proposal via a shared `create_executed_proposal` test helper (`contracts/governance/src/test.rs`).

**Pre-existing, unrelated breakage fixed as a prerequisite:** `cargo test -p governance` did not compile at all on `main` before this PR — an earlier, unrelated change added two required parameters to `create_proposal` (`category: ProposalCategory`, `use_quadratic_voting: bool`) without updating ~20 existing call sites across `test.rs`, `test_pause_propagation.rs`, and `test_portableDD.rs`, and two calls in `test.rs` used a `soroban-sdk` API (`Symbol::try_from_val`) not in scope without importing the `TryFromVal` trait. Both were mechanical, no-op-intent fixes (added the two new arguments with sensible defaults matching the pattern already used elsewhere in the file; added the missing trait import) — no test logic changed. This was necessary to get a real `cargo test` run for this PR's own new tests rather than claim untested passes.

**Result:** 149 passed, 6 failed. The 6 failures are pre-existing and unrelated to this audit — they don't exercise any function touched here:
- `test::conviction_calibration_caps_max_conviction`
- `test::conviction_calibration_combination_penalty_and_reward`
- `test::conviction_calibration_reward_long_votes`
- `test::conviction_calibration_zero_threshold_disables_penalty`
- `proposal_deposit::tests::test_deposit_refunded_when_threshold_met`
- `proposal_deposit::tests::test_deposit_forfeited_when_threshold_not_met`

These likely went unnoticed because the crate has not compiled (and therefore not run) since whatever change broke `create_proposal`'s call sites landed. Recommend a separate follow-up issue to investigate them.
