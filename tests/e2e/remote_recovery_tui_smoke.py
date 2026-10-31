#!/usr/bin/env python3
"""Real keys against the TLS peer owned by the client_workflow integration test."""
from remote_tui_smoke import RemoteTui
from local_tui_smoke import until
from pathlib import Path
import os
import signal
import ssl
import sys
import tempfile
import time
import urllib.request


def main():
    binary, origin, token, ca = sys.argv[1:]
    with tempfile.TemporaryDirectory(prefix="qgr-recovery-ui-") as directory:
        args = ["--state-dir", directory, "remote", "--gateway", origin,
                "--token-file", token, "--ca-bundle", ca]
        view = RemoteTui(Path(binary).resolve(), Path(directory), arguments=args)
        try:
            until(lambda: view.sees("Recorded sessions"))
            view.send(b"\r")
            until(lambda: view.sees("Sessions · live") and not view.sees("Loading"))
            view.send(b"!")
            until(lambda: view.sees("needs attention"))
            assert view.sees("fixture recording warning")
            view.send(b"v")
            until(lambda: view.sees("Activity: turn complete"))
            assert view.sees("OBSERVING")
            view.send(b"\x1bOQ")
            until(lambda: view.sees("Control granted"))
            assert view.sees("CONTROLLING")
            # Leave an unsent draft in the editor before losing the connection.
            view.send(b"UNSENT_DRAFT")
            until(lambda: view.sees("UNSENT_DRAFT"))
            request = urllib.request.Request(origin + "/drop", data=b"", method="POST")
            with urllib.request.urlopen(request, context=ssl.create_default_context(cafile=ca), timeout=3):
                pass
            until(lambda: view.sees("Observation reconnected"))
            assert view.sees("OBSERVING")
            assert view.sees("readable result")
            assert not view.sees("UNSENT_DRAFT")
            view.send(b"SHOULD_NOT_SEND\r")
            time.sleep(0.15)
            assert not view.sees("SHOULD_NOT_SEND")
            os.kill(view.pid, signal.SIGTERM)
            view.quit(b"")
            print("Attention filter, output recovery, control loss, and terminal restoration passed")
        except Exception:
            print(view.screen.text(), file=sys.stderr)
            raise
        finally:
            view.cleanup()


if __name__ == "__main__":
    main()
