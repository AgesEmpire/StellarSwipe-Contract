# Security Best Practices

This document captures security-relevant invariants and operational rules for the
contracts in this repository. It is intended for maintainers, auditors, and
integrators.

## Governance Timelock: Cancellation Rules

Queued governance timelock actions may be cancelled before they are executed.
Cancellation is a safety valve: it lets governance withdraw a proposal that was
queued in error, became unsafe, or was superseded. Because cancellation changes
the lifecycle of a privileged action, the rules below are normative.

### Cancellation authority

- **Governance only.** Only the governance authority (the account/contract that
  owns the timelock, e.g. the governance module or its designated admin) may
  cancel a queued action. Arbitrary callers, including the original proposer,
  must not be able to cancel unless they are the governance authority.
- **Authorization is checked on every call.** The cancellation entry point must
  verify the caller against the stored governance authority and reject any
  unauthorized caller. Authorization must not be inferred from the proposal
  contents or from prior state.
- **No self-cancellation bypass.** A queued action must not be able to cancel
  itself or another action through an execution path that skips the authority
  check.

### Allowed lifecycle states

A queued action moves through the following states:

1. `Queued` — the action has been scheduled and is waiting for its timelock
   delay to elapse. **Cancellation is allowed.**
2. `Ready` — the timelock delay has elapsed and the action is executable.
   **Cancellation is allowed** while the action has not yet been executed.
3. `Executed` — the action has been executed. **Cancellation is not allowed.**
   The action is terminal.
4. `Cancelled` — the action has already been cancelled. **Cancellation is not
   allowed** (idempotent no-op or explicit rejection). The action is terminal.

Cancellation must be rejected for any state other than `Queued` or `Ready`.

### Invariants

- **Cancelled actions can never be executed.** The execution path must reject
  any action whose state is `Cancelled`. Cancellation is terminal: once an
  action is cancelled, no later call may execute it, even after the timelock
  delay has elapsed.
- **Cancellation cannot corrupt timelock state.** Cancelling an action must
  only transition that action's state to `Cancelled`; it must not mutate the
  timelock delay, the governance authority, or the state of other queued
  actions.
- **State transitions are one-way.** `Queued`/`Ready` → `Cancelled` and
  `Queued`/`Ready` → `Executed` are the only terminal transitions. A terminal
  action (`Executed` or `Cancelled`) must never transition again.
- **Execution and cancellation are mutually exclusive.** If an action has been
  executed, cancellation must fail; if an action has been cancelled, execution
  must fail. There must be no ordering of calls that both executes and cancels
  the same action.

### Required test coverage

Tests must cover, at minimum:

- Authorized cancellation of an action in the `Queued` state.
- Authorized cancellation of an action in the `Ready` state.
- Unauthorized cancellation attempts in the `Queued` and `Ready` states, which
  must be rejected.
- Cancellation attempts against an `Executed` action, which must be rejected.
- Cancellation attempts against an already `Cancelled` action, which must be
  rejected (or be a no-op) without changing state.
- Execution attempts against a `Cancelled` action, which must be rejected.
- A regression test asserting that a cancelled action cannot be executed even
  after its timelock delay has elapsed.
