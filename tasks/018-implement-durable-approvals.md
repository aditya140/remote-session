# 018 — Implement one-winning-decision approvals with durable delivery

- **Status:** Complete for the selected Linux standalone development scope; external release qualification remains separately tracked
- **Milestone:** M1 remote approvals
- **Workstream:** Approval state
- **Depends on:** [009](009-implement-verified-codex-adapter.md), [012](012-implement-routing-and-presence.md), [015](015-implement-session-runtime-and-start-journal.md)
- **Index:** [Task plan](README.md)

**Scope:** The user selected Linux development completion in [ADR 0007](../docs/adr/0007-standalone-preview.md). See [current progress](PROGRESS.md) and [external release gates](../docs/validation/external-release-gates.md). Original criteria that require unrun environments or waived corporate inspection remain unchecked; this status does not claim full release qualification.

## Goal and repository context

One winning gateway decision must survive delivery failure without falsely claiming that an agent executed the response exactly once.

**Read first:** [PRD.md](../PRD.md) SE-9 and PO-3; [components.md](../components.md) §4 contract 6 and §6; [architecture.md](../architecture.md) §§6.4, 8.

## Owned scope

- `Gateway approvals/{decision,delivery}.rs, store/{approvals,outbox}.rs and migrations`
- `Sessiond approvals.rs, store/approvals.rs and approval migrations`

Owned paths define this task’s implementation boundary; current source and executed evidence are recorded below. Follow [the subcrate standard](../subcrate-standard.md), add only useful modules, and coordinate shared manifests/exports and migrations as described in the index.

## Implementation

1. Journal native requests and cancellation/timeout state, persist gateway pending approvals, and push owner-wide list/badge updates.
2. Validate current ownership/access and host policy before choosing a winner. Commit the conditional decision, audit record, and delivery obligation in one transaction.
3. Retry committed delivery until sessiond reports an application result; deduplicate by stable approval/decision identity and reconcile native local-resolution/restart races.
4. Expose committed versus applied/already-applied/cancelled/outcome-unknown states; support owner clients without a lease or writable attachment, including supported outside-root sessions.

## Acceptance criteria

- [x] Simultaneous owner answers create one committed decision and losing answers receive a stable resolved/conflict response.
- [x] A crash or dropped message after commit does not lose the delivery obligation; duplicate delivery cannot silently create a second native answer.
- [x] Policy changes, native cancellation/local resolution, revocation, timeout, and lost application acknowledgments have explicit outcomes.
- [x] A non-controller owner can answer; other users and revoked clients cannot. Revocation never auto-approves pending requests.

## Verification

- Run database winner races and outbox restart tests with fake adapters for uncertainty, then a real pinned Codex approval path. Drop delivery and acknowledgment independently.

Run the relevant formatting, lint, dependency-boundary, and supported-platform checks once code exists. Record real results and any unavailable environment; do not report planned or mocked external validation as completed.

## Boundaries and handoff

This task does not require 017: approval rights are deliberately independent of controller authority. Do not claim exactly-once native execution when the vendor cannot prove it.

**Deliver:** Approval APIs, durable delivery worker, native application journal, and status events for client/TUI consumers.

The configured Linux managed runtime now registers immutable native process sources, journals
actual Codex requests, preserves original request bodies on repeated native IDs, and resolves
or cancels pending requests on native resolution/end. Applying a committed winner requires
the original current owner-access fence at the actual native write; it requires no controller
or writable attachment. The actual pinned runtime proof drops an application reply, redelivers
the original winner and observes Unknown with one command effect.

Public `approval-status-v1` and `durable-approvals-v1` now compose owner-wide reads,
one-shot choice, primary winner/audit/outbox, current-host bounded delivery, signed cross-node
change hints and native journal reconciliation. `host-approvals-v1` requires the native
contract. The connector polls scoped native snapshots independently of input/upload;
missing pending requests cancel without changing an already committed decision.

The actual owned synchronous fixture passes pinned Codex → native journal → connector →
two gateways → SDK race/TUI cancel+confirm → outbox → native answer. Both native commands
produce one controlled file effect, one audit each and zero controller leases. Durable
status reads succeed with the connector offline. Agent creation now uses the public SDK,
qualified recording preflight and advertised native inventory. The SDK's actual TLS tests prove
lost choice replies send once and unsupported/unoffered choices send no frame. Real primary
row-lock expiry, winner/audit/outbox atomicity and claim restart run through PgBouncer.
The composed proxy independently drops delivery before native I/O and the application
acknowledgment after native I/O. The durable outbox makes at least three attempts; the native
command produces one marker. Native redelivery returns its journaled result without a second
answer. Database claim restart and native journal reopen fixtures cover process recovery.
The complete gateway/native crash matrix remains task 027 qualification; no exactly-once
vendor execution claim is made.
