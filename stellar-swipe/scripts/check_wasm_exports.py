#!/usr/bin/env python3
"""
check_wasm_exports.py — detect breaking changes to Soroban contract ABI exports.

Usage:
    python3 scripts/check_wasm_exports.py [--wasm-dir <dir>] [--format text|markdown]

    The script is run automatically in CI after the optimized WASM build step.
    It can also be run locally against any directory of *.wasm files.

Exit codes:
    0  All contracts match their committed baselines (or only gained exports).
    1  A breaking change was detected (removed or changed export) without an
       explicit acknowledgement file.
    2  New exports were found; baselines updated. Commit the updated JSON files.

What counts as a breaking change:
    - A function name present in the baseline is absent from the current WASM.
    - The Soroban contract-spec (contractspecv0 custom section) hash changed,
      indicating a parameter name / type / order change in an existing function.

Non-breaking:
    - New exports added since the last baseline snapshot.

Acknowledging a deliberate breaking change:
    Create the file  abi-baselines/<contract-name>.breaking.txt  in the repo.
    Its presence tells this script that the breaking change is intentional and
    paired with a migration / major-version bump.
    The script will still update the baseline so the next run passes.

Baseline files:
    abi-baselines/<contract-name>.json  — committed JSON describing the ABI.

    Format:
    {
      "contract": "<name>",
      "exports": ["fn1", "fn2", ...],
      "spec_hash": "<sha256 of raw contractspecv0 section bytes, or empty>"
    }
"""

import argparse
import hashlib
import json
import os
import struct
import sys
from pathlib import Path
from typing import Dict, List, Optional, Tuple

WORKSPACE_ROOT = Path(__file__).resolve().parent.parent
WASM_DIR_DEFAULT = WORKSPACE_ROOT / "target" / "wasm-optimized"
BASELINES_DIR = WORKSPACE_ROOT / "abi-baselines"


# ── WASM binary helpers ────────────────────────────────────────────────────────

def _read_leb128(data: bytes, pos: int) -> Tuple[int, int]:
    """Read an unsigned LEB128 integer from data[pos:].  Returns (value, bytes_consumed)."""
    result = 0
    shift = 0
    consumed = 0
    while True:
        b = data[pos + consumed]
        consumed += 1
        result |= (b & 0x7F) << shift
        shift += 7
        if (b & 0x80) == 0:
            break
    return result, consumed


def _parse_wasm_sections(data: bytes) -> Dict[int, bytes]:
    """Return {section_id: raw_section_bytes} for every section in the WASM binary."""
    if data[:4] != b"\x00asm":
        raise ValueError("Not a valid WASM binary (bad magic)")
    if data[4:8] != b"\x01\x00\x00\x00":
        raise ValueError("Unsupported WASM version")

    sections: Dict[int, bytes] = {}
    pos = 8
    while pos < len(data):
        section_id = data[pos]
        pos += 1
        size, n = _read_leb128(data, pos)
        pos += n
        sections[section_id] = data[pos : pos + size]
        pos += size
    return sections


def extract_export_names(wasm_data: bytes) -> List[str]:
    """Return all function export names from the WASM export section (id=7)."""
    try:
        sections = _parse_wasm_sections(wasm_data)
    except ValueError:
        return []

    exports_raw = sections.get(7)
    if not exports_raw:
        return []

    pos = 0
    count, n = _read_leb128(exports_raw, pos)
    pos += n
    names: List[str] = []
    for _ in range(count):
        name_len, n = _read_leb128(exports_raw, pos)
        pos += n
        name = exports_raw[pos : pos + name_len].decode("utf-8", errors="replace")
        pos += name_len
        export_kind = exports_raw[pos]
        pos += 1
        _index, n = _read_leb128(exports_raw, pos)
        pos += n
        if export_kind == 0x00:  # 0x00 = function
            names.append(name)
    return names


