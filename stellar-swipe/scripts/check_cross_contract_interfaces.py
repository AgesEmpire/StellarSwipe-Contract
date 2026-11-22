#!/usr/bin/env python3
"""Cross-contract interface compatibility check.

Dependent contracts call other contracts through ``#[contractclient]`` traits.
This script extracts every such trait (method signatures) plus the definitions
of the ``#[contracttype]`` / ``#[contracterror]`` types those signatures use,
and diffs the result against the committed artifact
``cross-contract-interfaces/interfaces.json``.

Exit codes:
  0 - source matches the committed artifact
  1 - breaking drift: a client, method or serialized type was removed or changed
  2 - additive drift: a new client, method or type is not yet recorded

Regenerate after an intentional change with:
  python3 stellar-swipe/scripts/check_cross_contract_interfaces.py --update
"""

import argparse
import json
import re
import sys
from pathlib import Path

STELLAR_SWIPE = Path(__file__).resolve().parents[1]
DEFAULT_CONTRACTS = STELLAR_SWIPE / "contracts"
DEFAULT_ARTIFACT = STELLAR_SWIPE / "cross-contract-interfaces" / "interfaces.json"

CLIENT_RE = re.compile(
    r'#\[contractclient\(\s*name\s*=\s*"(?P<client>\w+)"\s*\)\]\s*'
    r"(?:#\[[^\]]*\]\s*)*"
    r"pub\s+trait\s+\w+\s*\{"
)
TYPE_DEF_RE = re.compile(
    r"#\[(?:contracttype|contracterror)\]\s*(?:#\[[^\]]*\]\s*)*"
    r"(?:pub\s+)?(?P<kind>struct|enum)\s+(?P<name>\w+)"
)
FN_RE = re.compile(r"\bfn\s+(\w+)\s*\((.*?)\)\s*(?:->\s*([^;]+?)\s*)?;", re.S)
TYPE_REF_RE = re.compile(r"\b[A-Z]\w*\b")


def strip_comments(text):
    text = re.sub(r"/\*.*?\*/", "", text, flags=re.S)
    return re.sub(r"//[^\n]*", "", text)


def normalize(text):
    return " ".join(text.split())


def block_end(text, open_idx):
    """Index of the brace closing the one at open_idx."""
    depth = 0
    for i in range(open_idx, len(text)):
        if text[i] == "{":
            depth += 1
        elif text[i] == "}":
            depth -= 1
            if depth == 0:
                return i
    raise ValueError("unbalanced braces in contract source")


def item_end(text, pos):
    """End index (exclusive) of the struct/enum whose name ends at pos."""
    m = re.compile(r"[{;]").search(text, pos)
    if m.group() == ";":
        return m.end()
    return block_end(text, m.start()) + 1


def split_top_level(args):
    parts, depth, cur = [], 0, ""
    for c in args:
        if c in "<([":
            depth += 1
        elif c in ">)]":
            depth -= 1
        if c == "," and depth == 0:
            parts.append(cur)
            cur = ""
        else:
            cur += c
    parts.append(cur)
    return [normalize(p) for p in parts if p.strip()]


def trait_methods(body):
    methods = {}
    for m in FN_RE.finditer(body):
        name = m.group(1)
        args = ", ".join(split_top_level(m.group(2)))
        ret = normalize(m.group(3) or "")
        sig = f"fn {name}({args})" + (f" -> {ret}" if ret else "")
        methods[name] = sig
    return methods


def scan(contracts_dir):
    interfaces = {}
    for crate in sorted(p for p in contracts_dir.iterdir() if (p / "src").is_dir()):
        texts = {
            f: strip_comments(f.read_text(encoding="utf-8"))
            for f in sorted((crate / "src").rglob("*.rs"))
        }

        # Serialized types, indexed by name. A name defined more than once in
        # the crate cannot be resolved safely and is reported as unresolved.
        types = {}
        for text in texts.values():
            for m in TYPE_DEF_RE.finditer(text):
                end = item_end(text, m.end())
                definition = normalize(text[m.start():end])
                types.setdefault(m.group("name"), []).append(definition)

        for text in texts.values():
            for m in CLIENT_RE.finditer(text):
                open_idx = text.index("{", m.end() - 1)
                body = text[open_idx + 1:block_end(text, open_idx)]
                methods = trait_methods(body)

                refs = set(TYPE_REF_RE.findall(" ".join(methods.values())))
                resolved, unresolved = {}, []
                for ref in sorted(refs):
                    defs = types.get(ref, [])
                    if len(defs) == 1:
                        resolved[ref] = defs[0]
                    elif len(defs) > 1:
                        unresolved.append(ref)

                key = f"{crate.name}/{m.group('client')}"
                interfaces[key] = {
                    "methods": methods,
                    "types": resolved,
                    "unresolved": unresolved,
                }
    return {"schema_version": 1, "interfaces": interfaces}


