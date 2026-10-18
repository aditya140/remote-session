#!/usr/bin/env python3
"""Owned single-server development stack. Its synthetic issuer is never production auth."""
import argparse
import base64
import json
import os
from pathlib import Path
import re
import secrets
import socket
import shutil
import ssl
import stat
import sqlite3
import subprocess
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid

ROOT = Path(__file__).resolve().parents[2]
COMPOSE = Path(__file__).with_name("compose.yaml")
TENANT = "11111111-1111-4111-8111-111111111111"
GRAPH_CLIENT = "33333333-3333-4333-8333-333333333333"
CLIENT = "qgr-development-fixture-client"
AUDIENCE = "api://qgr-development-fixture"


def private(path, contents):
    path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(descriptor, "wb") as stream:
        stream.write(contents.encode() if isinstance(contents, str) else contents)
        stream.flush()
        os.fsync(stream.fileno())


def run(command, *, capture=False, report_failure=True, **kwargs):
    kwargs.setdefault("stdout", subprocess.PIPE if capture else None)
    kwargs.setdefault("stderr", subprocess.PIPE if capture else None)
    result = subprocess.run(command, check=False, **kwargs)
    if result.returncode:
        # Never echo a secret-bearing request/response or generated configuration.
        if capture and report_failure and result.stderr:
            sys.stderr.write(result.stderr.decode(errors="replace")[-8192:])
        raise RuntimeError(f"development command failed ({result.returncode}): {command[0]}")
    return result.stdout if capture else None


