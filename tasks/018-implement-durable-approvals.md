# 018 — Implement one-winning-decision approvals with durable delivery

- **Status:** Not started
- **Milestone:** M1 remote approvals
- **Workstream:** Approval state
- **Depends on:** [009](009-implement-verified-codex-adapter.md), [012](012-implement-routing-and-presence.md), [015](015-implement-session-runtime-and-start-journal.md)
- **Index:** [Task plan](README.md)

## Goal and repository context

One winning gateway decision must survive delivery failure without falsely claiming that an agent executed the response exactly once.

**Read first:** [PRD.md](../PRD.md) SE-9 and PO-3; [components.md](../components.md) §4 contract 6 and §6; [architecture.md](../architecture.md) §§6.4, 8.

## Owned scope

- `Gateway approvals/{decision,delivery}.rs, store/{approvals,outbox}.rs and migrations`
- `Sessiond approvals.rs, store/approvals.rs and approval migrations`

Paths describe proposed implementation locations; they do not exist yet unless another task creates them. Follow [the subcrate standard](../subcrate-standard.md), add only useful modules, and coordinate shared manifests/exports and migrations as described in the index.

## Implementation

1. Journal native requests and cancellation/timeout state, persist gateway pending approvals, and push owner-wide list/badge updates.
2. Validate current ownership/access and host policy before choosing a winner. Commit the conditional decision, audit record, and delivery obligation in one transaction.
3. Retry committed delivery until sessiond reports an application result; deduplicate by stable approval/decision identity and reconcile native local-resolution/restart races.
4. Expose committed versus applied/already-applied/cancelled/outcome-unknown states; support owner clients without a lease or writable attachment, including supported outside-root sessions.

## Acceptance criteria

- [ ] Simultaneous owner answers create one committed decision and losing answers receive a stable resolved/conflict response.
- [ ] A crash or dropped message after commit does not lose the delivery obligation; duplicate delivery cannot silently create a second native answer.
- [ ] Policy changes, native cancellation/local resolution, revocation, timeout, and lost application acknowledgments have explicit outcomes.
- [ ] A non-controller owner can answer; other users and revoked clients cannot. Revocation never auto-approves pending requests.

## Verification

- Run database winner races and outbox restart tests with fake adapters for uncertainty, then a real pinned Codex approval path. Drop delivery and acknowledgment independently.

Run the relevant formatting, lint, dependency-boundary, and supported-platform checks once code exists. Record real results and any unavailable environment; do not report planned or mocked external validation as completed.

## Boundaries and handoff

This task does not require 017: approval rights are deliberately independent of controller authority. Do not claim exactly-once native execution when the vendor cannot prove it.

**Deliver:** Approval APIs, durable delivery worker, native application journal, and status events for client/TUI consumers.
