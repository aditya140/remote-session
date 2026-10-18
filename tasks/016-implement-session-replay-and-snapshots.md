# 016 — Implement bounded replay and atomic snapshot/live cutover

- **Status:** Complete for the selected Linux standalone development scope; external release qualification remains separately tracked
- **Milestone:** M0 reattachment
- **Workstream:** Session replay
- **Depends on:** [015](015-implement-session-runtime-and-start-journal.md)
- **Index:** [Task plan](README.md)

**Scope:** The user selected Linux development completion in [ADR 0007](../docs/adr/0007-standalone-preview.md). See [current progress](PROGRESS.md) and [external release gates](../docs/validation/external-release-gates.md). Original criteria that require unrun environments or waived corporate inspection remain unchecked; this status does not claim full release qualification.

## Goal and repository context

A reconnecting client must recover output from the host with no hidden gap or duplicate, including a lost final frame that no later message exposes.

**Read first:** [PRD.md](../PRD.md) SE-7 and §7 Durability; [architecture.md](../architecture.md) §§6.5–6.6; [components.md](../components.md) §4 contract 7.

## Owned scope

- `Sessiond replay/{log,cursor,snapshot}.rs`
- `Replay integration fixtures; narrow PTY snapshot integration with 008`

Owned paths define this task’s implementation boundary; current source and executed evidence are recorded below. Follow [the subcrate standard](../subcrate-standard.md), add only useful modules, and coordinate shared manifests/exports and migrations as described in the index.

## Implementation

1. Assign durable per-session output cursors with a defined stream scope and retained floor/head. Implement configurable bounded replay meeting the agreed 10k-event/8 MB baseline.
2. Capture a PTY screen/scrollback snapshot at a known cursor and coordinate snapshot, replay, and live delivery so concurrent output is neither skipped nor repeated.
3. Return explicit history_truncated and resync metadata when the cursor is too old; define structured-agent fallback without pretending unavailable history exists.
4. Expose periodic head-cursor reconciliation for tail loss, and report last acknowledged input separately from output cursors. Keep recording storage independent.

## Acceptance criteria

- [x] Reconnect during active PTY/agent output reconstructs ordered output without duplicate rendering.
- [x] Old cursors produce an explicit truncated response and valid snapshot/live boundary.
- [x] A dropped final output frame is detected through head reconciliation even if the session becomes idle.
- [x] Replay memory/disk remains bounded and is never served from the gateway transcript database.

## Verification

- Run cursor-boundary, concurrent snapshot, wrap/retention, duplicate/out-of-order, restart, and tail-loss tests using deterministic event streams and a real PTY fixture.

Run the relevant formatting, lint, dependency-boundary, and supported-platform checks once code exists. Record real results and any unavailable environment; do not report planned or mocked external validation as completed.

## Boundaries and handoff

Historical transcript browsing, recording upload, and automatic resending of terminal input are outside this task.

**Deliver:** Host attachment/replay contract and fixtures consumed by the Rust SDK and end-to-end recovery tests.

## Current PTY evidence and remaining integration

The complete versioned checkpoint now preserves both grids, scrollback, saved cursor/attributes, origin/input modes, and pending UTF-8/ANSI/string parser state. Tests restore at every byte boundary, including alternate-screen exit. The documented parser patches cap OSC at 4 KiB and validate corrupted/truncated checkpoints and narrow Unicode resizing. The public fixture contains a real golden checkpoint.

SQLite schema 2 commits a checkpoint, its cursor, the event and pruning atomically before the required suffix is evicted. Restart after a pruned log reconstructs from that checkpoint plus a contiguous tail. Missing/corrupt state fails explicitly. Native-owner loss reports an unconfirmed final tail and refuses a complete screen claim. Finished/historical sessions retain no native grid; capture loads historical state transiently.

Local attachment pages have a fixed client/session/epoch/cursor, at most 256 KiB each and 32 pages/eight MiB total. Native capture and SDK assembly each admit four concurrent workers. Cached assembly caps at 32 captures/32 MiB with 30-second expiry; the SDK caps assembly at 25 seconds. Tests cover concurrent output during page capture, cross-client denial, pruning, restart, missing history and idle final-frame reconciliation. Gateway correlation keeps all snapshot pages until a final head and ignores late duplicate read replies; the actual two-gateway test covers that sequence.

Structured-agent reconstruction and complete native remote SDK/host cutovers now also pass. These component proofs do not establish full remote MVP completion.


## Remote PTY composition evidence

The fixed native QGVT1 checkpoint now crosses independently negotiated private IPC and public
WSS capabilities, with bounded page/assembly bytes and no partial checkpoint exposure. Actual
128×512 multi-page capture, fresh replay continuations, original-access expiry over a SQLite
write wait and read-only replay not extending a controller pass. SDK causal output tests and
the real remote keyboard/detach/restore proof pass. Structured agent replay now also passes. Native change subscriptions are now implemented below; broader
process-fault coverage remains. Content uses bounded reads triggered by pushed heads.

Structured managed replay now uses a distinct `agent_state_v1` format instead of terminal
bytes. Its versioned 256-event/512 KiB timeline has an exact cutover sequence and explicit
truncation/unconfirmed-tail flags. The same native transaction commits checkpoints before
pruning their required suffix. The actual pinned Codex runtime proof sets a two-event replay
cap, reopens the owner store and restores its structured messages. SDK page assembly validates
the complete typed content before releasing it. Public agent creation, native capture, structured keyboard rendering, explicit checkpoint
refresh and historical recovery now pass through two gateways. The SDK-owned observer
qualifies checkpoint-first delivery, idle continuation, final reply before disconnect and
bounded slow-consumer/cancellation behavior. Historical conversation truncation stays in
the agent checkpoint; it is distinct from a transport replay gap. Native subscriptions are qualified below; the complete end-to-end crash/network/platform
matrix remains a task 027 evidence gate.

## Native subscriptions and pushed output heads

`native-changes-v1` registers one coalesced revision per immutable scope (64 scope slots,
four native streams). Original suspend-aware access and the replacement barrier guard each
actual Unix write; idle expiry closes the stream. Three real Unix socket tests prove owner
isolation, 10,000-change coalescing, cancellation and expiry under blocked writes without
using a later access budget.

The connector negotiates `host-changes-v1`, subscribes before reading state and replaces its
read-only subscription on grant renewal. Metadata changes push owner-wide session state;
output-only changes push exact native floor/head/input counters to attached viewers. The
cache updates after queue admission, and bounded periodic full state repairs missed metadata.
Native recording/replay/command owners remain independent. Pushed heads use the gateway's
bounded metadata lane, so primary projection waits cannot sit in its control-response loop.

SDK cursors use pushed metadata/head advances as read-only continuation hints, preserve
causal checkpoint boundaries and ignore older metadata heads. The actual composed resize
proof receives a pushed head before making any replay request. Supporting TUI peers
negotiate `session-changes-v1` and stop polling session lists; idle output reconciliation
remains for a lost final notification. Payloads still use bounded checkpoint/replay reads.
