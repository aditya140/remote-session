#!/usr/bin/env python3
"""Disposable synchronous PostgreSQL pair; no production/synthetic-mode switch exists.

--probe-only verifies the database boundary without claiming gateway integration.
The normal mode also requires the Rust proof's completion marker.
"""
import argparse
import json
import os
import platform
from pathlib import Path
import re
import secrets
import socket
import subprocess
import tempfile
import threading
import time


def main():
    options = argparse.ArgumentParser()
    options.add_argument("--probe-only", action="store_true")
    options.add_argument("--report", type=Path,
                         help="Write a sanitized JSON result, including failed/unrun phases")
    args = options.parse_args()
    report = {
        "format": "qgr-synchronous-acceptance-v1",
        "status": "running",
        "environment": {"system": platform.system(), "arch": platform.machine()},
        "mode": "database_probe" if args.probe_only else "composed",
        "phases": {"synchronous_pair": "not_run", "public_session_pipeline": "not_run",
                   "managed_codex": "not_run", "managed_start_boundaries": "not_run",
                   "primary_volume_loss": "not_run"},
        "release_gates": {"real_entra": "not_run", "macos_services": "not_run",
                          "linux_user_manager": "not_run", "beta_scale": "not_run",
                          "production_fault_domains": "not_run"},
    }
    began = time.monotonic()
    try:
        execute(args, report)
        report["status"] = "passed"
    except BaseException as failure:
        report["status"] = "failed"
        for phase, status in report["phases"].items():
            if status == "running":
                report["phases"][phase] = "failed"
        # Exception messages/commands may contain private fixture bindings. Store
        # only allowlisted failure metadata in artifacts.
        report["failure"] = {"kind": type(failure).__name__}
        if isinstance(failure, subprocess.CalledProcessError):
            report["failure"]["exit_code"] = failure.returncode
        raise
    finally:
        report["elapsed_ms"] = int((time.monotonic() - began) * 1000)
        if args.report is not None:
            args.report.parent.mkdir(parents=True, exist_ok=True)
            with tempfile.NamedTemporaryFile(mode="w", dir=args.report.parent, delete=False) as output:
                json.dump(report, output, indent=2)
                output.write("\n")
                staging = Path(output.name)
            staging.replace(args.report)


