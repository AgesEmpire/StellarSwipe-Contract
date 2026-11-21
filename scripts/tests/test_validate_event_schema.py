"""Tests for scripts/validate_event_schema.py (issue #1221).

Run: python3 -m unittest discover -s scripts/tests -p 'test_*.py'
"""

import copy
import json
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import validate_event_schema as v  # noqa: E402


def event(contract, name, topics=None, body=None, version="1.0.0"):
    return {
        "schema_version": version,
        "contract": contract,
        "event_name": name,
        "topics_format": topics or ["symbol:event_name"],
        "body_fields": [{"name": n, "type": t} for n, t in (body or [("id", "u64")])],
    }


def base_document():
    events = [event(c, f"{c}_ping") for c in sorted(v.REQUIRED_CONTRACTS)]
    events.append(event("signal_registry", "signal_expired", body=[("signal_id", "u64"), ("provider", "Address")]))
    events.append(event("governance", "gov:stake", topics=["symbol:gov", "symbol:stake"]))
    return {"schema_version": "1.0.0", "events": events}


class StructureTests(unittest.TestCase):
    def test_real_schema_is_valid(self):
        document = json.loads(v.SCHEMA.read_text(encoding="utf-8"))
        self.assertEqual(v.validate_structure(document), [])

    def test_real_lock_is_current(self):
        document = json.loads(v.SCHEMA.read_text(encoding="utf-8"))
        lock = json.loads(v.LOCK.read_text(encoding="utf-8"))
        self.assertEqual(v.check_compatibility(document, lock), ([], []))

    def test_duplicate_identifier_rejected(self):
        doc = base_document()
        doc["events"].append(copy.deepcopy(doc["events"][-1]))
        self.assertTrue(any("duplicate event identifier" in e for e in v.validate_structure(doc)))

    def test_same_name_in_different_contracts_allowed(self):
        doc = base_document()
        doc["events"].append(event("oracle", "gov:stake", topics=["symbol:gov", "symbol:stake"]))
        self.assertEqual(v.validate_structure(doc), [])

    def test_identical_topics_under_different_names_collide(self):
        doc = base_document()
        doc["events"].append(event("governance", "gov:stake_v2", topics=["symbol:gov", "symbol:stake"]))
        self.assertTrue(any("topic collision" in e for e in v.validate_structure(doc)))

    def test_field_slot_collides_with_literal(self):
        doc = base_document()
        doc["events"].append(event("governance", "gov:any", topics=["symbol:gov", "pair"]))
        self.assertTrue(any("topic collision" in e for e in v.validate_structure(doc)))

    def test_different_topic_count_does_not_collide(self):
        doc = base_document()
        doc["events"].append(event("governance", "gov", topics=["symbol:gov"]))
        self.assertEqual(v.validate_structure(doc), [])

    def test_invalid_symbol_rejected(self):
        doc = base_document()
        doc["events"].append(event("governance", "x", topics=["symbol:has-dash"]))
        doc["events"].append(event("governance", "y", topics=["symbol:" + "a" * 33]))
        errors = v.validate_structure(doc)
        self.assertEqual(sum("not a valid Soroban symbol" in e for e in errors), 2)

    def test_topics_need_a_literal(self):
        doc = base_document()
        doc["events"].append(event("governance", "only_fields", topics=["pair"]))
        self.assertTrue(any("at least one literal" in e for e in v.validate_structure(doc)))


