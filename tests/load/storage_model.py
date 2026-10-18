#!/usr/bin/env python3
"""Explicit estimates from an actual baseline; never beta capacity evidence."""
import argparse
import json
import math
from pathlib import Path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("baseline", type=Path)
    parser.add_argument("--active-terminals", type=int, default=2000)
    parser.add_argument("--raw-bytes-per-active-second", type=float, default=51200)
    parser.add_argument("--duty-cycle", type=float, default=0.1)
    parser.add_argument("--held-fraction", type=float, default=0.05)
    parser.add_argument("--hold-extra-days", type=int, default=365)
    parser.add_argument("--replicas", type=int, default=1)
    parser.add_argument("--backup-copies", type=int, default=3)
    parser.add_argument("--wal-days", type=int, default=7)
    parser.add_argument("--row-index-factor", type=float, default=1.5)
    args = parser.parse_args()
    values = (args.raw_bytes_per_active_second, args.duty_cycle, args.held_fraction, args.row_index_factor)
    if not all(math.isfinite(value) for value in values) or args.active_terminals < 1 or args.raw_bytes_per_active_second <= 0 or not 0 <= args.duty_cycle <= 1 or not 0 <= args.held_fraction <= 1 or min(args.hold_extra_days, args.replicas, args.backup_copies, args.wal_days) < 0 or args.row_index_factor < 1:
        parser.error("invalid storage assumptions")
    report = json.loads(args.baseline.read_text())
    output = report["output_bytes"]
    normalized = report["recording"]["normalized_uncompressed_bytes"]
    compressed = report["recording"]["compressed_bytes"]
    if min(output, normalized, compressed) <= 0:
        parser.error("baseline must contain actual nonzero output and protected recording sizes")
    daily_raw = args.active_terminals * args.raw_bytes_per_active_second * args.duty_cycle * 86400
    expansion = normalized / output
    observed_compression = compressed / normalized
    wal_factor = report["database"]["wal_bytes_during_workload"] / output
    def estimate(compression):
        daily_content = daily_raw * expansion * compression
        ordinary = daily_content * 120
        held_copies = daily_content * args.held_fraction * (120 + args.hold_extra_days)
        data = (ordinary + held_copies) * args.row_index_factor
        wal = daily_raw * wal_factor * args.wal_days
        return {"daily_content_bytes": daily_content, "ordinary_rounded_120_day_bytes": ordinary,
                "additional_held_copy_bytes": held_copies, "primary_rows_indexes_estimate_bytes": data,
                "primary_wal_estimate_bytes": wal, "replicated_online_estimate_bytes": (data + wal) * (1 + args.replicas),
                "backup_full_copies_estimate_bytes": data * args.backup_copies,
                "combined_estimate_bytes": (data + wal) * (1 + args.replicas) + data * args.backup_copies}
    print(json.dumps({"kind": "estimate_only", "raw_daily_bytes": daily_raw, "assumptions": vars(args) | {"baseline": str(args.baseline)},
                      "measured_normalization_expansion": expansion, "measured_compressed_to_normalized_ratio": observed_compression,
                      "measured_workload_wal_per_output_byte": wal_factor, "synthetic_observed_compression_scenario": estimate(observed_compression),
                      "uncompressed_normalized_scenario": estimate(1.0),
                      "excluded_unbounded_costs": ["indefinite receipt/audit/fragment tombstone accumulation", "indefinite holds beyond the explicit model horizon", "other structured agent traffic", "incremental backup/WAL archive/restore staging", "provisioning safety margin"],
                      "limitations": ["repetitive synthetic bursts strongly favor compression", "row/index factor and future duty cycle are assumptions", "workload WAL includes starts, controllers and termination", "neither result qualifies beta storage capacity"]}, indent=2))


if __name__ == "__main__":
    main()
