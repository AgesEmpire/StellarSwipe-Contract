#!/usr/bin/env python3
"""Validate the event schema without network access or third-party packages.

Structural checks (issue #1221 extends the original ones):

* ``schema_version`` is set and every event repeats it.
* Every event has a unique identifier ``<contract>.<event_name>``.
* Topic literals are valid Soroban symbols (``[A-Za-z0-9_]``, 1..32 chars).
* **Topic uniqueness rule:** within one contract, no two events may publish
  topic tuples an indexer could confuse. Two events collide when they have
  the same topic count and, at every position, either both are the same
  literal or at least one is a data field (which can hold any value).
* Body fields are named, unique, and use a supported type.

Compatibility checks against ``docs/event_schema.lock.json``:

* **Breaking:** an event is removed, its topic signature changes, or an
  existing body field is removed, renamed, retyped or reordered.
* **Additive:** a new event, or new body fields appended after the existing
  ones.

A breaking change is accepted only when ``schema_version`` has a higher major
version than the lock *and* ``migration_notes`` has a non-empty note for each
affected event id. Any accepted difference leaves the lock stale; regenerate
it with ``--update-lock`` and commit it, so every change to the event surface
shows up in review.

Exit status: 0 = valid and lock current, 1 = any failure.
"""

import argparse
import json
import re
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SCHEMA = ROOT / "docs" / "event_schema.json"
LOCK = ROOT / "docs" / "event_schema.lock.json"
REQUIRED_CONTRACTS = {"signal_registry", "stake_vault", "fee_collector", "governance", "bridge"}
SCALAR_TYPES = {
    "Address", "Option<u64>", "String", "Symbol", "Vec<Address>",
    "Vec<String>", "bool", "i128", "u32", "u64", "object",
}
SYMBOL_RE = re.compile(r"^[A-Za-z0-9_]{1,32}$")
EVENT_NAME_RE = re.compile(r"^[A-Za-z0-9_:]{1,64}$")


def event_id(event):
    return f"{event['contract']}.{event['event_name']}"


def topic_signature(event):
    """Resolve ``topics_format`` into literal symbols and ``{field}`` slots."""
    signature = []
    for topic in event["topics_format"]:
        if topic == "symbol:event_name":
            signature.append(event["event_name"])
        elif topic.startswith("symbol:"):
            signature.append(topic[len("symbol:"):])
        else:
            signature.append("{" + topic + "}")
    return signature


def is_field(slot):
    return slot.startswith("{")


def topics_collide(a, b):
    if len(a) != len(b):
        return False
    return all(x == y or is_field(x) or is_field(y) for x, y in zip(a, b))


def body_signature(event):
    return [[field["name"], field["type"]] for field in event["body_fields"]]


def major(version):
    try:
        return int(str(version).split(".")[0])
    except ValueError:
        return -1


def validate_structure(document):
    """Return a list of error strings for the schema document itself."""
    errors = []
    version = document.get("schema_version")
    events = document.get("events")
    if not isinstance(version, str) or not version:
        return ["schema_version must be a non-empty string"]
    if not isinstance(events, list) or not events:
        return ["events must be a non-empty array"]

    seen_ids = set()
    contracts = set()
    signatures = {}  # contract -> [(id, signature)]
    for event in events:
        if not isinstance(event, dict) or not event.get("contract") or not event.get("event_name"):
            errors.append(f"incomplete event: {event!r}")
            continue
        eid = event_id(event)
        if eid in seen_ids:
            errors.append(f"duplicate event identifier: {eid}")
            continue
        seen_ids.add(eid)
        contracts.add(event["contract"])
        if not EVENT_NAME_RE.match(event["event_name"]):
            errors.append(f"invalid event_name: {eid}")
        if event.get("schema_version") != version:
            errors.append(f"version mismatch: {eid}")

        topics = event.get("topics_format")
        fields = event.get("body_fields")
        if not isinstance(topics, list) or not topics or not all(isinstance(t, str) for t in topics):
            errors.append(f"topics_format must be a non-empty array of strings: {eid}")
            continue
        if not isinstance(fields, list):
            errors.append(f"body_fields must be an array: {eid}")
            continue

        signature = topic_signature(event)
        literals = [slot for slot in signature if not is_field(slot)]
        if not literals:
            errors.append(f"topics must contain at least one literal symbol: {eid}")
        for slot in literals:
            if not SYMBOL_RE.match(slot):
                errors.append(f"topic literal {slot!r} is not a valid Soroban symbol: {eid}")
        for other_id, other_sig in signatures.get(event["contract"], []):
            if topics_collide(signature, other_sig):
                errors.append(
                    f"topic collision: {eid} {signature} is indistinguishable from "
                    f"{other_id} {other_sig}"
                )
        signatures.setdefault(event["contract"], []).append((eid, signature))

        names = [field.get("name") for field in fields if isinstance(field, dict)]
        if len(names) != len(fields) or None in names or len(names) != len(set(names)):
            errors.append(f"body field order contains duplicate or unnamed fields: {eid}")
        elif any(field.get("type") not in SCALAR_TYPES for field in fields):
            errors.append(f"unsupported body field type: {eid}")

    missing = sorted(REQUIRED_CONTRACTS - contracts)
    if missing:
        errors.append(f"schema is missing contract families: {', '.join(missing)}")

    notes = document.get("migration_notes", {})
    if not isinstance(notes, dict):
        errors.append("migration_notes must be an object mapping event id to note")
    return errors


