# 022 — Implement legal holds and database-coordinated retention

- **Status:** Not started
- **Milestone:** Recording release prerequisite per 002; PRD lists M2
- **Workstream:** Recording lifecycle
- **Depends on:** [021](021-implement-transcript-storage-and-access.md)
- **Index:** [Task plan](README.md)

## Goal and repository context

Monthly purge must honor content age and overlapping legal holds even under concurrent ingest, hold changes, and job failure.

**Read first:** [PRD.md](../PRD.md) PO-6, PO-8; [architecture.md](../architecture.md) §4.4; [components.md](../components.md) §§4, 7–8.

## Owned scope

- `Gateway transcripts/holds.rs, retention.rs, jobs.rs and hold/retention migrations`
- `Gateway admin hold endpoints and retention runbook`

Paths describe proposed implementation locations; they do not exist yet unless another task creates them. Follow [the subcrate standard](../subcrate-standard.md), add only useful modules, and coordinate shared manifests/exports and migrations as described in the index.

## Implementation

1. Implement platform-admin-only per-user holds with optional content-time windows and case references; audit placement/change/release without exposing holds or admin reads to affected users.
2. Preserve existing/new held transcript content plus referenced session/machine metadata and audit events. Apply explicit RLS/role policies to held storage as well as normal partitions.
3. Coordinate ingest, hold backfill, overlapping hold changes, and partition retirement with database-authoritative transactions/locks, not Redis leadership alone.
4. Drop a monthly partition only when its entire range is older than the cutoff. Releasing one hold preserves data covered by another; use original capture time and document backup/restore retention implications.

## Acceptance criteria

- [ ] At the default 90-day cutoff, purge respects monthly rounding to roughly 120 days and never silently shortens policy.
- [ ] Concurrent hold placement/change and partition deletion cannot lose covered content or references; crash recovery is repeatable.
- [ ] Owners cannot inspect hold state; unauthorized users cannot read held rows; admin actions and reads are audited.
- [ ] Releasing overlapping holds and retrying jobs preserves all still-required content and eventually purges eligible data.

## Verification

- Run real Postgres races for ingest/hold/purge/release, multiple workers, crash after copy-before-drop, content-time backfill, and cross-user held-table reads.

Run the relevant formatting, lint, dependency-boundary, and supported-platform checks once code exists. Record real results and any unavailable environment; do not report planned or mocked external validation as completed.

## Boundaries and handoff

No legal-policy invention, second approver requirement, object storage migration, or user notification of admin reads.

**Deliver:** Hold/retention APIs, protected held storage, deterministic race tests, and operational prerequisites for releasing persistent history.
