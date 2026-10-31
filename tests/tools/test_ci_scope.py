"""Verify that CI skips only known prose and cannot hide a failed required job."""
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

script = Path(__file__).resolve().parents[2] / ".github/scripts/ci_scope.py"
spec = importlib.util.spec_from_file_location("ci_scope", script)
ci = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ci)


class ScopeTests(unittest.TestCase):
    def test_prose_allowlist_is_narrow(self):
        self.assertTrue(ci.documentation_only(["README.md", "docs/new note.md", "tasks/036.md"]))
        for paths in [[], ["docs/metrics.json"], ["specs/README.md"], ["crates/a/src/lib.rs"],
                      ["Cargo.lock"], [".github/workflows/check.yml"], ["unknown.md"],
                      ["README.md", "deploy/images/Dockerfile"], ["docs/../script.md"]]:
            with self.subTest(paths=paths):
                self.assertFalse(ci.documentation_only(paths))

    def test_manual_new_branch_and_missing_base_require_full_ci(self):
        for event, payload in [("workflow_dispatch", {}), ("unknown", {}),
                               ("push", {"before": "0" * 40}), ("pull_request", {})]:
            self.assertEqual(ci.scope(event, payload)[0], "true")
        with patch.object(ci.subprocess, "run", side_effect=OSError("unavailable")):
            self.assertEqual(ci.scope("push", {"before": "a" * 40})[0], "true")

    def test_required_gate_distinguishes_intentional_skips_from_failures(self):
        for code, expected in [("true", "success"), ("false", "skipped")]:
            results = {name: {"result": expected} for name in ci.HEAVY_JOBS}
            results["changes"] = {"result": "success"}
            self.assertTrue(ci.gate(code, results))
            for name in results:
                for bad in {"failure", "cancelled", "success", "skipped"} - {results[name]["result"]}:
                    changed = {**results, name: {"result": bad}}
                    self.assertFalse(ci.gate(code, changed), (code, name, bad))
            self.assertFalse(ci.gate("", results))
            self.assertFalse(ci.gate(code, {k: v for k, v in results.items() if k != "check"}))

    def test_real_git_diffs_include_deletions_renames_and_multiple_commits(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            def git(*args):
                return subprocess.check_output(["git", "-C", directory, *args], stderr=subprocess.PIPE).decode().strip()
            git("init", "--quiet")
            git("config", "user.name", "CI fixture")
            git("config", "user.email", "fixture@example.invalid")
            (root / "README.md").write_text("initial\n")
            (root / "code.rs").write_text("fn main() {}\n")
            git("add", ".")
            git("commit", "--quiet", "-m", "initial")
            base = git("rev-parse", "HEAD")
            def decide(event="push"):
                payload = {"before": base} if event == "push" else {"pull_request": {"base": {"sha": base}}}
                event_file, output = root / "event.json", root / "output"
                event_file.write_text(json.dumps(payload))
                output.write_text("")
                subprocess.run(["python3", str(script)], cwd=root, check=True, capture_output=True,
                               env={**os.environ, "GITHUB_EVENT_NAME": event,
                                    "GITHUB_EVENT_PATH": str(event_file), "GITHUB_OUTPUT": str(output)})
                return output.read_text().strip()
            self.assertEqual(decide(), "code=true")  # Empty diffs can force a full rerun.
            (root / "README.md").write_text("updated\n")
            git("add", "README.md")
            git("commit", "--quiet", "-m", "prose")
            self.assertEqual(decide(), "code=false")
            self.assertEqual(decide("pull_request"), "code=false")
            (root / "docs").mkdir()
            git("mv", "code.rs", "docs/code.md")
            git("commit", "--quiet", "-m", "move code into docs")
            self.assertEqual(decide(), "code=true")
            base = git("rev-parse", "HEAD")
            git("rm", "docs/code.md")
            git("commit", "--quiet", "-m", "remove prose")
            self.assertEqual(decide(), "code=false")


if __name__ == "__main__":
    unittest.main()
