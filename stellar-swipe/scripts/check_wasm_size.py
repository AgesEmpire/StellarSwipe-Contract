#!/usr/bin/env python3
"""
check_wasm_size.py — WASM size regression gate for Soroban contracts.

Compares the byte size of every optimized *.wasm artifact against a committed
baseline.  Fails CI when any contract has grown beyond the allowed threshold,
catching accidental dependency additions, un-optimised codegen, or LTO
regressions before they reach production.

Usage:
    # CI (pipe wasm dir, read baseline, exit 1 on regression):
    python3 stellar-swipe/scripts/check_wasm_size.py

    # Update the baseline after an intentional size change:
    python3 stellar-swipe/scripts/check_wasm_size.py --update

    # Point at a non-default wasm directory:
    python3 stellar-swipe/scripts/check_wasm_size.py --wasm-dir path/to/wasm

Exit codes:
    0  All contracts are within the allowed threshold.
    1  One or more contracts exceed their baseline size limit.
    2  New contracts found (baseline updated); commit the JSON file.

Threshold:
    Defaults to 10% headroom above the committed baseline size.  Override with
    _threshold_pct in the baseline JSON or --threshold-pct on the CLI.

Baseline file:
    stellar-swipe/baselines/wasm_size_baseline.json

    Format:
    {
      "_comment": "...",
      "_threshold_pct": 10,
      "<contract-name>": <baseline_bytes>
    }

Updating the baseline:
    Run with --update.  The script will overwrite baselines/wasm_size_baseline.json
    with the current measurements.  Review the diff, then commit:

        python3 stellar-swipe/scripts/check_wasm_size.py --update
        git add stellar-swipe/baselines/wasm_size_baseline.json
        git commit -m "chore: update WASM size baseline — <reason>"
"""

import argparse
import json
import sys
from pathlib import Path

WORKSPACE_ROOT = Path(__file__).resolve().parent.parent
WASM_DIR_DEFAULT = WORKSPACE_ROOT / "target" / "wasm-optimized"
BASELINE_FILE = WORKSPACE_ROOT / "baselines" / "wasm_size_baseline.json"


# ── Helpers ────────────────────────────────────────────────────────────────────

def _human(n: int) -> str:
    """Format bytes as a human-readable string (e.g. '123.4 KB')."""
    if n < 1024:
        return f"{n} B"
    elif n < 1024 ** 2:
        return f"{n / 1024:.1f} KB"
    else:
        return f"{n / 1024 ** 2:.1f} MB"


def load_baseline() -> tuple[dict, int]:
    """Return (baseline_dict, threshold_pct).  Creates an empty baseline if missing."""
    if not BASELINE_FILE.exists():
        return {}, 10
    data = json.loads(BASELINE_FILE.read_text())
    threshold_pct = int(data.get("_threshold_pct", 10))
    return data, threshold_pct


def save_baseline(data: dict, threshold_pct: int) -> None:
    out = {k: v for k, v in data.items() if not k.startswith("_")}
    out["_comment"] = (
        "Baseline WASM byte sizes for optimized Soroban contracts. "
        "Update via: python3 stellar-swipe/scripts/check_wasm_size.py --update"
    )
    out["_threshold_pct"] = threshold_pct
    BASELINE_FILE.write_text(json.dumps(out, indent=2, sort_keys=True) + "\n")


def measure_wasm(wasm_dir: Path) -> dict[str, int]:
    """Return {contract_name: byte_size} for every *.wasm in wasm_dir."""
    result: dict[str, int] = {}
    for wasm_file in sorted(wasm_dir.glob("*.wasm")):
        name = wasm_file.stem
        result[name] = wasm_file.stat().st_size
    return result


# ── Main ──────────────────────────────────────────────────────────────────────

def main() -> int:
    parser = argparse.ArgumentParser(description="WASM size regression gate.")
    parser.add_argument(
        "--wasm-dir",
        default=str(WASM_DIR_DEFAULT),
        help="Directory containing optimized *.wasm files (default: target/wasm-optimized/)",
    )
    parser.add_argument(
        "--update",
        action="store_true",
        help="Update the baseline with current measurements and exit.",
    )
    parser.add_argument(
        "--threshold-pct",
        type=int,
        default=None,
        help="Allowed growth percentage above baseline (default: from baseline JSON or 10).",
    )
    args = parser.parse_args()

    wasm_dir = Path(args.wasm_dir)
    if not wasm_dir.is_dir():
        print(
            f"WASM directory not found: {wasm_dir}\n"
            "  Build optimized WASM first: cd stellar-swipe && ./scripts/build.sh",
            file=sys.stderr,
        )
        return 1

    baseline, threshold_pct = load_baseline()
    if args.threshold_pct is not None:
        threshold_pct = args.threshold_pct

    measurements = measure_wasm(wasm_dir)
    if not measurements:
        print(
            f"No *.wasm files found in {wasm_dir}.\n"
            "  Build optimized WASM first: cd stellar-swipe && ./scripts/build.sh",
            file=sys.stderr,
        )
        return 1

    if args.update:
        for name, size in measurements.items():
            baseline[name] = size
        save_baseline(baseline, threshold_pct)
        print(f"Baseline updated: {BASELINE_FILE}")
        print(f"  {len(measurements)} contract(s) recorded.")
        for name, size in sorted(measurements.items()):
            print(f"    {name}: {_human(size)}")
        return 0

    # ── Compare ───────────────────────────────────────────────────────────────
    failures: list[tuple[str, int, int, int]] = []  # (name, actual, baseline, limit)
    new_contracts: list[str] = []

    print(f"Checking WASM sizes (threshold: +{threshold_pct}%):\n")
    header = f"  {'Contract':<40} {'Actual':>10} {'Baseline':>10} {'Limit':>10}  Status"
    print(header)
    print("  " + "-" * (len(header) - 2))

    for name, actual in sorted(measurements.items()):
        base = baseline.get(name)
        if base is None:
            new_contracts.append(name)
            print(f"  {name:<40} {_human(actual):>10} {'—':>10} {'—':>10}  [NEW]")
            if args.update:
                baseline[name] = actual
            continue

        limit = base + base * threshold_pct // 100
        status = "OK" if actual <= limit else "FAIL"
        print(
            f"  {name:<40} {_human(actual):>10} {_human(base):>10} {_human(limit):>10}  [{status}]"
        )
        if actual > limit:
            failures.append((name, actual, base, limit))

    print()

    # ── Results ───────────────────────────────────────────────────────────────
    if failures:
        print("WASM SIZE REGRESSIONS DETECTED:")
        for name, actual, base, limit in failures:
            delta = actual - base
            print(f"  {name}: {_human(actual)} (baseline {_human(base)}, "
                  f"limit {_human(limit)}, over by {_human(delta)})")
        print()
        print("To accept these regressions after review:")
        print("  python3 stellar-swipe/scripts/check_wasm_size.py --update")
        print("  git add stellar-swipe/baselines/wasm_size_baseline.json")
        print("  git commit -m 'chore: update WASM size baseline — <reason>'")
        return 1

    if new_contracts:
        for name in new_contracts:
            baseline[name] = measurements[name]
        save_baseline(baseline, threshold_pct)
        print(f"INFO: {len(new_contracts)} new contract(s) added to baseline.")
        print("Please commit the updated stellar-swipe/baselines/wasm_size_baseline.json.")
        return 2

    total = sum(measurements.values())
    print(
        f"All {len(measurements)} contract(s) within size budget "
        f"(total optimized: {_human(total)})."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
