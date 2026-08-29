#!/usr/bin/env python3
"""
test_validate_deployment_manifest.py — unit tests for
scripts/validate_deployment_manifest.py (Issue #822).

Run with:
    python3 scripts/tests/test_validate_deployment_manifest.py
    # or: python3 -m unittest scripts.tests.test_validate_deployment_manifest -v
"""

import json
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPTS_DIR = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(SCRIPTS_DIR))

import validate_deployment_manifest as vdm  # noqa: E402

# Two real, independently-known-valid StrKeys used as fixtures throughout.
VALID_ACCOUNT = "GBBD47IF6LWK7P7MDEVSCWR7DPUWV3NY3DTQEVFL4NAT4AQH3ZLLFLA5"
VALID_CONTRACT = "CA4CVLTTJI2BGISKBYOBD3PQ7MB4JRK4LEVCNQPHLC6PWZMNG7KBYJQ6"
ANOTHER_VALID_CONTRACT = "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA23PQL"


def make_manifest(**overrides) -> dict:
    base = {
        "network": "testnet",
        "admin": VALID_ACCOUNT,
        "contracts": {
            "signal_registry": {
                "package": "signal_registry",
                "address": VALID_CONTRACT,
                "version": 2,
                "depends_on": {},
            },
            "fee_collector": {
                "package": "fee_collector",
                "address": ANOTHER_VALID_CONTRACT,
                "version": 2,
                "depends_on": {},
            },
        },
    }
    base.update(overrides)
    return base


class StrKeyValidationTests(unittest.TestCase):
    def test_valid_account_strkey(self):
        self.assertIsNone(
            vdm.strkey_error(VALID_ACCOUNT, vdm.STRKEY_VERSION_ACCOUNT, "admin account")
        )

    def test_valid_contract_strkey(self):
        self.assertIsNone(
            vdm.strkey_error(VALID_CONTRACT, vdm.STRKEY_VERSION_CONTRACT, "contract address")
        )

    def test_wrong_length_rejected(self):
        err = vdm.strkey_error(VALID_ACCOUNT[:-1], vdm.STRKEY_VERSION_ACCOUNT, "admin account")
        self.assertIsNotNone(err)
        self.assertIn("characters", err)

    def test_invalid_base32_characters_rejected(self):
        tampered = "0" + VALID_ACCOUNT[1:]  # '0' and '1' are not in the strkey alphabet
        err = vdm.strkey_error(tampered, vdm.STRKEY_VERSION_ACCOUNT, "admin account")
        self.assertIsNotNone(err)

    def test_corrupted_checksum_rejected(self):
        # Flip the last character — same length/alphabet, wrong checksum.
        last = VALID_ACCOUNT[-1]
        replacement = "A" if last != "A" else "B"
        tampered = VALID_ACCOUNT[:-1] + replacement
        err = vdm.strkey_error(tampered, vdm.STRKEY_VERSION_ACCOUNT, "admin account")
        self.assertIsNotNone(err)
        self.assertIn("checksum", err)

    def test_wrong_address_type_rejected(self):
        # A valid G... account passed where a C... contract was expected.
        err = vdm.strkey_error(VALID_ACCOUNT, vdm.STRKEY_VERSION_CONTRACT, "contract address")
        self.assertIsNotNone(err)
        self.assertIn("wrong address type", err)

    def test_none_rejected(self):
        err = vdm.strkey_error(None, vdm.STRKEY_VERSION_ACCOUNT, "admin account")
        self.assertIsNotNone(err)

    def test_non_string_rejected(self):
        err = vdm.strkey_error(12345, vdm.STRKEY_VERSION_ACCOUNT, "admin account")
        self.assertIsNotNone(err)


class ManifestSuccessCaseTests(unittest.TestCase):
    def test_minimal_valid_manifest_has_no_errors(self):
        manifest = make_manifest()
        self.assertEqual(vdm.validate_manifest(manifest), [])

    def test_null_address_is_valid_pre_deploy_state(self):
        manifest = make_manifest()
        manifest["contracts"]["signal_registry"]["address"] = None
        self.assertEqual(vdm.validate_manifest(manifest), [])

    def test_satisfied_dependency_is_valid(self):
        manifest = make_manifest()
        manifest["contracts"]["fee_collector"]["depends_on"] = {
            "signal_registry": {"min_version": 2}
        }
        self.assertEqual(vdm.validate_manifest(manifest), [])

    def test_exact_min_version_match_is_valid(self):
        manifest = make_manifest()
        manifest["contracts"]["signal_registry"]["version"] = 5
        manifest["contracts"]["fee_collector"]["depends_on"] = {
            "signal_registry": {"min_version": 5}
        }
        self.assertEqual(vdm.validate_manifest(manifest), [])


