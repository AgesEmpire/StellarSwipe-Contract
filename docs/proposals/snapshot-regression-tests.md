# Snapshot-Based Regression Tests & Invariant Checks

## Problem
The repository has extensive tests, but lacks automated snapshot/invariant
checks that catch subtle state drift (e.g. accidental storage-layout changes,
unexpected balance/reward drift) across releases.

## Proposed Solution
- Add a `tests/regression/snapshots/` convention: serialize key contract state
  (storage layout hashes, totals, invariant values) to JSON baseline files
  after a canonical sequence of operations.
- Add an `invariants.rs` helper module per contract exposing pure functions
  such as `total_staked_equals_sum_of_positions()`,
  `fee_pool_never_negative()`, `reward_debt_matches_accrued()`.
- A regression test runs the canonical scenario, computes the same invariants
  and state hash, and diffs against the stored snapshot — failing loudly with
  a readable diff if state drifted unexpectedly.
- Snapshot files are checked into `stellar-swipe/storage-snapshots/` (existing
  directory) and `stellar-swipe/baselines/` (existing directory) to reuse
  current conventions.

## Benefits
- Storage-layout changes that silently break upgrades are caught immediately.
- Invariant violations (double counting, negative balances, drift between
  staking and signal registry reputation) surface as explicit test failures
  instead of being discovered in production.

## Next Steps
- Enumerate core invariants per contract (stake_vault, signal_registry,
  fee_collector, analytics).
- Add a snapshot capture script under `scripts/` to regenerate baselines when
  a change is intentional.
- Wire the new regression suite into CI alongside existing tests.
