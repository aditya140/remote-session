# 016 — Implement bounded replay and atomic snapshot/live cutover

- **Status:** Not started
- **Milestone:** M0 reattachment
- **Workstream:** Session replay
- **Depends on:** [015](015-implement-session-runtime-and-start-journal.md)
- **Index:** [Task plan](README.md)

## Goal and repository context

A reconnecting client must recover output from the host with no hidden gap or duplicate, including a lost final frame that no later message exposes.

**Read first:** [PRD.md](../PRD.md) SE-7 and §7 Durability; [architecture.md](../architecture.md) §§6.5–6.6; [components.md](../components.md) §4 contract 7.

## Owned scope

- `Sessiond replay/{log,cursor,snapshot}.rs`
- `Replay integration fixtures; narrow PTY snapshot integration with 008`

Paths describe proposed implementation locations; they do not exist yet unless another task creates them. Follow [the subcrate standard](../subcrate-standard.md), add only useful modules, and coordinate shared manifests/exports and migrations as described in the index.

## Implementation

1. Assign durable per-session output cursors with a defined stream scope and retained floor/head. Implement configurable bounded replay meeting the agreed 10k-event/8 MB baseline.
2. Capture a PTY screen/scrollback snapshot at a known cursor and coordinate snapshot, replay, and live delivery so concurrent output is neither skipped nor repeated.
3. Return explicit history_truncated and resync metadata when the cursor is too old; define structured-agent fallback without pretending unavailable history exists.
4. Expose periodic head-cursor reconciliation for tail loss, and report last acknowledged input separately from output cursors. Keep recording storage independent.

## Acceptance criteria

- [ ] Reconnect during active PTY/agent output reconstructs ordered output without duplicate rendering.
- [ ] Old cursors produce an explicit truncated response and valid snapshot/live boundary.
- [ ] A dropped final output frame is detected through head reconciliation even if the session becomes idle.
- [ ] Replay memory/disk remains bounded and is never served from the gateway transcript database.

## Verification

- Run cursor-boundary, concurrent snapshot, wrap/retention, duplicate/out-of-order, restart, and tail-loss tests using deterministic event streams and a real PTY fixture.

Run the relevant formatting, lint, dependency-boundary, and supported-platform checks once code exists. Record real results and any unavailable environment; do not report planned or mocked external validation as completed.

## Boundaries and handoff

Historical transcript browsing, recording upload, and automatic resending of terminal input are outside this task.

**Deliver:** Host attachment/replay contract and fixtures consumed by the Rust SDK and end-to-end recovery tests.