def initialize(args):
    directory = args.state.resolve()
    directory.mkdir(mode=0o700, parents=True, exist_ok=True)
    metadata = directory.lstat()
    if not stat.S_ISDIR(metadata.st_mode) or metadata.st_uid != os.geteuid() or metadata.st_mode & 0o077:
        raise RuntimeError("development state must be an owned private directory")
    manifest = directory / "state.json"
    if manifest.exists():
        state = json.loads(manifest.read_text())
        if (state["project"], state["edge_port"], state["identity_port"]) != (args.project, args.edge_port, args.identity_port):
            raise RuntimeError("existing development identity/ports differ; refusing implicit reconfiguration")
        return state
    if not shutil.which("openssl"):
        raise RuntimeError("openssl is required to create the isolated development CA")
    if any(directory.iterdir()):
        raise RuntimeError("partial development initialization exists; review/move it before generating a new identity")
    certs = directory / "certificates"
    certs.mkdir(mode=0o700)
    private(certs / "ca.cnf", "[req]\nprompt=no\ndistinguished_name=dn\nx509_extensions=ca_ext\n[dn]\nCN=QGenie isolated development CA\n[ca_ext]\nbasicConstraints=critical,CA:TRUE\nkeyUsage=critical,keyCertSign,cRLSign\n")
    private(certs / "server.cnf", "[req]\nprompt=no\ndistinguished_name=dn\n[dn]\nCN=localhost\n[server_ext]\nbasicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature,keyEncipherment\nextendedKeyUsage=serverAuth\nsubjectAltName=DNS:localhost,DNS:edge,IP:127.0.0.1,IP:::1\n")
    for command in [
        ["openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "14", "-config", str(certs / "ca.cnf"), "-keyout", str(certs / "ca.key"), "-out", str(certs / "ca.pem")],
        ["openssl", "req", "-new", "-newkey", "rsa:2048", "-nodes", "-config", str(certs / "server.cnf"), "-keyout", str(certs / "server.key"), "-out", str(certs / "server.csr")],
        ["openssl", "x509", "-req", "-in", str(certs / "server.csr"), "-CA", str(certs / "ca.pem"), "-CAkey", str(certs / "ca.key"), "-CAcreateserial", "-days", "7", "-extfile", str(certs / "server.cnf"), "-extensions", "server_ext", "-out", str(certs / "server.pem")],
    ]:
        run(command, capture=True, timeout=30)
    private(certs / "redis.cnf", "[req]\nprompt=no\ndistinguished_name=dn\n[dn]\nCN=redis\n[server_ext]\nbasicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature,keyEncipherment\nextendedKeyUsage=serverAuth\nsubjectAltName=DNS:redis\n")
    for command in [
        ["openssl", "req", "-new", "-newkey", "rsa:2048", "-nodes", "-config", str(certs / "redis.cnf"), "-keyout", str(certs / "redis.key"), "-out", str(certs / "redis.csr")],
        ["openssl", "x509", "-req", "-in", str(certs / "redis.csr"), "-CA", str(certs / "ca.pem"), "-CAkey", str(certs / "ca.key"), "-CAcreateserial", "-days", "7", "-extfile", str(certs / "redis.cnf"), "-extensions", "server_ext", "-out", str(certs / "redis.pem")],
    ]:
        run(command, capture=True, timeout=30)
    state = {"version": 1, "project": args.project, "edge_port": args.edge_port,
             "identity_port": args.identity_port, "epoch": "development-" + str(uuid.uuid4())}
    passwords = {role: secrets.token_hex(32) for role in ("super", "migrate", "app", "audit", "ingest", "owner", "admin", "lifecycle", "replication", "redis", "graph")}
    inputs = directory / "input"
    for scope in ("gateway", "edge", "identity", "redis", "bouncer", "migrate", "jobs", "postgres"):
        (inputs / scope).mkdir(mode=0o700, parents=True)
    def put(scope, name, value):
        private(inputs / scope / name, json.dumps(value) + "\n" if isinstance(value, (dict, list)) else value)
    ca = (certs / "ca.pem").read_text()
    put("edge", "server.pem", (certs / "server.pem").read_text() + (certs / "server.key").read_text())
    put("gateway", "ca.pem", ca)
    put("gateway", "machine-key", base64.b64encode(secrets.token_bytes(32)).decode())
    put("gateway", "routing-key", base64.b64encode(secrets.token_bytes(32)).decode())
    put("gateway", "graph-secret", passwords["graph"])
    put("identity", "graph-secret", passwords["graph"])
    put("gateway", "identity.json", {"issuer": f"https://localhost:{args.identity_port}/{TENANT}/v2.0", "tenant": TENANT, "audience": AUDIENCE, "remote_scope": "Remote.Control", "admin_role": "Remote.Admin", "jwks_url": "https://edge:9443/keys", "ca_bundle": "/run/qgr/ca.pem"})
    put("gateway", "directory.json", {"tenant": TENANT, "client_id": GRAPH_CLIENT, "credential_file": "/run/qgr/graph-secret", "token_url": "https://edge:9443/token", "graph_base": "https://edge:9443/v1.0", "ca_bundle": "/run/qgr/ca.pem"})
    put("gateway", "redaction.json", ["development-fixture-secret"])
    put("gateway", "redis-url", f"rediss://:{passwords['redis']}@redis:6379/0")
    put("redis", "redis.pem", (certs / "redis.pem").read_text())
    put("redis", "redis.key", (certs / "redis.key").read_text())
    put("redis", "ca.pem", ca)
    put("redis", "redis.conf", f"bind 0.0.0.0\nport 0\ntls-port 6379\ntls-cert-file /run/qgr/redis.pem\ntls-key-file /run/qgr/redis.key\ntls-ca-cert-file /run/qgr/ca.pem\ntls-auth-clients no\nprotected-mode yes\nrequirepass {passwords['redis']}\nappendonly no\nsave \"\"\nmaxmemory 64mb\nmaxmemory-policy noeviction\n")
    logins = {"app": ("qgr_development_app", "qgr_app"), "audit": ("qgr_development_audit", "qgr_audit"), "ingest": ("qgr_development_ingest", "qgr_record_ingest"), "owner": ("qgr_development_owner", "qgr_history_owner"), "admin": ("qgr_development_admin", "qgr_history_admin"), "lifecycle": ("qgr_development_lifecycle", "qgr_record_lifecycle")}
    sql = "".join(f"CREATE ROLE {role} NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS;\n" for role in ("qgr_app", "qgr_audit", "qgr_record_ingest", "qgr_history_owner", "qgr_history_admin", "qgr_record_lifecycle"))
    sql += f"CREATE ROLE qgr_development_migrate LOGIN SUPERUSER PASSWORD '{passwords['migrate']}';\nCREATE ROLE qgr_replication LOGIN REPLICATION PASSWORD '{passwords['replication']}';\n"
    for key, (user, role) in logins.items():
        sql += f"CREATE ROLE {user} LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS PASSWORD '{passwords[key]}'; GRANT {role} TO {user};\n"
    put("postgres", "01_roles.sql", sql)
    put("postgres", "02_replication.sh", '#!/bin/sh\nset -eu\nprintf "host replication qgr_replication all scram-sha-256\\n" >> "$PGDATA/pg_hba.conf"\n')
    put("postgres", "super-password", passwords["super"])
    put("postgres", "replication.pgpass", f"postgres:5432:replication:qgr_replication:{passwords['replication']}\n")
    filenames = {"app": "database-url", "audit": "audit-url", "ingest": "ingest-url", "owner": "history-owner-url", "admin": "history-admin-url", "lifecycle": "lifecycle-url"}
    for key, (user, _) in logins.items():
        # Authority uses primary directly. Recording/history/lifecycle use transaction pooling.
        hostname = "postgres" if key in ("app", "audit") else "pgbouncer"
        put("gateway", filenames[key], f"postgres://{user}:{passwords[key]}@{hostname}:5432/qgr?sslmode=disable")
    put("migrate", "database-url", f"postgres://qgr_development_migrate:{passwords['migrate']}@postgres:5432/qgr?sslmode=disable")
    put("jobs", "database-url", f"postgres://{logins['lifecycle'][0]}:{passwords['lifecycle']}@pgbouncer:5432/qgr?sslmode=disable")
    put("bouncer", "users.txt", "".join(f'"{user}" "{passwords[key]}"\n' for key, (user, _) in logins.items()))
    put("bouncer", "pgbouncer.ini", "[databases]\nqgr = host=postgres port=5432 dbname=qgr\n[pgbouncer]\nlisten_addr = 0.0.0.0\nlisten_port = 5432\nunix_socket_dir = /tmp\nauth_type = scram-sha-256\nauth_file = /run/qgr/users.txt\npool_mode = transaction\ndefault_pool_size = 8\nmax_client_conn = 512\nmax_prepared_statements = 256\nignore_startup_parameters = extra_float_digits\nserver_reset_query_always = 0\nserver_idle_timeout = 60\n")
    private(manifest, json.dumps(state) + "\n")
    return state


class Stack:
    def __init__(self, args, state):
        self.args, self.state = args, state
        self.docker = ["docker"] + (["--host=" + args.docker_host] if args.docker_host else [])
        self.environment = os.environ.copy()
        self.environment.update(QGR_PROJECT=args.project, QGR_SECRETS_DIR=str(args.state.resolve()), QGR_EDGE_PORT=str(args.edge_port), QGR_IDENTITY_PORT=str(args.identity_port))
        self.compose = self.docker + ["compose", "--project-name", args.project, "--file", str(COMPOSE)]
    def call(self, *arguments, **kwargs):
        return run(self.compose + list(arguments), env=self.environment, **kwargs)
    def sql(self, sql, database="qgr"):
        return self.call("exec", "-T", "postgres", "psql", "--username=postgres", "--dbname=" + database, "--no-psqlrc", "--tuples-only", "--no-align", "--set=ON_ERROR_STOP=1", "--command", sql, capture=True, timeout=30).decode().strip()
    def wait(self, condition, label, seconds=180):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            try:
                if condition():
                    return
            except (RuntimeError, subprocess.TimeoutExpired, urllib.error.URLError):
                pass
            time.sleep(0.25)
        raise RuntimeError(f"development readiness timed out: {label}")
    def build(self):
        for target, tag in (("gateway", "qgenie-remote-gateway:development"), ("fixture-identity", "qgenie-remote-fixture-identity:development")):
            if self.args.build_target not in ("all", target):
                continue
            command = self.docker + ["build", "--network=" + self.args.build_network, "--file", str(ROOT / "deploy/images/Dockerfile"), "--target", target, "--tag", tag, "--build-arg", "HTTPS_PROXY", "--build-arg", "HTTP_PROXY", "--build-arg", "NO_PROXY"]
            if self.args.build_ca:
                command += ["--secret", "id=ca_bundle,src=" + str(self.args.build_ca.resolve())]
            # Docker build namespaces may lack the host's local proxy DNS alias.
            # Preserve the configured proxy and verified TLS, using its resolved address.
            for hostname in sorted({urllib.parse.urlsplit(self.environment.get(key, "")).hostname for key in ("HTTPS_PROXY", "HTTP_PROXY")} - {None}):
                command += ["--add-host", hostname + ":" + socket.gethostbyname(hostname)]
            run(command + [str(ROOT)], env=self.environment)
    def up(self):
        self.call("config", "--quiet")
        self.call("up", "--detach", "postgres", "standby", "redis", "pgbouncer", "fixture-identity", "edge", "otel", "prometheus")
        self.wait(lambda: self.sql("SELECT count(*) FROM pg_stat_replication WHERE application_name='qgr_development_standby' AND state='streaming'") == "1", "streaming standby")
        self.sql("ALTER SYSTEM SET synchronous_standby_names='FIRST 1 (qgr_development_standby)'")
        self.sql("SELECT pg_reload_conf()")
        self.wait(lambda: self.sql("SELECT count(*) FROM pg_stat_replication WHERE application_name='qgr_development_standby' AND sync_state='sync'") == "1", "actual synchronous standby")
        self.call("run", "--rm", "migrate")
        epoch = self.state["epoch"]
        if not re.fullmatch(r"development-[0-9a-f-]{36}", epoch):
            raise RuntimeError("invalid local development epoch")
        self.sql("BEGIN; SET LOCAL synchronous_commit='remote_apply'; SELECT pg_advisory_xact_lock(5856751401703146820); "
                 "UPDATE qgr_recordings.state SET durability='synchronous_standby',lifecycle_ready=true,recovery_pending=false,recovery_epoch='" + epoch + "'; COMMIT;")
        self.call("up", "--detach", "gateway-1", "gateway-2", "gateway-3", "gateway-4", "jobs")
        for node in range(1, 5):
            self.wait(lambda n=node: self.probe(n), f"gateway {node}")
        self.wait(lambda: self.http("/v1/machines", token=self.mint()) is not None, "verified edge/identity/directory")
        print("Four development gateways are ready with actual synchronous storage. This same-host synthetic stack does not qualify production durability or identity.")
    def probe(self, node):
        self.call("exec", "-T", f"gateway-{node}", "/app/gateway", "probe", capture=True, report_failure=False, timeout=5)
        return True
    def roll(self):
        # Restart alone retains the old container image. Apply additive migrations
        # first, then recreate only one gateway at a time with the selected image.
        image = self.environment.get("QGR_GATEWAY_IMAGE") or "qgenie-remote-gateway:development"
        expected = run(self.docker + ["image", "inspect", image, "--format", "{{.Id}}"],
                       env=self.environment, capture=True, timeout=10).decode().strip()
        self.call("run", "--rm", "--no-deps", "migrate")
        for node in range(1, 5):
            self.call("up", "--detach", "--no-deps", "--force-recreate", f"gateway-{node}")
            self.wait(lambda n=node: self.probe(n), f"rolling gateway {node}")
            container = self.call("ps", "--quiet", f"gateway-{node}", capture=True).decode().strip()
            actual = run(self.docker + ["inspect", "--format", "{{.Image}}", container],
                         env=self.environment, capture=True, timeout=10).decode().strip()
            if actual != expected:
                raise RuntimeError("rolling gateway image differs from the selected image; stopping rollout")
        self.call("up", "--detach", "--no-deps", "jobs")
        print("Four gateways recreated sequentially from the selected image; native ownership remains external.")
    def http(self, path, *, token=None, identity=False, form=None):
        port = self.args.identity_port if identity else self.args.edge_port
        context = ssl.create_default_context(cafile=str(self.args.state / "certificates/ca.pem"))
        opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), urllib.request.HTTPSHandler(context=context))
        headers = {"Authorization": "Bearer " + token} if token else {}
        data = urllib.parse.urlencode(form).encode() if form else None
        if form:
            headers["Content-Type"] = "application/x-www-form-urlencoded"
        with opener.open(urllib.request.Request(f"https://localhost:{port}{path}", data=data, headers=headers), timeout=5) as response:
            body = response.read(1024 * 1024 + 1)
            if len(body) > 1024 * 1024:
                raise RuntimeError("oversize development response")
            return json.loads(body)
    def mint(self):
        return self.http("/token", identity=True, form={"client_id": CLIENT, "grant_type": "urn:qgenie:development-fixture"})["access_token"]
    def backup(self):
        backups = self.args.state / "backups"
        backups.mkdir(mode=0o700, exist_ok=True)
        path = backups / (str(uuid.uuid4()) + ".dump")
        partial = path.with_suffix(".partial")
        descriptor = os.open(partial, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
        with os.fdopen(descriptor, "wb") as stream:
            run(self.compose + ["exec", "-T", "postgres", "pg_dump", "--username=postgres", "--dbname=qgr", "--format=custom"], env=self.environment, stdout=stream, timeout=60)
            stream.flush()
            os.fsync(stream.fileno())
        os.rename(partial, path)
        descriptor = os.open(backups, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(descriptor)
        finally:
            os.close(descriptor)
        print("Private development backup:", path)
        return path
    def restore_smoke(self):
        path = self.backup()
        name = "qgr_restore_" + uuid.uuid4().hex
        self.sql("CREATE DATABASE " + name, "postgres")
        try:
            with path.open("rb") as stream:
                self.call("exec", "-T", "postgres", "pg_restore", "--username=postgres", "--dbname=" + name, "--exit-on-error", stdin=stream, timeout=60)
            self.sql("BEGIN; SELECT pg_advisory_xact_lock(5856751401703146820); UPDATE qgr_recordings.state SET recovery_pending=true,recovery_epoch='restore-" + str(uuid.uuid4()) + "'; COMMIT;", name)
            version = self.sql("SELECT max(version) FROM qgr.schema_versions", name)
            if version != self.sql("SELECT max(version) FROM qgr.schema_versions"):
                raise RuntimeError("restored schema does not match source")
            if self.sql("SELECT recovery_pending FROM qgr_recordings.state", name) != "t":
                raise RuntimeError("restored authority was not quarantined")
            print("Logical backup restored into an isolated quarantined database. No live authority was switched.")
        finally:
            self.sql("DROP DATABASE " + name + " WITH (FORCE)", "postgres")

    def fault_smoke(self, on_redis_down=None, on_primary_down=None):
        invalid = subprocess.run(self.docker + ["run", "--rm", "--network=none", "--read-only", "--cap-drop=ALL", "qgenie-remote-gateway:development", "serve", "--database-url-file", "/absent-private-configuration"], env=self.environment, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=10)
        if invalid.returncode == 0 or b"Configuration" not in invalid.stderr:
            raise RuntimeError("invalid private configuration did not fail explicitly")
        identifiers = self.call("ps", "--all", "--quiet", capture=True).decode().split()
        containers = json.loads(run(self.docker + ["inspect", *identifiers], env=self.environment, capture=True))
        published = []
        for container in containers:
            service = container["Config"]["Labels"]["com.docker.compose.service"]
            for port, bindings in container["NetworkSettings"].get("Ports", {}).items():
                for binding in bindings or []:
                    if service != "edge" or binding["HostIp"] != "127.0.0.1":
                        raise RuntimeError("unexpected development port publication")
                    published.append((port, binding["HostPort"]))
        if sorted(published) != sorted([("8443/tcp", str(self.args.edge_port)), ("9443/tcp", str(self.args.identity_port))]):
            raise RuntimeError("development edge port publication differs from manifest")
        for path in ("/live", "/ready", "/metrics"):
            try:
                self.http(path)
            except urllib.error.HTTPError as error:
                if error.code == 404:
                    continue
            raise RuntimeError("private gateway endpoint was exposed through the edge")
        def text(node, path):
            return self.call("exec", "-T", "edge", "wget", "-Y", "off", "-qO-", f"http://gateway-{node}:9091/{path}", capture=True, timeout=8).decode()
        def metrics_contain(value):
            return all(value in text(node, "metrics") for node in range(1, 5))
        self.wait(lambda: metrics_contain("qgr_routing_available 1\n"), "all Redis subscribers initialized", seconds=30)
        try:
            self.call("stop", "--timeout", "5", "redis")
            self.wait(lambda: metrics_contain("qgr_routing_available 0\n"), "Redis degraded detail", seconds=30)
            for node in range(1, 5):
                self.probe(node)
                if text(node, "ready") != "ready degraded":
                    raise RuntimeError("Redis loss did not retain explicitly degraded readiness")
            self.http("/v1/machines", token=self.mint())
            if on_redis_down:
                on_redis_down()
        finally:
            self.call("start", "redis")
        self.wait(lambda: metrics_contain("qgr_routing_available 1\n"), "Redis resubscription", seconds=60)
        try:
            self.call("stop", "--timeout", "5", "postgres")
            self.wait(lambda: metrics_contain("qgr_authority_ready 0\n"), "primary loss removes readiness", seconds=30)
            for node in range(1, 5):
                self.call("exec", "-T", f"gateway-{node}", "/app/gateway", "probe", "--live", capture=True, timeout=5)
            if on_primary_down:
                on_primary_down()
        finally:
            self.call("start", "postgres")
        self.wait(lambda: metrics_contain("qgr_authority_ready 1\n"), "primary recovery", seconds=60)
        print("Private publication, blocked administrative edge paths, Redis degraded readiness/resubscription and primary-loss liveness/unreadiness/recovery passed.")

    def telemetry(self, emit=True):
        # Explicit operator/development probe, not a production authority source.
        # It runs bounded aggregate queries and prints no content or principal labels.
        metrics = {"qgr_development_collection_success": 0}
        disk = os.statvfs(self.args.state)
        metrics.update(qgr_development_storage_available_bytes=disk.f_bavail * disk.f_frsize,
                       qgr_development_storage_total_bytes=disk.f_blocks * disk.f_frsize)
        backups = list((self.args.state / "backups").glob("*.dump"))
        latest = max((p.stat().st_mtime for p in backups if p.is_file() and not p.is_symlink()), default=0)
        metrics["qgr_development_backup_age_seconds"] = max(0, time.time() - latest)
        try:
            values = json.loads(self.sql("SET statement_timeout='2s'; SET lock_timeout='1s'; SELECT json_build_object('qgr_development_gap_receipts_total',(SELECT count(*) FROM qgr_recordings.receipts WHERE kind='gap'),'qgr_development_approval_outbox_pending',(SELECT count(*) FROM qgr.approval_outbox WHERE outcome IS NULL),'qgr_development_synchronous_replicas',(SELECT count(*) FROM pg_stat_replication WHERE state='streaming' AND sync_state='sync'),'qgr_development_standby_replay_lag_bytes',(SELECT coalesce(max(pg_wal_lsn_diff(sent_lsn,replay_lsn)),0) FROM pg_stat_replication))").splitlines()[-1])
            metrics.update(values)
            metrics["qgr_development_collection_success"] = 1
            if self.args.native_state:
                directory = self.args.native_state.resolve()
                metadata = directory.stat()
                if metadata.st_uid != os.geteuid() or metadata.st_mode & 0o077:
                    raise RuntimeError("native probe state must be owned and private")
                path = directory / "recordings/metadata.sqlite"
                metadata = path.lstat()
                if not stat.S_ISREG(metadata.st_mode) or metadata.st_uid != os.geteuid() or metadata.st_mode & 0o077:
                    raise RuntimeError("native probe spool must be an owned private regular file")
                with sqlite3.connect(path.as_uri() + "?mode=ro", uri=True, timeout=1) as spool:
                    metrics["qgr_development_native_spool_bytes"] = spool.execute("SELECT coalesce(sum(bytes),0) FROM chunks").fetchone()[0]
                    metrics["qgr_development_native_spool_pending"] = sum(spool.execute(f"SELECT count(*) FROM {table}").fetchone()[0] for table in ("chunks", "gaps", "reservations"))
                    metrics["qgr_development_native_oldest_upload_age_seconds"] = spool.execute("SELECT coalesce(max(0,(julianday('now')-min(julianday(json_extract(metadata,'$.captured_to'))))*86400),0) FROM chunks").fetchone()[0]
                    metrics["qgr_development_native_recording_warnings"] = spool.execute("SELECT count(*) FROM warnings").fetchone()[0]
        except (RuntimeError, subprocess.TimeoutExpired, sqlite3.Error, OSError):
            metrics["qgr_development_collection_success"] = 0
        if emit:
            for name, value in sorted(metrics.items()):
                print(f"{name} {value}")
        return metrics


def main():
    os.umask(0o077)
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--state", type=Path, default=Path(__file__).with_name("secrets"))
    parser.add_argument("--project", default="qgr-development")
    parser.add_argument("--docker-host")
    parser.add_argument("--edge-port", type=int, default=8443)
    parser.add_argument("--identity-port", type=int, default=9443)
    parser.add_argument("--build-ca", type=Path)
    parser.add_argument("--build-network", choices=("default", "host"), default="default")
    parser.add_argument("--build-target", choices=("all", "gateway", "fixture-identity"), default="all")
    parser.add_argument("--client-bin",type=Path)
    parser.add_argument("--load-bin", type=Path)
    parser.add_argument("--native-state", type=Path)
    parser.add_argument("--load-sockets", type=int, default=8)
    parser.add_argument("--load-terminals", type=int, default=2)
    parser.add_argument("--load-samples", type=int, default=100)
    parser.add_argument("--monitor-interval-seconds", type=int, default=30)
    parser.add_argument("--monitor-iterations", type=int, default=0, help="0 keeps the aggregate probe running until stopped")
    parser.add_argument("command", choices=("init", "build", "up", "status", "down", "backup", "restore-smoke", "roll", "smoke", "fault-smoke", "baseline", "telemetry", "monitor", "monitor-smoke"))
    args = parser.parse_args()
    if not re.fullmatch(r"[a-z][a-z0-9_-]{1,40}", args.project) or not (1024 <= args.edge_port <= 65535 and 1024 <= args.identity_port <= 65535) or args.edge_port == args.identity_port:
        parser.error("invalid development project/ports")
    state = initialize(args)
    stack = Stack(args, state)
    if args.command == "init":
        stack.call("config", "--quiet")
        print("Private synthetic development inputs initialized; nothing has been deployed.")
    elif args.command == "build":
        stack.build()
    elif args.command == "up":
        stack.up()
    elif args.command == "status":
        stack.call("ps")
    elif args.command == "down":
        stack.call("down", "--remove-orphans")
    elif args.command == "backup":
        stack.backup()
    elif args.command == "restore-smoke":
        stack.restore_smoke()
    elif args.command == "fault-smoke":
        stack.fault_smoke()
    elif args.command == "telemetry":
        stack.telemetry()
    elif args.command in ("monitor", "monitor-smoke"):
        if not 1 <= args.monitor_interval_seconds <= 60 or args.monitor_iterations < 0:
            parser.error("monitor interval must be 1–60 seconds and iterations nonnegative")
        from monitor import monitor, exercise
        (monitor if args.command == "monitor" else exercise)(stack)
    elif args.command == "roll":
        stack.roll()
    elif args.command in ("smoke", "baseline"):
        if not args.client_bin:
            parser.error("smoke requires --client-bin selecting this revision's standalone CLI")
        if args.command == "baseline" and not args.load_bin:
            parser.error("baseline requires --load-bin selecting the remote_load SDK example")
        from smoke import exercise
        exercise(stack,args.client_bin,args.load_bin if args.command == "baseline" else None)


if __name__ == "__main__":
    main()