def extract_spec_hash(wasm_data: bytes) -> str:
    """Return SHA-256 of the raw `contractspecv0` custom section bytes, or '' if absent.

    The Soroban host embeds the contract spec (function signatures, types) in a
    custom WASM section named 'contractspecv0'.  Hashing the raw bytes gives a
    stable fingerprint: any change to parameter names, types, or order changes
    the hash, flagging a potential ABI break.
    """
    try:
        sections = _parse_wasm_sections(wasm_data)
    except ValueError:
        return ""

    custom_section_id = 0
    raw = sections.get(custom_section_id)
    if not raw:
        return ""

    # The custom section can appear multiple times in a WASM binary; we need to
    # re-parse from the raw binary to collect all of them (the dict above only
    # keeps the last one for each id).  Scan the binary for custom sections.
    custom_payloads: List[bytes] = []
    data = wasm_data
    pos = 8
    while pos < len(data):
        section_id = data[pos]
        pos += 1
        size, n = _read_leb128(data, pos)
        pos += n
        section_body = data[pos : pos + size]
        pos += size
        if section_id == 0x00:  # custom section
            # First field is the name (LEB128 length + UTF-8 bytes)
            name_len, nb = _read_leb128(section_body, 0)
            name = section_body[nb : nb + name_len].decode("utf-8", errors="replace")
            if name == "contractspecv0":
                payload = section_body[nb + name_len :]
                custom_payloads.append(payload)

    if not custom_payloads:
        return ""

    h = hashlib.sha256()
    for p in custom_payloads:
        h.update(p)
    return h.hexdigest()


# ── Baseline I/O ───────────────────────────────────────────────────────────────

def load_baseline(contract_name: str) -> Optional[dict]:
    path = BASELINES_DIR / f"{contract_name}.json"
    if not path.exists():
        return None
    return json.loads(path.read_text())


def save_baseline(contract_name: str, data: dict) -> None:
    BASELINES_DIR.mkdir(parents=True, exist_ok=True)
    path = BASELINES_DIR / f"{contract_name}.json"
    path.write_text(json.dumps(data, indent=2, sort_keys=True) + "\n")


def has_breaking_ack(contract_name: str) -> bool:
    """Return True if a deliberate-breaking-change acknowledgement file exists."""
    return (BASELINES_DIR / f"{contract_name}.breaking.txt").exists()


# ── Core analysis ──────────────────────────────────────────────────────────────

def analyze_contract(wasm_path: Path) -> dict:
    """Analyze one WASM file against its baseline.

    Returns a structured result dict with keys:
        contract, status, exports, exports_count,
        added, removed, spec_hash, baseline_spec_hash,
        spec_changed, baseline_updated, breaking, baseline,
        ack_file_present
    """
    contract_name = wasm_path.stem
    wasm_data = wasm_path.read_bytes()

    current_exports = sorted(extract_export_names(wasm_data))
    current_spec_hash = extract_spec_hash(wasm_data)

    current = {
        "contract": contract_name,
        "exports": current_exports,
        "spec_hash": current_spec_hash,
    }

    baseline = load_baseline(contract_name)
    ack = has_breaking_ack(contract_name)

    if baseline is None:
        save_baseline(contract_name, current)
        return {
            "contract": contract_name,
            "status": "first_run",
            "exports": current_exports,
            "exports_count": len(current_exports),
            "added": [],
            "removed": [],
            "spec_hash": current_spec_hash,
            "baseline_spec_hash": "",
            "spec_changed": False,
            "baseline_updated": True,
            "baseline": None,
            "ack_file_present": False,
        }

    baseline_exports = set(baseline.get("exports", []))
    current_exports_set = set(current_exports)
    baseline_spec_hash = baseline.get("spec_hash", "")

    removed = sorted(baseline_exports - current_exports_set)
    added = sorted(current_exports_set - baseline_exports)
    spec_changed = (
        current_spec_hash != baseline_spec_hash
        and baseline_spec_hash != ""
        and current_spec_hash != ""
    )
    breaking = bool(removed) or spec_changed

    baseline_updated = False

    if breaking:
        if ack:
            save_baseline(contract_name, current)
            baseline_updated = True
            return {
                "contract": contract_name,
                "status": "breaking_acknowledged",
                "exports": current_exports,
                "exports_count": len(current_exports),
                "added": added,
                "removed": removed,
                "spec_hash": current_spec_hash,
                "baseline_spec_hash": baseline_spec_hash,
                "spec_changed": spec_changed,
                "baseline_updated": True,
                "baseline": baseline,
                "ack_file_present": True,
            }
        else:
            return {
                "contract": contract_name,
                "status": "breaking",
                "exports": current_exports,
                "exports_count": len(current_exports),
                "added": added,
                "removed": removed,
                "spec_hash": current_spec_hash,
                "baseline_spec_hash": baseline_spec_hash,
                "spec_changed": spec_changed,
                "baseline_updated": False,
                "baseline": baseline,
                "ack_file_present": False,
            }

    updated = False
    if added:
        save_baseline(contract_name, current)
        updated = True

    return {
        "contract": contract_name,
        "status": "updated" if updated else "ok",
        "exports": current_exports,
        "exports_count": len(current_exports),
        "added": added,
        "removed": [],
        "spec_hash": current_spec_hash,
        "baseline_spec_hash": baseline_spec_hash,
        "spec_changed": False,
        "baseline_updated": updated,
        "baseline": baseline,
        "ack_file_present": False,
    }


