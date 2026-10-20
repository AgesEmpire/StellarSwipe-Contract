#!/usr/bin/env python3
"""Public contract error ABI compatibility check (issue #1222).

Clients decode contract failures by numeric error code, so the code behind
every ``#[contracterror]`` variant is public ABI. This script captures that
ABI in ``error-abi/public-error-abi.json`` and fails when the Rust source
drifts from it.

Rules, per crate and enum:

* **Breaking:** a variant or enum is removed, a variant's code changes
  (renumbering), or a code previously held by one variant is given to a
  different one (reuse).
* **Additive:** new variants or enums. Accepted, but the fixture must be
  regenerated so the new codes are pinned.
* Codes must be unique within an enum and every variant needs an explicit
  discriminant.

Breaking changes are only recorded with an explicit version and migration
note::

    python3 scripts/check_error_abi.py --update --version 2.0.0 \\
        --note "AdminError::Foo (7) removed; clients should treat 7 as Bar"

The new version must have a higher major version than the fixture. The note
and the list of breaking changes are appended to the fixture's
``migrations`` so reviewers and clients see them.

``--base-ref REF`` additionally compares the committed fixture with its
version at REF (e.g. the PR base), so hand-editing the fixture to hide a
renumbering is caught too.

Exit status: 0 = source matches fixture, 1 = any failure.
"""

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CONTRACTS = ROOT / "contracts"
FIXTURE = ROOT / "error-abi" / "public-error-abi.json"
# Test-only crates do not ship a public ABI.
EXCLUDED_CRATES = {"integration_tests", "stake_vault_kani"}

ENUM_RE = re.compile(r"\s*(?:#\s*\[[^\]]*\]\s*)*pub(?:\s*\([^)]*\))?\s+enum\s+(\w+)\s*\{")
VARIANT_RE = re.compile(r"^\s*(?:#\s*\[[^\]]*\]\s*)*(\w+)\s*(?:=\s*(0[xX][0-9a-fA-F_]+|[0-9_]+)\s*)?$")
VERSION_RE = re.compile(r"^\d+\.\d+\.\d+$")


def strip_comments_and_strings(src):
    """Blank out comments and string/char literals, keeping everything else."""
    out = []
    i, n = 0, len(src)
    while i < n:
        c = src[i]
        nxt = src[i + 1] if i + 1 < n else ""
        if c == "/" and nxt == "/":
            while i < n and src[i] != "\n":
                i += 1
        elif c == "/" and nxt == "*":
            depth, i = 1, i + 2
            while i < n and depth:
                if src.startswith("/*", i):
                    depth, i = depth + 1, i + 2
                elif src.startswith("*/", i):
                    depth, i = depth - 1, i + 2
                else:
                    i += 1
            out.append(" ")
        elif c == "r" and re.match(r'r#*"', src[i:i + 40]) and not (i and (src[i - 1].isalnum() or src[i - 1] == "_")):
            hashes = re.match(r'r(#*)"', src[i:]).group(1)
            end = src.find('"' + hashes, i + 2 + len(hashes))
            i = n if end < 0 else end + 1 + len(hashes)
            out.append('""')
        elif c == '"':
            i += 1
            while i < n and src[i] != '"':
                i += 2 if src[i] == "\\" else 1
            i += 1
            out.append('""')
        elif c == "'" and re.match(r"'(\\.[^']*|[^\\'])'", src[i:i + 12]):
            i += len(re.match(r"'(\\.[^']*|[^\\'])'", src[i:i + 12]).group(0))
            out.append("' '")
        else:
            out.append(c)
            i += 1
    return "".join(out)


