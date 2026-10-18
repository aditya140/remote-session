"""Actual standalone CLI/native owner against the four-container gateway deployment."""
import base64
import json
import os
from pathlib import Path
import subprocess
import shutil
import tempfile
import time
import platform
import re
import sqlite3


def process_sample(process):
    fields = Path(f"/proc/{process.pid}/stat").read_text().rsplit(")", 1)[1].split()
    pages = int(Path(f"/proc/{process.pid}/statm").read_text().split()[1])
    return {"rss_bytes": pages * os.sysconf("SC_PAGE_SIZE"), "cpu_seconds": (int(fields[11]) + int(fields[12])) / os.sysconf("SC_CLK_TCK")}


def exercise(stack, binary, load_binary=None):
    binary = Path(binary).resolve()
    with tempfile.TemporaryDirectory(prefix="qgr-deploy-") as temporary:
        root = Path(temporary)
        native_state, client_state = root / "native", root / "client"
        native_state.mkdir(mode=0o700)
        client_state.mkdir(mode=0o700)
        token_file = root / "token"
        token_file.write_text(stack.mint())
        token_file.chmod(0o600)
        config = root / "host.json"
        config.write_text(json.dumps({"gateway": f"https://localhost:{stack.args.edge_port}", "os_user": "development-smoke", "name": "development-native-smoke", "hostname": "development-native-smoke", "roots": [str(root)], "allow_private_file": True, "ca_bundle": str(stack.args.state.resolve() / "certificates/ca.pem")}))
        config.chmod(0o600)
        diagnostics = (root / "diagnostics.log").open("wb")
        def local(*args, state=native_state, input=None):
            result = subprocess.run([str(binary), "--state-dir", str(state), *args], input=input, stdout=subprocess.PIPE, stderr=diagnostics, timeout=20)
            if result.returncode:
                raise RuntimeError("standalone development command failed; no mutation was retried")
            return json.loads(result.stdout) if result.stdout.strip().startswith((b"{", b"[")) else None
        def remote(*args, input=None):
            return local("remote", "--gateway", f"https://localhost:{stack.args.edge_port}", "--token-file", str(token_file), "--ca-bundle", str(stack.args.state.resolve() / "certificates/ca.pem"), *args, state=client_state, input=input)
        native = connector = None
        try:
            binding = local("host", "--config", str(config), "enroll", "--token-file", str(token_file), "--acknowledge-recording")
            machine = binding["machine"]
            if not re.fullmatch(r"m_[0-9a-f-]{36}", machine):
                raise RuntimeError("unexpected machine identity for scoped development proof")
            native = subprocess.Popen([str(binary), "--state-dir", str(native_state), "sessiond", "--host-config", str(config)], stdin=subprocess.DEVNULL, stdout=diagnostics, stderr=diagnostics, start_new_session=True)
            stack.wait(lambda: local("status", "--json") is not None, "independent native owner", seconds=20)
            connector = subprocess.Popen([str(binary), "--state-dir", str(native_state), "host", "--config", str(config), "connect"], stdin=subprocess.DEVNULL, stdout=diagnostics, stderr=diagnostics, start_new_session=True)
            # A durable empty session list can precede connector activation.
            # Actual descriptor-scoped browsing proves the native route is live.
            stack.wait(lambda: remote("browse", machine, str(root)) is not None, "actual native connector authority", seconds=30)
            if load_binary:
                idle_before = {"native": process_sample(native), "connector": process_sample(connector)}
                idle_started = time.monotonic()
                time.sleep(2)
                idle_seconds = time.monotonic() - idle_started
                idle_after = {"native": process_sample(native), "connector": process_sample(connector)}
                idle = {name: {"rss_bytes": sample["rss_bytes"], "cpu_percent_one_core": 100 * (sample["cpu_seconds"] - idle_before[name]["cpu_seconds"]) / idle_seconds} for name, sample in idle_after.items()}
                load_config = root / "load.json"
                load_config.write_text(json.dumps({"acknowledge_synthetic_load": True, "gateway": f"https://localhost:{stack.args.edge_port}", "ca_bundle": str(stack.args.state.resolve() / "certificates/ca.pem"), "token_file": str(token_file), "state_directory": str(client_state), "machine": machine, "cwd": str(root), "sockets": stack.args.load_sockets, "terminals": stack.args.load_terminals, "samples_per_terminal": stack.args.load_samples, "burst_bytes": 51200}))
                load_config.chmod(0o600)
                lsn = stack.sql("SELECT pg_current_wal_lsn()")
                measured = subprocess.run([str(Path(load_binary).resolve()), str(load_config)], stdout=subprocess.PIPE, stderr=diagnostics, timeout=600)
                if measured.returncode:
                    raise RuntimeError("load measurement failed; no input/start mutation was retried; inspect private last-smoke.log")
                report = json.loads(measured.stdout)
                report.update({"platform": platform.platform(), "logical_cpus": os.cpu_count(), "memory_total_bytes": int(next(line.split()[1] for line in Path('/proc/meminfo').read_text().splitlines() if line.startswith('MemTotal:'))) * 1024, "idle_sample_seconds": idle_seconds, "idle_host": idle, "active_host": {"native": process_sample(native), "connector": process_sample(connector)}, "configured_terminals": stack.args.load_terminals, "samples_per_terminal": stack.args.load_samples, "burst_bytes_per_terminal": 51200})
                def stored():
                    path = native_state / "recordings/metadata.sqlite"
                    if not path.is_file():
                        return False
                    with sqlite3.connect(path.as_uri() + "?mode=ro", uri=True, timeout=1) as spool:
                        sealed = spool.execute("SELECT count(*) FROM streams WHERE sealed=1").fetchone()[0]
                        pending = sum(spool.execute(f"SELECT count(*) FROM {table}").fetchone()[0] for table in ("reservations", "chunks", "gaps"))
                    return sealed == stack.args.load_terminals and pending == 0
                stack.wait(stored, "load recording receipt persistence", seconds=30)
                report["recording"] = json.loads(stack.sql(f"SELECT json_build_object('chunks',count(*),'normalized_uncompressed_bytes',coalesce(sum((r.immutable->'stored'->>'uncompressed_bytes')::bigint),0),'compressed_bytes',coalesce(sum(octet_length(c.payload)),0)) FROM qgr_recordings.receipts r JOIN qgr_recordings.content c USING(machine,session,id) WHERE r.machine='{machine}' AND r.kind='content'"))
                if report["recording"]["normalized_uncompressed_bytes"] <= 0:
                    raise RuntimeError("no protected normalized recording bytes measured")
                report["recording"]["native_spool_pending_after_reconciliation"] = 0
                previous = stack.args.native_state
                stack.args.native_state = native_state
                try:
                    metrics = stack.telemetry(emit=False)
                    if metrics.get("qgr_development_collection_success") != 1 or metrics.get("qgr_development_native_spool_pending") != 0:
                        raise RuntimeError("native operational probe did not reconcile the actual private spool")
                    report["operational_probe"] = metrics
                    from monitor import publish
                    if not publish(stack, metrics):
                        raise RuntimeError("native operational metrics did not reach the private collector")
                    report["operational_probe_published"] = True
                finally:
                    stack.args.native_state = previous
                if not re.fullmatch(r"[0-9A-F]+/[0-9A-F]+", lsn):
                    raise RuntimeError("invalid WAL position")
                report["database"] = json.loads(stack.sql(f"SELECT json_build_object('wal_bytes_during_workload',pg_wal_lsn_diff(pg_current_wal_lsn(),'{lsn}'::pg_lsn),'standby_replay_lag_bytes',(SELECT pg_wal_lsn_diff(sent_lsn,replay_lsn) FROM pg_stat_replication WHERE application_name='qgr_development_standby'))"))
                containers = stack.call("ps", "--quiet", capture=True).decode().split()
                stats = subprocess.check_output(stack.docker + ["stats", "--no-stream", "--format", "{{json .}}", *containers], env=stack.environment, timeout=15)
                report["containers"] = [json.loads(line) for line in stats.decode().splitlines() if line]
                report["versions"] = {"rust": "1.99.1", "postgres": stack.sql("SHOW server_version"), "schema": stack.sql("SELECT max(version) FROM qgr.schema_versions")}
                destination = stack.args.state / "last-baseline.json"
                destination.write_text(json.dumps(report, indent=2) + "\n")
                destination.chmod(0o600)
                print(json.dumps(report, indent=2))
                return
            command = "stty -echo; printf 'DEPLOYMENT_NATIVE_READY\\n'; while IFS= read -r value; do printf '%s\\n' \"$value\"; case \"$value\" in LOCAL_WHILE_REDIS_DOWN|LOCAL_WHILE_PRIMARY_DOWN) : > \"$value\";; esac; done"
            started = remote("run", machine, "--cwd", str(root), "--", "/bin/sh", "-c", command)
            session = started["session"]["id"]
            key = started["key"]
            def observed(marker):
                page = remote("replay", machine, session)
                output = b"".join(base64.b64decode(event["event"]["data"]["data"]) for event in page.get("events", []) if event["event"].get("kind") == "pty_output")
                return marker in output
            stack.wait(lambda: observed(b"DEPLOYMENT_NATIVE_READY"), "initial native output")
            remote("send", machine, session, "development-fixture-secret BEFORE_ROLL", "--enter")
            stack.wait(lambda: observed(b"BEFORE_ROLL"), "native controlled input")
            def local_while_down(marker):
                # One deliberate local input, no retry. The owned shell writes
                # this marker before dependency recovery, proving execution.
                local("send", session, marker, "--enter")
                stack.wait(lambda: (root / marker).is_file(), "local execution during dependency loss", seconds=5)
            stack.fault_smoke(
                on_redis_down=lambda: local_while_down("LOCAL_WHILE_REDIS_DOWN"),
                on_primary_down=lambda: local_while_down("LOCAL_WHILE_PRIMARY_DOWN"),
            )
            stack.wait(lambda: remote("reconcile", machine, key)["session"] == session,
                       "original native key after dependency recovery", seconds=60)
            route = None
            def active_route():
                nonlocal route
                candidate = stack.sql(f"SELECT node FROM qgr.machine_routes WHERE machine='{machine}' AND activated")
                if not candidate:
                    return False  # A new generation can be awaiting activation.
                matched = re.fullmatch(r"development-([1-4])_msg_[0-9a-f]{32}", candidate)
                if not matched:
                    raise ValueError("active native route does not belong to this development deployment")
                # Routing scopes a configured node by its per-process nonce.
                route = matched[1]
                return True
            stack.wait(active_route, "actual native hosting gateway after recovery", seconds=60)
            node = "gateway-" + route
            stack.call("kill", "--signal", "SIGKILL", node)
            stack.call("up", "--detach", "--no-deps", node)
            stack.wait(lambda: stack.probe(int(node.rsplit("-", 1)[1])), "abrupt gateway recovery", seconds=45)
            stack.wait(lambda: remote("reconcile", machine, key)["session"] == session,
                       "original native key after hosting-gateway SIGKILL", seconds=60)
            stack.roll()
            if native.poll() is not None or connector.poll() is not None:
                raise RuntimeError("gateway recreation ended a native owner or connector")
            stack.wait(lambda: any(s["id"] == session and s["state"] == "running" for s in remote("sessions", machine)), "same native session after rolling restart", seconds=30)
            # Inventory is a durable projection and can be visible before the
            # connector has reactivated. Lookup is read-only and requires the
            # actual native route; retrying it never resends the original start.
            stack.wait(lambda: remote("reconcile", machine, key)["session"] == session, "original native start lookup after connector reactivation", seconds=60)
            remote("send", machine, session, "--stdin", "--enter", input=b"AFTER_ROLL")
            stack.wait(lambda: observed(b"AFTER_ROLL"), "new controlled input after explicit reconnect")
            cross = 0
            for _ in range(8):
                remote("sessions", machine)
                cross = sum(int(line.split()[1]) for node in range(1, 5) for line in stack.call("exec", "-T", "edge", "wget", "-Y", "off", "-qO-", f"http://gateway-{node}:9091/metrics", capture=True, timeout=8).decode().splitlines() if line.startswith("qgr_cross_node_requests_sent_total "))
                if cross:
                    break
                time.sleep(1.05)
                token_file.write_text(stack.mint())
            if not cross:
                raise RuntimeError("actual cross-node dispatch was not observed")
            remote("terminate", machine, session, "--yes")
            stack.wait(lambda: any(s["id"] == session and s["state"] == "exited" and s.get("ended_at") for s in remote("sessions", machine)), "native termination projection")
            def recording():
                index = remote("history-streams", session)
                output = b""
                for stream in index["streams"]:
                    after = None
                    for _ in range(16):
                        arguments = ["history", session, "--stream-epoch", stream["stream_epoch"]]
                        if after:
                            arguments += ["--after", after]
                        page = remote(*arguments)
                        for item in page["items"]:
                            for record in item.get("records", []):
                                if record["content"]["type"] == "output":
                                    output += base64.b64decode(record["content"]["data"])
                        after = page["next"]
                        if not after:
                            break
                if b"development-fixture-secret" in output:
                    raise RuntimeError("gateway recording redaction was absent")
                return all(marker in output for marker in (b"BEFORE_ROLL", b"AFTER_ROLL", b"LOCAL_WHILE_REDIS_DOWN", b"LOCAL_WHILE_PRIMARY_DOWN"))
            stack.wait(recording, "verified protected recording across gateway restarts", seconds=30)
            print("Actual CLI/native start, input, original-key reconciliation, four rolling gateway recreations, cross-node dispatch, termination and redacted protected history passed.")
            print("The same native PTY executed local input during Redis and primary outages and survived its actual hosting-gateway SIGKILL; retained recording and generation reconciliation passed.")
        finally:
            if connector and connector.poll() is None:
                connector.terminate()
                try:
                    connector.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    connector.kill()
                    connector.wait(timeout=5)
            if native and native.poll() is None:
                subprocess.run([str(binary), "--state-dir", str(native_state), "stop", "--terminate-sessions"], stdout=subprocess.DEVNULL, stderr=diagnostics, timeout=5, check=False)
                try:
                    native.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    native.kill()
                    native.wait(timeout=5)
            diagnostics.close()
            destination = stack.args.state / "last-smoke.log"
            # Diagnostic streams never contain bearer tokens or request bodies.
            # Preserve the owned private file when the temporary fixture is removed.
            descriptor = os.open(destination, os.O_WRONLY | os.O_CREAT | os.O_TRUNC | os.O_NOFOLLOW, 0o600)
            with os.fdopen(descriptor, "wb") as output, (root / "diagnostics.log").open("rb") as source:
                shutil.copyfileobj(source, output)
