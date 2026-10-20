#!/usr/bin/env python3
"""
storage_layout_diff.py — Storage layout snapshot and diff tool.

Generates a JSON snapshot of each contract crate's storage keys by parsing
#[contracttype] enum variants under names matching *Key or DataKey.  In PR
review mode it diffs a freshly generated snapshot against the committed
baseline and flags breaking changes (removed or renamed keys).

Usage
-----
  # Generate / update baselines (run locally or in CI on baseline-update branch):
  python3 scripts/storage_layout_diff.py --update

  # Check for breaking changes against the committed baseline (default CI mode):
  python3 scripts/storage_layout_diff.py

  # Check a specific contract:
  python3 scripts/storage_layout_diff.py --contract trade_executor

Exit codes
----------
  0  Clean — no breaking changes detected.
  1  Breaking change — a key was removed or renamed.
  2  New keys added — baseline updated (only in --update mode; commit the JSON).

The baseline files live in stellar-swipe/storage-baselines/<contract>.json.
"""

import argparse
import json
import os
import re
import sys
from pathlib import Path
from typing import Dict, List, Optional, Set

# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------

WORKSPACE_DIR = Path(__file__).resolve().parent.parent
CONTRACTS_DIR = WORKSPACE_DIR / "contracts"
BASELINE_DIR = WORKSPACE_DIR / "storage-baselines"

# Crates to scan.  Add new entries here when a new contract is introduced.
CONTRACT_CRATES: List[str] = [
    "auto_trade",
    "trade_executor",
    "stake_vault",
    "signal_registry",
    "fee_collector",
    "user_portfolio",
    "oracle",
    "governance",
    "analytics",
    "bridge",
    "shared",
    "common",
]

# Regex to find #[contracttype] enums whose names end with "Key" or are "DataKey".
_ENUM_HEADER_RE = re.compile(
    r'(?:^|\n)\s*#\[contracttype\][^\n]*\n\s*(?:#\[[^\]]*\]\s*\n\s*)*'
    r'(?:pub\s+)?enum\s+(\w*[Kk]ey\w*)\s*\{([^}]*)\}',
    re.DOTALL,
)
# Strip comments, attributes, and leading/trailing whitespace from variant text.
_STRIP_COMMENT_RE = re.compile(r'//[^\n]*')
_VARIANT_RE = re.compile(r'^\s*([A-Z][A-Za-z0-9_]*)(\([^)]*\))?\s*(?:=\s*\d+\s*)?[,\n]', re.MULTILINE)


# ---------------------------------------------------------------------------
# Parsing
# ---------------------------------------------------------------------------

def _parse_variants(body: str) -> List[str]:
    """Extract variant names from an enum body string."""
    clean = _STRIP_COMMENT_RE.sub("", body)
    return [m.group(1) for m in _VARIANT_RE.finditer(clean + "\n")]


def extract_storage_layout(crate_name: str) -> Dict[str, List[str]]:
    """
    Walk all .rs source files in a contract crate and extract every
    #[contracttype] enum whose name ends with 'Key'.

    Returns a dict mapping enum_name → sorted list of variant names.
    """
    src_dir = CONTRACTS_DIR / crate_name / "src"
    if not src_dir.is_dir():
        return {}

    layout: Dict[str, List[str]] = {}

    for rs_file in sorted(src_dir.rglob("*.rs")):
        try:
            source = rs_file.read_text(encoding="utf-8")
        except OSError:
            continue

        for m in _ENUM_HEADER_RE.finditer(source):
            enum_name = m.group(1)
            body = m.group(2)
            variants = _parse_variants(body)
            if variants:
                # Merge if the same enum name appears in multiple files
                existing = layout.get(enum_name, [])
                merged = sorted(set(existing) | set(variants))
                layout[enum_name] = merged

    return layout


# ---------------------------------------------------------------------------
# Baseline I/O
# ---------------------------------------------------------------------------

def baseline_path(contract: str) -> Path:
    return BASELINE_DIR / f"{contract}.json"


def load_baseline(contract: str) -> Optional[Dict[str, List[str]]]:
    p = baseline_path(contract)
    if not p.exists():
        return None
    with p.open(encoding="utf-8") as f:
        return json.load(f)


def save_baseline(contract: str, layout: Dict[str, List[str]]) -> None:
    BASELINE_DIR.mkdir(parents=True, exist_ok=True)
    p = baseline_path(contract)
    with p.open("w", encoding="utf-8") as f:
        json.dump(layout, f, indent=2, sort_keys=True)
        f.write("\n")
    print(f"  [saved] {p.relative_to(WORKSPACE_DIR)}")


