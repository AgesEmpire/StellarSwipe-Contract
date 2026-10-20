#!/usr/bin/env python3
"""
validate_deployment_manifest.py — validate a StellarSwipe deployment manifest
before it is used to deploy or upgrade contracts (Issue #822).

Usage:
    python3 scripts/validate_deployment_manifest.py [manifest.json ...]

    With no arguments, validates every deployments/*.manifest.json found in
    the workspace (skipped with a message if none exist). This step is run
    automatically near the start of scripts/deploy_testnet.sh and in CI, so
    a misconfigured manifest fails fast, before any contract is touched.

Exit codes:
    0  All manifests valid (or none found — nothing to check).
    1  One or more manifests failed validation. All problems found are
       printed together (this does not stop at the first error), each with
       enough context to fix it directly.

What is validated:
    - Required top-level fields: network (non-empty string), admin
      (StrKey), contracts (non-empty object).
    - `admin`, and each contract's `address` when set, are syntactically
      valid Stellar StrKeys: correct length, base32 alphabet, and a correct
      CRC16-XModem checksum — of the expected type (G... account for admin,
      C... contract for contract addresses). This catches copy/paste typos
      and truncated keys that `stellar contract deploy` would otherwise only
      discover mid-release.
    - Each contract entry declares a non-empty `package` and a positive
      integer `version`.
    - Every `depends_on` entry names another contract present in the same
      manifest (no dangling references to a contract that was never
      configured) and requires a `min_version` that the depended-on
      contract's declared `version` actually satisfies — mirroring the
      on-chain `shared::version::validate_callee_version` cross-contract
      compatibility check, but caught before deployment instead of at
      cross-contract call time.
    - The dependency graph is acyclic. A cycle makes it impossible to choose
      a deployment order (each contract's dependencies must exist before it
      does), so this is reported as a fatal misconfiguration.

Manifest schema:
    {
      "network": "testnet",
      "admin": "G...",
      "contracts": {
        "<logical_name>": {
          "package": "<cargo package name>",
          "address": "C..." | null,      // null before first deploy
          "version": <positive integer>,
          "depends_on": {
            "<other_logical_name>": {"min_version": <positive integer>},
            ...
          }
        },
        ...
      }
    }
"""

import argparse
import base64
import binascii
import json
import sys
from pathlib import Path
from typing import Dict, List, Optional

WORKSPACE_ROOT = Path(__file__).resolve().parent.parent
REPO_ROOT = WORKSPACE_ROOT.parent
DEFAULT_DEPLOYMENTS_DIR = REPO_ROOT / "deployments"

# ── StrKey validation (Stellar SEP-0023) ────────────────────────────────────
#
# A StrKey is base32(version_byte || payload || crc16_xmodem(version_byte || payload)),
# unpadded. For 32-byte payloads (ed25519 public keys, contract IDs) this is
# always exactly 56 base32 characters.

_BASE32_ALPHABET = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567"
STRKEY_VERSION_ACCOUNT = 6 << 3  # 'G...'
STRKEY_VERSION_CONTRACT = 2 << 3  # 'C...'
_STRKEY_LEN = 56  # 1-byte version + 32-byte payload + 2-byte checksum, base32-encoded


def _crc16_xmodem(data: bytes) -> int:
    """CRC16-XModem (poly 0x1021, init 0) — the checksum algorithm StrKey uses."""
    crc = 0x0000
    for byte in data:
        crc ^= byte << 8
        for _ in range(8):
            if crc & 0x8000:
                crc = ((crc << 1) ^ 0x1021) & 0xFFFF
            else:
                crc = (crc << 1) & 0xFFFF
    return crc


