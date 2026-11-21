#!/usr/bin/env python3
"""
check_error_codes.py — guard against renumbering or reusing #[contracterror] codes.

Usage:
    python3 scripts/check_error_codes.py

Exit codes:
    0  All baselines match the source.
    1  One or more violations found (renumbering or reuse detected).
    2  A new enum or variant was found; baseline updated automatically.
       Re-run to confirm. This is not an error in CI — adding new codes
       with fresh numbers is allowed.

How it works:
  1. Parse every *.rs file in contracts/ for `#[contracterror]` enums and their
     discriminant values.
  2. Compare each (enum, variant, value) triple against the corresponding JSON
     baseline in error-baselines/.
  3. Fail if:
       a) An existing variant's numeric value changed.
       b) A numeric value that was previously assigned to variant A now maps to
          a different variant B (reuse detection).
  4. Allow (and update the baseline for):
       a) Brand-new variants with a previously-unused number.
       b) Brand-new enums not yet in any baseline file.

Deprecating a code:
  Never reuse the number.  Add the variant name to the "deprecated" list in
  the baseline JSON.  See docs/source-verification.md for the full process.
"""

import json
import os
import re
import sys
from pathlib import Path
from typing import Dict, Tuple

WORKSPACE_ROOT = Path(__file__).resolve().parent.parent
CONTRACTS_DIR = WORKSPACE_ROOT / "contracts"
BASELINES_DIR = WORKSPACE_ROOT / "error-baselines"

# Matches:  SomeVariant = 42,
VARIANT_RE = re.compile(r"^\s+(\w+)\s*=\s*(\d+)\s*,?\s*(?://.*)?$")
ENUM_START_RE = re.compile(r"^\s*pub\s+enum\s+(\w+)")
CONTRACTERROR_RE = re.compile(r"#\[contracterror\]")


def parse_contracterror_enums(rs_file: Path) -> Dict[str, Dict[str, int]]:
    """Return {enum_name: {variant: code}} for all #[contracterror] enums in rs_file."""
    enums: Dict[str, Dict[str, int]] = {}
    text = rs_file.read_text()
    lines = text.splitlines()

    i = 0
    while i < len(lines):
        if CONTRACTERROR_RE.search(lines[i]):
            # Scan forward for `pub enum EnumName`
            j = i + 1
            enum_name = None
            while j < len(lines) and j < i + 10:
                m = ENUM_START_RE.search(lines[j])
                if m:
                    enum_name = m.group(1)
                    break
                j += 1

            if enum_name is None:
                i += 1
                continue

            # Collect variants until closing `}`
            variants: Dict[str, int] = {}
            depth = 0
            k = j
            while k < len(lines):
                line = lines[k]
                depth += line.count("{") - line.count("}")
                if depth < 0:
                    break
                m = VARIANT_RE.match(line)
                if m:
                    variants[m.group(1)] = int(m.group(2))
                k += 1

            if variants:
                enums[enum_name] = variants
            i = k
        else:
            i += 1

    return enums


def collect_workspace_enums() -> Dict[str, Dict[str, Dict[str, int]]]:
    """
    Return {crate_name: {enum_name: {variant: code}}} for all contracts/ crates.
    crate_name is the directory name under contracts/.
    """
    result: Dict[str, Dict[str, Dict[str, int]]] = {}
    for crate_dir in sorted(CONTRACTS_DIR.iterdir()):
        if not crate_dir.is_dir():
            continue
        crate_name = crate_dir.name
        crate_enums: Dict[str, Dict[str, int]] = {}
        for rs_file in sorted(crate_dir.rglob("*.rs")):
            # Skip test helpers and integration tests
            if any(p in rs_file.parts for p in ("integration_tests", "target")):
                continue
            found = parse_contracterror_enums(rs_file)
            crate_enums.update(found)
        if crate_enums:
            result[crate_name] = crate_enums
    return result


