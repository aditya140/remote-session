# 021 — Implement recording upload, durable ingest, and protected reads

- **Status:** Complete for the selected Linux standalone development scope; external release qualification remains separately tracked
- **Milestone:** Required with persistent recording; release mapping decided in 002
- **Workstream:** Gateway recordings
- **Depends on:** [010](010-implement-gateway-foundation-and-audit.md), [013](013-implement-host-enrollment-and-credentials.md), [020](020-implement-recording-spool.md)
- **Index:** [Task plan](README.md)

**Scope:** The user selected Linux development completion in [ADR 0007](../docs/adr/0007-standalone-preview.md). See [current progress](PROGRESS.md) and [external release gates](../docs/validation/external-release-gates.md). Original criteria that require unrun environments or waived corporate inspection remain unchecked; this status does not claim full release qualification.

## Goal and repository context

Persist recordings with inspectable receipt semantics and owner/admin access, without putting storage latency on the live session path.

**Read first:** [PRD.md](../PRD.md) PO-4, PO-7; [architecture.md](../architecture.md) §4.4; [components.md](../components.md) §§4, 7–8.

## Owned scope

- `Host recording.rs uploader`
- `Gateway transcripts/{ingest,receipts,read}.rs, api/transcripts.rs, transcript migrations and DB roles/RLS`

Owned paths define this task’s implementation boundary; current source and executed evidence are recorded below. Follow [the subcrate standard](../subcrate-standard.md), add only useful modules, and coordinate shared manifests/exports and migrations as described in the index.

## Implementation

1. Upload immutable spool batches asynchronously with bounded retries and authenticate the machine/session/owner relationship on every ingest.
2. Implement idempotent compressed chunk storage, stable capture-time validation, gap persistence, and receipts only after database commit. Define receipt recovery/reconciliation after database failover using 002's durability decision.
3. Implement paginated historical reads restricted to owners or platform admins; use transaction-local RLS context safe under PgBouncer transaction pooling and separate ingest/read/admin roles.
4. Require admin reason and applicable case reference; commit an admin-read audit record before returning content. Apply writer-side redaction and safe size/decompression limits.
5. Provide history availability and recording-warning metadata without using transcripts for live replay or exposing administrative hold details to owners.

## Acceptance criteria

- [x] A lost commit acknowledgment and retried upload produce one logical batch and an equivalent durable receipt.
- [x] Owner isolation holds at both API and database layers; pooled connections do not leak one user's context into another request.
- [x] Unauthorized reads and admin reads without required context fail; audit failure prevents admin content release.
- [x] Ingest outage/backlog never blocks the session path, and content-time validation prevents retries/backfill from extending retention.

## Verification

- Run real Postgres/PgBouncer isolation and ingest tests: cross-owner spoofing, duplicate/conflicting batches, lost ack, oversized compressed payload, pool reuse, audit failure, and database recovery receipt handling.

Run the relevant formatting, lint, dependency-boundary, and supported-platform checks once code exists. Record real results and any unavailable environment; do not report planned or mocked external validation as completed.

## Boundaries and handoff

Hold management and destructive retention are 022. This task alone does not authorize release of persistent recording without the agreed retention/hold gate.

**Deliver:** Asynchronous uploader, commit receipts, protected transcript API, and RLS roles extended by 022.

## Ingest validation component

The gateway now validates immutable recording preparation on a separate bounded CPU pool (16 actual jobs; permits remain with blocking work even if callers cancel). It enforces compressed/raw/line/record bounds, one zstd frame, compressed-byte SHA-256, canonical normalized records without input variants, original sequence/fragment order, exact capture boundaries, one UTC capture month, and explicit five-minute future/session-lifetime skew. Delayed uploads retain original age. Malformed/duplicate JSON fields, forged ordering, missing fragments and decompression bombs are rejected in component tests.

This preparation API returns no receipt. Subsequent protected persistence and audited history components are described below. Public routes, uploader and lifecycle protections now compose in the proofs below.

## Database boundary component

Schema 9 and `RecordingDatabase` isolate ingest, owner read, admin read and lifecycle credentials, rejecting privileged/table-owner/mixed memberships and unexpected grants. FORCE RLS protects receipt metadata, partitioned normal content and held content/case membership. No default partition exists. Owner/ingest contexts use transaction-local identities; missing context denies reads. Direct partitions and held/case data are not granted to owners/ingest/lifecycle roles. Admin normal reads require actor/reason; held reads additionally require matching case membership. Lifecycle code will maintain that coverage in 022.

