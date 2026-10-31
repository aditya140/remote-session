"""Contract failures must be detected without starting an application."""
import copy
import importlib.util
from pathlib import Path
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
def module(name):
    spec = importlib.util.spec_from_file_location(name, ROOT / "tools" / (name + ".py"))
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result

compat = module("check_compatibility")
examples = module("cli_examples")

class InterfaceTests(unittest.TestCase):
    def test_wire_changes_require_preserved_fields_types_and_required_inputs(self):
        old = {"type": "object", "properties": {"value": {"type": "string"}, "option": {"type": "boolean"}}, "required": ["value"]}
        additive = copy.deepcopy(old)
        additive["properties"]["extra"] = {"type": "string"}
        compat.schema(old, additive)
        for change in [lambda v: v["properties"].pop("value"),
                       lambda v: v["properties"]["value"].update(type="integer"),
                       lambda v: v["required"].append("option"),
                       lambda v: v["required"].clear(),
                       lambda v: v.update(allOf=[{"required": ["option"]}]),
                       lambda v: v.update({"not": {"required": ["value"]}})]:
            changed = copy.deepcopy(old)
            change(changed)
            with self.assertRaises(compat.Incompatible):
                compat.schema(old, changed)
        with self.assertRaises(compat.Incompatible):
            compat.schema({"enum": ["ok", "failed"]}, {"enum": ["ok"]})
        with self.assertRaises(compat.Incompatible):
            compat.schema({"enum": ["ok"]}, {"enum": ["ok", "new"]})
        compat.schema({"enum": ["ok"]}, {"enum": ["ok", "new"]}, enum_additions={"": ["new"]})

    def test_removed_commands_flags_and_new_required_values_fail(self):
        old = {"commands": [{"path": "qgenie-remote remote read", "subcommand_required": False,
                              "arguments": [{"id": "lines", "long": "lines", "required": False}]}]}
        compat.cli(old, copy.deepcopy(old))
        for change in [lambda v: v["commands"].clear(),
                       lambda v: v["commands"][0]["arguments"].clear(),
                       lambda v: v["commands"][0]["arguments"][0].update(long="count"),
                       lambda v: v["commands"][0]["arguments"].append({"id": "new", "required": True})]:
            changed = copy.deepcopy(old)
            change(changed)
            with self.assertRaises(compat.Incompatible):
                compat.cli(old, changed)

    def test_command_extraction_is_read_only_and_keeps_quoted_literals(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "docs").mkdir()
            (root / "docs/example.md").write_text("```sh\nqgenie-remote remote wait-output MACHINE SESSION \\\n --after 0 --literal '$(touch should-not-exist); ready'\n```\n```text\nqgenie-remote invalid-proposal\n```\n")
            found = examples.examples(root)
            self.assertEqual(len(found), 1)
            self.assertEqual(found[0]["args"][-1], "$(touch should-not-exist); ready")
            self.assertTrue(found[0]["args"][3].startswith("m_"))
            self.assertFalse((root / "should-not-exist").exists())

    def test_pinned_repository_baselines(self):
        compat.check(ROOT)

if __name__ == "__main__":
    unittest.main()
