#!/usr/bin/env python3
"""Run recording database contracts through a disposable transaction-pooling PgBouncer."""
import os
from pathlib import Path
import shutil
import socket
import subprocess
import tempfile
import time
from urllib.parse import urlsplit, urlunsplit


def main():
    binary = os.environ.get("QGR_PGBOUNCER_BINARY") or shutil.which("pgbouncer")
    if not binary:
        raise SystemExit("Install PgBouncer >= 1.21 or set QGR_PGBOUNCER_BINARY")
    parent_file = Path(os.environ["QGR_PG_TEST_URL_FILE"])
    parent = urlsplit(parent_file.read_text().strip())
    if parent.scheme not in ("postgres", "postgresql") or parent.hostname not in (
        "127.0.0.1", "localhost", "::1"
    ):
        raise SystemExit("This synthetic fixture requires a disposable loopback PostgreSQL")
    with socket.socket() as available:
        available.bind(("127.0.0.1", 0))
        port = available.getsockname()[1]
    with tempfile.TemporaryDirectory(prefix="qgr-bouncer-") as temporary:
        root = Path(temporary)
        root.chmod(0o700)
        users = root / "users.txt"
        users.write_text("".join(
            f'"{role}" "test-only"\n'
            for role in (
                "qgr_test_record_ingest", "qgr_test_history_owner",
                "qgr_test_history_admin", "qgr_test_record_lifecycle"
            )
        ))
        users.chmod(0o600)
        configuration = root / "pgbouncer.ini"
        configuration.write_text(
            f"[databases]\n* = host={parent.hostname} port={parent.port or 5432}\n"
            f"\n[pgbouncer]\nlisten_addr = 127.0.0.1\nlisten_port = {port}\n"
            f"unix_socket_dir = {root}\nunix_socket_mode = 0700\nauth_type = plain\n"
            f"auth_file = {users}\npool_mode = transaction\ndefault_pool_size = 1\n"
            "max_client_conn = 64\nmax_prepared_statements = 256\n"
            "ignore_startup_parameters = extra_float_digits\n"
            "server_reset_query_always = 0\nserver_idle_timeout = 5\n"
            f"pidfile = {root / 'pgbouncer.pid'}\n"
        )
        configuration.chmod(0o600)
        # Credentials remain private. Only the recording pool host/port changes; migration,
        # primary authority and unrelated outage fixtures continue using their original binding.
        userinfo = parent.netloc.rsplit("@", 1)[0]
        binding = root / "url"
        binding.write_text(urlunsplit(parent._replace(netloc=f"{userinfo}@127.0.0.1:{port}")) + "\n")
        binding.chmod(0o600)
        environment = os.environ.copy()
        environment["QGR_PGBOUNCER_TEST_URL_FILE"] = str(binding)
        project = Path(__file__).resolve().parents[2]
        if "QGR_GATEWAY_BINARY" not in environment:
            subprocess.run(["cargo", "build", "-p", "qgenie-remote-gateway", "--locked"],
                           cwd=project, env=environment, check=True)
        environment.setdefault("QGR_GATEWAY_BINARY", str(project / "target/debug/qgenie-remote-gateway"))
        with (root / "broker.log").open("w") as output:
            broker = subprocess.Popen([binary, str(configuration)], stdout=output, stderr=output)
            try:
                deadline = time.monotonic() + 10
                while True:
                    if broker.poll() is not None:
                        raise RuntimeError("PgBouncer stopped during startup")
                    try:
                        with socket.create_connection(("127.0.0.1", port), timeout=0.2):
                            break
                    except OSError:
                        if time.monotonic() >= deadline:
                            raise RuntimeError("PgBouncer did not become ready")
                        time.sleep(0.05)
                subprocess.run(
                    ["cargo", "test", "-p", "qgenie-remote-gateway", "--lib", "--locked", "--", "--include-ignored", "--skip", "transcripts::durability::"],
                    cwd=project, env=environment, check=True,
                )
            finally:
                broker.terminate()
                try:
                    broker.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    broker.kill()
                    broker.wait(timeout=5)


if __name__ == "__main__":
    main()