def build_lock(document):
    return {
        "_comment": (
            "Generated by scripts/validate_event_schema.py --update-lock. Records the "
            "topic signature and body layout clients and indexers depend on. Do not edit by hand."
        ),
        "schema_version": document["schema_version"],
        "events": {
            event_id(event): {
                "topics": topic_signature(event),
                "body": body_signature(event),
            }
            for event in sorted(document["events"], key=event_id)
        },
    }


def compare_to_lock(document, lock):
    """Return (breaking, additive) lists of human-readable differences.

    Each breaking entry is a tuple ``(event_id, message)``.
    """
    current = build_lock(document)["events"]
    locked = lock.get("events", {})
    breaking = []
    additive = []
    for eid, old in sorted(locked.items()):
        new = current.get(eid)
        if new is None:
            breaking.append((eid, "event removed"))
            continue
        if new["topics"] != old["topics"]:
            breaking.append((eid, f"topics changed {old['topics']} -> {new['topics']}"))
        old_body, new_body = old["body"], new["body"]
        if new_body[: len(old_body)] != old_body:
            breaking.append((eid, f"body changed {old_body} -> {new_body}"))
        elif len(new_body) > len(old_body):
            additive.append(f"{eid}: body fields appended {new_body[len(old_body):]}")
    for eid in sorted(set(current) - set(locked)):
        additive.append(f"{eid}: new event")
    return breaking, additive


def check_compatibility(document, lock):
    """Return (errors, stale_reasons)."""
    breaking, additive = compare_to_lock(document, lock)
    errors = []
    stale = list(additive)
    notes = document.get("migration_notes", {}) or {}
    bumped = major(document["schema_version"]) > major(lock.get("schema_version", "0"))
    for eid, message in breaking:
        note = notes.get(eid)
        if bumped and isinstance(note, str) and note.strip():
            stale.append(f"{eid}: {message} (accepted: {note.strip()})")
        else:
            errors.append(
                f"incompatible change to {eid}: {message}. Bump the schema_version major "
                f"version and add migration_notes[{eid!r}] if this is intentional."
            )
    if document["schema_version"] != lock.get("schema_version") and not errors:
        stale.append(f"schema_version {lock.get('schema_version')} -> {document['schema_version']}")
    return errors, stale


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--schema", type=Path, default=SCHEMA)
    parser.add_argument("--lock", type=Path, default=LOCK)
    parser.add_argument(
        "--update-lock",
        action="store_true",
        help="rewrite the lock file after all checks pass (refuses unapproved breaking changes)",
    )
    args = parser.parse_args(argv)

    try:
        document = json.loads(args.schema.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        print(f"event schema is unreadable: {error}", file=sys.stderr)
        return 1
    if not isinstance(document, dict):
        print("event schema must be a JSON object", file=sys.stderr)
        return 1

    errors = validate_structure(document)
    if errors:
        for error in errors:
            print(error, file=sys.stderr)
        return 1

    if args.lock.exists():
        try:
            lock = json.loads(args.lock.read_text(encoding="utf-8"))
        except json.JSONDecodeError as error:
            print(f"event schema lock is unreadable: {error}", file=sys.stderr)
            return 1
        errors, stale = check_compatibility(document, lock)
    else:
        errors, stale = [], [f"lock file {args.lock} does not exist"]

    if errors:
        for error in errors:
            print(error, file=sys.stderr)
        return 1

    if stale:
        if args.update_lock:
            args.lock.write_text(json.dumps(build_lock(document), indent=2) + "\n", encoding="utf-8")
            for reason in stale:
                print(f"lock updated: {reason}")
        else:
            print("event schema lock is stale:", file=sys.stderr)
            for reason in stale:
                print(f"  {reason}", file=sys.stderr)
            print(
                "Run `python3 scripts/validate_event_schema.py --update-lock` and commit "
                "docs/event_schema.lock.json.",
                file=sys.stderr,
            )
            return 1

    print(f"validated {len(document['events'])} events in schema v{document['schema_version']}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
