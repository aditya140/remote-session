#!/usr/bin/env python3
"""Real remote observe/control/input/release/detach and SIGTERM terminal proof."""
from remote_tui_smoke import RemoteTui
from local_tui_smoke import until
from pathlib import Path
import os
import signal
import sys
import tempfile
import time
import sqlite3

def select_session(view, suffix):
    # The common app header is present on every page; wait for the actual mode
    # before sending Enter so Esc/Enter cannot become an unintended Alt key.
    deadline = time.monotonic() + 15
    while not (view.sees("Machines") and not view.sees("Sessions ·")):
        view.send(b"\x1b")
        time.sleep(0.08)
        assert time.monotonic() < deadline, "machine page did not become ready"
    until(lambda: not view.sees("Loading"))
    view.send(b"\r")
    until(lambda: view.sees("Sessions ·") and not view.sees("Loading"))
    view.send(b"/\x15" + suffix.encode() + b"\r")
    until(lambda: view.sees("Filter: " + suffix))
    def selected():
        view.sees("Sessions")
        return any("›" in line and "Running" in line and suffix in line
                   for line in view.screen.text().splitlines())
    until(lambda: view.sees("Sessions") and any("Running" in line and suffix in line
          for line in view.screen.text().splitlines()))
    deadline = time.monotonic() + 10
    while not selected():
        lines = view.screen.text().splitlines()
        target = next(index for index, line in enumerate(lines) if "Running" in line and suffix in line)
        current = next((index for index, line in enumerate(lines) if "›" in line and suffix in line), 0)
        view.send(b"\x1b[A" if target < current else b"\x1b[B")
        time.sleep(0.08)
        assert time.monotonic() < deadline, "matching live session was not selectable"
    until(selected)
    view.send(b"/\x15\r")
    until(lambda: not view.sees("Filter: ") and selected())

def select_live(view, suffix):
    select_session(view, suffix)
    view.send(b"v")
    until(lambda: view.sees("Remote terminal"))
    until(lambda: view.sees("LIVE_UI_READY"), timeout=10)

def main():
    binary, origin, token, ca, suffix = sys.argv[1:]
    with tempfile.TemporaryDirectory(prefix="qgr-live-ui-") as temporary:
        views = []
        arguments = ["--state-dir", temporary, "remote", "--gateway", origin, "--token-file", token, "--ca-bundle", ca]
        try:
            view = RemoteTui(Path(binary).resolve(), Path(temporary), arguments=arguments)
            views.append(view)
            select_session(view, suffix)
            view.send(b"/q_NO_MATCH_SESSION_927\r")
            until(lambda: view.sees("Filter: q_NO_MATCH_SESSION_927"))
            view.send(b"v")
            time.sleep(0.2)
            assert not view.sees("Remote terminal"), "hidden session must not remain a live action target"
            view.send(b"/\x15\r\x1b")
            until(lambda: view.sees("Recorded sessions"))
            select_live(view, suffix)
            assert view.sees("OBSERVING")
            view.send(b"\x1bOQ")  # F2: explicit control acquisition.
            until(lambda: view.sees("Control granted"))
            view.send(b"printf 'LIVE_UI_TYPED\\n'\r")
            until(lambda: view.sees("LIVE_UI_TYPED"), timeout=10)
            view.send(b"\x1bOR")  # F3: explicit release.
            until(lambda: view.sees("Control released"))
            view.send(b"printf 'UNSENT_READ_ONLY_MARKER\\n'\r")
            time.sleep(0.3)
            assert not view.sees("UNSENT_READ_ONLY_MARKER")
            view.send(b"\x1b[21~")  # F10: detach, leaving the native process alive.
            until(lambda: view.sees("Sessions ·"))
            view.quit(b"q")
            terminated = RemoteTui(Path(binary).resolve(), Path(temporary), arguments=arguments)
            views.append(terminated)
            select_live(terminated, suffix)
            until(lambda: terminated.sees("LIVE_UI_TYPED"))
            os.kill(terminated.pid, signal.SIGTERM)
            terminated.quit(b"")
            created = RemoteTui(Path(binary).resolve(), Path(temporary), arguments=arguments)
            views.append(created)
            select_session(created, suffix)
            created.send(b"c")
            # A PTY read can contain the new header before the busy footer. Wait
            # for the roots reply's explicit ready message before sending Enter.
            until(lambda: created.sees("Enter browses this directory; F1 selects another allowed root."))
            created.send(b"\r")
            until(lambda: created.sees("Ctrl-N creates a new terminal"))
            created.send(b"\x0e")
            until(lambda: created.sees("Remote terminal"), timeout=15)
            until(lambda: created.sees("OBSERVING"))
            created.send(b"\x1bOQ")
            until(lambda: created.sees("Control granted"))
            created.send(b"stty -echo\r")
            time.sleep(0.2)
            created.send(b"printf 'LIVE_UI_CREATED_OK\\n'\r")
            until(lambda: created.sees("LIVE_UI_CREATED_OK"), timeout=10)
            with sqlite3.connect(Path(temporary) / "start-identities.sqlite") as db:
                records = db.execute("SELECT body FROM identities").fetchall()
            assert len(records) == 1, "new terminal must have one persisted start identity"
            saved = records[0][0]
            assert '"key"' in saved and '"fingerprint"' in saved
            assert '"command"' not in saved and "LIVE_UI_CREATED_OK" not in saved
            created.send(b"\x1b[17~")  # F6, then cancel the destructive action.
            until(lambda: created.sees("y confirms"))
            created.send(b"n")
            until(lambda: created.sees("Action cancelled"))
            assert created.sees("CONTROLLING")
            created.send(b"\x1b[17~")
            until(lambda: created.sees("y confirms"))
            created.send(b"y")
            until(lambda: created.sees("Exited"), timeout=10)
            created.send(b"\x1b[21~")
            until(lambda: created.sees("Sessions ·"))
            created.quit(b"q")
            recovered = RemoteTui(Path(binary).resolve(), Path(temporary), arguments=arguments)
            views.append(recovered)
            select_session(recovered, suffix)
            recovered.send(b"k")
            until(lambda: recovered.sees("Persisted start identities"))
            until(lambda: recovered.sees("1 identities"))
            recovered.send(b"\r")
            until(lambda: recovered.sees("Remote terminal"), timeout=10)
            until(lambda: recovered.sees("LIVE_UI_CREATED_OK"))
            until(lambda: recovered.sees("Exited"))
            recovered.send(b"\x1b[21~")
            until(lambda: recovered.sees("Sessions ·"))
            recovered.quit(b"q")
            print("Remote create, observe, control, input, release, detach, confirmed termination and SIGTERM mode restoration passed")
        except Exception:
            for view in views:
                print(view.screen.text(), file=sys.stderr)
            raise
        finally:
            for view in views:
                view.cleanup()

if __name__ == "__main__":
    main()