def parse_error_enums(src, where):
    """Return ({enum: {variant: code}}, [errors]) for one Rust source file."""
    enums, errors = {}, []
    clean = strip_comments_and_strings(src)
    for attr in re.finditer(r"#\s*\[\s*contracterror\s*\]", clean):
        m = ENUM_RE.match(clean, attr.end())
        if not m:
            errors.append(f"{where}: #[contracterror] is not followed by a pub enum")
            continue
        start = depth_pos = m.end()
        depth = 1
        while depth and depth_pos < len(clean):
            depth += {"{": 1, "}": -1}.get(clean[depth_pos], 0)
            depth_pos += 1
        body = clean[start:depth_pos - 1]
        name = m.group(1)
        variants, by_code = {}, {}
        for item in body.split(","):
            if not item.strip():
                continue
            vm = VARIANT_RE.match(item)
            if not vm:
                errors.append(f"{where}: {name}: cannot parse variant {item.strip()!r}")
                continue
            variant, raw = vm.groups()
            if raw is None:
                errors.append(f"{where}: {name}::{variant} has no explicit discriminant")
                continue
            code = int(raw.replace("_", ""), 0)
            if code in by_code:
                errors.append(f"{where}: {name}: code {code} used by both {by_code[code]} and {variant}")
            by_code[code] = variant
            variants[variant] = code
        enums[name] = variants
    return enums, errors


def collect_source_abi(contracts_dir=CONTRACTS):
    """Return ({crate: {enum_key: {variant: code}}}, [errors])."""
    abi, errors = {}, []
    for crate_dir in sorted(p for p in contracts_dir.iterdir() if (p / "src").is_dir()):
        if crate_dir.name in EXCLUDED_CRATES:
            continue
        per_name = {}  # enum name -> [(module, variants)]
        for path in sorted((crate_dir / "src").rglob("*.rs")):
            rel = path.relative_to(crate_dir / "src").with_suffix("")
            module = "::".join(p for p in rel.parts if p not in ("lib", "mod")) or "crate"
            enums, errs = parse_error_enums(path.read_text(encoding="utf-8"), str(path.relative_to(contracts_dir.parent)))
            errors.extend(errs)
            for name, variants in enums.items():
                per_name.setdefault(name, []).append((module, variants))
        crate_abi = {}
        for name, defs in per_name.items():
            if len(defs) == 1:
                crate_abi[name] = defs[0][1]
            else:
                # Same enum name in several modules: qualify each by module.
                for module, variants in defs:
                    crate_abi[f"{module}::{name}"] = variants
        if crate_abi:
            abi[crate_dir.name] = dict(sorted(crate_abi.items()))
    return abi, errors


def diff_abi(old, new):
    """Return (breaking, additive) change descriptions from ``old`` to ``new``."""
    breaking, additive = [], []
    for crate in sorted(set(old) | set(new)):
        old_enums, new_enums = old.get(crate, {}), new.get(crate, {})
        for enum in sorted(set(old_enums) | set(new_enums)):
            label = f"{crate}::{enum}"
            if enum not in new_enums:
                breaking.append(f"{label}: enum removed")
                continue
            if enum not in old_enums:
                additive.append(f"{label}: new enum")
                continue
            before, after = old_enums[enum], new_enums[enum]
            owner = {code: variant for variant, code in before.items()}
            for variant, code in sorted(before.items(), key=lambda kv: kv[1]):
                if variant not in after:
                    breaking.append(f"{label}::{variant} ({code}) removed")
                elif after[variant] != code:
                    breaking.append(f"{label}::{variant} renumbered {code} -> {after[variant]}")
            for variant, code in sorted(after.items(), key=lambda kv: kv[1]):
                if variant in before:
                    continue
                if code in owner and owner[code] != variant:
                    breaking.append(f"{label}::{variant} reuses code {code} of {owner[code]}")
                else:
                    additive.append(f"{label}::{variant} = {code} added")
    return breaking, additive


def major(version):
    return int(version.split(".")[0])


def load_fixture(path):
    return json.loads(path.read_text(encoding="utf-8"))


def check_history(fixture, base):
    """Breaking changes between two fixture versions must be declared."""
    breaking, _ = diff_abi(base["crates"], fixture["crates"])
    if not breaking:
        return []
    known = {m["version"]: m for m in fixture.get("migrations", [])}
    entry = known.get(fixture["abi_version"])
    declared = set(entry["changes"]) if entry else set()
    problems = []
    if major(fixture["abi_version"]) <= major(base["abi_version"]) or not entry or not entry.get("note"):
        problems.append(
            f"fixture has breaking changes since base (abi_version {base['abi_version']} -> "
            f"{fixture['abi_version']}) without a major version bump and migration note"
        )
    problems += [f"undeclared breaking change: {c}" for c in breaking if c not in declared]
    return problems


