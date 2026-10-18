# 002 — Resolve state authority, lifecycle, and protocol decisions

- **Status:** Complete for the selected Linux standalone development scope; external release qualification remains separately tracked
- **Milestone:** M0 foundation
- **Workstream:** Contracts
- **Depends on:** None
- **Index:** [Task plan](README.md)

**Scope:** The user selected Linux development completion in [ADR 0007](../docs/adr/0007-standalone-preview.md). See [current progress](PROGRESS.md) and [external release gates](../docs/validation/external-release-gates.md). Original criteria that require unrun environments or waived corporate inspection remain unchecked; this status does not claim full release qualification.

## Goal and repository context

The documents contain deliberately unresolved implementation choices and older examples that conflict with later component guidance. Give independent implementers one explicit contract baseline.

**Read first:** [components.md](../components.md) §§4, 7–8; [architecture.md](../architecture.md) §§4.3–4.4, 6–8, 9.2–9.3; [subcrate-standard.md](../subcrate-standard.md) §§4–6.

## Owned scope

- `docs/adr/ for authority, lifecycle, recording, and deployment decisions`
- `docs/contracts/operation-matrix.md; narrow reconciliations to the source design documents`

Owned paths define this task’s implementation boundary; current source and executed evidence are recorded below. Follow [the subcrate standard](../subcrate-standard.md), add only useful modules, and coordinate shared manifests/exports and migrations as described in the index.

## Implementation

1. Specify operation identities, start reconciliation stages, replay cursors, host/controller generations, approval decision versus native application outcomes, and the owner/controller/approver permission matrix. Record proposed mechanisms as decisions with rationale.
2. Resolve hostname-based enrollment versus persisted identity, Redis lease caching versus durable fencing, and independent session-owner service lifetime versus relying on setsid alone.
3. Define recording chunks, durable receipts, gap metadata, content-time retention, RLS roles, and hold/retention coordination. Reconcile the PRD's mandatory-recording wording with its M2 placement; explicitly record the release gate for protected persistent content without claiming a schedule is already agreed.
4. Specify separately versioned same-user IPC and prohibit remote administrative IPC. Define degraded readiness, schema compatibility ranges, service-readable secrets, and unique per-instance ports for four host-network gateway instances.
5. Record numeric encoding, binary framing, limits, and compatibility decisions for task 004. Leave externally unverifiable CLI, Codex, and Entra claims as named validation gates in 001, 009, and 019.

## Acceptance criteria

- [x] Each mutable state has one authority and documented crash, stale-authority, acknowledgment, and saturation behavior.
- [x] Contracts explicitly preserve owner approval without a lease, supported outside-root existing-session control, and session survival on revocation/pause.
- [x] Protocol and persistence implementers have no contradictory retry, generation, or recording semantics to choose between.
- [x] Unresolved external validation is listed with a task owner; no invented vendor capabilities or production guarantees are presented as proven.

## Verification

- Walk through a dropped start acknowledgment, stale controller input, lost approval delivery, filled recording spool, and a hold racing partition deletion using the written transitions.

Run the relevant formatting, lint, dependency-boundary, and supported-platform checks once code exists. Record real results and any unavailable environment; do not report planned or mocked external validation as completed.

## Boundaries and handoff

No production implementation, fabricated proof, or automatic expansion of MVP to desktop, Windows, or adoption.

**Deliver:** A shared contract baseline. Later tasks change shared contracts through a focused prerequisite change and corresponding fixtures.

## Deliverables and verification (2026-10-16)

- [Decision index](../docs/adr/README.md): authority, lifecycle, wire encoding, recording, and deployment baseline.
- [Operation matrix](../docs/contracts/operation-matrix.md): owner/controller/approver distinctions and separate termination authority.
- [Failure walkthroughs](../docs/contracts/failure-walkthroughs.md): dropped start reply, stale input, approval delivery/application uncertainty, spool/receipt failures, hold/purge race, missed revocation hints, tail loss, and connector pause.
- [Integration inventory](../docs/integration/qgenie-cli.md) and [progress](PROGRESS.md): named external inputs, affected tasks, and source/toolchain blocker.

The PRD now states the protected-recording release dependency without confirming revised dates. Architecture examples now use persisted enrollment identity, durable fencing, an approval outbox with separate native outcome, full-width counters, independent IPC/service lifetime, non-root-readable secrets, compatible schema ranges, and explicit development ports.

Written review checked all eight walkthroughs against the selected transitions. It identified and corrected an access-renewal issue: a new delegation uses the remaining authoritative cache lifetime, rather than extending a nearly expired read by another 10 seconds. The design does not claim atomic policy/native application across processes or loss-free recovery from an older disaster backup.

Verification completed:

- A one-off Python document check verified local link targets and closed code fences.
- The same check verified 28 unique task IDs, agreement between index/task prerequisites, and an acyclic dependency graph.
- Arithmetic checks verified the PTY frame bounds, the 850-second directory-plus-enforcement design budget, and distinct gateway/health ports.
- `git diff --check` passed.

No application, native-agent, tenant, service-manager, proxy, database, or load test ran. Those proofs remain in their owning tasks. Task 001 remains blocked, so this completion does not unblock task 003 or claim product implementation.
