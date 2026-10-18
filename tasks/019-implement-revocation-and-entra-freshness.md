# 019 — Implement bounded revocation and Entra disablement enforcement

- **Status:** Complete for the selected Linux standalone development scope; external release qualification remains separately tracked
- **Milestone:** M1 access revocation
- **Workstream:** Access authority
- **Depends on:** [011](011-implement-gateway-identity-and-enrollment.md), [012](012-implement-routing-and-presence.md)
- **Index:** [Task plan](README.md)

**Scope:** The user selected Linux development completion in [ADR 0007](../docs/adr/0007-standalone-preview.md). See [current progress](PROGRESS.md) and [external release gates](../docs/validation/external-release-gates.md). Original criteria that require unrun environments or waived corporate inspection remain unchecked; this status does not claim full release qualification.

## Goal and repository context

Closing access must meet the separate 10-second gateway and 15-minute Entra bounds while leaving host sessions running.

**Read first:** [PRD.md](../PRD.md) ID-6, PO-5, §8 success metrics; [architecture.md](../architecture.md) §8 gateway and Entra paths; [components.md](../components.md) §§4, 8.

## Owned scope

- `Gateway revocation/{commit,reconcile}.rs and identity/directory.rs`
- `Gateway admin revocation API, access freshness migrations, and conformance scenarios`

Owned paths define this task’s implementation boundary; current source and executed evidence are recorded below. Follow [the subcrate standard](../subcrate-standard.md), add only useful modules, and coordinate shared manifests/exports and migrations as described in the index.

## Implementation

1. Implement idempotent global/user/machine revocation with durable state and audit in one transaction, immediate accepting-node denial, invalidation hints, and propagation reporting.
2. Reconcile connected principals with authoritative state at least every 5 seconds; close/deny affected connections before stale authority exceeds the 10-second enforcement budget.
3. Select and validate a real directory/change-event mechanism. Include polling/detection, retry/deadline, and gateway enforcement in the total 15-minute budget; stale verification fails closed.
4. Store upstream enabled-state freshness separately from manual disablement so successful directory updates cannot clear admin revocation. User disablement also covers machine-token access.
5. Qualify suspend/resume and clock-failure behavior as part of elapsed freshness, including after a queued mutation. The native foundation uses a suspend-aware clock; gateway evidence/cache expiry must preserve that property too.

## Acceptance criteria

- [x] New and already-open user/host connections are denied within 10 seconds of gateway commit even when all Redis hints are suppressed.
- [ ] Actual Entra disablement, or verification too stale to satisfy the budget, blocks user and enrolled-machine access within 15 minutes.
- [x] Revocation issues no termination commands and does not resolve pending approvals or trigger automatic re-enrollment.
- [x] The API enforces platform-admin access, supports safe retries, records audit, and reports pending/completed propagation accurately.

## Verification

- Use fake clocks/provider failures for deterministic bounds; run two-gateway missed-broadcast and Postgres-outage tests. Record an authorized tenant disablement drill with still-valid credentials before claiming the Entra SLO.

Run the relevant formatting, lint, dependency-boundary, and supported-platform checks once code exists. Record real results and any unavailable environment; do not report planned or mocked external validation as completed.

## Boundaries and handoff

Token expiry alone is insufficient. Tenant permissions/integration access are external prerequisites; missing access remains an explicit release blocker.

**Deliver:** Current-access service and admin API, validated directory integration, and timed evidence used by all mutation paths and 027.

## Current evidence

The [directory integration](../docs/integration/entra-directory.md) documents the Graph request, explicit application credential configuration, permission qualification, 840-second read-start freshness, suspend-aware socket gate and resource limits. PostgreSQL upstream state is separate from manual denial. Restart requires a new provider read. Global/user/machine admin revocation commits flags, revoked keys, idempotency, revision and audit together.

Verified HTTPS provider tests and actual PostgreSQL/two-process WSS tests pass, including Redis hints suppressed throughout revocation and a primary lock stall while dispatch is blocked. The actual signed admin machine-revocation/native-connector proof denies remote access within ten seconds while the same local PTY and original key survive.

[ADR 0009](../docs/adr/0009-revocation-completion.md) defines conservative pending-to-completed
reporting after one full ten-second original-authority horizon from an observed primary row.
Four elapsed/concurrency tests and separate-OS-process pending/completed proofs pass. Even
after completion, a primary lock causes unavailable rather than a cached successful status.
The [sanitized result](../docs/validation/revocation-propagation-2026-10-18.json) distinguishes
this proof from real tenant disablement, suspend/resume and macOS, which remain external
release gates. Completion never asserts termination or quiescence of admitted native work.
