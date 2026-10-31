# Capacity and host resource evidence

## Release host measurement — 2026-10-26

[Task 030](../../tasks/030-complete-linux-release-readiness.md) measures both host processes
from the same release binary. The connector remains authenticated. No session is active.
The sample follows real PTY execution and recording acknowledgment in the composed fixture.
The measurement lasts 60 seconds with one RSS sample per second. It sums sessiond and
connector RSS and CPU. CPU percentages use one core. MB means 1,000,000 bytes.

| Measurement | Before repairs | Final release build | PRD target |
| --- | ---: | ---: | ---: |
| Aggregate maximum sampled RSS | 32.58 MB | 32.53 MB | Less than 30 MB |
| Aggregate idle CPU | 1.72% | 0.57% | Less than 0.5% |
| Resource target result | Not met | Not met | Both must pass |

The [before sample](host-footprint-before-2026-10-26.json) and
[final composed result](linux-completion-2026-10-26.json) retain exact bytes, CPU values,
binary hashes, and limitations. Explicit recording wakeups and notification-driven inventory
reads reduce idle CPU. Releasing sealed encoders removes a lifetime capacity limit. This
small workload does not establish a measurable RSS improvement from that lifecycle repair.

A diagnostic spot sample reports 23.21 MB aggregate proportional set size (PSS). PSS
apportions shared mappings between processes. Most RSS in this sample is file-backed;
only 7.57 MB is anonymous memory. This explains why summed RSS exceeds actual proportional
memory, but it does not replace the PRD's RSS metric or close its acceptance gate.

The functional composed proof reports its own status. The separate `host_footprint` phase
reports `target_not_met`. Neither result establishes a long-run or representative fleet
budget. The sampler excludes child memory and socket count. The next resource step is to
profile remaining timer work and resident code pages under representative use. Preserve
independent process ownership, recording, and execution-time authorization checks.

Reproduce the measurement with the release command in
[tests/load/README.md](../../tests/load/README.md). The final binary enables managed Codex.
Actual Codex proof phases use the pinned native executable and a synthetic provider.

## Reduced connected baseline — 2026-10-18

This executed local baseline does not establish beta-scale qualification. The sample used
the uncommitted task implementation on branch `work`, schema 15, Rust 1.99.1, PostgreSQL 17.11,
Redis 8, PgBouncer 1.24.1 and the pinned development images. The gateway image is 19,625,695
bytes and the separate synthetic identity image 5,621,503 bytes; both run as UID 10001.
The [aggregate sample](capacity-baseline-2026-10-18.json) preserves the executed 200-echo
report without credentials, content, machine identities or container IDs. This refreshed sample includes native change subscriptions, bounded revocation reporting and
transport/replay/approval instrumentation. Later probe-only runs can overwrite the private
development `last-baseline.json` without changing this sample.

The execution environment reports Linux 6.18.55 x86_64, glibc 2.41, five logical CPUs and
18,882,674,688 bytes of visible memory. It has a 32 GiB writable filesystem. Container
limits, private bridges and same-host physical synchronous standby are defined in Compose.
The release builds and workspace checks completed before this sample; the private
evaluator and collector were running. Corporate Entra and
representative network paths were not used. The short CPU snapshot does not establish
steady-state utilization or peak memory.

The reproducible command and exact measurement definition are in
[tests/load/README.md](../../tests/load/README.md). Eight persistent authenticated WSS clients
and two simultaneous actual native PTYs ran 100 unique echo requests each. Each terminal
first emitted 51,200 repetitive bytes. Native stdin echo was disabled. SDK input was never
resent. Original persisted keys reconciled to both native sessions. Known terminals were
explicitly terminated and the sealed native recording spool reconciled to zero pending
reservations, chunks and gaps. Actual native lag, pending and warning metrics also reached
the private evaluator, as recorded in the [alert validation](operational-alerts-2026-10-18.json).

| Measurement | Executed result |
| --- | ---: |
| Persistent sockets | 8 |
| Simultaneous terminal echo workers | 2 |
| Echo samples | 200 |
| Input-to-native-output observation p50 | 46.53 ms |
| p95 | 58.03 ms |
| p99 | 60.13 ms |
| Echo sampling duration | 3.500 s |
| Native output observed, including initial bursts | 105,410 bytes |
| Protected normalized recording bytes | 173,041 bytes |
| Protected compressed bytes / chunks | 3,278 bytes / 5 |
| Workload WAL, including start/control/audit/termination | 232,480 bytes |
| Final standby replay lag | 0 bytes |
| Idle sessiond RSS / connector RSS | 27.50 MiB / 24.10 MiB |
| Two-second idle sessiond CPU / connector CPU | 1.00% / 0.00% of one core |
| RSS sampled after load, sessiond / connector | 31.37 MiB / 25.11 MiB |
| Known native cleanup failures / input retries | 0 / 0 |

Latency includes the input acknowledgment and a bounded native replay read with a
one-millisecond polling interval. No direct PTY baseline was subtracted. This cannot
establish the PRD's pure echo-overhead target or predict interactive TUI latency. Supporting
TUI peers now consume pushed heads; 250 ms idle reconciliation repairs missed hints. Final zero byte lag is a point observation,
not a five-second write-lag distribution. Repetitive bursts compress unusually well: normalized expansion was 1.642×,
compressed/normalized ratio 0.01894 and total workload WAL/output ratio 2.205×.

The [reproducible storage estimate](storage-model-2026-10-18.json) from
`tests/load/storage_model.py` incorporates these measurements with explicit future traffic
assumptions. At 2,000 terminals, 51,200 raw bytes/s while active, 10% duty cycle, 120-day
rounded ordinary retention, 5% held copies with a further 365-day hold horizon, one replica,
three full backup copies, seven days of WAL and 1.5× row/index allowance, it estimates about
51.9 TiB using this synthetic compression. The uncompressed-normalized scenario is about
1,453.9 TiB. These are estimates, not recommended allocations. Indefinite audit/receipt
tombstones, unlimited holds, other structured traffic, archive/restore staging and safety
margin are explicitly outside those finite totals. Representative traffic, WAL and backups
must be measured before assigning storage.

Actual dependency/private-port and isolated backup/restore drills pass; the connected PTY
survives all four rolling gateway restarts and protected history is verified/redacted.
The separate synchronous primary-volume-loss proof covers held references and acknowledged
content. The separate synchronous managed proof measured five native approval delivery attempts
(0.03573 seconds aggregate, including fault-injected attempts), with its precise definition
in that artifact. This reduced PTY baseline does not qualify production failover, off-server
restore, real tenant, systemd/launchd lifetime, release latency distributions or on-call response.

Remaining beta evidence: 200,000 sockets, 20,000 simultaneous structured streams, 2,000
simultaneous terminal burst producers at 50 KB/s, distributed generators, latency and lag
distributions, representative duty/compression, long-run RSS/CPU/disk, suspended hosts,
production fault domains, backup destination and named operators. Tool limits and local
extrapolations do not satisfy those workloads. Task 028 is complete for Linux development; these [external release gates](external-release-gates.md) remain unrun.
