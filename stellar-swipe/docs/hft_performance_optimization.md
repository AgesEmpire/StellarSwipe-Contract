# HFT Performance Optimization Guide

## Overview

StellarSwipe core trading contracts are optimized for high-frequency copy-trade scenarios under Soroban's 100M CPU instruction budget per transaction. This document describes profiling methodology, optimizations applied, and regression guardrails.

---

## Profiling Methodology

### Instruction counting (primary metric)

In Soroban tests, read cumulative CPU cost after each top-level invocation:

```rust
let instructions = env.cost_estimate().budget().cpu_instruction_cost();
```

The budget auto-resets before each top-level contract call, so this approximates per-operation cost.

### Baseline constants (`stellar_swipe_common::perf`)

| Constant | Purpose |
|---|---|
| `DEFAULT_INSTRUCTION_BUDGET` | 100M Soroban default |
| `REGRESSION_BUDGET_PCT` | 80% — CI failure threshold |
| `BASELINE_COPY_TRADE_INSTRUCTIONS` | ~8M target for trade_executor |
| `BASELINE_AUTO_TRADE_INSTRUCTIONS` | ~12M target for auto_trade |
| `BASELINE_FEE_COLLECT_INSTRUCTIONS` | ~5M target for fee_collector |

### Load tests

| Test | Location | What it validates |
|---|---|---|
| 1,000 sequential trades | `auto_trade/tests/load/test_high_volume.rs` | Throughput + max per-trade budget |
| Copy-trade regression | `trade_executor/src/tests/test_latency_benchmark.rs` | Single + batch latency |
| Auto-trade regression | `auto_trade/tests/load/test_perf_regression.rs` | Hot path + rate-limit bookkeeping |

---

## Optimizations Applied

### 1. Transaction-scoped caching (`common/src/perf.rs`)

`tx_cache_or_compute` stores computed values in **temporary storage** (auto-evicted after the transaction):

- Fee config in `fee_collector` (`fee_cache.rs`)
- Reusable across multiple `collect_fee` / `batch_collect_fees` calls in one tx

### 2. O(1) rate limiting (`common/src/rate_limit.rs`)

Replaced timestamp-vector pruning with counter windows:

```
RateLimitWindow { window_start, count }
```

Legacy timestamp vectors are migrated on first read. Reduces per-action cost from O(n) to O(1).

### 3. Batch execution hoisting (`trade_executor`)

`batch_execute` loads once per batch:

- User portfolio address
- Estimated fee
- Daily volume limit
- Circuit breaker state

Passed via `BatchExecutionContext` to each trade, avoiding repeated instance storage reads.

### 4. Fee collection batching (`fee_collector`)

`batch_collect_fees(items)` — up to 20 trades per call with shared fee config cache and single token client reuse per item.

### 5. Provider active signal cache (`signal_registry`)

`ProviderCacheKey::ActiveSignalCount` — O(1) signal limit checks instead of full-map scans on every submission.

### 6. Auto-trade hot path

- **Conditional logging**: `is_info_logging_enabled()` gates string allocation on `execute_trade`
- **Rate limit bookkeeping**: `record_transfer` called after successful fills (fixes check-only bug)

---

## Operation Batching API

| Contract | Function | Max batch | Amortized reads |
|---|---|---|---|
| trade_executor | `batch_execute` | 10 | Portfolio, fee, CB, daily limit |
| fee_collector | `batch_collect_fees` | 20 | Fee optimization config, burn rate |

---

## Production Recommendations

1. **Set log level to `Warn`** on auto_trade in production (`set_log_level`) to skip Info hot-path overhead entirely.
2. **Use `batch_execute`** for keeper-driven copy trades targeting the same portfolio contract.
3. **Use `batch_collect_fees`** when settling multiple trades in one settlement transaction.
4. **Monitor instruction usage** on testnet with realistic storage footprint before mainnet — debug-mode test counts are directional, not absolute.
5. **Shard signal storage** (future): per-signal persistent keys remain the largest scale opportunity for signal_registry.

---

## Running Benchmarks

```bash
cd stellar-swipe

# Core perf module
cargo test -p stellar_swipe_common perf

# Rate limit regression
cargo test -p stellar_swipe_common rate_limit

# Trade executor latency
cargo test -p trade_executor test_latency

# Auto-trade load + regression (requires testutils)
cargo test -p auto_trade --features testutils test_perf_regression
cargo test -p auto_trade --features testutils test_1000_sequential
```

---

## Regression Policy

CI fails when any hot-path operation exceeds:

- **80%** of the 100M instruction budget (`regression_budget_limit()`)
- **3×** the documented baseline constant for that operation

Update baselines only when a deliberate optimization changes expected costs — document the change in this file.