def diff(baseline, current):
    breaking, additive = [], []
    base = baseline["interfaces"]
    cur = current["interfaces"]

    for key in sorted(base.keys() - cur.keys()):
        breaking.append(f"{key}: client interface removed")
    for key in sorted(cur.keys() - base.keys()):
        additive.append(f"{key}: new client interface")

    for key in sorted(base.keys() & cur.keys()):
        b, c = base[key], cur[key]

        for name in sorted(b["methods"].keys() - c["methods"].keys()):
            breaking.append(f"{key}: method `{name}` removed")
        for name in sorted(c["methods"].keys() - b["methods"].keys()):
            additive.append(f"{key}: method `{name}` added")
        for name in sorted(b["methods"].keys() & c["methods"].keys()):
            if b["methods"][name] != c["methods"][name]:
                breaking.append(
                    f"{key}: signature of `{name}` changed\n"
                    f"      was: {b['methods'][name]}\n"
                    f"      now: {c['methods'][name]}"
                )

        for name in sorted(b["types"].keys() - c["types"].keys()):
            breaking.append(
                f"{key}: serialized type `{name}` removed or no longer uniquely defined"
            )
        for name in sorted(c["types"].keys() - b["types"].keys()):
            additive.append(f"{key}: serialized type `{name}` added")
        for name in sorted(b["types"].keys() & c["types"].keys()):
            if b["types"][name] != c["types"][name]:
                breaking.append(
                    f"{key}: serialized type `{name}` changed\n"
                    f"      was: {b['types'][name]}\n"
                    f"      now: {c['types'][name]}"
                )

    return breaking, additive


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--contracts-dir", type=Path, default=DEFAULT_CONTRACTS)
    parser.add_argument("--artifact", type=Path, default=DEFAULT_ARTIFACT)
    parser.add_argument(
        "--update",
        action="store_true",
        help="rewrite the committed artifact from the current source",
    )
    args = parser.parse_args()

    current = scan(args.contracts_dir)
    rendered = json.dumps(current, indent=2, sort_keys=True) + "\n"

    if args.update:
        args.artifact.parent.mkdir(parents=True, exist_ok=True)
        args.artifact.write_text(rendered, encoding="utf-8")
        print(f"Wrote {len(current['interfaces'])} cross-contract interface(s) "
              f"to {args.artifact}")
        return 0

    if not args.artifact.exists():
        print(f"ERROR: interface artifact missing: {args.artifact}", file=sys.stderr)
        print("  Generate it with:", file=sys.stderr)
        print("    python3 stellar-swipe/scripts/check_cross_contract_interfaces.py --update",
              file=sys.stderr)
        return 1

    baseline = json.loads(args.artifact.read_text(encoding="utf-8"))
    breaking, additive = diff(baseline, current)

    if breaking:
        print("ERROR: breaking cross-contract interface change(s) detected:", file=sys.stderr)
        for line in breaking:
            print(f"  - {line}", file=sys.stderr)
        print("", file=sys.stderr)
        print("  Dependent contracts compile and serialize against these interfaces.", file=sys.stderr)
        print("  Either restore the previous signature, or bump the callee interface", file=sys.stderr)
        print("  version, update every dependent caller, and follow", file=sys.stderr)
        print("  docs/CROSS_CONTRACT_INTERFACE_VERSIONING.md.  Then regenerate with:", file=sys.stderr)
        print("    python3 stellar-swipe/scripts/check_cross_contract_interfaces.py --update",
              file=sys.stderr)
        return 1

    if additive:
        print("ERROR: cross-contract interface artifact is stale (additive change(s)):", file=sys.stderr)
        for line in additive:
            print(f"  - {line}", file=sys.stderr)
        print("  Regenerate and commit the artifact:", file=sys.stderr)
        print("    python3 stellar-swipe/scripts/check_cross_contract_interfaces.py --update",
              file=sys.stderr)
        return 2

    print(f"OK: {len(current['interfaces'])} cross-contract interface(s) match the artifact.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
