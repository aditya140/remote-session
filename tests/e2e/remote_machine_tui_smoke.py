#!/usr/bin/env python3
"""Actual owner machine metadata and explicit revocation keys on an owned fixture."""
from remote_tui_smoke import RemoteTui
from local_tui_smoke import until
from pathlib import Path
import json
import subprocess
import sys
import tempfile
import time


def main():
    binary, origin, token, ca, machine = sys.argv[1:]
    binary = Path(binary).resolve()
    arguments = ["remote", "--gateway", origin, "--token-file", token, "--ca-bundle", ca]
    def inventory():
        result = subprocess.run([str(binary), *arguments, "machines"], check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=15)
        return next((item for item in json.loads(result.stdout) if item["id"] == machine), None)
    original = inventory()
    assert original is not None
    with tempfile.TemporaryDirectory(prefix="qgr-machine-ui-") as temporary:
        view = RemoteTui(binary, Path(temporary), arguments=arguments)
        def open_dialog(keys, title):
            for _ in range(20):
                view.send(keys)
                time.sleep(0.05)
                if view.sees(title):
                    return
            raise AssertionError("machine action dialog did not open: " + title)
        def returned():
            view.send(b"\x1b")
            until(lambda: view.sees("Recorded sessions"))
        try:
            until(lambda: view.sees("Recorded sessions"), timeout=15)
            view.send(b"/q_NO_MATCH_FILTER_928\r")
            until(lambda: view.sees("Filter: q_NO_MATCH_FILTER_928"))
            view.send(b"\x1bOQ\x1b[17~")
            time.sleep(0.2)
            assert not view.sees("Rename machine") and not view.sees("Revoke machine access"), "hidden rows must not remain action targets"
            assert inventory()["name"] == original["name"]
            view.send(b"/\x15\r")
            open_dialog(b"\x1bOQ", "Rename machine")
            view.send(b"\x15UNSENT_MACHINE_NAME\x1b")
            until(lambda: view.sees("Recorded sessions"))
            assert inventory()["name"] == original["name"]
            open_dialog(b"\x1bOQ", "Rename machine")
            view.send(b"\x15machine-ui-renamed\r")
            until(lambda: view.sees("Machine metadata saved"), timeout=15)
            returned()
            until(lambda: view.sees("machine-ui-renamed"))
            assert inventory()["name"] == "machine-ui-renamed"
            open_dialog(b"\x1bOR", "Edit machine label · key")
            view.send(b"ui_fixture\r")
            until(lambda: view.sees("Edit machine label · value"))
            view.send(b"verified\r")
            until(lambda: view.sees("Machine metadata saved"), timeout=15)
            returned()
            assert inventory()["labels"]["ui_fixture"] == "verified"
            view.send(b"/verified\r")
            until(lambda: view.sees("Filter: verified") and view.sees("machine-ui-renamed"))
            view.send(b"/\x15\r")
            open_dialog(b"\x1bOR", "Edit machine label · key")
            view.send(b"ui_fixture\r")
            until(lambda: view.sees("Edit machine label · value"))
            view.send(b"\x04")
            until(lambda: view.sees("Remove machine label"))
            view.send(b"n")
            until(lambda: view.sees("Recorded sessions"))
            assert inventory()["labels"]["ui_fixture"] == "verified"
            open_dialog(b"\x1bOR", "Edit machine label · key")
            view.send(b"ui_fixture\r")
            until(lambda: view.sees("Edit machine label · value"))
            view.send(b"\x04")
            until(lambda: view.sees("Remove machine label"))
            view.send(b"y")
            until(lambda: view.sees("Machine metadata saved"), timeout=15)
            returned()
            assert "ui_fixture" not in inventory()["labels"]
            open_dialog(b"\x1b[17~", "Revoke machine access")
            view.send(b"n")
            until(lambda: view.sees("Recorded sessions"))
            assert inventory() is not None
            open_dialog(b"\x1b[17~", "Revoke machine access")
            view.send(b"y")
            until(lambda: view.sees("Machine remote access revoked"), timeout=15)
            returned()
            assert inventory() is None
            view.quit(b"q")
            print("Actual owner rename/label/remove/cancel/revoke UI and terminal restoration passed")
        except Exception:
            print(view.screen.text(), file=sys.stderr)
            raise
        finally:
            view.cleanup()


if __name__ == "__main__":
    main()
