#!/usr/bin/env python3
"""Seed completed history, restart the actual daemon, and exercise CLI pages and starts."""
from local_tui_smoke import exchange, request, until
from pathlib import Path
import json
import sqlite3
import subprocess
import sys
import tempfile
import uuid


def main():
    binary = Path(sys.argv[1]).resolve()
    with tempfile.TemporaryDirectory(prefix="qgr-history-") as directory:
        state = Path(directory)
        command = [str(binary), "--state-dir", str(state), "--root", str(state)]

        def cli(*arguments):
            result = subprocess.run(command + list(arguments), check=True, capture_output=True, text=True, timeout=15)
            return json.loads(result.stdout)

        def start():
            daemon = subprocess.Popen(command + ["sessiond"], stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL)
            until(lambda: (state / "dispatch.sock").exists())
            until(lambda: request(state, "status"))
            return daemon

        def stop(daemon):
            if daemon.poll() is None:
                try:
                    exchange(state, "admin.sock", {"ipc_major": 1, "command": {"type": "shutdown", "terminate_sessions": True}})
                    daemon.wait(timeout=5)
                finally:
                    if daemon.poll() is None:
                        daemon.kill()
                        daemon.wait(timeout=5)

        daemon = start()
        try:
            key = "idem_" + str(uuid.uuid4())
            original = cli("run", "--key", key, "--cwd", str(state), "--", "/bin/true")
            until(lambda: request(state, "list")[0]["state"] == "exited")
            stop(daemon)
            with sqlite3.connect(state / "sessions.db") as db:
                digest, saved = db.execute("SELECT fingerprint,spec FROM sessions WHERE id=?", [original["id"]]).fetchone()
                for _ in range(4100):
                    spec = json.loads(saved)
                    spec["key"] = "idem_" + str(uuid.uuid4())
                    db.execute("INSERT INTO sessions(id,idempotency_key,fingerprint,spec,state,created_ms,head,generation,ended_ms) VALUES(?,?,?,?,?,?,?,?,?)",
                               ["s_" + str(uuid.uuid4()), spec["key"], digest, json.dumps(spec), '"exited"', "1", bytes(8), bytes(8), "2"])
            daemon = start()
            assert len(cli("list", "--json")) == 4101
            first = cli("list", "--page")
            second = cli("list", "--page", "--after", first["next"])
            assert len(first["sessions"]) == len(second["sessions"]) == 64
            assert {s["id"] for s in first["sessions"]}.isdisjoint(s["id"] for s in second["sessions"])
            assert cli("run", "--key", key, "--cwd", str(state), "--", "/bin/true")["id"] == original["id"]
            live = cli("run", "--cwd", str(state), "--", "/bin/sh", "-c", "read ignored")
            active = cli("list", "--active")
            assert [s["id"] for s in active["sessions"]] == [live["id"]]
            assert active["next"] is None
            assert len(cli("list", "--json")) == 4102
            print("PASS: 4,100 seeded records, restart, full CLI traversal, explicit pages, original-key reconciliation, new start and live discovery")
        finally:
            stop(daemon)


if __name__ == "__main__":
    main()