def load_baseline(crate_name: str) -> Dict[str, Dict[str, int]]:
    """Return the stored baseline for crate_name, or {} if none exists."""
    path = BASELINES_DIR / f"{crate_name}.json"
    if not path.exists():
        return {}
    data = json.loads(path.read_text())
    return data.get("enums", {})


def save_baseline(crate_name: str, enums: Dict[str, Dict[str, int]]) -> None:
    path = BASELINES_DIR / f"{crate_name}.json"
    existing: dict = {}
    if path.exists():
        existing = json.loads(path.read_text())

    existing["enums"] = {
        enum_name: dict(sorted(variants.items(), key=lambda kv: kv[1]))
        for enum_name, variants in sorted(enums.items())
    }
    if "_comment" not in existing:
        existing["_comment"] = (
            f"Baseline error codes for the {crate_name} crate. "
            "Do not renumber existing codes; add new variants with unused numbers only."
        )
    existing["crate"] = crate_name
    path.write_text(json.dumps(existing, indent=2) + "\n")


def check_crate(
    crate_name: str,
    current: Dict[str, Dict[str, int]],
    baseline: Dict[str, Dict[str, int]],
) -> Tuple[bool, bool]:
    """
    Returns (has_violations, has_new_codes).
    Prints human-readable diagnostics.
    """
    has_violations = False
    has_new_codes = False

    for enum_name, variants in current.items():
        b_enum = baseline.get(enum_name, {})
        # Invert baseline: code → variant name (for reuse detection)
        b_code_to_variant: Dict[int, str] = {v: k for k, v in b_enum.items()}

        for variant, code in variants.items():
            if variant in b_enum:
                # Variant already baselined — code must not change.
                if b_enum[variant] != code:
                    print(
                        f"ERROR [{crate_name}::{enum_name}] "
                        f"variant '{variant}' was code {b_enum[variant]}, "
                        f"now {code} — renumbering is not allowed."
                    )
                    has_violations = True
            else:
                # New variant — check whether its code was previously assigned to another variant.
                if code in b_code_to_variant:
                    old_variant = b_code_to_variant[code]
                    print(
                        f"ERROR [{crate_name}::{enum_name}] "
                        f"code {code} was previously assigned to '{old_variant}' "
                        f"and is now reused for '{variant}' — reuse is not allowed."
                    )
                    has_violations = True
                else:
                    print(
                        f"NEW   [{crate_name}::{enum_name}] "
                        f"variant '{variant}' = {code} (baseline updated)"
                    )
                    has_new_codes = True

    return has_violations, has_new_codes


def main() -> int:
    workspace_enums = collect_workspace_enums()

    overall_violations = False
    overall_new = False

    for crate_name, current_enums in sorted(workspace_enums.items()):
        baseline_enums = load_baseline(crate_name)
        violations, new_codes = check_crate(crate_name, current_enums, baseline_enums)

        if violations:
            overall_violations = True

        if new_codes and not violations:
            # Auto-update the baseline with the new variants.
            merged = {}
            for enum_name, variants in current_enums.items():
                merged[enum_name] = {**baseline_enums.get(enum_name, {}), **variants}
            save_baseline(crate_name, merged)
            overall_new = True

        if not baseline_enums and not violations:
            # First time we've seen this crate — write initial baseline.
            save_baseline(crate_name, current_enums)
            print(f"INIT  [{crate_name}] baseline created")
            overall_new = True

    if overall_violations:
        print(
            "\nFAIL: one or more error-code violations detected.\n"
            "  Renumbering or reusing an existing code breaks clients that rely\n"
            "  on stable numeric values.  To deprecate a code, add it to the\n"
            "  'deprecated' list in the baseline JSON — never reuse the number.\n"
            "  See docs/source-verification.md for the full deprecation process."
        )
        return 1

    if overall_new:
        print(
            "\nINFO: new error codes were added and the baseline was updated.\n"
            "      Commit the updated error-baselines/*.json files."
        )
        return 2

    print("OK: all error codes match their baselines.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