# ---------------------------------------------------------------------------
# Diff logic
# ---------------------------------------------------------------------------

def diff_layouts(
    contract: str,
    baseline: Dict[str, List[str]],
    current: Dict[str, List[str]],
) -> Dict[str, object]:
    """
    Compare baseline vs current storage layout.

    Returns a report dict with:
      - removed_enums:  enums that existed in baseline but not in current.
      - removed_keys:   per-enum variants that were removed.
      - added_enums:    enums new in current (non-breaking).
      - added_keys:     per-enum variants added in current (non-breaking).
      - breaking:       True if any removal was detected.
    """
    baseline_enums: Set[str] = set(baseline.keys())
    current_enums: Set[str] = set(current.keys())

    removed_enums = sorted(baseline_enums - current_enums)
    added_enums = sorted(current_enums - baseline_enums)
    removed_keys: Dict[str, List[str]] = {}
    added_keys: Dict[str, List[str]] = {}

    for enum_name in baseline_enums & current_enums:
        old_variants = set(baseline[enum_name])
        new_variants = set(current[enum_name])
        removed = sorted(old_variants - new_variants)
        added = sorted(new_variants - old_variants)
        if removed:
            removed_keys[enum_name] = removed
        if added:
            added_keys[enum_name] = added

    breaking = bool(removed_enums or removed_keys)

    return {
        "contract": contract,
        "breaking": breaking,
        "removed_enums": removed_enums,
        "removed_keys": removed_keys,
        "added_enums": added_enums,
        "added_keys": added_keys,
    }


def print_report(report: Dict[str, object]) -> None:
    contract = report["contract"]
    breaking = report["breaking"]

    if not breaking and not report["added_enums"] and not report["added_keys"]:
        print(f"  [ok]      {contract}: no storage layout changes")
        return

    if breaking:
        print(f"  [BREAK]   {contract}: BREAKING storage layout change detected!")
        for enum_name in report["removed_enums"]:
            print(f"            - removed enum: {enum_name}")
        for enum_name, keys in report["removed_keys"].items():
            for key in keys:
                print(f"            - removed key:  {enum_name}::{key}")
    if report["added_enums"] or report["added_keys"]:
        for enum_name in report["added_enums"]:
            print(f"  [new]     {contract}: added enum {enum_name}")
        for enum_name, keys in report["added_keys"].items():
            for key in keys:
                print(f"  [new]     {contract}: added key  {enum_name}::{key}")


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

def main() -> int:
    parser = argparse.ArgumentParser(
        description="Snapshot and diff Soroban contract storage layouts."
    )
    parser.add_argument(
        "--update",
        action="store_true",
        help="Write current layout as the new baseline (use after intentional changes).",
    )
    parser.add_argument(
        "--contract",
        metavar="NAME",
        help="Restrict to a single contract crate.",
    )
    parser.add_argument(
        "--baseline-dir",
        metavar="DIR",
        help=f"Override baseline directory (default: {BASELINE_DIR}).",
    )
    args = parser.parse_args()

    global BASELINE_DIR
    if args.baseline_dir:
        BASELINE_DIR = Path(args.baseline_dir).resolve()

    crates = [args.contract] if args.contract else CONTRACT_CRATES

    any_breaking = False
    any_new = False

    print(f"Storage layout diff — workspace: {WORKSPACE_DIR}")
    print(f"Baselines:  {BASELINE_DIR}")
    print()

    for crate in crates:
        current = extract_storage_layout(crate)
        if not current:
            # No storage keys found — skip silently (contract may be empty or
            # use a non-standard naming convention).
            continue

        baseline = load_baseline(crate)

        if baseline is None:
            # First run for this crate — always write baseline.
            print(f"  [new]     {crate}: no baseline found — creating one")
            save_baseline(crate, current)
            any_new = True
            continue

        if args.update:
            if current != baseline:
                print(f"  [update]  {crate}: baseline updated")
                save_baseline(crate, current)
                any_new = True
            else:
                print(f"  [ok]      {crate}: baseline already up to date")
            continue

        # Diff mode (default CI path)
        report = diff_layouts(crate, baseline, current)
        print_report(report)

        if report["breaking"]:
            any_breaking = True
        if report["added_enums"] or report["added_keys"]:
            any_new = True

    print()

    if any_breaking:
        print("RESULT: Breaking storage layout changes detected.")
        print("        Review the removals above.  If intentional, run with --update")
        print("        and commit the updated baseline files.")
        return 1

    if any_new and not args.update:
        print("RESULT: New storage keys detected.")
        print("        Run with --update to record them in the baseline, then commit.")
        return 2

    print("RESULT: Storage layout is clean — no breaking changes.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
