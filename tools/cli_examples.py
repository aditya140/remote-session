#!/usr/bin/env python3
"""Extract shell command examples as argv. Never execute a documented command."""
import json
from pathlib import Path
import re
import shlex
import sys

UUID = "550e8400-e29b-41d4-a716-446655440001"
REPLACEMENTS = {
    "OTHER_MACHINE_ID": "m_550e8400-e29b-41d4-a716-446655440002",
    "OTHER_SESSION_ID": "s_550e8400-e29b-41d4-a716-446655440002",
    "MACHINE_ID": "m_" + UUID, "SESSION_ID": "s_" + UUID,
    "MACHINE": "m_" + UUID, "SESSION": "s_" + UUID,
    "KEY": "idem_" + UUID, "TURN_ID": "example-turn",
}

def examples(root):
    result = []
    for path in sorted([root / "README.md", *root.joinpath("docs").rglob("*.md")]):
        if not path.is_file():
            continue
        lines = path.read_text().splitlines()
        shell = False
        index = 0
        while index < len(lines):
            line = lines[index].strip()
            index += 1
            if line.startswith("```"):
                shell = line[3:].strip() in {"sh", "shell", "bash", "console"}
                continue
            if not shell or not re.match(r"(?:target/(?:debug|release)/)?qgenie-remote(?:\s|$)", line):
                continue
            start = index
            while line.endswith("\\"):
                if index >= len(lines):
                    raise ValueError(f"Unfinished command: {path}:{start}")
                line = line[:-1] + lines[index].strip()
                index += 1
            args = shlex.split(line, comments=True)
            args[0] = "qgenie-remote"
            for n, arg in enumerate(args):
                for placeholder, value in REPLACEMENTS.items():
                    arg = re.sub(r"\b" + placeholder + r"\b", value, arg)
                args[n] = arg
            result.append({"file": str(path.relative_to(root)), "line": start, "args": args})
    return result

if __name__ == "__main__":
    print(json.dumps(examples(Path(sys.argv[1]).resolve())))
