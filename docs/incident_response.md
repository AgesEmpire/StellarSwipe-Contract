# Incident Response: Emergency Pause for `stake_vault`

This runbook documents the emergency pause and recovery controls for the
`stake_vault` contract (issue #982). It covers who may pause, which entry
points are affected, how safe exits behave, and how to recover.

## Scope

The pause is narrowly scoped to **risk-bearing operations** in `stake_vault`.
It is intended for active incidents (exploit in progress, oracle failure,
unexpected accounting drift) where halting new risk is safer than continuing
to accept deposits.

## Authorization

- **Who may pause / unpause:** the contract administrator (owner/admin) only.
- Non-admin callers must be rejected; the pause flag must not be settable by
  arbitrary users or by the fee collector.
- Authorization is enforced on-chain at the entry point, not only in the UI.

## Affected entry points

While paused, the following **risk-bearing** entry points are blocked and
must revert (or return an explicit paused error):

- `deposit` / `stake` (new stake creation)
- any other entry point that increases protocol risk or mints new stake
  positions

## Safe-exit paths (must remain open)

While paused, users must **always** be able to exit:

- `withdraw` / `unstake` and any other path that reduces a user's position
  or returns funds to the user.
- Safe-exit paths are **not** gated by the pause flag. Do not add pause
  checks to withdrawal logic.

If a safe-exit path is ever blocked during a pause, treat it as a
high-severity regression and roll back immediately.

## Pause / unpause semantics

- `pause` and `unpause` are **idempotent**: calling `pause` while already
  paused, or `unpause` while already unpaused, is a no-op and must not error.
- Both transitions emit an **audit event** (e.g. `Paused` / `Unpaused` with
  the acting admin and timestamp) so the state change is observable off-chain.
- The current pause state is surfaced to clients via a read-only view
  (e.g. `is_paused()` / `paused()`), so front-ends and monitors can reflect
  the state without simulating a transaction.

## Recovery procedure

1. **Detect & confirm** the incident (monitoring alert, anomalous deposits,
   oracle deviation).
2. **Pause** via the admin key. Confirm the `Paused` event and that
   `is_paused()` returns true.
3. **Verify safe exits** still work: perform a small `withdraw`/`unstake`
   and confirm it succeeds while paused.
4. **Mitigate** the root cause (patch, parameter change, oracle fix).
5. **Unpause** via the admin key. Confirm the `Unpaused` event and that
   `is_paused()` returns false.
6. **Post-incident:** document timeline, root cause, and any follow-up
   hardening. Re-run the pause/unpause and safe-exit tests.

## Failure modes

- **Admin key unavailable:** pause cannot be triggered. Ensure the admin
  key is held in a recoverable, monitored custody setup.
- **Pause blocks withdrawals:** critical regression — roll back the change
  that added the pause check to safe-exit paths.
- **Repeated pause/unpause reverts:** violates idempotency; fix the guard
  so repeated calls are no-ops.
- **Missing events:** state changes are unauditable; ensure both transitions
  emit events before relying on the pause in production.

## Tests

Pause and recovery behavior is protected by tests covering:

- admin-only pause/unpause authorization,
- risk-bearing entry points reverting while paused,
- safe-exit paths succeeding while paused,
- idempotent repeated pause/unpause calls,
- emitted `Paused` / `Unpaused` events.
