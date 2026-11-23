# Per-entrypoint Soroban resource ceilings

Issue #1402. This page lists the CPU-instruction ceilings that are enforced today, where each one is checked, and which recorded baselines are not enforced.

## How a ceiling is defined and enforced

- **Ceiling = baseline + 10%**, computed as `baseline + baseline * 10 / 100` (integer math). The constant is `REGRESSION_THRESHOLD_PCT` in `contracts/common/src/budget_regression.rs`.
- **Fixtures are fixed.** Each measuring test uses constant inputs, so the same run gives the same instruction count.
- **Enforcement is in `cargo test`.** `budget_regression::measure_and_emit` asserts inside the test, so `cargo test --workspace --all-targets` (the CI "Tests" step in `.github/workflows/ci.yml`) fails when a ceiling is exceeded.
- **Changing a ceiling** needs an intentional cost increase, a new baseline constant in the test, and the reason in the PR. Do not raise the 10% threshold to get a red build green.

## Enforced CPU-instruction ceilings

| Entrypoint | Baseline | Ceiling | Measured in |
|---|---:|---:|---|
| `user_portfolio.record_trade` | 5,000,000 | 5,500,000 | `contracts/user_portfolio/src/tests/test_pnl.rs` |
| `oracle.set_price` | 3,000,000 | 3,300,000 | `contracts/oracle/src/test.rs` |
| `oracle.get_price` | 2,000,000 | 2,200,000 | `contracts/oracle/src/test.rs` |
| `stake_vault.deposit_stake` | 6,000,000 | 6,600,000 | `contracts/stake_vault/src/tests.rs` |
| `stake_vault.withdraw_stake` | 6,000,000 | 6,600,000 | `contracts/stake_vault/src/tests.rs` |
| `auto_trade.execute_auto_trade` | 12,000,000 | 13,200,000 | `contracts/auto_trade/tests/load/test_perf_regression.rs` |
| `trade_executor.execute_copy_trade` | 8,000,000 | 8,800,000 | `contracts/trade_executor/src/tests/test_latency_benchmark.rs` |
| `fee_collector.collect_fee` | 5,500,000 | 6,050,000 | `contracts/fee_collector/src/test.rs` |
| `signal_registry.submit_signal` | 15,000,000 | 16,500,000 | `contracts/signal_registry/src/test.rs` |
| `signal_registry_create_adopt_execute_workflow` | 15,000,000 | 16,500,000 | `contracts/integration_tests/tests/integration/test_e2e_resource_budget.rs` |
| `signal_registry_creation_burst_storage_workflow` (25 signals) | 20,000,000 | 22,000,000 | `contracts/integration_tests/tests/integration/test_e2e_resource_budget.rs` |

## Global per-transaction caps

`contracts/common/src/resource_budget.rs` defines two practical caps:

- CPU: `MAX_CPU_INSTRUCTIONS` = 100,000,000 instructions
- Memory: `MAX_MEMORY_BYTES` = 41,943,040 bytes (40 MiB)

These are coded as practical limits. They have not been checked against current network settings here. `resource_budget::assert_within_budget` enforces them, but no test currently passes live measurements into it, so these caps are not checked against real entrypoints yet.

## Recorded but not enforced

These entries exist in the baseline files without a test that measures them:

- `baselines/instruction_budget_baseline.json`:
  - `signal_registry`: `get_active_signals_50`, `update_leaderboard_index`, `get_provider_leaderboard`
  - `auto_trade.batch_execute`
  - `trade_executor`: `execute_trade`, `batch_execute`
  - `fee_collector.distribute_fees`
  - `governance`: `propose`, `vote`, `finalize_proposal`
  - `bridge`: `initiate_transfer`, `finalize_transfer`
  - `user_portfolio.get_positions`
  - `analytics.emit_weekly_health_report`
  - `stake_vault.stake` and `stake_vault.unstake`. The tests use the names `deposit_stake` and `withdraw_stake`, so the names have drifted.
- `baselines/resource_budget_baseline.json` (`CORE_FLOW_BASELINES` in `resource_budget.rs`): the CPU, memory, storage and host-call values are not compared against live runs.

`scripts/check_budget_baseline.py` and `scripts/update_budget_baseline.sh` read `BUDGET_METRIC:` lines, but CI does not call them. Without `--nocapture`, cargo hides the output of passing tests, so the script may receive no lines. Neither was run while writing this page.

## Reproduce

```bash
cd stellar-swipe
cargo test --workspace --all-targets              # enforces every ceiling in the table above
cargo test --workspace --all-targets -- --nocapture 2>&1 \
  | python3 scripts/check_budget_baseline.py      # prints measured vs. limit per measured entrypoint
```