def strkey_error(value: object, expected_version: int, type_label: str) -> Optional[str]:
    """Return an actionable error string if `value` is not a valid StrKey of
    the expected type, or `None` if it is valid."""
    if not isinstance(value, str):
        return f"expected a {type_label} StrKey string, got {type(value).__name__}"
    if len(value) != _STRKEY_LEN:
        return (
            f"expected a {_STRKEY_LEN}-character {type_label} StrKey, "
            f"got {len(value)} characters: {value!r}"
        )
    bad_chars = sorted(set(c for c in value if c not in _BASE32_ALPHABET))
    if bad_chars:
        return f"{type_label} StrKey contains invalid base32 characters: {bad_chars!r}"
    try:
        raw = base64.b32decode(value, casefold=False)
    except (binascii.Error, ValueError) as e:
        return f"{type_label} StrKey failed base32 decoding: {e}"
    if len(raw) != 35:
        return f"{type_label} StrKey decoded to {len(raw)} bytes, expected 35"
    version_byte, payload, checksum = raw[0], raw[1:33], raw[33:35]
    expected_checksum = _crc16_xmodem(bytes([version_byte]) + payload).to_bytes(2, "little")
    if checksum != expected_checksum:
        return f"{type_label} StrKey has an invalid checksum (typo or corrupted address): {value!r}"
    if version_byte != expected_version:
        got_prefix = value[0]
        want_prefix = "G" if expected_version == STRKEY_VERSION_ACCOUNT else "C"
        return (
            f"{type_label} StrKey has the wrong address type: starts with "
            f"'{got_prefix}' but a '{want_prefix}...' {type_label} was expected: {value!r}"
        )
    return None


# ── Manifest validation ─────────────────────────────────────────────────────


def _validate_top_level(data: dict) -> List[str]:
    errors = []
    if not isinstance(data.get("network"), str) or not data.get("network").strip():
        errors.append("top-level 'network' must be a non-empty string")

    admin = data.get("admin")
    err = strkey_error(admin, STRKEY_VERSION_ACCOUNT, "admin account")
    if err:
        errors.append(f"top-level 'admin': {err}")

    contracts = data.get("contracts")
    if not isinstance(contracts, dict) or not contracts:
        errors.append("top-level 'contracts' must be a non-empty object")
    return errors


def _validate_contract_entries(contracts: Dict[str, dict]) -> List[str]:
    errors = []
    for name, entry in contracts.items():
        if not isinstance(entry, dict):
            errors.append(f"contracts.{name}: must be an object")
            continue

        package = entry.get("package")
        if not isinstance(package, str) or not package.strip():
            errors.append(f"contracts.{name}.package: must be a non-empty string")

        version = entry.get("version")
        if not isinstance(version, int) or isinstance(version, bool) or version <= 0:
            errors.append(f"contracts.{name}.version: must be a positive integer, got {version!r}")

        address = entry.get("address")
        if address is not None:
            err = strkey_error(address, STRKEY_VERSION_CONTRACT, "contract address")
            if err:
                errors.append(f"contracts.{name}.address: {err}")

        depends_on = entry.get("depends_on", {})
        if not isinstance(depends_on, dict):
            errors.append(f"contracts.{name}.depends_on: must be an object")
            continue
        for dep_name, dep_spec in depends_on.items():
            if dep_name == name:
                errors.append(f"contracts.{name}.depends_on: cannot depend on itself")
            if dep_name not in contracts:
                errors.append(
                    f"contracts.{name}.depends_on references unknown contract "
                    f"'{dep_name}' (not present in this manifest's 'contracts')"
                )
            if not isinstance(dep_spec, dict):
                errors.append(f"contracts.{name}.depends_on.{dep_name}: must be an object")
                continue
            min_version = dep_spec.get("min_version")
            if not isinstance(min_version, int) or isinstance(min_version, bool) or min_version <= 0:
                errors.append(
                    f"contracts.{name}.depends_on.{dep_name}.min_version: "
                    f"must be a positive integer, got {min_version!r}"
                )
    return errors