# ── Renderers ──────────────────────────────────────────────────────────────────

def render_text(results: List[dict]) -> int:
    """Print text-formatted output (original behavior). Return exit code."""
    any_breaking = False
    any_updated = False

    for r in results:
        s = r["status"]

        if s == "first_run":
            print(
                f"  [{r['contract']}] No baseline found — created initial snapshot "
                f"({r['exports_count']} exports). "
                f"Commit abi-baselines/{r['contract']}.json."
            )

        elif s == "breaking_acknowledged":
            print(
                f"  [{r['contract']}] Breaking change ACKNOWLEDGED "
                f"(abi-baselines/{r['contract']}.breaking.txt present)."
            )
            if r["removed"]:
                print(f"    Removed exports: {r['removed']}")
            if r["spec_changed"]:
                print(
                    f"    Spec hash changed: {r['baseline_spec_hash'][:16]}... "
                    f"\u2192 {r['spec_hash'][:16]}..."
                )

        elif s == "breaking":
            any_breaking = True
            print(f"  [{r['contract']}] BREAKING CHANGE DETECTED:", file=sys.stderr)
            if r["removed"]:
                print(
                    f"    Removed exports (callers will break): {r['removed']}",
                    file=sys.stderr,
                )
            if r["spec_changed"]:
                print(
                    f"    Contract spec hash changed (parameter signatures may have "
                    f"changed):\n"
                    f"      baseline : {r['baseline_spec_hash'][:32]}...\n"
                    f"      current  : {r['spec_hash'][:32]}...",
                    file=sys.stderr,
                )
            print(
                f"    To acknowledge this deliberate breaking change, create:\n"
                f"      abi-baselines/{r['contract']}.breaking.txt\n"
                f"    and ensure a migration / major-version bump accompanies it.",
                file=sys.stderr,
            )

        elif s == "updated":
            any_updated = True
            print(
                f"  [{r['contract']}] New exports added: {r['added']} "
                f"\u2014 updating baseline."
            )

        else:  # ok
            print(
                f"  [{r['contract']}] OK "
                f"({r['exports_count']} exports, spec hash matches)."
            )

    print()
    if any_breaking:
        print(
            "RESULT: One or more breaking ABI changes detected without acknowledgement.\n"
            "        See errors above. Fix or acknowledge before merging.",
            file=sys.stderr,
        )
        return 1

    if any_updated:
        print(
            "RESULT: Baselines updated for new or intentionally changed exports.\n"
            "        Commit the updated abi-baselines/*.json files."
        )
        return 2

    print("RESULT: All contract ABIs match their committed baselines.")
    return 0


