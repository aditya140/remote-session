#!/usr/bin/env python3
"""Start the local Keycloak deployment and use the actual standalone application."""
import argparse
import getpass
import json
import os
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "deploy/compose"))
from stack import private


def main():
    os.umask(0o077)
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--state", type=Path, default=ROOT / "target/dev-stack")
    parser.add_argument("--root", type=Path, default=ROOT / "target/dev-workspace")
    parser.add_argument("--client-bin", type=Path)
    parser.add_argument("--docker-host")
    parser.add_argument("--build-ca", type=Path)
    parser.add_argument("--skip-build", action="store_true", help="Use application images and CLI already built from this checkout")
    parser.add_argument("--device", action="store_true", help="Use a device code when the browser runs on another machine")
    parser.add_argument("command", choices=("init", "build", "up", "login", "host", "client", "status", "down"))
    args = parser.parse_args()
    state, root = args.state.resolve(), args.root.resolve()
    stack = [sys.executable, str(ROOT / "deploy/compose/stack.py"), "--state", str(state),
             "--project", "qgr-local", "--identity", "keycloak", "--build-cache", "mounts"]
    for option, value in (("--docker-host", args.docker_host), ("--build-ca", args.build_ca)):
        if value:
            stack += [option, str(value)]
    def call(command, **kwargs):
        subprocess.run(command, cwd=ROOT, check=True, **kwargs)
    call(stack + ["init"])
    root.mkdir(mode=0o700, parents=True, exist_ok=True)
    config = state / "host.json"
    settings = {"gateway": "https://localhost:8443", "os_user": getpass.getuser(),
                "name": "local-development", "hostname": "local-development",
                "roots": [str(root)], "allow_private_file": True,
                "ca_bundle": str(state / "certificates/ca.pem")}
    if not config.exists():
        private(config, json.dumps(settings, indent=2) + "\n")
    elif any(json.loads(config.read_text()).get(key) != settings[key] for key in ("gateway", "roots", "ca_bundle")):
        parser.error("existing host settings differ; review them before changing the allowed root")
    if args.client_bin:
        binary = args.client_bin.resolve()
    else:
        metadata = subprocess.check_output(["cargo", "metadata", "--no-deps", "--format-version", "1"], cwd=ROOT)
        binary = Path(json.loads(metadata)["target_directory"]) / "debug/qgenie-remote"
    if args.command == "build" or (args.command == "up" and not args.skip_build):
        if not args.client_bin:
            environment = os.environ.copy()
            environment.setdefault("CARGO_PROFILE_DEV_DEBUG", "0")
            call(["cargo", "build", "--locked", "-p", "qgenie-remote", "--features", "managed-codex"], env=environment)
        call(stack + ["build"])
    host = [str(binary), "--state-dir", str(state / "native"), "host", "--config", str(config)]
    if args.command in ("up", "status"):
        call(stack + [args.command])
    elif args.command == "login":
        call(host + ["login", "--oauth-config", str(state / "oauth.json"), "--acknowledge-recording"] + (["--device"] if args.device else []))
    elif args.command == "host":
        call(host + ["up", "--no-service"])
        call(host + ["doctor"])
    elif args.command == "client":
        call([str(binary), "--state-dir", str(state / "client"), "remote", "--gateway", "https://localhost:8443",
              "--oauth-config", str(state / "oauth.json"), "--ca-bundle", str(state / "certificates/ca.pem")] + (["--device"] if args.device else []))
    elif args.command == "down":
        if (state / "native/connector.lock").exists():
            call(host + ["down", "--keep-sessions"])
        call(stack + ["down"])
        print("Database volumes and native sessions were preserved. Terminate sessions explicitly when no longer needed.")
    if args.command in ("init", "up"):
        print("Gateway: https://localhost:8443\nKeycloak: https://localhost:9443")
        print(f"Local usernames/passwords: {state / 'users.json'}\nBrowser CA to trust: {state / 'certificates/ca.pem'}")
        print("Next: tools/dev.py login, then tools/dev.py host, then tools/dev.py client. Use the same options on each command.")


if __name__ == "__main__":
    main()
