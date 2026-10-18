# Operational recovery and qualification

These procedures distinguish the exercised development deployment from production
operations that need assigned owners and infrastructure. Never infer native process death
from an offline connector, and never replay input or invent a new start key to resolve an
uncertain response. Preserve original session/start/decision/receipt identities.

## Gateway drain and compatible upgrades

Use `stack.py roll` on the owned development project. It applies additive migrations,
recreates one gateway at a time, waits for actual readiness and verifies its running image
matches the image selected at rollout start. Plain restart retains the previous image.
SIGTERM marks the node unready before socket drain; reconnect does not resend a mutation.
`smoke` has exercised four recreations with the same native session,
original start lookup and redacted history. Keep the 75-second container stop grace greater
than the 60-second drain budget.

Before changing a binary or schema, compare its supported schema range and apply only its
explicit additive migrations with the isolated migration credential. Runtime gateways do
not receive migration credentials. Verify readiness, directory access and protected writes
afterward. Roll back a binary only if its schema range includes the installed schema; never
rewrite an applied migration. Native owner upgrades have their own idle/explicit-shutdown
guard. A gateway/connector update grants no permission to restart sessiond. Linux user-manager
and macOS launchd upgrade/logout behavior remain unqualified here.

## Redis loss and recovery

Inspect private `/ready` and `/metrics`. Redis loss retains primary-backed readiness with
`ready degraded` and `qgr_routing_available 0`; liveness remains. Same-node operations can
continue only under current primary/native authority. Cross-node requests can fail or remain
uncertain. Native execution and independent capture continue. Restore the configured TLS
Redis service and verify resubscription, current host generation and explicit replay/resync.
Affinity is a load hint and cached presence never grants authority.

`stack.py fault-smoke` actually stops/restores Redis and verifies this behavior. It also
stops/restores the primary and checks that all four nodes become unready while liveness
continues. Run it only on its owned development project.

## PostgreSQL failover and receipts

Primary loss denies new admissions/authority changes, and established access expires at its
original deadline. Sessiond and native processes keep running. A lost or canceled COMMIT is
uncertain, including when the old primary committed locally. Never acknowledge content,
admin history or completed maintenance from a locally committed/canceled transaction.

For an actual failover, first fence the old primary so it cannot accept writes, select the
qualified standby and preserve its recovery timeline. Promotion requires operator ownership;
there is no automated failover command in this product. Keep restored/promoted recording
state quarantined with a new recovery epoch until matching synchronous durability and
lifecycle qualification are established. Inventory/control must reconcile durable/native
generation floors without lowering either. Obtain independently authenticated current
epoch evidence before reclaiming any native spool entry. Retry the exact immutable chunk
or gap identity to recover its original receipt; never create replacement identities or
delete the spool merely to clear a backlog.

`tests/e2e/recording_durability.py` exercises actual primary tmpfs destruction, standby
promotion, preserved held bytes/audits, quarantine and exact receipts on owned test resources.
It establishes same-host behavior, not independent production fault domains or operator
readiness. A promoted server without a synchronous partner cannot acknowledge protected
writes just because old acknowledged content is readable.

## Backups, restore and expiry

`stack.py backup` writes private custom-format dumps and publishes their final filenames
only after successful completion and file/directory fsync. Failed partial files do not count
as fresh backups. `restore-smoke` restores into a generated isolated database, sets a new
epoch/quarantine, checks schema and drops only that database. This drill passed. It never
switches live authority. Verify the backup itself, required roles/configuration, encryption
and restore access before declaring a production recovery point.

Production needs an off-server encrypted target, named backup and restore owners, retention,
restore-point objectives, periodic restore drills and a documented expiry process. None is
assigned/configured in this checkout. Backups can retain content beyond online retention;
holds and legal requirements must be considered before deleting them. Do not count a local
dump on the primary server as protection against server or site loss.

## Recording pressure and maintenance

Inspect native warnings, durable gap receipts, pending spool bytes and oldest-upload age.
Investigate uploader credentials/epoch, synchronous standby, redaction/configuration,
database permissions and actual disk space. Capture admission must remain independent of
network/SQLite stalls. The native cap evicts content only after durable ordered loss metadata;
this is an explicit loss, not successful recording. Keep native capture running through a
connector repair. Do not unlink SQLite/WAL/chunk files, lower retention or manually clear
warnings/receipts to manufacture health.

Lifecycle work uses the separate bounded `jobs` command/credential and PostgreSQL lock, not
Redis leadership. Development schedules it hourly. Alert on failure or exhausted work; retry
bounded units, respecting uncertainty. See [recording lifecycle](../operations/recording-lifecycle.md)
for hold privacy, retained tombstones, month retirement and recovery boundaries.