class CompatibilityTests(unittest.TestCase):
    def setUp(self):
        self.doc = base_document()
        self.lock = v.build_lock(self.doc)

    def find(self, name):
        return next(e for e in self.doc["events"] if e["event_name"] == name)

    def test_unchanged_schema_is_compatible(self):
        self.assertEqual(v.check_compatibility(self.doc, self.lock), ([], []))

    def test_new_event_is_additive_but_lock_is_stale(self):
        self.doc["events"].append(event("bridge", "bridge_new"))
        errors, stale = v.check_compatibility(self.doc, self.lock)
        self.assertEqual(errors, [])
        self.assertEqual(stale, ["bridge.bridge_new: new event"])

    def test_appended_body_field_is_additive(self):
        self.find("signal_expired")["body_fields"].append({"name": "at", "type": "u64"})
        errors, stale = v.check_compatibility(self.doc, self.lock)
        self.assertEqual(errors, [])
        self.assertEqual(len(stale), 1)

    def test_removed_event_is_breaking(self):
        self.doc["events"].remove(self.find("signal_expired"))
        errors, _ = v.check_compatibility(self.doc, self.lock)
        self.assertTrue(any("signal_registry.signal_expired: event removed" in e for e in errors))

    def test_topic_change_is_breaking(self):
        self.find("gov:stake")["topics_format"] = ["symbol:gov", "symbol:stake", "user"]
        errors, _ = v.check_compatibility(self.doc, self.lock)
        self.assertTrue(any("topics changed" in e for e in errors))

    def test_body_retype_reorder_and_removal_are_breaking(self):
        for mutate in (
            lambda f: f[0].update(type="u32"),
            lambda f: f.reverse(),
            lambda f: f.pop(),
            lambda f: f[1].update(name="author"),
        ):
            doc = copy.deepcopy(self.doc)
            fields = next(e for e in doc["events"] if e["event_name"] == "signal_expired")["body_fields"]
            mutate(fields)
            errors, _ = v.check_compatibility(doc, self.lock)
            self.assertTrue(any("body changed" in e for e in errors), fields)

    def test_breaking_change_needs_major_bump_and_note(self):
        self.find("gov:stake")["topics_format"] = ["symbol:gov", "symbol:staked"]
        # Note alone is not enough.
        self.doc["migration_notes"] = {"governance.gov:stake": "renamed topic"}
        self.assertTrue(v.check_compatibility(self.doc, self.lock)[0])
        # Major bump alone is not enough.
        bumped = copy.deepcopy(self.doc)
        bumped["migration_notes"] = {}
        bumped["schema_version"] = "2.0.0"
        for e in bumped["events"]:
            e["schema_version"] = "2.0.0"
        self.assertTrue(v.check_compatibility(bumped, self.lock)[0])
        # Both: accepted, but the lock must be regenerated.
        bumped["migration_notes"] = {"governance.gov:stake": "renamed topic"}
        errors, stale = v.check_compatibility(bumped, self.lock)
        self.assertEqual(errors, [])
        self.assertTrue(any("accepted: renamed topic" in s for s in stale))

    def test_cli_fails_on_stale_lock_and_update_lock_refreshes_it(self):
        with tempfile.TemporaryDirectory() as tmp:
            schema = Path(tmp) / "schema.json"
            lock = Path(tmp) / "lock.json"
            schema.write_text(json.dumps(self.doc))
            lock.write_text(json.dumps(self.lock))
            self.assertEqual(v.main(["--schema", str(schema), "--lock", str(lock)]), 0)

            self.doc["events"].append(event("bridge", "bridge_new"))
            schema.write_text(json.dumps(self.doc))
            self.assertEqual(v.main(["--schema", str(schema), "--lock", str(lock)]), 1)
            self.assertEqual(v.main(["--schema", str(schema), "--lock", str(lock), "--update-lock"]), 0)
            self.assertEqual(v.main(["--schema", str(schema), "--lock", str(lock)]), 0)

    def test_update_lock_refuses_unapproved_breaking_change(self):
        with tempfile.TemporaryDirectory() as tmp:
            schema = Path(tmp) / "schema.json"
            lock = Path(tmp) / "lock.json"
            lock.write_text(json.dumps(self.lock))
            self.doc["events"].remove(self.find("signal_expired"))
            schema.write_text(json.dumps(self.doc))
            args = ["--schema", str(schema), "--lock", str(lock), "--update-lock"]
            self.assertEqual(v.main(args), 1)
            self.assertEqual(json.loads(lock.read_text()), self.lock)


if __name__ == "__main__":
    unittest.main()
