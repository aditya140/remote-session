#!/usr/bin/env python3
"""Keep required CI results present while avoiding runtime work for prose-only edits."""
import json
import os
from pathlib import PurePosixPath, Path
import re
import subprocess
import sys

ROOT_DOCUMENTS = {
    "README.md", "AGENTS.md", "PRD.md", "architecture.md", "components.md",
    "subcrate-standard.md",
}
HEAVY_JOBS = {"check", "postgres", "recording", "development-deployment"}


def documentation_only(paths):
    def prose(path):
        parts = PurePosixPath(path).parts
        return path in ROOT_DOCUMENTS or (
            len(parts) > 1 and parts[0] in {"docs", "tasks"}
            and path.endswith(".md") and ".." not in parts
        )
    return bool(paths) and all(prose(path) for path in paths)


def scope(event_name, event):
    if event_name == "push":
        base = event.get("before", "")
    elif event_name == "pull_request":
        base = event.get("pull_request", {}).get("base", {}).get("sha", "")
    else:
        return "true", "Manual or unknown event requires all checks."
    if not re.fullmatch(r"[0-9a-f]{40}", base) or base == "0" * 40:
        return "true", "No usable base commit; run all checks."
    try:
        present = subprocess.run(["git", "cat-file", "-e", base + "^{commit}"],
                                 capture_output=True, check=False)
        if present.returncode:
            subprocess.run(["git", "fetch", "--no-tags", "--depth=1", "origin", base],
                           capture_output=True, check=True, timeout=60)
        # Disable rename detection so a move from code into docs includes the
        # deleted code path. NUL separation also handles whitespace in filenames.
        paths = subprocess.check_output(
            ["git", "diff", "--no-renames", "--name-only", "-z", base, "HEAD"],
            stderr=subprocess.PIPE, timeout=30,
        ).decode("utf-8").split("\0")
        paths = [path for path in paths if path]
    except (OSError, UnicodeError, subprocess.SubprocessError):
        return "true", "Could not establish the changed paths; run all checks."
    if documentation_only(paths):
        return "false", f"All {len(paths)} changed paths are approved prose documents."
    return "true", "Code, configuration, other inputs, or an empty diff require all checks."


def gate(code, results):
    if code not in {"true", "false"} or set(results) != HEAVY_JOBS | {"changes", "interface-docs"}:
        return False
    if results["changes"].get("result") != "success":
        return False
    expected = "success" if code == "true" else "skipped"
    docs_expected = "skipped" if code == "true" else "success"
    return (results["interface-docs"].get("result") == docs_expected
            and all(results[name].get("result") == expected for name in HEAVY_JOBS))


def main():
    if len(sys.argv) == 2 and sys.argv[1] == "gate":
        results = json.loads(os.environ["RESULTS"])
        for name, job in results.items():
            print(f"{name}: {job.get('result')}")
        return 0 if gate(os.environ.get("CODE", ""), results) else 1
    try:
        event = json.loads(Path(os.environ["GITHUB_EVENT_PATH"]).read_text())
        code, reason = scope(os.environ.get("GITHUB_EVENT_NAME", ""), event)
    except (KeyError, OSError, ValueError, TypeError, AttributeError):
        code, reason = "true", "Event context unavailable; run all checks."
    print(reason)
    with open(os.environ["GITHUB_OUTPUT"], "a") as output:
        output.write(f"code={code}\n")
    if summary := os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(summary, "a") as output:
            output.write(f"Full CI required: **{code}**. {reason}\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
