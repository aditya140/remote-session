#!/usr/bin/env python3
"""Conservative checks for pinned CLI and wire contracts. No network or application dispatch."""
import hashlib
import json
from pathlib import Path
import sys

class Incompatible(ValueError):
    pass

def require(ok, path, message):
    if not ok:
        raise Incompatible(f"{path}: {message}")

def branch_key(branch):
    if not isinstance(branch, dict):
        return json.dumps(branch, sort_keys=True)
    tags = {k: v.get("const", v.get("enum")) for k, v in branch.get("properties", {}).items()
            if "const" in v or "enum" in v}
    return json.dumps({k: branch[k] for k in ("type", "$ref", "const") if k in branch} | {"tags": tags}, sort_keys=True)

def schema(old, new, path="", enum_additions=None):
    enum_additions = enum_additions or {}
    if not isinstance(old, dict) or not isinstance(new, dict):
        require(old == new, path, "schema changed")
        return
    for keyword in ("type", "$ref", "const", "format", "pattern", "minimum", "maximum",
                    "exclusiveMinimum", "exclusiveMaximum", "minLength", "maxLength",
                    "minItems", "maxItems", "additionalProperties", "unevaluatedProperties"):
        require(old.get(keyword) == new.get(keyword), path, f"{keyword} changed")
    require(set(old.get("required", [])) == set(new.get("required", [])), path, "required fields changed")
    if "enum" in old or "enum" in new:
        before, after = set(old.get("enum", [])), set(new.get("enum", []))
        require(before <= after, path, "enum member removed")
        require(after - before <= set(enum_additions.get(path, [])), path, "unreviewed enum result added")
    for keyword in ("$defs", "properties"):
        for name, value in old.get(keyword, {}).items():
            require(name in new.get(keyword, {}), path, f"{keyword}/{name} removed")
            schema(value, new[keyword][name], f"{path}/{keyword}/{name}", enum_additions)
    if "items" in old or "items" in new:
        schema(old.get("items"), new.get("items"), path + "/items", enum_additions)
    handled = {"$defs", "properties", "items", "required", "enum", "oneOf", "anyOf", "allOf"}
    annotations = {"title", "description", "$schema", "$id", "default", "examples", "deprecated"}
    for keyword in (old.keys() | new.keys()) - handled - annotations:
        require(old.get(keyword) == new.get(keyword), path, f"{keyword} changed")
    for keyword in ("oneOf", "anyOf", "allOf"):
        require((keyword in old) == (keyword in new), path, f"{keyword} constraint changed")
        if keyword == "allOf" and keyword in old:
            require(len(old[keyword]) == len(new[keyword]), path, "allOf constraints changed")
        if keyword in old:
            available = {}
            for branch in new.get(keyword, []):
                available.setdefault(branch_key(branch), []).append(branch)
            for index, branch in enumerate(old[keyword]):
                candidates = available.get(branch_key(branch), [])
                require(candidates, path, f"{keyword} variant removed or changed")
                errors = []
                for candidate in candidates:
                    try:
                        schema(branch, candidate, f"{path}/{keyword}/{index}", enum_additions)
                        break
                    except Incompatible as error:
                        errors.append(str(error))
                else:
                    raise Incompatible(errors[0])

def cli(old, new):
    commands = {c["path"]: c for c in new["commands"]}
    for before in old["commands"]:
        path = before["path"]
        require(path in commands, path, "command removed")
        after = commands[path]
        require(before["subcommand_required"] == after["subcommand_required"], path, "subcommand requirement changed")
        args = {a["id"]: a for a in after["arguments"]}
        prior = {a["id"] for a in before["arguments"]}
        for arg in before["arguments"]:
            require(arg["id"] in args, path, f"argument {arg['id']} removed")
            require(arg == args[arg["id"]], path, f"argument {arg['id']} changed")
        for arg in after["arguments"]:
            require(arg["id"] in prior or not arg["required"], path, f"new required argument {arg['id']}")

def check(root):
    manifest = json.loads((root / "conformance/compatibility/baselines.json").read_text())
    require(manifest["format"] == 1, "manifest", "unsupported format")
    names = set()
    versions = set()
    for baseline in manifest["baselines"]:
        name = baseline["name"]
        require(name not in names and Path(name).name == name and name not in {".", ".."}, "manifest", "duplicate or invalid baseline name")
        names.add(name)
        require(baseline["status"] in {"preview", "released"}, name, "unknown status")
        require(set(baseline["files"]) == {"cli.json", "public-v1.json", "rest-v1.json", "recording-v1.json"}, name, "baseline must include all four contracts")
        if baseline["status"] == "released":
            version = baseline.get("version")
            require(isinstance(version, str) and version and version not in versions, name, "missing or duplicate release version")
            versions.add(version)
    releases = [b for b in manifest["baselines"] if b["status"] == "released"]
    active = [b for b in manifest["baselines"] if b["status"] == "preview"] + releases[-3:]
    require(active, "manifest", "no compatibility baseline")
    for baseline in active:
        require(len(baseline["revision"]) == 40 and all(c in "0123456789abcdef" for c in baseline["revision"]), baseline["name"], "invalid baseline revision")
        directory = root / "conformance/compatibility" / baseline["name"]
        for name, digest in baseline["files"].items():
            path = directory / name
            require(hashlib.sha256(path.read_bytes()).hexdigest() == digest, str(path), "baseline digest changed")
            before = json.loads(path.read_text())
            if name == "cli.json":
                cli(before, json.loads((root / "conformance/cli/current.json").read_text()))
            elif name.endswith("-v1.json"):
                schema(before, json.loads((root / "conformance/schema" / name).read_text()),
                       enum_additions=baseline.get("reviewed_enum_additions", {}).get(name, {}))
        print(f"Compatible with {baseline['status']} baseline {baseline['name']} ({baseline['revision']})")
    if not releases:
        print("No released baselines exist. This does not establish N-2 released-client qualification.")

if __name__ == "__main__":
    try:
        check(Path(__file__).resolve().parents[1])
    except (Incompatible, KeyError, OSError, ValueError) as error:
        sys.exit(str(error))