## Revocation and directory diagnostics

Verify the current issuer/audience/tenant and stable directory object, manual kill switches,
machine-key revision, fresh accountEnabled evidence and independent native access deadline.
A valid JWT alone is not current directory evidence. Restarts and successful token refreshes
cannot reset original evidence age. A stale directory provider denies affected principals
without killing their owned native processes. Redis loss cannot extend revocation authority.

Never log bearer tokens, proof material, terminal content or per-user/session metric labels.
Revocation initially reports pending. After observing the committed primary row for a full
ten-second suspend-aware authority horizon, that gateway can report completed: old authority
can no longer admit remote I/O. Each status response still requires the primary; restart,
eviction or another node's first observation waits again. Completion does not terminate or
wait for native work already admitted. See [ADR 0009](../adr/0009-revocation-completion.md).
Production Entra tenant and suspend/platform qualification remain outstanding.

## Telemetry, alerts and assignments

Private gateway metrics expose authority readiness, routing availability, draining,
connections, pending routes, cross-node requests and aggregate directory freshness. Fixed
counters also expose transport bytes, heartbeat failures, queue saturation for exactly
control/event/bulk priorities, client/replay requests and approval choice attempts. The
native approval delivery histogram measures each claimed delivery attempt from submission
through received/persisted result or error, including failed/redelivered attempts. It excludes
claim SQL and later retry scheduling; it is not commit-to-native-application latency.
Scraping a single coherent histogram snapshot cannot mix cumulative bucket updates. The
freshness sample uses the original suspend-aware clock and considers principals used in
the last 900 seconds. It never refreshes evidence, changes last-use times or publishes
principal identities. Clock/lock failures have a separate collection-failure gauge.

Compose starts a pinned Prometheus 3.7.3 evaluator on the private bridge. It scrapes all
four gateways and the OTel operator exporter every 15 seconds and evaluates
`deploy/compose/alerts.yaml`. The 13 rules cover authority/readiness, missing scrapes,
directory freshness, routing, probe failure/staleness, recording gaps/lag/warnings, disk,
completed-backup age and the synchronous replica. Rule fixtures exercise sustained failure,
recovery and a missing probe with the real evaluator binary. No notification receiver or
external destination is configured; firing state is available through the private API.
Prometheus data is ephemeral, bounded to a 128 MiB tmpfs with six-hour/96 MB retention and
16 MB WAL segments. Recreation discards operational samples, never product recordings.

Run the scheduled operator probe alongside the development stack:

```sh
python3 deploy/compose/stack.py --native-state /absolute/private/native-state monitor
python3 deploy/compose/stack.py monitor-smoke
```

The default schedule is every 30 seconds until stopped. `--monitor-interval-seconds` (1–60)
and `--monitor-iterations` (0 means continuous) support bounded drills. The probe needs the
same explicit local Docker/operator privileges as `telemetry`; collectors receive neither
Docker sockets nor database/native credentials. It sends aggregate OTLP/HTTP through an
owned edge exec into the private collector, with no new published port. `--native-state`
adds read-only private spool lag/bytes/warnings. Without that option the native measurements
are uncollected, rather than healthy zeros. Native probes must use the actual active state;
a removed smoke fixture directory is not a long-lived monitoring target.

Collection failures publish a zero success gauge. Stopping the probe triggers its independent
staleness/missing alert. To inspect firing state, use an owned private edge exec to read
`http://prometheus:9090/api/v1/alerts`; do not expose this endpoint through the public edge.
`monitor-smoke` recreates only the ephemeral monitoring pair, verifies all rule fixtures,
all four gateway scrapes, directory metrics and aggregate sample ingestion. CI runs it.
The host probe itself must be kept running by the operator; Compose cannot schedule a
host process or read a host user's private native state. Platform service installation
and actual notification routing remain operational qualification steps.

Transport callbacks increment atomics without locks or I/O. Labels never contain principals,
session IDs, approval contents or terminal output. Representative latency distributions and
alert thresholds still need qualification. These development metrics and
rule tests do not establish production notification delivery or on-call readiness.

Current assignments: production on-call **unassigned**, database/failover owner **unassigned**,
backup/restore owner and target **unassigned**, identity/certificate renewal owner **unassigned**,
capacity/three-server hosts **unallocated**. Before wider rollout, assign each role, verify alert
routing and rehearse the relevant procedure with its actual credentials and failure domains.
