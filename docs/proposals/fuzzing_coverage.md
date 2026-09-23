# Fuzzing Coverage for Contracts

## Problem
Critical contracts should be robust to unexpected inputs, but current tests
may not cover the full range of adversarial or edge-case conditions.

## Proposed Solution
- Add `cargo-fuzz` (libFuzzer) targets under `fuzz/fuzz_targets/` for each
  contract's public entrypoints (stake_vault, fee_collector, signal_registry).
- Seed corpora from existing unit/integration test fixtures.
- Fuzz targets to cover: malformed instruction args, boundary numeric values
  (0, i128::MAX/MIN), out-of-order call sequences, and storage state
  transitions reached via random call sequences (stateful fuzzing).
- Wire a `fuzz.yml` GitHub Actions workflow that runs each target for a fixed
  short duration (e.g. 60s) per PR, and a longer nightly run (e.g. 30m).
- Track and triage crashes/regressions via a `fuzz/corpus` artifact + issue
  template referencing the failing input.

## Rollout
1. Scaffold `fuzz/` crate + one target per contract (no CI yet).
2. Add PR-time short fuzz run as a required check.
3. Add nightly extended run with artifact upload on crash.
