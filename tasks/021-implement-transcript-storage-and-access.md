# 021 — Implement recording upload, durable ingest, and protected reads

- **Status:** Not started
- **Milestone:** Required with persistent recording; release mapping decided in 002
- **Workstream:** Gateway recordings
- **Depends on:** [010](010-implement-gateway-foundation-and-audit.md), [013](013-implement-host-enrollment-and-credentials.md), [020](020-implement-recording-spool.md)
- **Index:** [Task plan](README.md)

## Goal and repository context

Persist recordings with inspectable receipt semantics and owner/admin access, without putting storage latency on the live session path.

**Read first:** [PRD.md](../PRD.md) PO-4, PO-7; [architecture.md](../architecture.md) §4.4; [components.md](../components.md) §§4, 7–8.

## Owned scope

- `Host recording.rs uploader`
- `Gateway transcripts/{ingest,receipts,read}.rs, api/transcripts.rs, transcript migrations and DB roles/RLS`

Paths describe proposed implementation locations; they do not exist yet unless another task creates them. Follow [the subcrate standard](../subcrate-standard.md), add only useful modules, and coordinate shared manifests/exports and migrations as described in the index.

## Implementation

1. Upload immutable spool batches asynchronously with bounded retries and authenticate the machine/session/owner relationship on every ingest.
2. Implement idempotent compressed chunk storage, stable capture-time validation, gap persistence, and receipts only after database commit. Define receipt recovery/reconciliation after database failover using 002's durability decision.
3. Implement paginated historical reads restricted to owners or platform admins; use transaction-local RLS context safe under PgBouncer transaction pooling and separate ingest/read/admin roles.
4. Require admin reason and applicable case reference; commit an admin-read audit record before returning content. Apply writer-side redaction and safe size/decompression limits.
5. Provide history availability and recording-warning metadata without using transcripts for live replay or exposing administrative hold details to owners.

## Acceptance criteria

- [ ] A lost commit acknowledgment and retried upload produce one logical batch and an equivalent durable receipt.
- [ ] Owner isolation holds at both API and database layers; pooled connections do not leak one user's context into another request.
- [ ] Unauthorized reads and admin reads without required context fail; audit failure prevents admin content release.
- [ ] Ingest outage/backlog never blocks the session path, and content-time validation prevents retries/backfill from extending retention.

## Verification

- Run real Postgres/PgBouncer isolation and ingest tests: cross-owner spoofing, duplicate/conflicting batches, lost ack, oversized compressed payload, pool reuse, audit failure, and database recovery receipt handling.

Run the relevant formatting, lint, dependency-boundary, and supported-platform checks once code exists. Record real results and any unavailable environment; do not report planned or mocked external validation as completed.

## Boundaries and handoff

Hold management and destructive retention are 022. This task alone does not authorize release of persistent recording without the agreed retention/hold gate.

**Deliver:** Asynchronous uploader, commit receipts, protected transcript API, and RLS roles extended by 022.
