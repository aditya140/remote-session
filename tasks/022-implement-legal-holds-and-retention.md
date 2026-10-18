# 022 — Implement legal holds and database-coordinated retention

- **Status:** Complete for the selected Linux standalone development scope; external release qualification remains separately tracked
- **Milestone:** Recording release prerequisite per 002; PRD lists M2
- **Workstream:** Recording lifecycle
- **Depends on:** [021](021-implement-transcript-storage-and-access.md)
- **Index:** [Task plan](README.md)

**Scope:** The user selected Linux development completion in [ADR 0007](../docs/adr/0007-standalone-preview.md). See [current progress](PROGRESS.md) and [external release gates](../docs/validation/external-release-gates.md). Original criteria that require unrun environments or waived corporate inspection remain unchecked; this status does not claim full release qualification.

## Goal and repository context

Monthly purge must honor content age and overlapping legal holds even under concurrent ingest, hold changes, and job failure.

**Read first:** [PRD.md](../PRD.md) PO-6, PO-8; [architecture.md](../architecture.md) §4.4; [components.md](../components.md) §§4, 7–8.

## Owned scope

- `Gateway transcripts/holds.rs, retention.rs, jobs.rs and hold/retention migrations`
- `Gateway admin hold endpoints and retention runbook`

Owned paths define this task’s implementation boundary; current source and executed evidence are recorded below. Follow [the subcrate standard](../subcrate-standard.md), add only useful modules, and coordinate shared manifests/exports and migrations as described in the index.

## Implementation

1. Implement platform-admin-only per-user holds with optional content-time windows and case references; audit placement/change/release without exposing holds or admin reads to affected users.
2. Preserve existing/new held transcript content plus referenced session/machine metadata and audit events. Apply explicit RLS/role policies to held storage as well as normal partitions.
3. Coordinate ingest, hold backfill, overlapping hold changes, and partition retirement with database-authoritative transactions/locks, not Redis leadership alone.
4. Drop a monthly partition only when its entire range is older than the cutoff. Releasing one hold preserves data covered by another; use original capture time and document backup/restore retention implications.

## Acceptance criteria

- [x] At the default 90-day cutoff, purge respects monthly rounding to roughly 120 days and never silently shortens policy.
- [x] Concurrent hold placement/change and partition deletion cannot lose covered content or references; crash recovery is repeatable.
- [x] Owners cannot inspect hold state; unauthorized users cannot read held rows; admin actions and reads are audited.
- [x] Releasing overlapping holds and retrying jobs preserves all still-required content and eventually purges eligible data.

## Verification

- Run real Postgres races for ingest/hold/purge/release, multiple workers, crash after copy-before-drop, content-time backfill, and cross-user held-table reads.

Run the relevant formatting, lint, dependency-boundary, and supported-platform checks once code exists. Record real results and any unavailable environment; do not report planned or mocked external validation as completed.

## Boundaries and handoff

No legal-policy invention, second approver requirement, object storage migration, or user notification of admin reads.

**Deliver:** Hold/retention APIs, protected held storage, deterministic race tests, and operational prerequisites for releasing persistent history.

## Current implementation and verification

Schema 12 adds per-user, revision-fenced holds, optional capture windows/case references, overlapping coverage, narrow lifecycle functions, ingest preservation and bounded copy/cleanup/retirement. The final partition drop rechecks exact held bytes and every current active hold under the same PostgreSQL advisory lock used by ingest and hold changes. Audit failure rolls back hold changes and transactional partition deletion. Released/changed holds stop authorizing held reads immediately while cleanup continues. Ordinary expiry remains the public receipt disposition, so private preservation does not disclose a hold to its affected user.

Real application-role PostgreSQL and PgBouncer 1.24.1 tests pass overlapping holds/windows, nullable case preservation, changed-case denial, late held content/gaps without partition recreation, concurrent uncommitted hold placement versus retirement, two workers, worker pool restart after committed copying, cancelled uncommitted copy, exact-byte preservation checks, audit rollback of hold/drop, monthly boundary rounding, release/expiry and retained metadata/receipt references. The restart proof closes/reopens the worker pool; it is not a database-volume loss drill.

`POST /v1/admin/holds` and the bounded `jobs` command are wired. Signed-role HTTP tests deny ordinary users and reject malformed context; both public HTTP and actual CLI process reject private synthetic qualification. The fixed scheduled-maintenance capability cannot change a hold. Lifecycle/authority pools must share the same database identity. Strict Clippy and schema/boundary checks pass. See [the lifecycle runbook](../docs/operations/recording-lifecycle.md).

Successful production endpoint/job execution and held byte/reference/audit preservation now pass the actual synchronous-pair primary-volume-loss drill. Protected upload/history and native spool/HTTPS composition also pass. Deployed nightly scheduling, independent physical fault domains, full remote process faults and backup retention/restore evidence remain open. Persistent real-user release remains gated.
