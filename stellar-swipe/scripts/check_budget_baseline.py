#!/usr/bin/env python3
"""
CI budget regression gate.

Parses cargo test output for lines of the form:
  BUDGET_METRIC: <contract>.<entrypoint>=<instructions>

Compares each measured value against the committed baseline in
baselines/instruction_budget_baseline.json.  Exits non-zero (fails CI) when
any entrypoint exceeds baseline * (1 + threshold_pct / 100).

Usage:
  cargo test --workspace 2>&1 | python3 scripts/check_budget_baseline.py

To update the baseline after an intentional change:
  python3 scripts/check_budget_baseline.py --update
  git add baselines/instruction_budget_baseline.json
  git commit -m "chore: update instruction budget baseline"
"""

import json
import re
import sys
import os
import pathlib

REPO_ROOT = pathlib.Path(__file__).parent.parent
BASELINE_FILE = REPO_ROOT / "baselines" / "instruction_budget_baseline.json"

METRIC_RE = re.compile(r"BUDGET_METRIC:\s+(\w+)\.(\w+)=(\d+)")


def load_baseline():
    with open(BASELINE_FILE) as f:
        data = json.load(f)
    threshold_pct = data.get("_threshold_pct", 10)
    return data, threshold_pct


def check(lines, update=False):
    baseline, threshold_pct = load_baseline()
    measurements = {}
    for line in lines:
        m = METRIC_RE.search(line)
        if m:
            contract, entrypoint, cost = m.group(1), m.group(2), int(m.group(3))
            measurements.setdefault(contract, {})[entrypoint] = cost

    failures = []
    for contract, eps in measurements.items():
        for ep, actual in eps.items():
            base = baseline.get(contract, {}).get(ep)
            if base is None:
                print(f"  [NEW] {contract}.{ep}: {actual} instructions (no baseline yet)")
                if update:
                    baseline.setdefault(contract, {})[ep] = actual
                continue
            limit = base + base * threshold_pct // 100
            status = "OK" if actual <= limit else "FAIL"
            print(f"  [{status}] {contract}.{ep}: {actual} vs baseline {base} (limit {limit})")
            if actual > limit:
                failures.append((contract, ep, actual, limit))
            if update:
                baseline[contract][ep] = actual

    if update:
        # Strip metadata keys before writing
        out = {k: v for k, v in baseline.items() if not k.startswith("_")}
        out["_comment"] = baseline.get("_comment", "")
        out["_threshold_pct"] = threshold_pct
        with open(BASELINE_FILE, "w") as f:
            json.dump(out, f, indent=2)
        print(f"\nBaseline updated: {BASELINE_FILE}")

    if failures:
        print("\nBUDGET REGRESSIONS DETECTED:")
        for contract, ep, actual, limit in failures:
            print(f"  {contract}.{ep}: {actual} > {limit}")
        print("\nTo accept these regressions, run:")
        print("  python3 scripts/check_budget_baseline.py --update")
        print("  git add baselines/instruction_budget_baseline.json")
        sys.exit(1)
    elif measurements:
        print(f"\nAll {sum(len(v) for v in measurements.values())} entrypoints within budget.")
    else:
        print("No BUDGET_METRIC lines found in input — ensure tests emit them.")


if __name__ == "__main__":
    update = "--update" in sys.argv
    lines = sys.stdin.read().splitlines()
    check(lines, update=update)