class ManifestMisconfigurationTests(unittest.TestCase):
    def test_missing_network_rejected(self):
        manifest = make_manifest()
        del manifest["network"]
        errors = vdm.validate_manifest(manifest)
        self.assertTrue(any("network" in e for e in errors))

    def test_empty_network_rejected(self):
        manifest = make_manifest(network="   ")
        errors = vdm.validate_manifest(manifest)
        self.assertTrue(any("network" in e for e in errors))

    def test_missing_admin_rejected(self):
        manifest = make_manifest()
        del manifest["admin"]
        errors = vdm.validate_manifest(manifest)
        self.assertTrue(any("admin" in e for e in errors))

    def test_malformed_admin_address_rejected(self):
        manifest = make_manifest(admin="NOT-A-REAL-ADDRESS")
        errors = vdm.validate_manifest(manifest)
        self.assertTrue(any("admin" in e for e in errors))

    def test_empty_contracts_rejected(self):
        manifest = make_manifest(contracts={})
        errors = vdm.validate_manifest(manifest)
        self.assertTrue(any("contracts" in e for e in errors))

    def test_missing_package_rejected(self):
        manifest = make_manifest()
        del manifest["contracts"]["signal_registry"]["package"]
        errors = vdm.validate_manifest(manifest)
        self.assertTrue(any("package" in e for e in errors))

    def test_zero_version_rejected(self):
        manifest = make_manifest()
        manifest["contracts"]["signal_registry"]["version"] = 0
        errors = vdm.validate_manifest(manifest)
        self.assertTrue(any("version" in e for e in errors))

    def test_negative_version_rejected(self):
        manifest = make_manifest()
        manifest["contracts"]["signal_registry"]["version"] = -1
        errors = vdm.validate_manifest(manifest)
        self.assertTrue(any("version" in e for e in errors))

    def test_non_integer_version_rejected(self):
        manifest = make_manifest()
        manifest["contracts"]["signal_registry"]["version"] = "two"
        errors = vdm.validate_manifest(manifest)
        self.assertTrue(any("version" in e for e in errors))

    def test_malformed_contract_address_rejected(self):
        manifest = make_manifest()
        manifest["contracts"]["signal_registry"]["address"] = "CTRUNCATED"
        errors = vdm.validate_manifest(manifest)
        self.assertTrue(any("address" in e for e in errors))

    def test_account_address_used_as_contract_address_rejected(self):
        manifest = make_manifest()
        manifest["contracts"]["signal_registry"]["address"] = VALID_ACCOUNT
        errors = vdm.validate_manifest(manifest)
        self.assertTrue(any("wrong address type" in e for e in errors))

    def test_dependency_on_unknown_contract_rejected(self):
        manifest = make_manifest()
        manifest["contracts"]["fee_collector"]["depends_on"] = {
            "does_not_exist": {"min_version": 1}
        }
        errors = vdm.validate_manifest(manifest)
        self.assertTrue(any("unknown contract" in e for e in errors))

    def test_self_dependency_rejected(self):
        manifest = make_manifest()
        manifest["contracts"]["fee_collector"]["depends_on"] = {
            "fee_collector": {"min_version": 1}
        }
        errors = vdm.validate_manifest(manifest)
        self.assertTrue(any("cannot depend on itself" in e for e in errors))

    def test_unsatisfied_min_version_rejected(self):
        manifest = make_manifest()
        manifest["contracts"]["signal_registry"]["version"] = 1
        manifest["contracts"]["fee_collector"]["depends_on"] = {
            "signal_registry": {"min_version": 2}
        }
        errors = vdm.validate_manifest(manifest)
        self.assertTrue(
            any("requires 'signal_registry' version >= 2" in e for e in errors)
        )

    def test_two_contract_cycle_rejected(self):
        manifest = make_manifest()
        manifest["contracts"]["signal_registry"]["depends_on"] = {
            "fee_collector": {"min_version": 1}
        }
        manifest["contracts"]["fee_collector"]["depends_on"] = {
            "signal_registry": {"min_version": 1}
        }
        errors = vdm.validate_manifest(manifest)
        self.assertTrue(any("circular dependency" in e for e in errors))

    def test_three_contract_cycle_rejected(self):
        manifest = make_manifest()
        manifest["contracts"]["oracle"] = {
            "package": "oracle",
            "address": None,
            "version": 1,
            "depends_on": {"signal_registry": {"min_version": 1}},
        }
        manifest["contracts"]["signal_registry"]["depends_on"] = {
            "fee_collector": {"min_version": 1}
        }
        manifest["contracts"]["fee_collector"]["depends_on"] = {
            "oracle": {"min_version": 1}
        }
        errors = vdm.validate_manifest(manifest)
        self.assertTrue(any("circular dependency" in e for e in errors))

    def test_all_problems_reported_together_not_just_first(self):
        manifest = make_manifest(admin="BAD")
        manifest["contracts"]["signal_registry"]["version"] = 0
        manifest["contracts"]["fee_collector"]["package"] = ""
        errors = vdm.validate_manifest(manifest)
        # Fail-fast means "exit nonzero immediately on any problem" at the
        # process level, but the caller should see every problem, not just
        # the first one found.
        self.assertGreaterEqual(len(errors), 3)


class ManifestFileTests(unittest.TestCase):
    def test_valid_manifest_file_round_trips(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "testnet.manifest.json"
            path.write_text(json.dumps(make_manifest()))
            self.assertEqual(vdm.validate_manifest_file(path), [])

    def test_invalid_json_reported(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "broken.manifest.json"
            path.write_text("{not valid json")
            errors = vdm.validate_manifest_file(path)
            self.assertTrue(any("invalid JSON" in e for e in errors))

    def test_non_object_root_reported(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "array.manifest.json"
            path.write_text("[1, 2, 3]")
            errors = vdm.validate_manifest_file(path)
            self.assertTrue(any("must be a JSON object" in e for e in errors))

    def test_missing_file_reported(self):
        errors = vdm.validate_manifest_file(Path("/nonexistent/path/x.manifest.json"))
        self.assertTrue(any("could not read file" in e for e in errors))


class CheckedInManifestTests(unittest.TestCase):
    """The manifest(s) actually committed to the repo must themselves be valid."""

    def test_testnet_manifest_is_valid(self):
        path = vdm.DEFAULT_DEPLOYMENTS_DIR / "testnet.manifest.json"
        self.assertTrue(path.exists(), f"expected {path} to exist")
        self.assertEqual(vdm.validate_manifest_file(path), [])


if __name__ == "__main__":
    unittest.main()
