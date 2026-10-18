#!/usr/bin/env python3
"""Check the actual selected binary's complete generated schema against the pinned proof."""
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile

binary = Path(sys.argv[1]).resolve()
assert subprocess.check_output([str(binary), "--version"], text=True).strip() == "codex-cli 0.159.2"
root = Path(__file__).resolve().parents[2]
expected = json.loads((root / "conformance/codex/0.159.2/schema-sha256.json").read_text())
with tempfile.TemporaryDirectory(prefix="qgr-schema-") as directory:
    subprocess.run([str(binary), "app-server", "generate-json-schema", "--out", directory], check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    generated = Path(directory)
    actual = {str(path.relative_to(generated)): hashlib.sha256(path.read_bytes()).hexdigest() for path in sorted(generated.rglob("*.json"))}
    assert actual == expected, "Native schemas changed; review the adapter and re-run real contract proofs."
print(f"PASS: pinned Codex 0.159.2 and {len(expected)} generated native schema hashes")
