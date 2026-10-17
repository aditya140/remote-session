# 020 — Implement the durable recording spool and explicit loss gaps

- **Status:** Not started
- **Milestone:** M0 recording proof; release mapping decided in 002
- **Workstream:** Host recording
- **Depends on:** [001](001-inspect-existing-qgenie-integration.md), [015](015-implement-session-runtime-and-start-journal.md)
- **Index:** [Task plan](README.md)

## Goal and repository context

Recording must remain enabled independently of bounded live replay, while storage failures cannot halt session execution.

**Read first:** [PRD.md](../PRD.md) PO-4; [architecture.md](../architecture.md) §4.4; [components.md](../components.md) §4 contract 8 and §7 recording proposal.

## Owned scope

- `Sessiond recording/{spool,chunk,gaps}.rs and recording migrations`
- `Recording event hooks and private spool files`

Paths describe proposed implementation locations; they do not exist yet unless another task creates them. Follow [the subcrate standard](../subcrate-standard.md), add only useful modules, and coordinate shared manifests/exports and migrations as described in the index.

## Implementation

1. Create immutable chunk identities/boundaries with stable capture time, sequence ranges, stream metadata, redacted content, and replay-independent storage budgets.
2. Record normalized agent events and terminal output/resizes/timing. Reuse QGenie redaction before content leaves the host; never separately store raw no-echo keystrokes.
3. Reclaim uploaded content only after the durable commit receipt defined in 002, or the explicit cap policy. At the cap discard oldest unacknowledged content, reserve space for coalesced durable gap metadata, and admit recent output.
4. Continue recording locally during pause/disconnection; expose backlog, gaps, rate/quota warnings, and receipt/gap upload interfaces.

## Acceptance criteria

- [ ] Postgres/gateway outages and a full spool do not stop session output, control, or process execution.
- [ ] Lost or duplicate receipts preserve correct reclamation; crashes cannot erase acknowledged gap information or rewrite chunk capture times.
- [ ] Known secrets are redacted and terminal password/no-echo input is absent from recorded raw input.
- [ ] Warnings survive reconnect and recovered recording retains earlier loss markers instead of presenting complete history.

## Verification

- Run disk-cap, disk-write failure, restart, duplicate receipt, long-disconnect, pause/resume, redaction, and no-echo PTY scenarios with a fake uploader.

Run the relevant formatting, lint, dependency-boundary, and supported-platform checks once code exists. Record real results and any unavailable environment; do not report planned or mocked external validation as completed.

## Boundaries and handoff

No PostgreSQL content store, retention job, or live replay replacement. Mandatory recording cannot be disabled through a feature flag.

**Deliver:** Spool fetch/ack APIs, immutable batch fixtures, and persistent gap/warning events for 021 and the TUI.
