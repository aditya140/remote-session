#!/usr/bin/env python3
"""Actual owner approval keys, cancel/confirm and terminal restoration."""
from remote_tui_smoke import RemoteTui
from local_tui_smoke import until
from pathlib import Path
import sys
import time
import tempfile


def main():
    binary, origin, token, ca, approval, machine, session = sys.argv[1:]
    arguments = ["remote", "--gateway", origin, "--token-file", token, "--ca-bundle", ca]
    with tempfile.TemporaryDirectory(prefix="qgr-approval-ui-") as temporary:
        view = RemoteTui(Path(binary).resolve(), Path(temporary), arguments=arguments)
        try:
            until(lambda: view.sees("Recorded sessions"), timeout=15)
            until(lambda: view.sees("Approvals: 1 pending"), timeout=15)
            assert view.sees("codex 0.159.2") and view.sees("CLI 0.1.0")
            view.send(b"/\x15" + machine.encode() + b"\r")
            until(lambda: view.sees("Filter: " + machine))
            view.send(b"\r")
            until(lambda: view.sees("Sessions · recorded history") and not view.sees("Loading"))
            view.send(b"/\x15" + session.encode() + b"\r")
            until(lambda: view.sees("Filter: " + session))
            view.send(b"v")
            until(lambda: view.sees("Remote agent") and view.sees("OBSERVING"))
            view.send(b"\x1b[18~")  # F7 opens approvals while observing, with no lease.
            until(lambda: view.sees("Owner approvals"), timeout=10)
            until(lambda: view.sees("Policy revision:"), timeout=10)
            for _ in range(200):
                if view.sees(approval):
                    break
                view.send(b"\x1b[B")
                time.sleep(0.05)
            until(lambda: view.sees(approval) and view.sees("1: AllowOnce"))
            view.send(b"1")
            until(lambda: view.sees("Commit AllowOnce?"))
            view.send(b"n")
            until(lambda: view.sees("1: AllowOnce"))
            view.send(b"1")
            until(lambda: view.sees("Commit AllowOnce?"))
            view.send(b"y")
            until(lambda: view.sees("OutcomeUnknown") or view.sees("Applied"), timeout=20)
            assert not view.sees("1: AllowOnce"), "resolved request became actionable again"
            view.send(b"1y")
            view.send(b"r")
            until(lambda: view.sees("OutcomeUnknown") or view.sees("Applied"))
            view.send(b"\x1b")
            until(lambda: view.sees("Remote agent"))
            assert not view.sees("CONTROLLING")
            view.send(b"\x1b[21~")
            until(lambda: view.sees("Sessions · recorded history"))
            until(lambda: view.sees("Approvals: 0 pending"), timeout=15)
            view.quit(b"q")
            print("Actual owner inbox/cancel/confirm/status and terminal restoration passed")
        except Exception:
            print(view.screen.text(), file=sys.stderr)
            raise
        finally:
            view.cleanup()


if __name__ == "__main__":
    main()
