#!/usr/bin/env python3
"""Exercise the actual gateway process against configured disposable PostgreSQL."""
import os
from pathlib import Path
import signal
import socket
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request

binary = Path(sys.argv[1]).resolve()
app_secret = Path(sys.argv[2]).resolve()

def port():
    with socket.socket() as stream:
        stream.bind(("127.0.0.1", 0))
        return stream.getsockname()[1]

public_port, health_port = port(), port()
while health_port == public_port:
    health_port = port()

def status(port_number, path):
    try:
        with urllib.request.urlopen(f"http://127.0.0.1:{port_number}{path}", timeout=0.5) as response:
            return response.status
    except urllib.error.HTTPError as error:
        return error.code

with tempfile.TemporaryFile() as log:
    process = subprocess.Popen([str(binary), "serve", "--database-url-file", str(app_secret), "--listen", f"127.0.0.1:{public_port}", "--health-listen", f"127.0.0.1:{health_port}"], stdout=log, stderr=subprocess.STDOUT)
    try:
        deadline = time.monotonic() + 10
        while True:
            assert process.poll() is None, "gateway process exited before readiness"
            try:
                if status(health_port, "/ready") == 200:
                    break
            except (OSError, urllib.error.URLError):
                pass
            assert time.monotonic() < deadline, "gateway readiness timed out"
            time.sleep(0.02)
        assert status(health_port, "/live") == 200
        assert status(public_port, "/v1/admin/audit") == 401
        os.kill(process.pid, signal.SIGTERM)
        assert process.wait(timeout=12) == 0, "SIGTERM did not drain cleanly"
    finally:
        if process.poll() is None:
            process.kill()
            process.wait()
print("PASS: real gateway readiness, liveness, protected audit API, SIGTERM drain and exit")
