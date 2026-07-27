# Batched Processing for Fee Collection, Reward Settlement & Analytics

## Problem
Fee collection, reward settlement, and analytics updates are currently performed
as independent per-event operations. Under heavier traffic this multiplies
transaction overhead and hurts throughput.

## Proposed Solution
Introduce a batching layer that accumulates pending operations and flushes them
in a single transaction once a batch size or time threshold is reached:

- `BatchQueue` storage entry per subsystem (fees, rewards, analytics) holding
  pending entries + a `last_flush_ledger` timestamp.
- `queue_fee_collection(...)`, `queue_reward_settlement(...)`,
  `queue_analytics_update(...)` entrypoints that append to the queue instead of
  executing immediately.
- `flush_batch(subsystem)` entrypoint (permissionless or keeper-triggered) that
  processes all queued entries in one pass, emitting a single aggregated event
  instead of one event per operation.
- Configurable thresholds (`max_batch_size`, `max_batch_age_ledgers`) stored in
  contract config so batch aggressiveness can be tuned without redeploying.

## Benefits
- Amortizes fixed per-transaction overhead (base fee, storage read/write) across
  many operations.
- Reduces event log noise via aggregated batch events.
- Improves throughput under load by decoupling "record intent" from "settle".

## Next Steps
- Define `BatchQueue` data types in `contracts/common`.
- Wire queue/flush entrypoints into `fee_collector`, `stake_vault` (rewards),
  and `analytics` contracts.
- Add regression tests comparing batched vs. immediate settlement outcomes.
