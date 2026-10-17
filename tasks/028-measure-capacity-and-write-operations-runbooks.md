# 028 — Measure resource use and document operational recovery

- **Status:** Not started
- **Milestone:** M1 baseline; M2 scale qualification
- **Workstream:** Observability and operations
- **Depends on:** [019](019-implement-revocation-and-entra-freshness.md), [022](022-implement-legal-holds-and-retention.md), [025](025-integrate-cli-and-host-services.md), [026](026-build-development-compose-stack.md)
- **Index:** [Task plan](README.md)

## Goal and repository context

Replace planning estimates with measured traffic/recording costs and give operators concrete recovery procedures before wider rollout.

**Read first:** [PRD.md](../PRD.md) §§7–8, 12; [architecture.md](../architecture.md) §§9.5–9.7, 13; [components.md](../components.md) §8.

## Owned scope

- `Load/fault tooling under tests/load/ or xtask/`
- `Gateway/host metric instrumentation and deploy/compose/otel/ configuration`
- `docs/runbooks/ and docs/validation/capacity.md`

Paths describe proposed implementation locations; they do not exist yet unless another task creates them. Follow [the subcrate standard](../subcrate-standard.md), add only useful modules, and coordinate shared manifests/exports and migrations as described in the index.

## Implementation

1. Instrument connections, routing/local affinity, queue pressure, replay, approvals, revocation freshness, recording gaps/backlog, database lag, and host footprint with bounded-cardinality metrics and redacted correlated logs.
2. Measure echo overhead, approval delivery, reconnect success, start duplication, and idle host RSS/CPU. Measure recording duty cycle/compression and model monthly rounding, held copies, WAL, replicas, and backups.
3. Provide a scalable load harness for the beta targets: 200k sockets, 20k streaming sessions, 2k terminals with 50 KB/s bursts, p99 echo overhead below 150 ms, and Postgres write lag below 5 seconds. Distinguish a reduced local baseline from full-scale evidence.
4. Write runbooks for gateway drain, Redis recovery, manual Postgres failover/receipt reconciliation, backup restore and expiry, recording pressure, revocation diagnostics, and compatible upgrades. Record actual on-call/backup ownership or the unresolved assignment.
5. List prerequisites for the later three-server deployment and off-server backups; do not assume the design's capacity estimates or storage allocation are confirmed.

## Acceptance criteria

- [ ] A reproducible baseline report records hardware, versions, workload, sample sizes, limitations, and results against applicable PRD targets.
- [ ] Alerts cover stale access authority, gaps, recording lag, disk pressure, readiness, and backup age without sensitive content or unbounded user/session labels.
- [ ] The storage model uses measured inputs and explicitly includes roughly 120-day rounded retention plus holds and backup/replication overhead.
- [ ] Recovery/runbook steps are exercised where infrastructure exists; missing production capacity, tenant access, backup target, or operators remain named blockers.
- [ ] Full beta-scale qualification is marked complete only after the stated workload actually runs; local extrapolation is labeled as an estimate.

## Verification

- Run the supported baseline and telemetry checks, a backup/restore drill on test data, and selected failure runbooks. Schedule the full load/failover evidence on suitable infrastructure before beta, preserving the unrun status meanwhile.

Run the relevant formatting, lint, dependency-boundary, and supported-platform checks once code exists. Record real results and any unavailable environment; do not report planned or mocked external validation as completed.

## Boundaries and handoff

No claim that the one-server dev stack is highly available, no automatic production deployment, and no changes to retention merely to fit storage.

**Deliver:** Capacity evidence, alert definitions, operational runbooks, and an explicit beta-readiness gap list. This work can run alongside 027 using separate test environments.