def git_show(ref, path):
    rel = path.resolve().relative_to(Path(subprocess.check_output(
        ["git", "rev-parse", "--show-toplevel"], cwd=ROOT, text=True).strip()))
    result = subprocess.run(["git", "show", f"{ref}:{rel.as_posix()}"], cwd=ROOT,
                            capture_output=True, text=True)
    return json.loads(result.stdout) if result.returncode == 0 else None


def write_fixture(path, fixture):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(fixture, indent=2) + "\n", encoding="utf-8")


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--contracts-dir", type=Path, default=CONTRACTS)
    parser.add_argument("--fixture", type=Path, default=FIXTURE)
    parser.add_argument("--update", action="store_true", help="regenerate the fixture from source")
    parser.add_argument("--version", help="new abi_version, required with --update for breaking changes")
    parser.add_argument("--note", help="migration note, required with --update for breaking changes")
    parser.add_argument("--base-ref", help="git ref whose fixture the committed one must be compatible with")
    args = parser.parse_args(argv)

    source, errors = collect_source_abi(args.contracts_dir)
    if errors:
        for error in errors:
            print(f"ERROR {error}", file=sys.stderr)
        return 1

    if not args.fixture.exists():
        if not args.update:
            print(f"ERROR fixture {args.fixture} is missing; run with --update", file=sys.stderr)
            return 1
        write_fixture(args.fixture, {
            "_comment": "Public #[contracterror] codes. Generated by scripts/check_error_abi.py; "
                        "see docs/error_abi_compatibility.md. Do not edit by hand.",
            "abi_version": args.version or "1.0.0",
            "crates": source,
            "migrations": [],
        })
        print(f"created {args.fixture}")
        return 0

    fixture = load_fixture(args.fixture)
    status = 0

    if args.base_ref:
        base = git_show(args.base_ref, args.fixture)
        if base is None:
            print(f"NOTE  no fixture at {args.base_ref}; skipping history check")
        else:
            for problem in check_history(fixture, base):
                print(f"ERROR {problem}", file=sys.stderr)
                status = 1

    breaking, additive = diff_abi(fixture["crates"], source)
    if args.update:
        if breaking:
            if not args.version or not VERSION_RE.match(args.version) or not args.note:
                for change in breaking:
                    print(f"ERROR {change}", file=sys.stderr)
                print("ERROR breaking changes need --version X.Y.Z and --note", file=sys.stderr)
                return 1
            if major(args.version) <= major(fixture["abi_version"]):
                print(f"ERROR --version {args.version} must bump the major version of "
                      f"{fixture['abi_version']}", file=sys.stderr)
                return 1
            fixture["abi_version"] = args.version
            fixture.setdefault("migrations", []).append(
                {"version": args.version, "note": args.note, "changes": breaking})
        elif args.version:
            if not VERSION_RE.match(args.version):
                print(f"ERROR invalid --version {args.version}", file=sys.stderr)
                return 1
            fixture["abi_version"] = args.version
        fixture["crates"] = source
        write_fixture(args.fixture, fixture)
        for change in breaking + additive:
            print(f"UPDATED {change}")
        return status

    for change in breaking:
        print(f"ERROR {change}", file=sys.stderr)
    for change in additive:
        print(f"STALE {change}", file=sys.stderr)
    if breaking:
        print("Renumbering, removing or reusing a public error code breaks clients. If this is "
              "intentional, rerun with --update --version <next major> --note '<migration note>'.",
              file=sys.stderr)
        status = 1
    elif additive:
        print("New error codes are not pinned yet. Run `python3 stellar-swipe/scripts/check_error_abi.py "
              "--update` and commit stellar-swipe/error-abi/public-error-abi.json.", file=sys.stderr)
        status = 1
    if status == 0:
        total = sum(len(v) for enums in source.values() for v in enums.values())
        print(f"error ABI v{fixture['abi_version']} matches source ({total} codes)")
    return status


if __name__ == "__main__":
    raise SystemExit(main())
