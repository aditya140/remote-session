# 019 — Implement bounded revocation and Entra disablement enforcement

- **Status:** Not started
- **Milestone:** M1 access revocation
- **Workstream:** Access authority
- **Depends on:** [011](011-implement-gateway-identity-and-enrollment.md), [012](012-implement-routing-and-presence.md)
- **Index:** [Task plan](README.md)

## Goal and repository context

Closing access must meet the separate 10-second gateway and 15-minute Entra bounds while leaving host sessions running.

**Read first:** [PRD.md](../PRD.md) ID-6, PO-5, §8 success metrics; [architecture.md](../architecture.md) §8 gateway and Entra paths; [components.md](../components.md) §§4, 8.

## Owned scope

- `Gateway revocation/{commit,reconcile}.rs and identity/directory.rs`
- `Gateway admin revocation API, access freshness migrations, and conformance scenarios`

Paths describe proposed implementation locations; they do not exist yet unless another task creates them. Follow [the subcrate standard](../subcrate-standard.md), add only useful modules, and coordinate shared manifests/exports and migrations as described in the index.

## Implementation

1. Implement idempotent global/user/machine revocation with durable state and audit in one transaction, immediate accepting-node denial, invalidation hints, and propagation reporting.
2. Reconcile connected principals with authoritative state at least every 5 seconds; close/deny affected connections before stale authority exceeds the 10-second enforcement budget.
3. Select and validate a real directory/change-event mechanism. Include polling/detection, retry/deadline, and gateway enforcement in the total 15-minute budget; stale verification fails closed.
4. Store upstream enabled-state freshness separately from manual disablement so successful directory updates cannot clear admin revocation. User disablement also covers machine-token access.

## Acceptance criteria

- [ ] New and already-open user/host connections are denied within 10 seconds of gateway commit even when all Redis hints are suppressed.
- [ ] Actual Entra disablement, or verification too stale to satisfy the budget, blocks user and enrolled-machine access within 15 minutes.
- [ ] Revocation issues no termination commands and does not resolve pending approvals or trigger automatic re-enrollment.
- [ ] The API enforces platform-admin access, supports safe retries, records audit, and reports pending/completed propagation accurately.

## Verification

- Use fake clocks/provider failures for deterministic bounds; run two-gateway missed-broadcast and Postgres-outage tests. Record an authorized tenant disablement drill with still-valid credentials before claiming the Entra SLO.

Run the relevant formatting, lint, dependency-boundary, and supported-platform checks once code exists. Record real results and any unavailable environment; do not report planned or mocked external validation as completed.

## Boundaries and handoff

Token expiry alone is insufficient. Tenant permissions/integration access are external prerequisites; missing access remains an explicit release blocker.

**Deliver:** Current-access service and admin API, validated directory integration, and timed evidence used by all mutation paths and 027.
