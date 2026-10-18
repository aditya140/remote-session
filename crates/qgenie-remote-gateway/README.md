# qgenie-remote-gateway

HTTP gateway application, PostgreSQL authority foundation, and append-only audit. Direct product dependencies: proto, policy, and transport with server-ws; no session owner, adapters, UI, keyring, or local IPC dependencies.

## Running and credentials

```sh
qgenie-remote-gateway migrate --database-url-file /run/secrets/migration_dsn
qgenie-remote-gateway serve --database-url-file /run/secrets/app_dsn \
  --audit-database-url-file /run/secrets/audit_dsn \
  --node-id node-1 --listen 0.0.0.0:8081 --health-listen 0.0.0.0:9091
```

Each node uses explicit public/health ports; the four-node deployment is task 026. Terminate TLS at the trusted corporate edge. Libraries do not install tracing; the binary installs caller-configured JSON tracing. DSNs load from bounded regular files (maximum 16 KiB), allow service-readable mode 0440/0600, reject world-readable files, and have redacted Debug/error output. Credentials do not appear in argv. Config validates node IDs, distinct endpoints, and pool bounds.

Use a dedicated migration identity owning the `qgr` schema and able to create the initial NOLOGIN groups. Login roles for the service are provisioned separately and granted `qgr_app` or `qgr_audit`. App and audit credentials must be separate, non-superuser, non-owner, without privileged-role membership, BYPASSRLS, CREATEDB, or CREATEROLE. App credentials cannot read audit content; the audit identity can read it and append the mandatory read event. Neither identity can modify/delete/truncate history, supply audit IDs, or run migrations.

## Public interfaces and limits

`Gateway::connect(config, authenticator)` composes the application. The verified identity adapter supplies `Principal`; `RejectAuthentication` is the safe default without identity configuration. Task 011 now provides JWT/JWKS and proof-of-possession enrollment; see the [identity configuration and verified contracts](../../docs/integration/gateway-identity.md). Health routes are unauthenticated; all domain/audit routes require authentication. `GET /v1/admin/audit?after=0&limit=100` additionally requires a platform-admin principal and the separately configured audit credential. The read and its own audit event commit in one transaction.

`Store` owns bounded PostgreSQL pools (16 app + 2 audit by default), three-second acquire, five-second statement, three-second lock, and ten-second idle-transaction budgets. Public HTTP work is capped at 256 in-flight requests with load shedding, 64 KiB bodies, and a ten-second request deadline. `/live` reports process liveness. `/ready` checks PostgreSQL and supported schema within two seconds; drain makes it fail immediately. PostgreSQL loss denies authoritative work; Routing preserves same-node reads with live PostgreSQL checks during Redis outage; see the [routing contract](../../docs/integration/gateway-routing.md). [Directory configuration and admin revocation](../../docs/integration/entra-directory.md) provide bounded Graph evidence, independent connection watches and socket I/O gates. Native propagation and real tenant qualification remain task 019.

SIGINT/SIGTERM mark the service unready before draining HTTP work and closing pools. The `jobs` command runs bounded recording lifecycle units under an isolated lifecycle credential. Each unit is audited and production durability/recovery qualification is mandatory; see the lifecycle runbook.

## Migration and audit convention

Migrations are embedded, ordered, checksummed SQL owned by the domain that adds them. A PostgreSQL transaction advisory lock serializes migration on the database. Unknown versions/checksum changes fail. Current foundation schema support is `[1,12]`; enabling identity requires schema 3, routing requires schema 4, directory freshness requires schema 5, admin revocation requires schema 6, and native access routing requires schema 7, and controller proposal/renewal ledgers require schema 8, and recording role/RLS foundations require schema 9, recording transaction/epoch/fragment foundations require schema 10, transaction-bound audited history requires schema 11, and holds/retention require schema 12; later additive migrations must deliberately update this range and keep rolling compatibility. Fresh migration, idempotent rerun, and a real foundation-to-identity schema upgrade are verified. SQLite runtime migrations remain separate.

Domain modules use the crate-private transaction-aware `Store::append_audit` before committing their own change. Audit metadata is a typed set of request/decision IDs and generations; it contains no prompt/input/provider credentials. Database triggers make history append-only. Audit cursor assignment takes a short transaction lock so committed cursor order cannot skip a prior uncommitted writer. Keep the audit append near the domain commit; measure this serialization point in task 028. All audit queries use bound values.

## Verification

```sh
cargo test -p qgenie-remote-gateway
cargo clippy -p qgenie-remote-gateway --all-targets -- -D warnings
cargo build -p qgenie-remote-gateway
QGR_GATEWAY_BINARY="$PWD/target/debug/qgenie-remote-gateway" \
QGR_PG_TEST_URL_FILE=/private/disposable-postgres-dsn \
QGR_REDIS_TEST_URL_FILE=/private/disposable-redis-url \
cargo test -p qgenie-remote-gateway --lib -- --include-ignored --skip transcripts::durability::
python3 tests/e2e/gateway_smoke.py target/debug/qgenie-remote-gateway /private/app-dsn
```

The PostgreSQL test creates a generated test database, checks migration/re-run, role denials, atomic commit/rollback, commit-ordered audit cursors, authenticated/admin-only audit access, schema drift/unsupported versions, actual PostgreSQL connection loss/recovery, and readiness/drain. It requires a disposable PostgreSQL credential with database/role creation rights. CI runs it against PostgreSQL 17 and Redis 8. The same fixture starts two actual gateway processes and proves verified WSS routing, identity separation, bounded observer behavior, Redis interruption/recovery, and presence; host/native integration remains separate. Real Linux PostgreSQL and gateway-process SIGTERM checks passed in the workspace; this is not tenant identity or deployment qualification.

