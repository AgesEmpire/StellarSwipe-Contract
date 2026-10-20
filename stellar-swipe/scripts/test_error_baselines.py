#!/usr/bin/env python3
"""
test_error_baselines.py — verify that every committed error-baseline JSON is
internally consistent and matches the taxonomy rules.

This is the *test* counterpart to check_error_codes.py.  Where
check_error_codes.py compares source against baselines at CI time,
this script validates the baselines themselves to catch drift or
corruption independent of a Rust build.

Tests performed:
  1. Every baseline file is valid JSON with the required top-level keys.
  2. No two variants within a single enum share a numeric code (uniqueness).
  3. No variant has code 0 (reserved / falsy in Soroban contract-spec).
  4. Crate names in the files match the file names.
  5. All variant names follow UpperCamelCase.
  6. The "deprecated" list (if present) only references codes that no longer
     appear in "enums" (i.e. they have genuinely been removed, not just
     renamed without updating the deprecated list).
  7. Category coverage: every baseline file for a crate in the KNOWN_CRATES
     list must exist.

Exit codes:
  0  All checks pass.
  1  One or more checks failed.

Usage:
  python3 stellar-swipe/scripts/test_error_baselines.py
"""

import json
import re
import sys
from pathlib import Path
from typing import Any, Dict, List, Tuple

WORKSPACE_ROOT = Path(__file__).resolve().parent.parent
BASELINES_DIR = WORKSPACE_ROOT / "error-baselines"

# Crates that must have a baseline file.  Extend this list when a new contract
# crate is added to the workspace.
KNOWN_CRATES = [
    "auto_trade",
    "bridge",
    "common",
    "fee_collector",
    "governance",
    "oracle",
    "shared",
    "signal_registry",
    "stake_vault",
    "trade_executor",
    "user_portfolio",
]

UPPER_CAMEL_RE = re.compile(r"^[A-Z][A-Za-z0-9]*$")

# ── Helpers ────────────────────────────────────────────────────────────────────


def fail(msg: str) -> None:
    print(f"FAIL  {msg}")


def ok(msg: str) -> None:
    print(f"  OK  {msg}")


# ── Per-file checks ────────────────────────────────────────────────────────────


def check_file(path: Path) -> List[str]:
    """Return a list of failure messages for the given baseline file."""
    failures: List[str] = []
    crate_name = path.stem  # filename without .json

    # 1. Valid JSON and required keys.
    try:
        data: Dict[str, Any] = json.loads(path.read_text())
    except json.JSONDecodeError as exc:
        failures.append(f"[{crate_name}] invalid JSON: {exc}")
        return failures  # Nothing else we can check.

    for key in ("crate", "enums"):
        if key not in data:
            failures.append(f"[{crate_name}] missing required key '{key}'")

    if failures:
        return failures

    # 4. Crate name in file should be related to the file name.
    # The Rust package name may differ from the directory name (e.g.
    # "stellar_swipe_common" vs file "common.json"), so we only flag cases
    # where the field looks entirely unrelated (neither a substring nor a
    # transformed variant of the file stem).
    declared_crate: str = data["crate"]
    crate_stem_normalized = crate_name.replace("-", "_")
    declared_normalized = declared_crate.replace("-", "_")
    if (
        crate_stem_normalized not in declared_normalized
        and declared_normalized not in crate_stem_normalized
    ):
        failures.append(
            f"[{crate_name}] 'crate' field is '{data['crate']}' "
            f"which does not appear related to file name '{path.name}'"
        )

    enums: Dict[str, Dict[str, int]] = data.get("enums", {})
    deprecated_list: List[Dict[str, Any]] = data.get("deprecated", [])

    # Collect all codes still present in "enums" for deprecated-list check.
    all_live_codes: set[Tuple[str, int]] = set()

    for enum_name, variants in enums.items():
        codes_seen: Dict[int, str] = {}

        for variant, code in variants.items():
            all_live_codes.add((enum_name, code))

            # 2. Uniqueness within enum.
            if code in codes_seen:
                failures.append(
                    f"[{crate_name}::{enum_name}] code {code} is assigned to "
                    f"both '{codes_seen[code]}' and '{variant}'"
                )
            else:
                codes_seen[code] = variant

            # 3. Code 0 is reserved.
            if code == 0:
                failures.append(
                    f"[{crate_name}::{enum_name}] variant '{variant}' uses "
                    f"code 0, which is reserved"
                )

            # 5. UpperCamelCase variant names.
            if not UPPER_CAMEL_RE.match(variant):
                failures.append(
                    f"[{crate_name}::{enum_name}] variant '{variant}' is not "
                    f"UpperCamelCase"
                )

    # 6. Deprecated entries must not reuse codes still live in "enums".
    for dep in deprecated_list:
        dep_enum = dep.get("enum", "")
        dep_code = dep.get("code")
        dep_variant = dep.get("variant", "?")
        if dep_code is None:
            continue
        if (dep_enum, dep_code) in all_live_codes:
            failures.append(
                f"[{crate_name}] deprecated entry '{dep_variant}' (code "
                f"{dep_code}) still appears in 'enums' — "
                f"remove it from 'enums' or from 'deprecated'"
            )

    return failures


# ── Entry point ────────────────────────────────────────────────────────────────


def main() -> int:
    if not BASELINES_DIR.is_dir():
        print(f"ERROR: baselines directory not found: {BASELINES_DIR}")
        return 1

    overall_failures: List[str] = []

    # Check each existing baseline file.
    baseline_files = sorted(BASELINES_DIR.glob("*.json"))
    for bf in baseline_files:
        file_failures = check_file(bf)
        if file_failures:
            overall_failures.extend(file_failures)
        else:
            ok(f"{bf.name}")

    # 7. Coverage: all known crates must have a baseline.
    existing_crates = {p.stem for p in baseline_files}
    for crate in KNOWN_CRATES:
        if crate not in existing_crates:
            overall_failures.append(
                f"[{crate}] no baseline file found in {BASELINES_DIR} — "
                f"run 'python3 stellar-swipe/scripts/check_error_codes.py' "
                f"to generate it"
            )

    if overall_failures:
        print()
        print("Failures:")
        for msg in overall_failures:
            fail(msg)
        print(f"\n{len(overall_failures)} check(s) failed.")
        return 1

    print(f"\nAll {len(baseline_files)} baseline file(s) passed.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
