"""Tests for scripts/check_error_abi.py (issue #1222).

Run: python3 -m unittest discover -s stellar-swipe/scripts/tests -p 'test_check_error_abi.py'
"""

import copy
import json
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import check_error_abi as abi  # noqa: E402

SOURCE = textwrap.dedent('''
    use soroban_sdk::contracterror;

    // #[contracterror] in a comment is ignored.
    const MSG: &str = "duplicate #[contracterror] discriminant";

    #[contracterror]
    #[derive(Copy, Clone, Debug, Eq, PartialEq)]
    #[repr(u32)]
    pub enum DemoError {
        /// Doc comment with = 99 in it.
        Unauthorized = 1,
        NotFound = 2, // trailing comment
        #[allow(dead_code)]
        Overflow = 0x10,
    }
''')


def write_crate(root, crate, files):
    for name, body in files.items():
        path = root / crate / "src" / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(body)


class ParserTests(unittest.TestCase):
    def test_parses_variants_ignoring_comments_strings_and_attributes(self):
        enums, errors = abi.parse_error_enums(SOURCE, "demo.rs")
        self.assertEqual(errors, [])
        self.assertEqual(enums, {"DemoError": {"Unauthorized": 1, "NotFound": 2, "Overflow": 16}})

    def test_duplicate_code_in_enum_is_an_error(self):
        src = "#[contracterror]\npub enum E { A = 1, B = 1 }"
        _, errors = abi.parse_error_enums(src, "x.rs")
        self.assertTrue(any("code 1 used by both A and B" in e for e in errors))

    def test_missing_discriminant_is_an_error(self):
        _, errors = abi.parse_error_enums("#[contracterror]\npub enum E { A = 1, B }", "x.rs")
        self.assertTrue(any("E::B has no explicit discriminant" in e for e in errors))

    def test_same_enum_name_in_two_modules_is_qualified(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write_crate(root, "shared", {
                "lib.rs": "",
                "a.rs": "#[contracterror]\npub enum CapError { X = 1 }",
                "b/mod.rs": "#[contracterror]\npub enum CapError { Y = 1 }",
            })
            source, errors = abi.collect_source_abi(root)
            self.assertEqual(errors, [])
            self.assertEqual(source, {"shared": {"a::CapError": {"X": 1}, "b::CapError": {"Y": 1}}})

    def test_real_source_matches_committed_fixture(self):
        source, errors = abi.collect_source_abi()
        self.assertEqual(errors, [])
        fixture = abi.load_fixture(abi.FIXTURE)
        self.assertEqual(abi.diff_abi(fixture["crates"], source), ([], []))


class DiffTests(unittest.TestCase):
    OLD = {"c": {"E": {"A": 1, "B": 2, "C": 3}}}

    def diff(self, mutate):
        new = copy.deepcopy(self.OLD)
        mutate(new["c"])
        return abi.diff_abi(self.OLD, new)

    def test_no_change(self):
        self.assertEqual(abi.diff_abi(self.OLD, self.OLD), ([], []))

    def test_renumbering_is_breaking(self):
        breaking, _ = self.diff(lambda c: c["E"].update(B=20))
        self.assertEqual(breaking, ["c::E::B renumbered 2 -> 20"])

    def test_removal_is_breaking(self):
        breaking, _ = self.diff(lambda c: c["E"].pop("C"))
        self.assertEqual(breaking, ["c::E::C (3) removed"])

    def test_rename_is_removal_plus_reuse(self):
        def rename(c):
            c["E"].pop("B")
            c["E"]["Bee"] = 2
        breaking, _ = self.diff(rename)
        self.assertEqual(breaking, ["c::E::B (2) removed", "c::E::Bee reuses code 2 of B"])

    def test_enum_removal_is_breaking(self):
        breaking, _ = self.diff(lambda c: c.pop("E"))
        self.assertEqual(breaking, ["c::E: enum removed"])

    def test_additions_are_additive(self):
        def add(c):
            c["E"]["D"] = 4
            c["F"] = {"X": 1}
        breaking, additive = self.diff(add)
        self.assertEqual(breaking, [])
        self.assertEqual(additive, ["c::E::D = 4 added", "c::F: new enum"])


class CliTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.contracts = self.root / "contracts"
        self.fixture = self.root / "abi.json"
        self.set_source("A = 1, B = 2")
        self.assertEqual(self.run_cli("--update"), 0)

    def tearDown(self):
        self.tmp.cleanup()

    def set_source(self, variants):
        write_crate(self.contracts, "demo", {"lib.rs": f"#[contracterror]\npub enum E {{ {variants} }}"})

    def run_cli(self, *extra):
        return abi.main(["--contracts-dir", str(self.contracts), "--fixture", str(self.fixture), *extra])

    def test_clean_source_passes(self):
        self.assertEqual(self.run_cli(), 0)

    def test_accidental_renumbering_fails_and_update_refuses_it(self):
        self.set_source("A = 1, B = 3")
        self.assertEqual(self.run_cli(), 1)
        self.assertEqual(self.run_cli("--update"), 1)
        self.assertEqual(self.run_cli("--update", "--version", "1.1.0", "--note", "n"), 1)
        self.assertEqual(json.loads(self.fixture.read_text())["crates"]["demo"]["E"]["B"], 2)

    def test_new_code_leaves_fixture_stale_until_updated(self):
        self.set_source("A = 1, B = 2, C = 3")
        self.assertEqual(self.run_cli(), 1)
        self.assertEqual(self.run_cli("--update"), 0)
        self.assertEqual(self.run_cli(), 0)

    def test_intentional_break_is_recorded_with_version_and_note(self):
        self.set_source("A = 1")
        note = "B (2) removed; clients map it to A"
        self.assertEqual(self.run_cli("--update", "--version", "2.0.0", "--note", note), 0)
        fixture = json.loads(self.fixture.read_text())
        self.assertEqual(fixture["abi_version"], "2.0.0")
        self.assertEqual(fixture["migrations"], [
            {"version": "2.0.0", "note": note, "changes": ["demo::E::B (2) removed"]},
        ])
        self.assertEqual(self.run_cli(), 0)


class HistoryTests(unittest.TestCase):
    BASE = {"abi_version": "1.0.0", "crates": {"c": {"E": {"A": 1, "B": 2}}}, "migrations": []}

    def test_hand_edited_fixture_is_caught(self):
        edited = copy.deepcopy(self.BASE)
        edited["crates"]["c"]["E"]["B"] = 5
        problems = abi.check_history(edited, self.BASE)
        self.assertTrue(any("without a major version bump" in p for p in problems))
        self.assertIn("undeclared breaking change: c::E::B renumbered 2 -> 5", problems)

    def test_declared_break_passes(self):
        change = "c::E::B renumbered 2 -> 5"
        edited = copy.deepcopy(self.BASE)
        edited["crates"]["c"]["E"]["B"] = 5
        edited["abi_version"] = "2.0.0"
        edited["migrations"] = [{"version": "2.0.0", "note": "why", "changes": [change]}]
        self.assertEqual(abi.check_history(edited, self.BASE), [])

    def test_additions_need_no_declaration(self):
        edited = copy.deepcopy(self.BASE)
        edited["crates"]["c"]["E"]["C"] = 3
        self.assertEqual(abi.check_history(edited, self.BASE), [])


if __name__ == "__main__":
    unittest.main()
