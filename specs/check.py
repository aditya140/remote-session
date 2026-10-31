#!/usr/bin/env python3
"""Check the pinned Bend models. Installation is explicit and separate."""

import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import tomllib

ROOT = Path(__file__).resolve().parent
PIN = json.loads((ROOT / "toolchain.json").read_text())

# Each mutation changes a model while leaving its laws and proofs intact.
MUTATIONS = {
    "authorization": ("case Interrupt{}: True{}", "case Interrupt{}: True{}\n    case Approve{}: True{}"),
    "start": ("case Accepted{} Dispatch{}:", "case Unknown{} Dispatch{}: (Launching{}, Launch{})\n    case Accepted{} Dispatch{}:"),
    "approval": ("case Queued{} Deliver{True{}}:", "case Unknown{} Deliver{True{}}: (Submitted{}, SubmitNative{})\n    case Queued{} Deliver{True{}}:"),
    "control": ("Bool.and(active, Bool.and(lease_live,", "Bool.and(True{}, Bool.and(lease_live,"),
    "lifetime": ("case _: Keep{}", "case _: Kill{}"),
    "access": ("case g0 _: g0", "case Grant{d, c, r} _: Grant{1n+d, c, r}"),
    "replay": ("case EQ{}: Apply{}", "case EQ{}: Duplicate{}"),
    "recording": ("case True{} True{} True{}: Reclaimed{}", "case True{} False{} True{}: Reclaimed{}"),
    "retention": ("case False{} False{} True{}: False{}", "case True{} False{} True{}: False{}"),
    "bounds": ("(Budget{used, 0n}, Full{})", "(Budget{0n, 0n}, Full{})"),
    "capabilities": ("Bool.and(host, adapter)", "host"),
    "protocol": ("[InputTag{}, Number{s}, Number{n}, Number{h}, Number{c}]", "[InputTag{}, Number{s}, Number{n}, Number{h}]"),
    "client": ("C.eq(current_target, target)", "True{}"),
    "privacy": ("case History{content, held, count}: content", "case History{content, held, count}: count"),
    "control/lease-expiry": ("Bool.and(lease_live,", "Bool.and(True{},"),
    "control/holder": ("C.eq(holder, who)", "True{}"),
    "control/host-generation": ("C.eq(current_host, host)", "True{}"),
    "control/controller-generation": ("C.eq(current_generation, generation)", "True{}"),
    "control/eligible-activation": ("case True{} True{}: proposed", "case True{} True{}: current"),
    "access/revocation": ("case g0 _: g0", "case Grant{d, c, r} TokenRefresh{}: Grant{d, c, False{}}\n    case g0 _: g0"),
    "access/failed-clock": ("case g0 _: g0", "case Grant{d, c, r} RedisHint{}: Grant{d, True{}, r}\n    case g0 _: g0"),
    "capabilities/gateway": ("Bool.and(gateway,", "Bool.and(True{},"),
    "recording/durability": ("case True{} True{} True{}: Reclaimed{}", "case True{} True{} _: Reclaimed{}"),
    "client/fatal-recovery": ("case Fatal{}: False{}", "case Fatal{}: True{}"),
    "client/credential-owner": ("Bool.and(publisher_live,", "Bool.and(True{},"),
    "client/credential-sign-in": ("Bool.not(sign_in_required)", "True{}"),
    "client/credential-expiry": ("Bool.not(expired)", "True{}"),
    "client/failed-turn": ("case NativeFailed{}: Failed{}", "case NativeFailed{}: Completed{}"),
    "client/interrupted-turn": ("case NativeInterrupted{}: Interrupted{}", "case NativeInterrupted{}: Completed{}"),
    "client/legacy-turn": ("case LegacyUnknown{}: HistoryUnavailable{}", "case LegacyUnknown{}: Completed{}"),
    "client/turn-target": ("C.eq(target, turn)", "True{}"),
    "client/latest-turn": ("case True{}: Some{terminal_result(status)}", "case True{}: older"),
}


def run(args, cwd, env, timeout=180):
    return subprocess.run(args, cwd=cwd, env=env, text=True, stdout=subprocess.PIPE,
                          stderr=subprocess.STDOUT, timeout=timeout)


