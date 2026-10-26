#!/usr/bin/env python3
"""Actual keys/terminal cleanup against the owned HTTPS/PG/Redis recording fixture."""
from local_tui_smoke import Tui, until
from screen_probe import ScreenProbe
from pathlib import Path
import os
import signal
import sys
import tempfile
import re

class RemoteTui(Tui):
    def __init__(self, *args, **kwargs):
        self.screen, self.consumed = ScreenProbe(), 0
        super().__init__(*args, **kwargs)

    def sees(self, text):
        data = self.read()
        self.screen.feed(data[self.consumed:])
        self.consumed = len(data)
        return re.sub(r"\s+", "", text) in re.sub(r"\s+", "", self.screen.text())

def main():
    binary, origin, token, ca = sys.argv[1:]
    binary = Path(binary).resolve()
    with tempfile.TemporaryDirectory(prefix="qgr-remote-ui-") as directory:
        views = []
        try:
            arguments = ["remote", "--gateway", origin, "--token-file", token, "--ca-bundle", ca]
            view = RemoteTui(binary, Path(directory), arguments=arguments)
            views.append(view)
            until(lambda: view.sees("Recorded sessions"), timeout=15)
            view.send(b"\r")
            until(lambda: view.sees("Sessions ·"))
            view.send(b"b")
            until(lambda: not view.sees("Loading") and view.sees("Exited"))
            view.send(b"\r")
            until(lambda: view.sees("Recorded streams"))
            view.send(b"\r")
            until(lambda: view.sees("Recorded history · gaps remain explicit"))
            until(lambda: view.sees("GAP"))
            until(lambda: view.sees("after"))
            assert not view.sees("pipeline-secret"), "gateway redaction was absent from rendered history"
            assert b"\x1b]52;" not in view.output, "historical output escaped into a terminal clipboard command"
            view.quit(b"q")
            terminated = RemoteTui(binary, Path(directory), arguments=arguments)
            views.append(terminated)
            until(lambda: terminated.sees("Recorded sessions"), timeout=15)
            os.kill(terminated.pid, signal.SIGTERM)
            terminated.quit(b"")
            print("Remote machines/session/history keyboard flow, redacted output, explicit gaps and SIGTERM terminal restoration passed")
        except Exception:
            for view in views:
                print(view.screen.text(), file=sys.stderr)
            raise
        finally:
            for view in views:
                view.cleanup()

if __name__ == "__main__":
    main()