def render_markdown(results: List[dict]) -> str:
    """Return a markdown-formatted ABI diff report."""
    lines = []
    lines.append("## WASM ABI Export Diff Report")
    lines.append("")

    # Summary table
    lines.append("| Contract | Status | Exports | Details |")
    lines.append("|---|---|---|---|")

    for r in results:
        s = r["status"]
        name = r["contract"]
        count = r["exports_count"]

        if s == "ok":
            status = "OK"
            details = "Spec hash matches baseline"
        elif s == "first_run":
            status = "NEW"
            details = f"Initial snapshot created ({count} exports)"
        elif s == "updated":
            added = r["added"]
            status = "UPDATED"
            detail_parts = []
            if added:
                detail_parts.append(f"+{len(added)} export{'s' if len(added)>1 else ''}")
            detail_parts.append("Baseline updated")
            details = ", ".join(detail_parts)
        elif s == "breaking_acknowledged":
            status = "ACKNOWLEDGED"
            detail_parts = []
            if r["removed"]:
                detail_parts.append(f"-{len(r['removed'])} export{'s' if len(r['removed'])>1 else ''}")
            if r["spec_changed"]:
                detail_parts.append("spec hash changed")
            detail_parts.append("breaking.txt present")
            details = ", ".join(detail_parts)
        elif s == "breaking":
            status = "**BREAKING**"
            detail_parts = []
            if r["removed"]:
                detail_parts.append(f"-{len(r['removed'])} export{'s' if len(r['removed'])>1 else ''}")
            if r["spec_changed"]:
                detail_parts.append("spec hash changed")
            details = ", ".join(detail_parts) if detail_parts else "ABI mismatch"

        lines.append(f"| `{name}` | {status} | {count} | {details} |")

    lines.append("")

    # Per-contract details
    for r in results:
        s = r["status"]
        name = r["contract"]

        if s == "ok":
            continue

        lines.append(f"### `{name}`")
        lines.append("")

        if s == "first_run":
            lines.append(
                f"No baseline found — created initial snapshot "
                f"({r['exports_count']} exports).\n"
            )
            lines.append(
                f"> Commit `abi-baselines/{name}.json` to version the baseline."
            )

        elif s == "updated":
            if r["added"]:
                lines.append(f"**New exports added:** `{'`, `'.join(r['added'])}`")
            lines.append("")
            lines.append("> Baseline auto-updated. Commit the updated `abi-baselines/*.json` files.")

        elif s == "breaking_acknowledged":
            lines.append("**Breaking change ACKNOWLEDGED** (`breaking.txt` present).")
            if r["removed"]:
                lines.append(f"")
                lines.append(f"- Removed exports: `{'`, `'.join(r['removed'])}`")
            if r["spec_changed"]:
                lines.append(f"")
                lines.append(f"- Spec hash changed: `{r['baseline_spec_hash'][:16]}...` → `{r['spec_hash'][:16]}...`")
            lines.append("")
            lines.append("> Baseline updated to reflect intentional change.")

        elif s == "breaking":
            lines.append("**BREAKING CHANGE DETECTED** — review required.")
            if r["removed"]:
                lines.append(f"")
                lines.append(f"- Removed exports (callers will break): `{'`, `'.join(r['removed'])}`")
            if r["spec_changed"]:
                lines.append(f"")
                lines.append(
                    f"- Contract spec hash changed (parameter signatures may have changed):\n"
                    f"  - baseline: `{r['baseline_spec_hash'][:32]}...`\n"
                    f"  - current:  `{r['spec_hash'][:32]}...`"
                )
            lines.append("")
            lines.append(
                "> **To acknowledge this deliberate breaking change:**\n"
                f"> 1. Create `abi-baselines/{name}.breaking.txt` with a short reason.\n"
                "> 2. Ensure a migration / major-version bump accompanies it.\n"
                "> 3. Re-run CI to update the baseline."
            )

        lines.append("")

    # Result line
    any_breaking = any(r["status"] == "breaking" for r in results)
    any_updated = any(r["status"] in ("updated", "first_run", "breaking_acknowledged") for r in results)

    lines.append("---")
    lines.append("")
    if any_breaking:
        lines.append(
            "**Result: Breaking ABI changes detected without acknowledgement.**\n"
            "Fix or acknowledge before merging (see details above)."
        )
    elif any_updated:
        lines.append(
            "**Result: Baselines updated.**\n"
            "Commit the updated `abi-baselines/*.json` files (and any `.breaking.txt` files) "
            "alongside this PR."
        )
    else:
        lines.append(
            "**Result: All contract ABIs match their committed baselines.**"
        )

    return "\n".join(lines)


