#!/usr/bin/env python3
"""Advertised agent creation, native controlled prompt, structured replay and cleanup."""
from remote_tui_smoke import RemoteTui
from local_tui_smoke import until
from pathlib import Path
import sys
import tempfile
import time
import sqlite3


def main():
    binary, origin, token, ca = sys.argv[1:]
    with tempfile.TemporaryDirectory(prefix="qgr-agent-ui-") as temporary:
        arguments = ["--state-dir", temporary, "remote", "--gateway", origin,
                     "--token-file", token, "--ca-bundle", ca]
        view = RemoteTui(Path(binary).resolve(), Path(temporary), arguments=arguments)
        try:
            until(lambda: view.sees("Recorded sessions"), timeout=15)
            view.send(b"c")  # No selected session: discover roots before the first creation.
            until(lambda: view.sees("Choose an allowed root"))
            view.send(b"\r")
            until(lambda: view.sees("Ctrl-N creates a new terminal"))
            view.send(b"\x1b[15~")  # F5 chooses the advertised installed agent.
            until(lambda: view.sees("New remote agent") and view.sees("codex 0.159.2"))
            view.send(b"\x0e")
            until(lambda: view.sees("Remote agent") and view.sees("OBSERVING"), timeout=15)
            view.send(b"UNSENT_AGENT_READ_ONLY\r")
            time.sleep(0.1)
            assert not view.sees("UNSENT_AGENT_READ_ONLY")
            view.send(b"\x1bOQ")
            until(lambda: view.sees("Control granted"), timeout=10)
            view.send(b"UI_PROMPT_ONLY controlled native prompt\r")
            until(lambda: view.sees("public approved response"), timeout=15)
            view.send(b"\x1b[19~")  # F8 installs a structured checkpoint.
            until(lambda: view.sees("public approved response"))
            with sqlite3.connect(Path(temporary) / "start-identities.sqlite") as db:
                records = db.execute("SELECT body FROM identities").fetchall()
            assert len(records) == 1
            assert "UI_PROMPT_ONLY" not in records[0][0]
            view.send(b"\x1b[17~")
            until(lambda: view.sees("y confirms"))
            view.send(b"n")
            until(lambda: view.sees("Action cancelled"))
            view.send(b"\x1b[17~")
            until(lambda: view.sees("y confirms"))
            view.send(b"y")
            until(lambda: view.sees("Exited"), timeout=10)
            view.send(b"\x1b[21~")
            until(lambda: view.sees("Sessions ·"))
            view.quit(b"q")
            recovered = RemoteTui(Path(binary).resolve(), Path(temporary), arguments=arguments)
            try:
                until(lambda: recovered.sees("Recorded sessions"), timeout=15)
                recovered.send(b"k")
                until(lambda: recovered.sees("1 identities"))
                recovered.send(b"\r")
                until(lambda: recovered.sees("Remote agent"), timeout=15)
                until(lambda: recovered.sees("public approved response") and recovered.sees("Exited"))
                recovered.send(b"\x1b[21~")
                until(lambda: recovered.sees("Sessions ·"))
                deadline = time.monotonic() + 12
                while True:
                    recovered.send(b"\x1b")
                    until(lambda: recovered.sees("Machines") and not recovered.sees("Sessions ·") and not recovered.sees("Loading"))
                    recovered.send(b"\r")
                    until(lambda: recovered.sees("Sessions ·"))
                    recovered.send(b"b")
                    until(lambda: not recovered.sees("Loading"))
                    recovered.send(b"/\x15Exited\r")
                    until(lambda: recovered.sees("Filter: Exited"))
                    recovered.send(b"u")
                    time.sleep(0.15)
                    if recovered.sees("Resume agent history"):
                        break
                    assert time.monotonic() < deadline, "confirmed historical resume was not offered"
                recovered.send(b"\r")
                until(lambda: recovered.sees("Ctrl-N resumes history"))
                recovered.send(b"\x0e")
                until(lambda: recovered.sees("Remote agent") and recovered.sees("OBSERVING"), timeout=15)
                until(lambda: recovered.sees("PARTIAL HISTORY"))
                recovered.send(b"\x1bOQ")
                until(lambda: recovered.sees("Control granted"))
                recovered.send(b"UI_PROMPT_ONLY resumed native conversation\r")
                until(lambda: recovered.sees("public approved response"), timeout=15)
                recovered.send(b"\x1b[17~")
                until(lambda: recovered.sees("y confirms"))
                recovered.send(b"y")
                until(lambda: recovered.sees("Exited"), timeout=10)
                with sqlite3.connect(Path(temporary) / "start-identities.sqlite") as db:
                    assert db.execute("SELECT count(*) FROM identities").fetchone()[0] == 2
                recovered.send(b"\x1b[21~")
                until(lambda: recovered.sees("Sessions ·"))
                recovered.quit(b"q")
            except Exception:
                print(recovered.screen.text(), file=sys.stderr)
                raise
            finally:
                recovered.cleanup()
            print("Actual advertised agent/create/prompt/checkpoint/recovery/historical resume/terminate and terminal restoration passed")
        except Exception:
            print(view.screen.text(), file=sys.stderr)
            raise
        finally:
            view.cleanup()


if __name__ == "__main__":
    main()
