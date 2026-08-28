#!/usr/bin/env python3
"""Validate the event schema without network access or third-party packages."""

import json
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SCHEMA = ROOT / "docs" / "event_schema.json"
REQUIRED_CONTRACTS = {"signal_registry", "stake_vault", "fee_collector", "governance", "bridge"}
SCALAR_TYPES = {
    "Address", "Option<u64>", "String", "Symbol", "Vec<Address>",
    "Vec<String>", "bool", "i128", "u32", "u64", "object",
}


def main() -> int:
    try:
        document = json.loads(SCHEMA.read_text(encoding="utf-8"))
        version = document["schema_version"]
        events = document["events"]
    except (OSError, json.JSONDecodeError, KeyError, TypeError) as error:
        print(f"event schema is unreadable: {error}", file=sys.stderr)
        return 1

    if not isinstance(version, str) or not version:
        print("schema_version must be a non-empty string", file=sys.stderr)
        return 1
    if not isinstance(events, list) or not events:
        print("events must be a non-empty array", file=sys.stderr)
        return 1

    seen = set()
    contracts = set()
    for event in events:
        key = (event.get("contract"), event.get("event_name"))
        if None in key or key in seen:
            print(f"duplicate or incomplete event: {key}", file=sys.stderr)
            return 1
        seen.add(key)
        contracts.add(key[0])
        if event.get("schema_version") != version:
            print(f"version mismatch: {key}", file=sys.stderr)
            return 1
        topics = event.get("topics_format")
        fields = event.get("body_fields")
        if not isinstance(topics, list) or not topics:
            print(f"topics_format must be non-empty: {key}", file=sys.stderr)
            return 1
        if not isinstance(fields, list):
            print(f"body_fields must be an array: {key}", file=sys.stderr)
            return 1
        names = [field.get("name") for field in fields]
        if None in names or len(names) != len(set(names)):
            print(f"body field order contains duplicate or unnamed fields: {key}", file=sys.stderr)
            return 1
        if any(field.get("type") not in SCALAR_TYPES for field in fields):
            print(f"unsupported body field type: {key}", file=sys.stderr)
            return 1

    missing = sorted(REQUIRED_CONTRACTS - contracts)
    if missing:
        print(f"schema is missing contract families: {', '.join(missing)}", file=sys.stderr)
        return 1
    print(f"validated {len(events)} events in schema v{version}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())