# ── Entry point ────────────────────────────────────────────────────────────────

def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--wasm-dir",
        default=str(WASM_DIR_DEFAULT),
        help="Directory containing optimized *.wasm files (default: target/wasm-optimized/)",
    )
    parser.add_argument(
        "--format",
        choices=["text", "markdown"],
        default="text",
        help="Output format (default: text)",
    )
    args = parser.parse_args()

    wasm_dir = Path(args.wasm_dir)
    if not wasm_dir.is_dir():
        print(
            f"WASM directory not found: {wasm_dir}\n"
            f"Build the contracts first: cd stellar-swipe && ./scripts/build.sh",
            file=sys.stderr,
        )
        return 1

    wasm_files = sorted(wasm_dir.glob("*.wasm"))
    if not wasm_files:
        print(f"No *.wasm files found in {wasm_dir}", file=sys.stderr)
        return 1

    print(f"Checking {len(wasm_files)} WASM contract(s) in {wasm_dir}:", file=sys.stderr if args.format == "markdown" else sys.stdout)

    results = []
    for wasm_path in wasm_files:
        result = analyze_contract(wasm_path)
        results.append(result)

    if args.format == "markdown":
        print(render_markdown(results))
    else:
        for r in results:
            s = r["status"]
            if s == "first_run":
                print(
                    f"  [{r['contract']}] No baseline found — created initial snapshot "
                    f"({r['exports_count']} exports). "
                    f"Commit abi-baselines/{r['contract']}.json."
                )
            elif s == "breaking_acknowledged":
                print(
                    f"  [{r['contract']}] Breaking change ACKNOWLEDGED "
                    f"(abi-baselines/{r['contract']}.breaking.txt present)."
                )
                if r["removed"]:
                    print(f"    Removed exports: {r['removed']}")
                if r["spec_changed"]:
                    print(
                        f"    Spec hash changed: {r['baseline_spec_hash'][:16]}... "
                        f"\u2192 {r['spec_hash'][:16]}..."
                    )
            elif s == "breaking":
                print(f"  [{r['contract']}] BREAKING CHANGE DETECTED:", file=sys.stderr)
                if r["removed"]:
                    print(
                        f"    Removed exports (callers will break): {r['removed']}",
                        file=sys.stderr,
                    )
                if r["spec_changed"]:
                    print(
                        f"    Contract spec hash changed (parameter signatures may have "
                        f"changed):\n"
                        f"      baseline : {r['baseline_spec_hash'][:32]}...\n"
                        f"      current  : {r['spec_hash'][:32]}...",
                        file=sys.stderr,
                    )
                print(
                    f"    To acknowledge this deliberate breaking change, create:\n"
                    f"      abi-baselines/{r['contract']}.breaking.txt\n"
                    f"    and ensure a migration / major-version bump accompanies it.",
                    file=sys.stderr,
                )
            elif s == "updated":
                print(
                    f"  [{r['contract']}] New exports added: {r['added']} "
                    f"\u2014 updating baseline."
                )
            else:
                print(
                    f"  [{r['contract']}] OK "
                    f"({r['exports_count']} exports, spec hash matches)."
                )

        print()
        any_breaking = any(r["status"] == "breaking" for r in results)
        any_updated = any(r["status"] in ("updated", "first_run", "breaking_acknowledged") for r in results)

        if any_breaking:
            print(
                "RESULT: One or more breaking ABI changes detected without acknowledgement.\n"
                "        See errors above. Fix or acknowledge before merging.",
                file=sys.stderr,
            )
            return 1

        if any_updated:
            print(
                "RESULT: Baselines updated for new or intentionally changed exports.\n"
                "        Commit the updated abi-baselines/*.json files."
            )
            return 2

        print("RESULT: All contract ABIs match their committed baselines.")

    return 0


if __name__ == "__main__":
    sys.exit(main())
