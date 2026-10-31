#!/usr/bin/env python3
"""Sample two existing, owned Linux host processes. Do not start or stop them."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import time


def sample(pid):
    root = Path(f"/proc/{pid}")
    fields = (root / "stat").read_text().rsplit(")", 1)[1].split()
    if fields[0] in ("Z", "X"):
        raise RuntimeError("The measured host process exited")
    pages = int((root / "statm").read_text().split()[1])
    return {
        "start_ticks": int(fields[19]),
        "rss_bytes": pages * os.sysconf("SC_PAGE_SIZE"),
        "cpu_seconds": (int(fields[11]) + int(fields[12])) / os.sysconf("SC_CLK_TCK"),
        "threads": int(fields[17]),
    }


def measure(native, connector, seconds):
    pids = {"native": native, "connector": connector}
    executables = [Path(f"/proc/{pid}/exe").resolve(strict=True) for pid in pids.values()]
    if executables[0] != executables[1]:
        raise RuntimeError("Measure both host processes from the same binary")
    with executables[0].open("rb") as stream:
        binary_hash = hashlib.file_digest(stream, "sha256").hexdigest()
    # Let the prior workflow and its recording acknowledgement settle.
    time.sleep(3)
    initial = {name: sample(pid) for name, pid in pids.items()}
    samples = [initial]
    started = time.monotonic()
    while time.monotonic() - started < seconds:
        time.sleep(min(1, max(0, seconds - (time.monotonic() - started))))
        current = {name: sample(pid) for name, pid in pids.items()}
        for name in pids:
            if current[name]["start_ticks"] != initial[name]["start_ticks"]:
                raise RuntimeError("A measured host process was replaced")
        samples.append(current)
    elapsed = time.monotonic() - started
    processes = {}
    for name in pids:
        processes[name] = {
            "rss_max_bytes": max(point[name]["rss_bytes"] for point in samples),
            "rss_final_bytes": samples[-1][name]["rss_bytes"],
            "cpu_percent_one_core": 100 * (samples[-1][name]["cpu_seconds"] - initial[name]["cpu_seconds"]) / elapsed,
            "threads_max": max(point[name]["threads"] for point in samples),
        }
    rss = max(sum(point[name]["rss_bytes"] for name in pids) for point in samples)
    cpu = sum(processes[name]["cpu_percent_one_core"] for name in pids)
    return {
        "format": "qgr-host-footprint-v1",
        "platform": {"system": platform.system(), "arch": platform.machine()},
        "binary_sha256": binary_hash,
        "binary_bytes": executables[0].stat().st_size,
        "sample_seconds": elapsed,
        "sample_count": len(samples),
        "processes": processes,
        "aggregate_rss_max_bytes": rss,
        "aggregate_cpu_percent_one_core": cpu,
        "targets": {
            "rss_less_than_bytes": 30_000_000,
            "cpu_less_than_percent_one_core": 0.5,
            "rss_met": rss < 30_000_000,
            "cpu_met": cpu < 0.5,
        },
        "limits": ["RSS sampled once per second; transient peaks can be missed",
                   "No network socket count or child-process memory measurement",
                   "One Linux fixture host; no fleet or long-run qualification"],
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--native", type=int, required=True)
    parser.add_argument("--connector", type=int, required=True)
    parser.add_argument("--seconds", type=int, default=60)
    parser.add_argument("--report", type=Path, required=True)
    args = parser.parse_args()
    if platform.system() != "Linux" or not 30 <= args.seconds <= 300:
        parser.error("Measurement requires Linux and a 30–300 second interval")
    report = measure(args.native, args.connector, args.seconds)
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(json.dumps(report, indent=2) + "\n")


if __name__ == "__main__":
    main()
