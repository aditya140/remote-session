# Recording lifecycle operation

This describes the protected recording components, admin hold route and bounded `jobs`
command. Public history/upload, native uploader and development lifecycle scheduling are
composed and verified on owned fixtures. Production recovery qualification remains separate.

## Credentials and authorization

Use distinct non-owner credentials for authority, audit, ingest, owner history, admin
history and recording lifecycle. `RecordingDatabase::connect` checks exclusive purpose,
memberships and table privileges. Lifecycle credentials have narrow function access and
audit append access; they cannot directly read or modify recordings or hold tables.

`change_hold` and `retention_step` require a currently verified platform administrator,
current directory evidence and the isolated lifecycle pool. The administrator plane remains
available during the user kill switch. No affected-user notification or hold metadata belongs
in owner responses. Held reads require the current matching case and an audited reason.

For the hold endpoint, configure `serve --recording-lifecycle-database-url-file PATH`
alongside identity/directory configuration. Startup verifies that the lifecycle credential
belongs to the same database identity as authority. `POST /v1/admin/holds` accepts `HoldChange`
for placement, revision changes and release; its caller needs a signed platform-admin role.
The separate configured ingest/owner/admin pools expose protected upload and audited history
endpoints; those credentials must remain isolated from the lifecycle credential.

## Policy and work units

The database computes `cutoff = clock_timestamp() - retention_days`; the default is 90 days.
UTC monthly partitions are eligible only when their entire half-open range ends at or before
the cutoff. Monthly rounding can retain the earliest rows for about 120 days plus scheduling
delay. Capture time, rather than upload/retry time, determines age. Gap metadata spanning
months expires according to its end month.

`HoldChange` uses a stable typed ID and full-u64 revision. Revision 1 places a hold; each
change/release increments it by one. An identical retry is idempotent, while changed content
under the same revision conflicts. The affected user is immutable. Optional capture windows
are inclusive; a matching interval conservatively preserves the whole overlapping chunk or
gap. A missing case preserves data but authorizes no held read until a case is assigned.

A successful hold change durably fences deletion immediately. Copying existing content
continues in bounded maintenance units. New matching uploads preserve held copies within
their ingest transaction, including late content whose ordinary partition is retired. A
policy-expired receipt still reports ordinary expiry; it neither promises owner availability
nor reveals private hold coverage.

Call `retention_step` with a reason and batch size 1–32 until it returns `Idle`. Every call
uses its own transaction. Scheduling and cancellation belong to the caller. Retry failed
units with bounded backoff; a timeout is not proof of rollback, and the next unit reconciles
the committed database state. PostgreSQL 17+ bounds active SQL, with two-second statement/lock
deadlines, but disables these timers during synchronous COMMIT. The owned commit supervisor
uses the original three-second transaction budget, then up to one second each for protocol
cancellation and response drain. Pool cleanup also bounds its response drain. Canceled
frontends are retired. Cancellation is not rollback: no receipt, admin page or completed job
may be reported from a canceled/local-only commit. Native authority still expires independently
if the cancellation connection or database is unreachable. Do not run an unbounded retry loop.

Nightly scheduling can invoke the dedicated command with a lifecycle credential binding:

```sh
qgenie-remote-gateway jobs --database-url-file /run/secrets/recording-lifecycle-url \
  --batch-size 32 --max-steps 1024 --reason 'scheduled retention'
```

The command uses the fixed maintenance capability, without a fabricated human token. It
cannot place/change/release a hold and audits using reserved system identity
`u_00000000-0000-0000-0000-000000000000`. Each unit commits before the next begins. It exits
nonzero on interruption, unavailable qualification, SQL/audit failure or exhausted work budget; a scheduler
must retry incomplete work and alert on repeated failures. SIGINT/SIGTERM cancels the current
future and closes the pool; the next invocation reconciles committed state. Development
Compose runs the bounded scheduler hourly; omitted `--interval-seconds` retains the one-shot
behavior above. No production schedule or operating ownership is assigned.

Ingest, hold changes, copy and retirement share transaction-scoped PostgreSQL advisory lock
5856751401703146820. Redis leadership grants no permission to delete. Copy transactions
release the lock between batches. Final retirement verifies exact held bytes and all current
active overlapping coverage under that lock, then drops the partition and records its retired
tombstone in one audited transaction. A hold committed before final deletion participates in
that check. Retired months are never automatically recreated.

Changing/releasing a hold immediately changes held-read authorization. Bounded cleanup
removes obsolete coverage, preserving copies still required by any other hold. Eligible
uncovered held copies then expire. Crashed or multiple workers resume the same database
states. Audit failure rolls back hold mutation, copy/cleanup or transactional partition DROP.

## Metadata, recovery and backups

Held copies reference immutable receipt ledgers, which reference original session/machine/
owner metadata. Audit records are append-only. Receipt/fragment tombstones, referenced
metadata and audit events currently remain indefinitely; their separate bounded retirement
policy is not implemented. The accepted retry floor defaults to 1970. Never delete tombstones
while still accepting fresh retries for their original time range. Include this metadata growth
in task 028 capacity measurements.

New databases start unqualified and recovery-quarantined. Production ingest and destructive
maintenance require qualified synchronous/quorum replication and `remote_apply`; tests alone
have a private synthetic single-server path. An unavailable durable boundary cannot silently
downgrade to asynchronous acknowledgment. Directory and original authority deadlines still
apply while waiting for SQL/commit.

Before any promotion/restore is released, fence old connections, establish a new independently
authenticated recovery epoch, reconcile receipt coverage, hold state and authority generations,
and account for acknowledged bytes absent from the restored volume. Retained host content can
be retried; old receipts cannot reconstruct content the host already reclaimed. Record explicit
recovery loss when evidence establishes missing ranges. The disposable synchronous-pair/primary-tmpfs-loss drill now passes, including receipt/hold/audit
coverage, new recovery epoch and quarantine. Automated deployed recovery/drain and backup
restores remain unqualified. A promoted single node cannot acknowledge new content or admin
audit pages until a synchronous partner is re-established.

Live partition deletion does not erase backups. Backup expiry, restore reconciliation and
operating ownership must be assigned and tested before production release. No backup/legal
retention period or zero-loss recovery result has been established by these component tests.

An upgrade from experimental schema-9/11 held rows requires reconciliation into actual hold
records before schema 12 can add its coverage foreign keys. Do not invent a legal hold or
silently discard the old rows to make the migration pass.
