# WASM Size and Instruction Budget Regression Benchmarks

Contract size and instruction budgets are hard limits on Soroban: a contract
that exceeds the upload size limit cannot be deployed, and one that burns too
many CPU instructions will fail at runtime. This document explains how both
types of regression are detected automatically in CI.

---

## WASM size regression gate

### What it checks

After every optimized WASM build, CI runs `scripts/check_wasm_size.py` against
`target/wasm-optimized/`. It compares the byte size of each `*.wasm` against
the committed baseline in `baselines/wasm_size_baseline.json` and fails if any
contract has grown more than the allowed threshold (default: **10%**).

### Baseline file

```
stellar-swipe/baselines/wasm_size_baseline.json
```

```json
{
  "_comment": "Baseline WASM byte sizes for optimized Soroban contracts.",
  "_threshold_pct": 10,
  "signal_registry": 286720,
  "auto_trade": 196608,
  ...
}
```

The `_threshold_pct` field controls how much growth is allowed before CI fails.
All sizes are in bytes and represent the `stellar contract optimize` output
(wasm-opt pipeline applied on top of `--release --opt-level=z --lto`).

### CI output format

```
Checking WASM sizes (threshold: +10%):

  Contract                                  Actual   Baseline      Limit  Status
  ─────────────────────────────────────────────────────────────────────────────
  auto_trade                               192.0 KB   192.0 KB   211.2 KB  [OK]
  fee_collector                            120.0 KB   120.0 KB   132.0 KB  [OK]
  signal_registry                          282.0 KB   280.0 KB   308.0 KB  [OK]

All 10 contract(s) within size budget (total optimized: 1.2 MB).
```

When a regression is detected:

```
  auto_trade                               215.0 KB   192.0 KB   211.2 KB  [FAIL]

WASM SIZE REGRESSIONS DETECTED:
  auto_trade: 215.0 KB (baseline 192.0 KB, limit 211.2 KB, over by 23.0 KB)
```

### Updating the baseline intentionally

When a size increase is justified (new feature, dependency upgrade, etc.):

```bash
cd stellar-swipe

# 1. Build the optimized WASM
./scripts/build.sh

# 2. Update the baseline with current measurements
python3 scripts/check_wasm_size.py --update
# or use the convenience wrapper:
./scripts/update_wasm_size_baseline.sh

# 3. Review the diff
git diff baselines/wasm_size_baseline.json

# 4. Commit with a clear reason
git add baselines/wasm_size_baseline.json
git commit -m "chore: update WASM size baseline — added batch settlement feature to trade_executor"
```

The commit message should explain *why* the size changed. Reviewers will use
this to evaluate whether the growth is proportionate to the feature added.

### Adjusting the threshold

If the default 10% is too strict or too loose for a specific project phase,
edit `_threshold_pct` in the baseline JSON:

```json
{
  "_threshold_pct": 15,
  ...
}
```

---

## Instruction budget regression gate

### What it checks

`scripts/check_budget_baseline.py` parses test output for lines of the form:

```
BUDGET_METRIC: <contract>.<entrypoint>=<instructions>
```

and compares each measurement against `baselines/instruction_budget_baseline.json`.
CI fails when an entrypoint exceeds `baseline * (1 + threshold_pct / 100)`.

### Emitting budget metrics from tests

Add a line to any Soroban unit test after calling a contract entrypoint:

```rust
println!(
    "BUDGET_METRIC: signal_registry.submit_signal={}",
    env.budget().cpu_instruction_count()
);
```

The test output is piped into `check_budget_baseline.py` during CI:

```yaml
- name: Tests
  run: cd stellar-swipe && cargo test --workspace --all-targets 2>&1 \
       | tee /tmp/test_output.txt | python3 scripts/check_budget_baseline.py
```

### Baseline file

```
stellar-swipe/baselines/instruction_budget_baseline.json
```

```json
{
  "_comment": "Baseline CPU instruction counts for key contract entrypoints.",
  "_threshold_pct": 10,
  "signal_registry": {
    "submit_signal": 15000000
  }
}
```

### Updating the baseline

```bash
cd stellar-swipe
cargo test --workspace --all-targets 2>&1 \
  | python3 scripts/check_budget_baseline.py --update

git add baselines/instruction_budget_baseline.json
git commit -m "chore: update instruction budget baseline — <reason>"
```

---

## Soroban limits reference

| Limit | Value |
|---|---|
| Max WASM upload size | 128 KB (after stellar contract optimize) |
| Max CPU instructions per TX | 100,000,000 |
| Max memory | 40 MB |

The WASM size gate fires well before the Soroban upload limit is reached,
giving developers early warning while there is still room to optimize.

See also: [Soroban resource limits](https://developers.stellar.org/docs/networks/resource-limits-fees)
