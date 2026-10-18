"""Private aggregate development probes; no Docker socket or credentials enter collectors."""
import json
import math
from pathlib import Path
import re
import subprocess
import time
import urllib.parse


def publish(stack, values=None):
    try:
        values = dict(stack.telemetry(emit=False) if values is None else values)
        if any(not re.fullmatch(r"qgr_development_[a-z_]+", name)
               or not math.isfinite(float(value)) for name, value in values.items()):
            raise ValueError("invalid aggregate sample")
    except (OSError, RuntimeError, subprocess.TimeoutExpired, ValueError, TypeError):
        values = {"qgr_development_collection_success": 0}
    values["qgr_development_probe_timestamp_seconds"] = time.time()
    timestamp = str(time.time_ns())
    body = json.dumps({"resourceMetrics": [{
        "resource": {"attributes": [{"key": "service.name", "value": {"stringValue": "qgr-development-probe"}}]},
        "scopeMetrics": [{"scope": {"name": "qgr-development"}, "metrics": [
            {"name": name, "gauge": {"dataPoints": [{"timeUnixNano": timestamp, "asDouble": float(value)}]}}
            for name, value in sorted(values.items())]}],
    }]})
    response = stack.call("exec", "-T", "edge", "wget", "-Y", "off", "-T", "4", "-qO-",
                          "--header", "Content-Type: application/json", "--post-data", body,
                          "http://otel:4318/v1/metrics", capture=True, timeout=8)
    partial = json.loads(response).get("partialSuccess", {})
    if int(partial.get("rejectedDataPoints", 0)):
        raise RuntimeError("collector rejected aggregate development metrics")
    return values["qgr_development_collection_success"] == 1


def monitor(stack):
    deadline = time.monotonic()
    iteration = 0
    while not stack.args.monitor_iterations or iteration < stack.args.monitor_iterations:
        try:
            success = publish(stack)
        except (OSError, RuntimeError, subprocess.TimeoutExpired, ValueError):
            success = False
        print("Development operational probe: " + ("collected and published" if success else "unavailable"), flush=True)
        iteration += 1
        if not stack.args.monitor_iterations or iteration < stack.args.monitor_iterations:
            deadline = max(deadline + stack.args.monitor_interval_seconds, time.monotonic())
            time.sleep(max(0, deadline - time.monotonic()))


def get(stack, path):
    result = json.loads(stack.call("exec", "-T", "edge", "wget", "-Y", "off", "-T", "4", "-qO-",
                                   "http://prometheus:9090" + path, capture=True, timeout=8))
    if result.get("status") != "success":
        raise RuntimeError("private evaluator query failed")
    return result["data"]


def exercise(stack):
    # Monitoring data is explicitly ephemeral in this development stack. Recreate
    # to evaluate the selected configuration instead of retaining old rules.
    stack.call("up", "--detach", "--no-deps", "--force-recreate", "otel", "prometheus")
    stack.call("run", "--rm", "--no-deps", "--entrypoint", "/bin/promtool", "prometheus",
               "test", "rules", "/etc/prometheus/alerts-test.yaml", capture=True, timeout=30)
    stack.wait(lambda: publish(stack), "aggregate probe publication", seconds=30)
    def value(expression):
        return get(stack, "/api/v1/query?" + urllib.parse.urlencode({"query": expression}))["result"]
    stack.wait(lambda: bool(value("qgr_development_collection_success == 1")),
               "evaluated private operational sample", seconds=45)
    stack.wait(lambda: any(float(row["value"][1]) == 4 for row in value('count(up{job="qgenie-remote-gateway"} == 1)')),
               "all four gateway scrapes", seconds=45)
    stack.wait(lambda: any(float(row["value"][1]) == 4 for row in value('count(qgr_directory_enabled == 1 and qgr_directory_collection_success == 1)')),
               "elapsed directory evidence on all gateways", seconds=45)
    for name in ("qgr_transport_bytes_in_total", "qgr_replay_requests_total",
                 "qgr_native_approval_delivery_attempt_seconds_count"):
        stack.wait(lambda metric=name: any(float(row["value"][1]) == 4
                   for row in value(f'count({metric})')),
                   "bounded transport/replay/approval observations", seconds=45)
    stack.wait(lambda: any(float(row["value"][1]) == 12
               for row in value('count(qgr_transport_queue_full_total)')),
               "three fixed queue priorities on all gateways", seconds=45)
    rules = get(stack, "/api/v1/rules")["groups"]
    observed = {rule["name"]: rule for group in rules for rule in group["rules"]}
    expected = set(re.findall(r"alert: (\w+)", Path(__file__).with_name("alerts.yaml").read_text()))
    if set(observed) != expected or any(rule.get("health") != "ok" for rule in observed.values()):
        raise RuntimeError("configured development alerts are missing or unhealthy")
    print(f"All {len(expected)} alert rules passed real evaluator firing/recovery fixtures; four private gateway scrapes and aggregate probe ingestion passed.")
