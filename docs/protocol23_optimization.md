# Protocol 23 Parallelization Optimization

This document audits the StellarSwipe contract storage design for Stellar Protocol 23 parallel transaction execution.

## Goals

- Reduce storage contention across concurrent transactions.
- Prefer per-user / per-position keys over global counters and shared state.
- Document which operations can safely run in parallel.

## Findings

### Key storage access patterns

- `UserPortfolio` uses per-position keys (`Position(id)`) and per-user lists (`UserPositions(user)`).
- `TradeExecutor` validates and records position usage through `UserPortfolio`.
- `SignalRegistry` stores signal metadata and expiry information per signal id.

### Parallel-friendly patterns

- Read-only queries such as `get_price`, `check_subscription`, `has_position`, and active signal listings are safe for parallel execution.
- Per-user position creation and query operations can run in parallel when they only touch a single user's storage namespace.
- Per-signal expiry checks and cleanup can be batched without global state conflicts.

### Potential contention points

- Global counters such as `NextPositionId` and `NextSignalId` serialize access.
- `UserPortfolio` open and close functions currently update both the position object and user position lists.
- `SignalRegistry` cleanup operations that mutate shared active/expired lists can conflict with concurrent signal creation.

## Recommended optimizations

- Replace a single `NextPositionId` counter with user-scoped or contract-scoped segment counters to reduce write contention.
- Store user position indices in append-only per-user maps rather than using shared `Vec` lists where possible.
- Use immutable signal metadata keys and tombstones to avoid large shared list rewrites during expiry cleanup.

## Parallel execution classification

### Safe for parallel execution

- `get_price`
- `check_subscription`
- `has_position`
- `get_active_signals`
- `get_signal_for_viewer`

### Requires serialization or locking

- `open_position`
- `close_position`
- `execute_copy_trade`
- `cleanup_expired_signals`

## Next steps

1. Measure testnet throughput before and after storage key refactoring.
2. Convert shared counters to per-user or shard-based key schemes.
3. Add benchmark flows that simulate high parallel load on opening and closing positions.