def execute(args, report):
    environment = os.environ.copy()
    if not args.probe_only and environment.get("QGR_CODEX_BINARY") and platform.system() != "Linux":
        raise RuntimeError("The composed managed Codex proof is qualified only on Linux")
    report["phases"]["synchronous_pair"] = "running"
    for key in ("DOCKER_HOST", "DOCKER_CONTEXT", "DOCKER_TLS", "DOCKER_TLS_VERIFY", "DOCKER_CERT_PATH"):
        environment.pop(key, None)
    docker = ["docker", "--host=unix:///var/run/docker.sock"]

    def command(arguments, **kwargs):
        return subprocess.run(arguments, env=environment, text=True, capture_output=True,
                              check=True, timeout=45, **kwargs).stdout.strip()

    report["docker_version"] = command(docker + ["info", "--format", "{{.ServerVersion}}"])
    identifier = "qgr-recording-proof-" + secrets.token_hex(6)
    primary, standby = identifier + "-primary", identifier + "-standby"
    resources, network = [], False
    with tempfile.TemporaryDirectory(prefix="qgr-durability-") as temporary:
        root = Path(temporary)
        root.chmod(0o700)
        password = secrets.token_hex(24)
        private = root / "postgres.env"
        private.write_text(f"POSTGRES_PASSWORD={password}\nPGPASSWORD={password}\n")
        private.chmod(0o600)
        listener, controller = None, None
        stopped = threading.Event()

        def sql(container, statement):
            return command(docker + ["exec", "--user", "postgres", container,
                                     "psql", "-X", "-h", "127.0.0.1", "-v", "ON_ERROR_STOP=1", "-U", "postgres",
                                     "-d", "postgres", "-Atc", statement])

        def until(predicate, timeout=30):
            deadline = time.monotonic() + timeout
            while True:
                try:
                    if predicate():
                        return
                except (subprocess.CalledProcessError, OSError):
                    pass
                if time.monotonic() >= deadline:
                    raise RuntimeError("Disposable PostgreSQL pair did not become ready")
                time.sleep(0.1)

        def binding(container, filename):
            mapping = json.loads(command(docker + ["inspect", "--format", "{{json .NetworkSettings.Ports}}", container]))
            port = int(mapping["5432/tcp"][0]["HostPort"])
            path = root / filename
            path.write_text(f"postgres://postgres:{password}@127.0.0.1:{port}/postgres\n")
            path.chmod(0o600)
            return path

        def promote():
            # Only this harness's exact randomly generated resource is removed. Its entire
            # data directory is disposable tmpfs, physically separate from the standby.
            command(docker + ["rm", "--force", primary])
            resources.remove(primary)
            sql(standby, "SELECT pg_promote(true,10)")
            until(lambda: sql(standby, "SELECT NOT pg_is_in_recovery()") == "t")

        def control():
            while not stopped.is_set():
                try:
                    peer, _ = listener.accept()
                except socket.timeout:
                    continue
                except OSError:
                    return
                with peer:
                    peer.settimeout(30)
                    try:
                        data = bytearray()
                        while not data.endswith(b"\n"):
                            part = peer.recv(1024)
                            if not part or len(data) + len(part) > 1024:
                                raise ValueError("invalid control request")
                            data.extend(part)
                        action = json.loads(data)["action"]
                        if action == "pause_standby":
                            command(docker + ["pause", standby])
                        elif action == "resume_standby":
                            command(docker + ["unpause", standby])
                        elif action == "promote_after_primary_loss":
                            promote()
                        else:
                            raise ValueError("unknown fixture action")
                        peer.sendall(b'{"ok":true}\n')
                    except Exception:
                        peer.sendall(b'{"ok":false}\n')

        try:
            command(docker + ["network", "create", identifier])
            network = True
            resources.append(primary)
            command(docker + ["run", "--detach", "--name", primary, "--network", identifier,
                              "--network-alias", "primary", "--env-file", str(private),
                              "--publish", "127.0.0.1::5432", "--tmpfs",
                              "/var/lib/postgresql/data:rw,size=768m,mode=0700", "postgres:17-alpine"])
            until(lambda: sql(primary, "SELECT 1") == "1")
            command(docker + ["exec", "--user", "postgres", primary, "sh", "-c",
                              "printf '\nhost replication postgres all scram-sha-256\n' >> \"$PGDATA/pg_hba.conf\""])
            sql(primary, "SELECT pg_reload_conf()")
            resources.append(standby)
            command(docker + ["run", "--detach", "--name", standby, "--network", identifier,
                              "--user", "postgres", "--env-file", str(private),
                              "--env", "PGAPPNAME=qgr_sync_standby", "--env",
                              "PGDATA=/var/lib/postgresql/data/replica", "--publish", "127.0.0.1::5432",
                              "--tmpfs", "/var/lib/postgresql/data:rw,size=768m,mode=1777",
                              "--entrypoint", "sh", "postgres:17-alpine", "-c",
                              'pg_basebackup -h primary -U postgres -D "$PGDATA" -R -X stream -w && exec postgres'])
            until(lambda: sql(standby, "SELECT pg_is_in_recovery()") == "t")
            until(lambda: sql(primary, "SELECT count(*) FROM pg_stat_replication WHERE application_name='qgr_sync_standby' AND state='streaming'") == "1")
            sql(primary, "ALTER SYSTEM SET synchronous_standby_names='FIRST 1 (qgr_sync_standby)'")
            sql(primary, "SELECT pg_reload_conf()")
            until(lambda: sql(primary, "SELECT count(*) FROM pg_stat_replication WHERE application_name='qgr_sync_standby' AND sync_state='sync'") == "1")
            primary_binding = binding(primary, "primary-url")
            standby_binding = binding(standby, "standby-url")
            report["postgres_version"] = sql(primary, "SHOW server_version")
            report["phases"]["synchronous_pair"] = "passed"
            print("Synchronous streaming pair ready", flush=True)
            if args.probe_only:
                sql(primary, "CREATE TABLE public.qgr_sync_probe(id integer PRIMARY KEY)")
                sql(primary, "SET synchronous_commit='remote_apply'; INSERT INTO public.qgr_sync_probe VALUES(1)")
                assert sql(standby, "SELECT count(*) FROM public.qgr_sync_probe") == "1"
                report["phases"]["primary_volume_loss"] = "database_probe_passed"
                promote()
                assert sql(standby, "SELECT count(*) FROM public.qgr_sync_probe") == "1"
                print("Database-only remote-apply/primary-volume-loss probe passed", flush=True)
                return
            listener = socket.socket(socket.AF_UNIX)
            redis = identifier + "-redis"
            resources.append(redis)
            command(docker + ["run", "--detach", "--name", redis, "--network", identifier,
                              "--publish", "127.0.0.1::6379", "--tmpfs", "/data:rw,size=64m",
                              "redis:8-alpine", "redis-server", "--save", "", "--appendonly", "no"])
            until(lambda: command(docker + ["exec", redis, "redis-cli", "ping"]) == "PONG")
            mapping = json.loads(command(docker + ["inspect", "--format", "{{json .NetworkSettings.Ports}}", redis]))
            redis_binding = root / "redis-url"
            redis_binding.write_text(f"redis://127.0.0.1:{mapping['6379/tcp'][0]['HostPort']}/0\n")
            redis_binding.chmod(0o600)
            endpoint = root / "control.sock"
            listener.bind(str(endpoint))
            endpoint.chmod(0o600)
            listener.listen(4)
            listener.settimeout(0.2)
            controller = threading.Thread(target=control, daemon=True)
            controller.start()
            marker = root / "completed.json"
            pipeline_marker = root / "pipeline-completed.json"
            metrics_marker = root / "gateway-observations.json"
            environment.update(QGR_SYNC_PRIMARY_URL_FILE=str(primary_binding),
                               QGR_SYNC_STANDBY_URL_FILE=str(standby_binding),
                               QGR_SYNC_CONTROL_SOCKET=str(endpoint), QGR_SYNC_PROOF_FILE=str(marker),
                               QGR_PG_TEST_URL_FILE=str(primary_binding),
                               QGR_REDIS_TEST_URL_FILE=str(redis_binding),
                               QGR_METRICS_PROOF_FILE=str(metrics_marker),
                               QGR_SYNC_PIPELINE_PROOF_FILE=str(pipeline_marker))
            project = Path(__file__).resolve().parents[2]
            environment.setdefault("QGR_GATEWAY_BINARY", str(project / "target/debug/qgenie-remote-gateway"))
            report["phases"]["public_session_pipeline"] = "running"
            if environment.get("QGR_CODEX_BINARY"):
                report["phases"]["managed_codex"] = "running"
            subprocess.run(["cargo", "build", "-p", "qgenie-remote-gateway", "--locked"], cwd=project, env=environment, check=True)
            managed = ["--features", "managed-codex"] if environment.get("QGR_CODEX_BINARY") else []
            subprocess.run(["cargo", "test", "-p", "qgenie-remote", "--test", "host_enrollment", "--locked", *managed,
                            "recording::native_spool_https_upload_lost_ack_and_epoch_reclamation",
                            "--", "--ignored", "--exact", "--nocapture"], cwd=project, env=environment, check=True)
            if not pipeline_marker.is_file() or json.loads(pipeline_marker.read_text()) != {"complete": True}:
                raise RuntimeError("Native HTTPS upload proof did not produce its completion marker")
            report["phases"]["public_session_pipeline"] = "passed"
            report["gateway_observations"] = json.loads(metrics_marker.read_text())
            report["approval_latency_definition"] = "Native delivery attempt through received/persisted result or error; includes fault-injected attempts, excludes claim SQL and retry scheduling. Aggregate count/sum, not a release latency distribution."
            if managed:
                report["phases"]["managed_codex"] = "passed"
                report["managed_codex_pin"] = "0.159.2"
                report["phases"]["managed_start_boundaries"] = "running"
                proof = subprocess.run(
                    ["cargo", "test", "-p", "qgenie-remote-sessiond", "--all-features",
                     "--lib", "--locked", "actual_managed", "--", "--ignored"],
                    cwd=project, env=environment, text=True, capture_output=True,
                    timeout=120, check=True)
                expected = (
                    "actual_managed_start_crash_boundaries_preserve_keys_without_relaunch",
                    "actual_managed_owner_crash_fences_history_without_relaunch",
                    "actual_managed_owner_deduplicates_records_replays_and_survives_caller_loss",
                )
                if not all(re.search(r"test [^\n]*" + name + r" \.\.\. ok", proof.stdout)
                           for name in expected):
                    raise RuntimeError("Managed start proof omitted a required native process test")
                report["phases"]["managed_start_boundaries"] = "passed"
                print("Actual pinned managed start crash matrix, owner SIGKILL and caller-loss recovery passed",
                      flush=True)
            report["phases"]["primary_volume_loss"] = "running"
            subprocess.run(["cargo", "test", "-p", "qgenie-remote-gateway", "--lib", "--locked",
                            "transcripts::durability::actual_synchronous_receipts_survive_primary_volume_loss",
                            "--", "--ignored", "--exact", "--nocapture"], cwd=project, env=environment, check=True)
            if not marker.is_file() or json.loads(marker.read_text()) != {"complete": True}:
                raise RuntimeError("Gateway durability proof did not produce its completion marker")
            report["phases"]["primary_volume_loss"] = "passed"
        finally:
            stopped.set()
            if listener:
                listener.close()
            if controller:
                controller.join(timeout=5)
            for resource in reversed(resources):
                subprocess.run(docker + ["unpause", resource], env=environment,
                               capture_output=True, timeout=20)
                subprocess.run(docker + ["rm", "--force", resource], env=environment,
                               capture_output=True, timeout=20)
            if network:
                subprocess.run(docker + ["network", "rm", identifier], env=environment,
                               capture_output=True, timeout=20)


if __name__ == "__main__":
    main()