## Recording components

`RecordingValidation::prepare` bounds sixteen actual blocking jobs, decompression, normalized JSONL, fragments and original capture/lifetime times. `RecordingRedaction` seals a separately redacted payload and stored digest while preserving uploaded receipt identity. These preparation values grant no authority.

`RecordingDatabase` isolates ingest, owner history, admin history and lifecycle credentials. It rejects privileged/table-owner/mixed memberships and unexpected table grants. FORCE RLS protects ordinary/held content, gaps, receipts, fragments and case coverage. Direct partition access is denied. Source identities and current primary locks bind every domain operation; history context is additionally bound to its actual transaction ID.

Schema 10 commits content/gaps plus immutable per-recovery-epoch receipts atomically. Every duplicate writes a fresh commit barrier. Schema 11 adds bounded owner/admin pages, full-u64 keyset cursors, audited reasons/exact released IDs and recovery-epoch cursor invalidation. Held reads require matching current case coverage; audit failure prevents a returned page. Owner DTOs contain no hold or administrative context and never certify capture completeness.

Schema 12 adds revisioned per-user holds, optional capture windows/case references, overlapping coverage, new-upload preservation and bounded copy/retirement/cleanup. Ingest and lifecycle changes share one PostgreSQL transaction advisory lock. Final DROP verifies every current active hold and exact preserved bytes. Changed/released holds stop authorizing reads immediately. Public receipt disposition follows ordinary expiry so private preservation cannot disclose hold state.

`Gateway::enable_recording_lifecycle` checks an independently generated database identity against authority. Configure `serve --recording-lifecycle-database-url-file PATH` with identity/directory configuration to serve `POST /v1/admin/holds`. Signed platform-admin authority is mandatory. The `jobs --database-url-file PATH --batch-size 32 --max-steps 1024` command uses the dedicated lifecycle service credential and reserved system audit identity for fixed retention work; it cannot change holds. Scheduling belongs to deployment. See [the lifecycle runbook](../../docs/operations/recording-lifecycle.md).

Production requires schema 12, current applicable identity/directory evidence, lifecycle readiness, no recovery quarantine, and a qualified synchronous/quorum standby with `remote_apply`. PostgreSQL 17+ enforces active-SQL budgets; synchronous COMMIT instead uses an owned three-second transaction budget with bounded cancellation/drain. A canceled local commit cannot yield a receipt/page, and its frontend is retired. Synthetic single-server qualification is private to tests; neither HTTP nor CLI can enable it. Protected upload/history endpoints and native uploader are composed. Schema 13 preserves native-observed end metadata; unknown ends retain a pinned final-projection upper bound. No persistent real-user release has been qualified.

Actual PostgreSQL and PgBouncer 1.24.1 transaction-pooling tests pass signed identity isolation, atomic receipts, lost ACK/conflicts, full-u64 fragments, epoch quarantine, audited history, reused-backend stale context, late held uploads, overlapping holds, concurrent placement/retirement, worker pool restart after committed copy, corruption/audit rollback and monthly rounding. The public hold/CLI boundary tests prove signed-role checks and rejection of unqualified synthetic state; successful production HTTP/job execution also passes on the disposable synchronous pair. Use `tests/e2e/pgbouncer_recordings.py` for the pooling proof. The disposable synchronous/primary-volume-loss proof now passes; production deployment, backup restore, real tenant and capacity qualification remain open.


Enable upload/history with `--recording-ingest-database-url-file`,
`--recording-owner-database-url-file`, `--recording-admin-database-url-file` and
`--recording-redaction-file`, in addition to identity/directory configuration. The literal
policy file is a private bounded JSON array of strings. Purpose credentials remain separate
from authority, audit export and lifecycle. The three recording pools must match the
authority database identity. See [recording integration](../../docs/integration/recording.md)
for routes, bounds, original native access/epoch fencing and uploader retry behavior.
`tests/e2e/recording_durability.py` runs real HTTPS/native lost-ACK recovery plus synchronous
receipt/audit/hold/job and primary-tmpfs-loss proofs. This same-host fixture does not qualify
independent production fault domains, backups or tenant configuration.


Public PTY start additionally requires schema 14 and configured protected recording pools.
Admission uses current identity/directory plus an isolated ingest preflight and a fresh
remote-apply barrier. Native authority is preserved through launch. Primary authority writes
now share the bounded commit supervisor; a canceled synchronous commit is uncertain and its
connection is retired even if its caller disconnects. New session projection and its start
audit commit atomically; repeated inventory does not duplicate that audit.


Public controller acquisition/renewal queries native floors and confirms the PostgreSQL
proposal only after the exact native ACK. Confirmation persists its earlier native boundary.
Host-only control RPCs and PTY action result DTOs are independently scoped/correlated.
Input and ACKs use watched socket gates and original native grants instead of per-keystroke
SQL. Signed Redis hints retain nonce identity; missing cache denies delivery. Routing owns
32 request workers, one ordered response/event worker and one metadata job; original raw
byte permits stay owned until work finishes. Owner termination is explicitly confirmed and
audited before native delivery. Full process failover/managed/approval qualification remains.
