#!/usr/bin/env python3
"""
check_error_codes.py  –  Guard against #[contracterror] renumbering and code reuse.

Usage
-----
  python3 scripts/check_error_codes.py               # CI mode: exits non-zero on violations
  python3 scripts/check_error_codes.py --update-baseline  # write/refresh baselines then exit 0

Exit codes
----------
  0  All baselines match; or baseline update completed.
  1  A violation was found (renumbered variant or reused numeric code).
  2  New codes not yet in baseline were detected (run --update-baseline and commit).

Deprecating an error code
-------------------------
Mark the variant with a doc comment like `/// DEPRECATED – replaced by Foo`.
Never reuse the numeric value for a different variant name.  The baseline will
retain the original mapping so the CI always catches accidental reuse.

Requesting a policy exception
------------------------------
If an error code genuinely needs renumbering (extremely rare – treat this like a
breaking API change), update the baseline file under error-baselines/ manually,
add a comment in the baseline JSON explaining the reason, and get the change
reviewed and approved before merging.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import sys
from pathlib import Path
from typing import Dict, List, Tuple

# ---------------------------------------------------------------------------
# Paths
# ---------------------------------------------------------------------------

SCRIPT_DIR = Path(__file__).parent.resolve()
WORKSPACE_ROOT = SCRIPT_DIR.parent          # stellar-swipe/
CONTRACTS_DIR = WORKSPACE_ROOT / "contracts"
BASELINES_DIR = WORKSPACE_ROOT / "error-baselines"

# ---------------------------------------------------------------------------
# Parsing
# ---------------------------------------------------------------------------

# Matches the whole body of a #[contracterror] enum, capturing:
#   group 1 – optional contracterror args, e.g. `export = false`
#   group 2 – enum name
#   group 3 – enum body (everything between { and })
_ENUM_RE = re.compile(
    r"#\[contracterror(?:\(([^)]*)\))?\]"   # #[contracterror] or #[contracterror(…)]
    r"(?:\s*#\[[^\]]*\])*"                  # zero or more extra attributes
    r"\s*(?:pub\s+)?enum\s+(\w+)\s*\{([^}]*)\}",
    re.DOTALL,
)

# Matches  VariantName = <number>  within an enum body (ignores doc comments)
_VARIANT_RE = re.compile(r"\b(\w+)\s*=\s*(\d+)")


def _parse_file(path: Path) -> List[Tuple[str, Dict[str, int], bool]]:
    """Return list of (enum_name, {variant: code}, is_exported)."""
    try:
        text = path.read_text(encoding="utf-8")
    except OSError:
        return []

    results = []
    for m in _ENUM_RE.finditer(text):
        args_str = m.group(1) or ""
        exported = "export" not in args_str or "true" in args_str
        enum_name = m.group(2)
        body = m.group(3)
        variants: Dict[str, int] = {}
        for vm in _VARIANT_RE.finditer(body):
            variants[vm.group(1)] = int(vm.group(2))
        if variants:
            results.append((enum_name, variants, exported))
    return results


def collect_source_enums() -> Dict[str, Dict[str, Dict[str, int]]]:
    """
    Walk all contract crates and return:
        { crate_name: { EnumName: { VariantName: code } } }

    Skips #[contracterror(export = false)] enums – clients never see those codes.
    """
    source: Dict[str, Dict[str, Dict[str, int]]] = {}
    if not CONTRACTS_DIR.is_dir():
        sys.exit(f"ERROR: contracts dir not found: {CONTRACTS_DIR}")

    for rs_file in sorted(CONTRACTS_DIR.rglob("*.rs")):
        # Skip test files
        rel = rs_file.relative_to(CONTRACTS_DIR)
        parts = rel.parts
        if any(p in ("tests", "test", "integration_tests") for p in parts):
            continue
        if rs_file.stem.startswith("test"):
            continue

        crate_name = parts[0]
        for enum_name, variants, exported in _parse_file(rs_file):
            if not exported:
                continue
            source.setdefault(crate_name, {})
            if enum_name in source[crate_name]:
                # Merge if same enum name appears in multiple files within a crate
                source[crate_name][enum_name].update(variants)
            else:
                source[crate_name][enum_name] = dict(variants)
    return source


# ---------------------------------------------------------------------------
# Baseline I/O
# ---------------------------------------------------------------------------

def load_baselines() -> Dict[str, Dict[str, Dict[str, int]]]:
    """Load all committed baselines from BASELINES_DIR."""
    baselines: Dict[str, Dict[str, Dict[str, int]]] = {}
    if not BASELINES_DIR.is_dir():
        return baselines
    for json_file in sorted(BASELINES_DIR.rglob("*.json")):
        rel = json_file.relative_to(BASELINES_DIR)
        crate_name = rel.parts[0]
        enum_name = json_file.stem
        try:
            data = json.loads(json_file.read_text(encoding="utf-8"))
        except json.JSONDecodeError as exc:
            sys.exit(f"ERROR: cannot parse {json_file}: {exc}")
        baselines.setdefault(crate_name, {})[enum_name] = {k: int(v) for k, v in data.items()}
    return baselines


def write_baselines(source: Dict[str, Dict[str, Dict[str, int]]]) -> None:
    """Write / update baseline files from current source state."""
    existing = load_baselines()

    for crate_name, enums in source.items():
        for enum_name, variants in enums.items():
            out_dir = BASELINES_DIR / crate_name
            out_dir.mkdir(parents=True, exist_ok=True)
            out_file = out_dir / f"{enum_name}.json"

            # Merge: keep deprecated variants (not in source) so codes are never reused.
            merged: Dict[str, int] = {}
            if crate_name in existing and enum_name in existing[crate_name]:
                merged.update(existing[crate_name][enum_name])
            merged.update(variants)

            out_file.write_text(
                json.dumps(dict(sorted(merged.items(), key=lambda kv: kv[1])), indent=2) + "\n",
                encoding="utf-8",
            )
            print(f"  updated: {out_file.relative_to(WORKSPACE_ROOT)}")


# ---------------------------------------------------------------------------
# Checking
# ---------------------------------------------------------------------------

def check(
    source: Dict[str, Dict[str, Dict[str, int]]],
    baselines: Dict[str, Dict[str, Dict[str, int]]],
) -> Tuple[List[str], List[str]]:
    """
    Compare source against baselines.

    Returns (violations, warnings):
      violations – fatal (renumbered or reused code) → exit 1
      warnings   – new codes not yet in baseline → exit 2 if no violations
    """
    violations: List[str] = []
    warnings: List[str] = []

    for crate_name, enum_map in baselines.items():
        src_crate = source.get(crate_name, {})
        for enum_name, baseline_variants in enum_map.items():
            src_variants = src_crate.get(enum_name, {})

            # Build reverse maps
            baseline_by_code: Dict[int, str] = {v: k for k, v in baseline_variants.items()}
            src_by_code: Dict[int, str] = {v: k for k, v in src_variants.items()}

            for variant, code in baseline_variants.items():
                if variant not in src_variants:
                    # Variant removed from source – that's fine (it may be deprecated).
                    # The baseline retains it so the code can never be reused.
                    continue
                if src_variants[variant] != code:
                    violations.append(
                        f"RENUMBER  {crate_name}::{enum_name}::{variant}  "
                        f"baseline={code}  source={src_variants[variant]}\n"
                        f"         Changing an existing error code breaks clients that rely on "
                        f"the numeric value.  Revert the change or mark the old variant "
                        f"DEPRECATED and add a new variant with a fresh number."
                    )

            for code, src_variant in src_by_code.items():
                baseline_variant = baseline_by_code.get(code)
                if baseline_variant is not None and baseline_variant != src_variant:
                    violations.append(
                        f"REUSE     {crate_name}::{enum_name}  code={code}  "
                        f"was={baseline_variant}  now={src_variant}\n"
                        f"          Reusing a numeric code for a different variant silently "
                        f"breaks clients.  Use a fresh, never-before-used number."
                    )

    # Detect new codes not yet in baseline
    for crate_name, enum_map in source.items():
        for enum_name, src_variants in enum_map.items():
            baseline_variants = baselines.get(crate_name, {}).get(enum_name, {})
            new_variants = {k: v for k, v in src_variants.items() if k not in baseline_variants}
            if enum_name not in baselines.get(crate_name, {}):
                warnings.append(
                    f"NEW ENUM  {crate_name}::{enum_name}  "
                    f"({len(src_variants)} variant(s)) – not yet in baseline"
                )
            elif new_variants:
                for var, code in sorted(new_variants.items(), key=lambda kv: kv[1]):
                    warnings.append(
                        f"NEW CODE  {crate_name}::{enum_name}::{var}={code} – not yet in baseline"
                    )

    return violations, warnings


# ---------------------------------------------------------------------------
# Entry point
# ---------------------------------------------------------------------------

def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument(
        "--update-baseline",
        action="store_true",
        help="Write/refresh baseline files from the current source and exit 0.",
    )
    args = parser.parse_args()

    print("==> Scanning contract sources for #[contracterror] enums …")
    source = collect_source_enums()
    total_enums = sum(len(e) for e in source.values())
    total_variants = sum(len(v) for e in source.values() for v in e.values())
    print(f"    found {total_enums} enum(s) / {total_variants} variant(s) across {len(source)} crate(s)")

    if args.update_baseline:
        print("==> Updating baselines …")
        write_baselines(source)
        print("Done.  Commit the updated files under error-baselines/ with your PR.")
        sys.exit(0)

    baselines = load_baselines()
    if not baselines:
        print(
            "WARNING: No baseline files found under error-baselines/.\n"
            "Run with --update-baseline to create the initial baseline, then commit the result."
        )
        sys.exit(2)

    violations, warnings = check(source, baselines)

    if warnings:
        print("\n[WARN] New error codes not yet in the baseline:")
        for w in warnings:
            print(f"  {w}")
        print(
            "\n  Run `python3 scripts/check_error_codes.py --update-baseline` and commit\n"
            "  the updated baseline file(s) alongside your PR.\n"
        )

    if violations:
        print("\n[FAIL] Error code stability violations detected:\n")
        for v in violations:
            print(f"  {v}\n")
        print(
            "Changing or reusing an existing error code is a breaking change for any\n"
            "client that pattern-matches on the numeric value.  See error-baselines/\n"
            "for the committed mapping.  If you need to retire a variant, keep its\n"
            "number in the enum (mark it DEPRECATED) so it can never be reused.\n"
        )
        sys.exit(1)

    if warnings:
        print("[WARN] Baseline is out of date (new codes present).  Update and commit it.")
        sys.exit(2)

    print("All error codes match their committed baselines.")


if __name__ == "__main__":
    main()
