# 002 — Resolve state authority, lifecycle, and protocol decisions

- **Status:** Not started
- **Milestone:** M0 foundation
- **Workstream:** Contracts
- **Depends on:** None
- **Index:** [Task plan](README.md)

## Goal and repository context

The documents contain deliberately unresolved implementation choices and older examples that conflict with later component guidance. Give independent implementers one explicit contract baseline.

**Read first:** [components.md](../components.md) §§4, 7–8; [architecture.md](../architecture.md) §§4.3–4.4, 6–8, 9.2–9.3; [subcrate-standard.md](../subcrate-standard.md) §§4–6.

## Owned scope

- `docs/adr/ for authority, lifecycle, recording, and deployment decisions`
- `docs/contracts/operation-matrix.md; narrow reconciliations to the source design documents`

Paths describe proposed implementation locations; they do not exist yet unless another task creates them. Follow [the subcrate standard](../subcrate-standard.md), add only useful modules, and coordinate shared manifests/exports and migrations as described in the index.

## Implementation

1. Specify operation identities, start reconciliation stages, replay cursors, host/controller generations, approval decision versus native application outcomes, and the owner/controller/approver permission matrix. Record proposed mechanisms as decisions with rationale.
2. Resolve hostname-based enrollment versus persisted identity, Redis lease caching versus durable fencing, and independent session-owner service lifetime versus relying on setsid alone.
3. Define recording chunks, durable receipts, gap metadata, content-time retention, RLS roles, and hold/retention coordination. Reconcile the PRD's mandatory-recording wording with its M2 placement; explicitly record the release gate for protected persistent content without claiming a schedule is already agreed.
4. Specify separately versioned same-user IPC and prohibit remote administrative IPC. Define degraded readiness, schema compatibility ranges, service-readable secrets, and unique per-instance ports for four host-network gateway instances.
5. Record numeric encoding, binary framing, limits, and compatibility decisions for task 004. Leave externally unverifiable CLI, Codex, and Entra claims as named validation gates in 001, 009, and 019.

## Acceptance criteria

- [ ] Each mutable state has one authority and documented crash, stale-authority, acknowledgment, and saturation behavior.
- [ ] Contracts explicitly preserve owner approval without a lease, supported outside-root existing-session control, and session survival on revocation/pause.
- [ ] Protocol and persistence implementers have no contradictory retry, generation, or recording semantics to choose between.
- [ ] Unresolved external validation is listed with a task owner; no invented vendor capabilities or production guarantees are presented as proven.

## Verification

- Walk through a dropped start acknowledgment, stale controller input, lost approval delivery, filled recording spool, and a hold racing partition deletion using the written transitions.

Run the relevant formatting, lint, dependency-boundary, and supported-platform checks once code exists. Record real results and any unavailable environment; do not report planned or mocked external validation as completed.

## Boundaries and handoff

No production implementation, fabricated proof, or automatic expansion of MVP to desktop, Windows, or adoption.

**Deliver:** A shared contract baseline. Later tasks change shared contracts through a focused prerequisite change and corresponding fixtures.