def checked(args, cwd, env):
    result = run(args, cwd, env)
    if result.returncode:
        raise RuntimeError(result.stdout.strip())
    return result.stdout.strip()


def source_hashes():
    return {
        str(path.relative_to(ROOT)): hashlib.sha256(path.read_bytes()).hexdigest()
        for path in sorted(ROOT.rglob("*"))
        if path.is_file() and path.suffix in {".bend", ".py", ".rs", ".mjs", ".md"}
        and "__pycache__" not in path.parts
    } | {"toolchain.json": hashlib.sha256((ROOT / "toolchain.json").read_bytes()).hexdigest()}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bend-source", type=Path, required=True)
    parser.add_argument("--bun", default="bun")
    parser.add_argument("--mode", choices=["kernel", "standard"], default="kernel")
    parser.add_argument("--mutations", action="store_true")
    parser.add_argument("--compare-rust", action="store_true")
    parser.add_argument("--report", type=Path)
    args = parser.parse_args()
    source = args.bend_source.resolve()
    bun = shutil.which(args.bun)
    if not bun:
        parser.error("Bun is unavailable. Install the version in specs/toolchain.json.")
    env = {**os.environ, "BEND_NO_TELEMETRY": "1"}
    actual = sorted(p.parent.name for p in ROOT.glob("*/PROOF.bend"))
    if actual != PIN["domains"]:
        raise RuntimeError("Proof directories differ from the pinned domain manifest.")
    if {name.split("/")[0] for name in MUTATIONS} != set(actual):
        raise RuntimeError("Mutation domains differ from the pinned domain manifest.")
    if checked(["git", "rev-parse", "HEAD"], source, env) != PIN["bend_commit"]:
        raise RuntimeError("Bend checkout does not match the pinned commit.")
    if checked(["git", "status", "--porcelain"], source, env):
        raise RuntimeError("Bend source must be a clean checkout.")

    # An empty working directory keeps Bun from loading the product's .env or bunfig.
    with tempfile.TemporaryDirectory(prefix="qgr-bend-check-") as temporary:
        cwd = Path(temporary)
        command = [bun, str(source / "bend2/main.ts")]
        if checked([bun, "--version"], cwd, env) != PIN["bun_version"]:
            raise RuntimeError("Bun version differs from specs/toolchain.json.")
        if checked(command + ["version"], cwd, env) != "bend " + PIN["bend_version"]:
            raise RuntimeError("Bend version differs from specs/toolchain.json.")
        report = {
            "checked_at_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
            "toolchain": PIN, "mode": args.mode, "source_sha256": source_hashes(),
            "laws": {}, "mutations": {}, "rust_comparison": "not_run",
            "client_comparison": "not_run",
        }
        if args.mode == "kernel":
            lean = checked(["lean", "--version"], cwd, env)
            if not lean.startswith("Lean (version " + PIN["lean_version"] + ","):
                raise RuntimeError("Lean version differs from specs/toolchain.json.")
            report["lean"] = lean
            # Use the kernel built from this checkout, never an inherited override.
            env.pop("BENDTT", None)
        flag = "--verdict" if args.mode == "kernel" else "--check-only"
        for domain in actual:
            output = checked(command + [str(ROOT / domain / "PROOF.bend"), flag], cwd, env)
            if "ALL PROOFS CHECK" not in output:
                raise RuntimeError(f"{domain}: checker returned no proof verdict")
            count = sum(line.startswith("law ") for line in (ROOT / domain / "LAWS.bend").read_text().splitlines())
            report["laws"][domain] = {"status": "passed", "count": count}
            print(f"PASS {domain}: {count} laws ({args.mode})", flush=True)
        if args.mutations:
            for regression, (old, new) in MUTATIONS.items():
                domain = regression.split("/")[0]
                copy = cwd / ("mutation-" + regression.replace("/", "-"))
                copy.mkdir()
                shutil.copy2(ROOT / "Common.bend", copy / "Common.bend")
                shutil.copytree(ROOT / domain, copy / domain)
                model = copy / domain / "Model.bend"
                text = model.read_text()
                # Protocol's pattern occurs in encode and decode. Change encode only.
                expected = 2 if regression == "protocol" else 1
                if text.count(old) != expected:
                    raise RuntimeError(f"{regression}: mutation anchor changed")
                model.write_text(text.replace(old, new, 1))
                result = run(command + [str(copy / domain / "PROOF.bend"), flag], cwd, env)
                if (result.returncode != 1 or "SOME PROOFS FAIL" not in result.stdout
                        or "Location: Laws." not in result.stdout):
                    raise RuntimeError(f"{regression}: expected a law failure, got:\n{result.stdout}")
                report["mutations"][regression] = "rejected_by_law"
                print(f"PASS {regression}: regression rejected", flush=True)
        if args.compare_rust:
            implementation_paths = [
                "Cargo.lock", "rust-toolchain.toml",
                "Cargo.toml",
                "conformance/schema/public-v1.json",
            ]
            for crate in ("policy", "proto", "client"):
                crate_path = ROOT.parent / "crates" / ("qgenie-remote-" + crate)
                implementation_paths.append(str((crate_path / "Cargo.toml").relative_to(ROOT.parent)))
                implementation_paths.extend(str(path.relative_to(ROOT.parent))
                                            for path in sorted((crate_path / "src").rglob("*.rs")))
            report["implementation_sha256"] = {
                name: hashlib.sha256((ROOT.parent / name).read_bytes()).hexdigest()
                for name in implementation_paths
            }
            report["rustc"] = checked(["rustc", "--version"], ROOT.parent, env)
            harness = cwd / "rust"
            (harness / "src/bin").mkdir(parents=True)
            dependencies = tomllib.loads((ROOT.parent / "Cargo.toml").read_text())["workspace"]["dependencies"]
            paths = "".join(
                f'qgenie-remote-{crate} = {{ path = '
                + json.dumps(str(ROOT.parent / "crates" / ("qgenie-remote-" + crate))) + ' }\n'
                for crate in ("policy", "proto", "client"))
            (harness / "Cargo.toml").write_text(
                '[package]\nname = "qgr-spec-conformance"\nversion = "0.0.0"\nedition = "2024"\n'
                '[profile.dev]\ndebug = 0\nincremental = false\n[dependencies]\n' + paths
                + 'serde_json = ' + json.dumps(dependencies["serde_json"]) + '\n'
                + 'tokio = { version = ' + json.dumps(dependencies["tokio"]["version"])
                + ', features = ["time"] }\n')
            # Preserve the product's dependency versions; Cargo adds only this harness.
            shutil.copy2(ROOT.parent / "Cargo.lock", harness / "Cargo.lock")
            for name in ("policy", "client"):
                shutil.copy2(ROOT / f"conformance/{name}.rs", harness / f"src/bin/{name}.rs")
            for domain, name, key in (("authorization", "policy", "rust_comparison"),
                                      ("client", "client", "client_comparison")):
                model_js = cwd / f"{name}.mjs"
                checked(command + [str(ROOT / domain / "Model.bend"), "-o", str(model_js)], cwd, env)
                result = run(["cargo", "run", "--offline", "--quiet", "--bin", name,
                              "--manifest-path", str(harness / "Cargo.toml")], ROOT.parent, env, timeout=600)
                if result.returncode:
                    raise RuntimeError("Rust comparison failed (fetch product dependencies first):\n" + result.stdout)
                rows = cwd / f"{name}.rows"
                rows.write_text(result.stdout)
                output = checked([bun, str(ROOT / f"conformance/{name}.mjs"), str(model_js), str(rows),
                                  str(ROOT.parent / "conformance/schema/public-v1.json")], cwd, env)
                report[key] = json.loads(output)
                print(f"PASS {domain}: {report[key]['combinations']:,} Rust comparisons match", flush=True)
        if args.report:
            args.report.parent.mkdir(parents=True, exist_ok=True)
            args.report.write_text(json.dumps(report, indent=2) + "\n")


if __name__ == "__main__":
    main()
