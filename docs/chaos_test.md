# Chaos Test — Randomised Contract Call Ordering

**Issue:** #681  
**Location:** `stellar-swipe/contracts/integration_tests/tests/integration/test_chaos_ordering.rs`

## Purpose

Deterministic integration tests exercise expected, ordered call sequences.
The chaos test issues a *randomised* mix of valid cross-contract operations so
that ordering-dependent bugs (race conditions, state-machine invariant breaks,
incorrect accounting carry-over) are exposed.

## What Is Tested

| Operation | Contract | Invariant checked |
|-----------|----------|-------------------|
| `deposit_stake` | StakeVault | Per-staker balance increases by amount |
| `withdraw_stake` | StakeVault | Balance zeroed; failures tolerated |
| `open_position` | UserPortfolio | Position ID registered |
| `close_position` | UserPortfolio | Position ID removed |
| `FeeTick` | stub | No panic |
| `SignalTick` | stub | No panic |

After **every** operation the test asserts:

1. **Stake sum invariant** — the sum of individual staker balances returned by
   `get_stake` equals the running total tracked by the test harness.
2. **Non-negativity** — no individual balance or position count can go below zero.

## PRNG

A standard 64-bit **Linear Congruential Generator** (LCG) with Knuth's MMIX
parameters provides reproducible pseudo-random operation sequences:

```
state = state * 6_364_136_223_846_793_005 + 1_442_695_040_888_963_407  (mod 2^64)
```

The LCG is seeded once per test run and then used to:
- Pick which operation to execute (weighted by type frequency).
- Pick which staker / user is involved.

## Running the Tests

### Default seed (42)

```sh
cd stellar-swipe
cargo test --test test_chaos_ordering
```

### All chaos seeds

```sh
cargo test --test test_chaos_ordering -- --nocapture
```

### Custom seed (ad-hoc exploration)

```sh
CHAOS_SEED=98765 cargo test --test test_chaos_ordering chaos_env_seed -- --nocapture
```

### Reproducing a specific failure

When a CI run fails the seed is printed to stdout:

```
chaos seed: 98765
  step   0: Stake { staker_idx: 1, amount: 500000000 } — OK
  ...
FAILED: stake sum mismatch
```

Pin the seed as a regression test:

```sh
CHAOS_SEED=98765 cargo test --test test_chaos_ordering chaos_env_seed
```

Then add a dedicated `#[test]` in `test_chaos_ordering.rs`:

```rust
#[test]
fn chaos_seed_98765_regression() {
    run_chaos(98765);
}
```

## Built-in Regression Seeds

| Test name | Seed | Description |
|-----------|------|-------------|
| `chaos_default_seed` | 42 | Standard smoke run |
| `chaos_seed_137` | 137 | Alternate ordering |
| `chaos_seed_9999` | 9 999 | Heavy stake/unstake interleaving |
| `chaos_env_seed` | `$CHAOS_SEED` | Ad-hoc / CI reproduction |

## Adding New Operations

1. Add a new variant to the `Op` enum.
2. Add a weight to `OP_WEIGHTS` (must stay in sync with variant count).
3. Add the execution arm in `run_chaos`'s match block.
4. Add any new invariant checks to `assert_invariants`.
