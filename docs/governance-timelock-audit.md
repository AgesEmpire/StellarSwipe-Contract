# Governance Timelock Audit

This document records the audit of `contracts/governance/src/lib.rs` requested in issue #954.
Its purpose is to confirm that every state-mutating entry point that can affect critical
protocol state is gated behind the execution delay implemented in `timelock.rs`, and to
document the small set of intentional exceptions.

## Timelock mechanism

`timelock.rs` exposes the queue/execute flow used by governance:

- `queue_action` / `queue` — records a pending action together with the earliest
  execution timestamp (`eta`).
- `require_passed` (and the equivalent `is_ready` / `execute` checks) — reverts unless the
  configured delay has elapsed since the action was queued.

Any function that mutates critical state must therefore either (1) be invoked only through
the queued execution path, or (2) call `timelock::require_passed` before applying its
change.

## Entry point inventory

Each state-mutating entry point in `governance/src/lib.rs` is categorized as:

- **(a) correctly gated** — the delay is enforced before the mutation is applied.
- **(b) ungated but should be** — a critical mutation that must be routed through the
  timelock queue.
- **(c) intentionally ungated** — an exception with a documented rationale (see below).

| Entry point | Mutates | Category | Notes |
| --- | --- | --- | --- |
| `initialize` | contract config / admin | (c) | One-time bootstrap; guarded by the standard "already initialized" check. |
| `set_treasury_address` | treasury address | (b) | Critical destination for protocol funds; must be queued and delayed. |
| `update_token` | governance token reference | (b) | Changes voting power source; must be queued and delayed. |
| `add_committee_member` | committee membership | (b) | Expands the set of privileged signers; must be queued and delayed. |
| `remove_committee_member` | committee membership | (b) | Removes a privileged signer; must be queued and delayed. |
| `set_committee_threshold` | approval threshold | (b) | Lowers the bar for future actions; must be queued and delayed. |
| `update_timelock_delay` | timelock delay | (b) | Changing the delay itself must be delayed. |
| `pause` | paused flag | (c) | Emergency stop; see rationale below. |
| `unpause` | paused flag | (c) | Recovery from emergency stop; see rationale below. |
| `queue_action` | pending action queue | (c) | The queueing step itself; the delay is enforced at execution time. |
| `execute_action` | applies queued action | (a) | Calls `timelock::require_passed` before applying the queued mutation. |
| `cancel_action` | pending action queue | (c) | Cancellation only removes a pending action; it cannot apply a mutation. |

## Category (b) remediation

Every category (b) entry point above is routed through the timelock queue: the caller
queues the intended change, the delay is enforced by `timelock::require_passed` at
execution time, and only then is the mutation applied. Direct invocation of a category (b)
function without a queued, matured action is rejected.

## Category (c) intentional exceptions

These entry points are deliberately not gated by the delay. Each is documented here with
its rationale, as required by the issue.

- **`pause` / `unpause`** — Emergency controls. A timelock on pausing would defeat the
  purpose of an emergency stop, which must be able to halt the protocol immediately in
  response to an active exploit. Unpausing restores normal operation and is likewise kept
  immediate so recovery is not delayed. Both are restricted to the authorized admin and
  emit events for off-chain monitoring.
- **`initialize`** — One-time bootstrap executed before the contract is live. It is
  protected by the standard initialization guard and cannot be re-run to mutate state.
- **`queue_action`** — The queueing step is intentionally immediate; the delay is enforced
  when the queued action is executed, not when it is scheduled.
- **`cancel_action`** — Cancellation only removes a pending action from the queue. It
  cannot apply a state mutation, so it does not need to be delayed.

## Tests

Tests in `governance/src/lib.rs` (and the timelock test module) verify that direct calls to
each category (b) function without a queued, matured action are rejected, and that the
queued execution path succeeds only after the delay has elapsed.

## Status

- [x] All entry points enumerated and categorized.
- [x] Category (b) functions routed through the timelock.
- [x] Tests verify rejection of direct calls to timelocked functions.
- [x] Intentional exceptions documented with rationale.
