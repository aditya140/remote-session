#!/usr/bin/env python3
"""Exercise real TUI keys and session survival through separate processes.

Usage: python3 tests/e2e/local_tui_smoke.py target/debug/qgenie-remote
This is a local preview test, not gateway or native service evidence.
"""
import base64
import fcntl
import json
import os
from pathlib import Path
import pty
import re
import select
import signal
import socket
import struct
import subprocess
import sys
import tempfile
import termios
import time
import uuid


def exact(stream, count):
    result = b""
    while len(result) < count:
        piece = stream.recv(count - len(result))
        if not piece:
            raise RuntimeError("IPC reply ended early")
        result += piece
    return result


def exchange(state, endpoint, payload):
    with socket.socket(socket.AF_UNIX) as stream:
        stream.settimeout(3)
        stream.connect(str(state / endpoint))
        def send(value):
            data = json.dumps(value).encode()
            stream.sendall(struct.pack(">I", len(data)) + data)

        def receive():
            length = struct.unpack(">I", exact(stream, 4))[0]
            assert 0 < length <= 1024 * 1024
            return json.loads(exact(stream, length))

        if endpoint == "dispatch.sock":
            send({"type": "hello", "body": {"major": 1, "minor": 0, "capabilities": ["local-v1"]}})
            assert receive() == {"type": "welcome", "body": {"major": 1, "minor": 0}}
            send({"type": "local", "body": payload})
            response = receive()
            assert response["type"] == "local_reply"
            return response["body"]
        send(payload)
        return receive()


def request(state, kind, body=None):
    operation = {"type": kind}
    if body is not None:
        operation["body"] = body
    response = exchange(state, "dispatch.sock", {
        "v": 1, "id": "req_" + str(uuid.uuid4()),
        "client": "client_" + str(uuid.uuid4()), "body": operation,
    })["body"]
    if response["type"] == "error":
        raise RuntimeError(response["body"]["message"])
    return response.get("body")


def until(callback, timeout=8):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        result = callback()
        if result:
            return result
        time.sleep(0.02)
    raise TimeoutError("expected local preview state did not arrive")


class Tui:
    def __init__(self, binary, state, initial=None, arguments=None):
        self.before = state / (str(uuid.uuid4()) + ".termios.json")
        self.pid, self.fd = pty.fork()
        if self.pid == 0:
            fcntl.ioctl(0, termios.TIOCSWINSZ, struct.pack("HHHH", 32, 120, 0, 0))
            self.before.write_text(json.dumps(termios.tcgetattr(0)[:6]))
            os.environ["TERM"] = "xterm-256color"
            os.environ["SHELL"] = "/bin/sh"
            args = [str(binary)] + (arguments if arguments is not None else ["--state-dir", str(state), "tui"])
            if initial:
                args += ["--session", initial]
            os.execv(str(binary), args)
        self.output = b""

    def send(self, data):
        os.write(self.fd, data)

    def read(self):
        if select.select([self.fd], [], [], 0.02)[0]:
            try:
                data = os.read(self.fd, 65536)
            except OSError:
                data = b""
            self.output = (self.output + data)[-4 * 1024 * 1024:]
        return self.output

    def sees(self, text):
        # Ratatui can move over unchanged spaces instead of writing them.
        # Compare visible tokens, not raw bytes containing cursor-position CSI.
        rendered = re.sub(rb"\x1b\[[0-?]*[ -/]*[@-~]", b"", self.read())
        return re.sub(rb"\s+", b"", text.encode()) in re.sub(rb"\s+", b"", rendered)

    def quit(self, data):
        self.send(data)
        status = None
        def exited():
            nonlocal status
            self.read()
            found, status = os.waitpid(self.pid, os.WNOHANG)
            return bool(found)
        until(exited)
        assert os.waitstatus_to_exitcode(status) == 0, self.output[-2000:]
        assert termios.tcgetattr(self.fd)[:6] == json.loads(self.before.read_text()), "terminal mode was not restored"
        os.close(self.fd)
        self.pid = None

    def cleanup(self):
        if self.pid:
            try:
                os.kill(self.pid, signal.SIGKILL)
                os.waitpid(self.pid, 0)
            except ProcessLookupError:
                pass
            os.close(self.fd)
            self.pid = None


def main():
    binary = Path(sys.argv[1] if len(sys.argv) > 1 else "target/debug/qgenie-remote").resolve()
    assert binary.is_file(), "build qgenie-remote first"
    with tempfile.TemporaryDirectory(prefix="qgr-tui-") as directory:
        state = Path(directory).resolve()
        daemon = subprocess.Popen([str(binary), "--state-dir", str(state), "--root", str(state), "sessiond"], stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL)
        views = []
        try:
            until(lambda: (state / "dispatch.sock").exists())
            until(lambda: request(state, "status"))
            first = Tui(binary, state)
            views.append(first)
            until(lambda: first.sees("Press n"))
            first.send(b"n")
            until(lambda: first.sees("Working directory"))
            # Replace the default cwd with the fixture's allowed root.
            first.send(b"\x7f" * len(str(Path.cwd())))
            first.send(str(state).encode() + b"\r")
            until(lambda: first.sees("CONTROLLING"))
            first.send(b"printf 'UI_SMOKE_OK\\n'\r")
            sessions = until(lambda: request(state, "list"))
            assert len(sessions) == 1
            session = sessions[0]["id"]

            def output_has(needle, view=first):
                view.read()
                batch = request(state, "replay", {"session": session, "after": "0"})
                output = b"".join(base64.b64decode(event["body"]["data"]) for event in batch["events"] if event["body"]["kind"] == "pty_output")
                return needle in output

            until(lambda: output_has(b"\r\nUI_SMOKE_OK\r\n"))
            first.send(b"\x1dd")
            until(lambda: (first.read(), request(state, "list")[0]["controller"] is None)[1])
            first.quit(b"q")
            assert request(state, "list")[0]["state"] == "running"

            second = Tui(binary, state, session)
            views.append(second)
            until(lambda: second.sees("OBSERVING"))
            second.send(b"\x1dc")
            until(lambda: (second.read(), request(state, "list")[0]["controller"] is not None)[1])
            second.send(b"printf 'REATTACHED_OK\\n'\r")
            until(lambda: output_has(b"\r\nREATTACHED_OK\r\n", second))
            second.quit(b"\x1dq")
            assert request(state, "list")[0]["state"] == "running"
            third = Tui(binary, state, session)
            views.append(third)
            until(lambda: third.sees("OBSERVING"))
            os.kill(third.pid, signal.SIGTERM)
            third.quit(b"")
            assert request(state, "list")[0]["state"] == "running"
            request(state, "terminate", {"session": session, "confirmed": True})
            print("PASS: new shell, real keyboard input, detach, session survival, observe, control, reattach, quit, SIGTERM, and terminal mode restoration")
        finally:
            for view in views:
                view.cleanup()
            try:
                exchange(state, "admin.sock", {"ipc_major": 1, "command": {"type": "shutdown", "terminate_sessions": True}})
                daemon.wait(timeout=5)
            finally:
                if daemon.poll() is None:
                    daemon.kill()
                    daemon.wait(timeout=5)


if __name__ == "__main__":
    main()