def _validate_version_compatibility(contracts: Dict[str, dict]) -> List[str]:
    """Each dependency's declared min_version must be satisfied by the
    depended-on contract's own declared version — mirrors the runtime
    `shared::version::validate_callee_version` check, but at manifest time."""
    errors = []
    for name, entry in contracts.items():
        if not isinstance(entry, dict):
            continue
        depends_on = entry.get("depends_on", {})
        if not isinstance(depends_on, dict):
            continue
        for dep_name, dep_spec in depends_on.items():
            target = contracts.get(dep_name)
            if not isinstance(target, dict) or not isinstance(dep_spec, dict):
                continue  # already reported by _validate_contract_entries
            min_version = dep_spec.get("min_version")
            target_version = target.get("version")
            if not isinstance(min_version, int) or not isinstance(target_version, int):
                continue  # already reported
            if target_version < min_version:
                errors.append(
                    f"contracts.{name} requires '{dep_name}' version >= {min_version}, "
                    f"but contracts.{dep_name}.version is {target_version}"
                )
    return errors


def _find_dependency_cycle(contracts: Dict[str, dict]) -> Optional[List[str]]:
    """DFS cycle detection over the depends_on graph. Returns the cycle path
    (as logical names) if one exists, else None."""
    WHITE, GRAY, BLACK = 0, 1, 2
    color = {name: WHITE for name in contracts}
    path: List[str] = []

    def visit(node: str) -> Optional[List[str]]:
        color[node] = GRAY
        path.append(node)
        entry = contracts.get(node) or {}
        depends_on = entry.get("depends_on", {}) if isinstance(entry, dict) else {}
        if isinstance(depends_on, dict):
            for dep_name in depends_on:
                if dep_name not in color:
                    continue  # unknown dependency already reported elsewhere
                if color[dep_name] == GRAY:
                    cycle_start = path.index(dep_name)
                    return path[cycle_start:] + [dep_name]
                if color[dep_name] == WHITE:
                    result = visit(dep_name)
                    if result:
                        return result
        path.pop()
        color[node] = BLACK
        return None

    for name in contracts:
        if color[name] == WHITE:
            cycle = visit(name)
            if cycle:
                return cycle
    return None


def validate_manifest(data: dict) -> List[str]:
    """Validate a parsed manifest dict. Returns a list of error strings
    (empty if the manifest is valid)."""
    errors = _validate_top_level(data)

    contracts = data.get("contracts")
    if not isinstance(contracts, dict) or not contracts:
        # Nothing further can be checked meaningfully without contracts.
        return errors

    errors.extend(_validate_contract_entries(contracts))
    errors.extend(_validate_version_compatibility(contracts))

    cycle = _find_dependency_cycle(contracts)
    if cycle:
        errors.append(
            "circular dependency detected in 'depends_on': " + " -> ".join(cycle)
        )

    return errors


def validate_manifest_file(path: Path) -> List[str]:
    try:
        raw = path.read_text()
    except OSError as e:
        return [f"could not read file: {e}"]
    try:
        data = json.loads(raw)
    except json.JSONDecodeError as e:
        return [f"invalid JSON: {e}"]
    if not isinstance(data, dict):
        return ["manifest root must be a JSON object"]
    return validate_manifest(data)


# ── Entry point ────────────────────────────────────────────────────────────


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument(
        "manifests",
        nargs="*",
        help="Path(s) to manifest JSON file(s). Defaults to deployments/*.manifest.json.",
    )
    args = parser.parse_args()

    if args.manifests:
        paths = [Path(p) for p in args.manifests]
    else:
        paths = sorted(DEFAULT_DEPLOYMENTS_DIR.glob("*.manifest.json"))
        if not paths:
            print(
                f"No deployment manifests found under {DEFAULT_DEPLOYMENTS_DIR} "
                f"(*.manifest.json) — nothing to validate."
            )
            return 0

    any_invalid = False
    for path in paths:
        if not path.exists():
            print(f"[{path}] FAIL: file not found", file=sys.stderr)
            any_invalid = True
            continue

        errors = validate_manifest_file(path)
        if errors:
            any_invalid = True
            print(f"[{path}] INVALID — {len(errors)} problem(s):", file=sys.stderr)
            for err in errors:
                print(f"    - {err}", file=sys.stderr)
        else:
            print(f"[{path}] OK")

    if any_invalid:
        print(
            "\nRESULT: one or more deployment manifests are invalid. "
            "Fix the problems above before deploying.",
            file=sys.stderr,
        )
        return 1

    print("\nRESULT: all deployment manifests are valid.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
