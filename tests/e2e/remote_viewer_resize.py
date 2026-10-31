#!/usr/bin/env python3
"""Reconnect real observers and resize their windows without native control."""
from remote_tui_smoke import RemoteTui
from remote_live_tui_smoke import select_live
from local_tui_smoke import until
from screen_probe import ScreenProbe
from pathlib import Path
import fcntl
import os
import signal
import struct
import sys
import tempfile
import termios


def main():
    binary, origin, token, ca, suffix = sys.argv[1:]
    with tempfile.TemporaryDirectory(prefix="qgr-viewer-resize-") as temporary:
        arguments = ["--state-dir", temporary, "remote", "--gateway", origin,
                     "--token-file", token, "--ca-bundle", ca]
        for rows, columns in [(38, 128), (28, 110)]:
            view = RemoteTui(Path(binary).resolve(), Path(temporary), arguments=arguments)
            try:
                select_live(view, suffix)
                until(lambda: view.sees("OBSERVING"))
                view.screen = ScreenProbe(rows, columns)
                fcntl.ioctl(view.fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, columns, 0, 0))
                os.kill(view.pid, signal.SIGWINCH)
                view.send(b"\x1b[19~")  # F8 requests a read-only checkpoint.
                until(lambda: view.sees("LIVE_UI_READY") and view.sees("OBSERVING"))
                view.send(b"\x1b[21~")  # F10 leaves the native session alive.
                until(lambda: view.sees("Sessions ·"))
                view.quit(b"q")
            finally:
                view.cleanup()
    print("Two real observer connections resized their own windows without taking control")


if __name__ == "__main__":
    main()