Real application-role tests seed disposable synthetic rows and check owner/machine spoofing boundaries, size-one pool context reset, missing context, direct partition/held-row denial and isolated roles. Subsequent signed-identity ingest/history tests and actual PgBouncer proofs extend this boundary evidence. There is no public API to release these seeded rows.

The standalone writer redaction backstop now returns a sealed protected recording, keeps original receipt metadata/digest, clears unprotected upload data, and hashes stored bytes separately. Known literals across record fields and chunk boundaries, binary preservation and bounded overlapping-match saturation are tested. This is sample literal integration, not corporate redaction qualification. See [recording integration](../docs/integration/recording.md) for the current contract and remaining work.

## Atomic ingest component

Schema 10 binds active machine/key/session/owner under ordered primary row locks, sets transaction-local RLS identity, serializes lifecycle operations, pins stream/fragment identity and journals per-recovery-epoch receipts. Protected content/gap and receipt insertion commit together. Retried logical batches reconcile the existing receipt; changed metadata/content conflicts. Duplicate lookups write a fresh durability barrier before commit, so an interrupted original synchronous COMMIT cannot become a false durable retry acknowledgment. Source capture time remains the partition/retention age.

Actual PostgreSQL tests pass signed machine scope, one logical batch after lost ACK, rollback on late receipt failure, conflicts/fragment collisions, typed gap retries, full-u64 sequences, primary revocation and epoch quarantine/reissue. Synthetic single-server qualification is confined to tests. Production APIs require current directory proof, synchronous/quorum standby plus remote-apply commit, and schema 12 lifecycle protection. PostgreSQL 17+ active-SQL timeouts are supplemented by an owned three-second commit budget and bounded protocol cancellation/drain, since PostgreSQL disables its normal timers during synchronous COMMIT. Public upload/history and independent native epoch binding are now composed; local synchronous primary-volume-loss and HTTPS/native lost-ACK proofs pass. Deployed standby/backup restoration and full remote capture faults remain.

## Audited history and transaction pooling

Schema 11 binds RLS context to the current transaction ID as well as principal. Bounded owner/admin history uses immutable stream epochs, full-u64 keyset pagination and recovery-epoch cursor invalidation. Held reads require matching case coverage; admin audits record reason and exact chunk/gap IDs before a page can be released. Audit failure denies the result. Owner responses omit hold and administrative details.

The 21-test gateway suite passed with recording pools routed through PgBouncer 1.24.1 transaction mode, one backend per database/user and prepared-statement support. Two separate client pools demonstrably reused one PostgreSQL backend while stale session context granted no access. The reproducible harness is `tests/e2e/pgbouncer_recordings.py`; CI wiring exists, with no observed CI result claimed. These proofs remain separate from native/uploader and synchronous-standby/restore qualification.


## Protected network composition

The opt-in gateway composition installs isolated ingest/owner/admin pools with database
identity matching and startup cleanup. Machine-only content/gap/epoch POSTs use current
primary/Directory scope; owner and admin history use separate POST routes and credentials,
with no-store responses. Admin reasons stay in the request body and audit commits before
content release. The synchronous harness verifies successful HTTP paths, owner/machine token
separation, cross-owner denial, lost ACK reconciliation, redacted history, gap retries and a
valid upload larger than 64 KiB under the explicit 128 KiB content-route limit. Native receipt
reclamation independently authenticates the epoch and rechecks original authority after
SQLite waits. Real HTTPS and an actual sessiond process pass recovered spool/lost response
composition. These tests use synthetic identities and a disposable same-host synchronous
pair, not a real tenant or independent production fault domains.


Schema 13 takes original native-observed end metadata when available and preserves the
first final projection upper bound for unknown/legacy ends. Native schema 5 persists end
observations across restart. Projection retries cannot rewrite a known end, resurrect a
finished session or extend an unknown final bound. The explicit five-minute source-clock
skew remains; a history page still cannot certify native completeness.

Public PTY start now passes the actual persisted-key SDK/WSS/gateway/activated connector/native process path in the owned synchronous recording harness. Same-key retry produces one child/start audit; changed parameters are NotSent; unready storage cannot launch. The connector automatically uploads real native capture and end metadata. Public remote control/input and managed runtime now pass the composed proof; see PROGRESS.md for current evidence.
