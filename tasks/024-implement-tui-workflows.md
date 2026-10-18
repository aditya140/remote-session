# 024 — Implement the TUI machine, session, terminal, and approval flows

- **Status:** Complete for the selected Linux standalone development scope; external release qualification remains separately tracked
- **Milestone:** M0 minimal TUI / M1 full TUI
- **Workstream:** TUI
- **Depends on:** [023](023-implement-rust-client-sdk.md)
- **Index:** [Task plan](README.md)

**Scope:** The user selected Linux development completion in [ADR 0007](../docs/adr/0007-standalone-preview.md). See [current progress](PROGRESS.md) and [external release gates](../docs/validation/external-release-gates.md). Original criteria that require unrun environments or waived corporate inspection remain unchecked; this status does not claim full release qualification.

## Goal and repository context

Deliver the first-release user interface using the client SDK, with visible control, recording, and reconnect state.

**Read first:** [PRD.md](../PRD.md) §§5.2, 5.4, CL-1; [components.md](../components.md) §5 TUI layout; [architecture.md](../architecture.md) §§6.3–6.6.

## Owned scope

- `crates/qgenie-remote-tui/ including state/update/effects/views/widgets`

Owned paths define this task’s implementation boundary; current source and executed evidence are recorded below. Follow [the subcrate standard](../subcrate-standard.md), add only useful modules, and coordinate shared manifests/exports and migrations as described in the index.

## Implementation

1. Build machine/session list, filtering, inventory/presence, rename/label/revoke actions, scoped new-session folder selection, and advertised-agent choice.
2. Implement structured transcript/tool/diff display, PTY pane, observe/attach, explicit takeover confirmation, detach via Ctrl-] d, interrupt, terminate, and historical resume as distinct actions.
3. Provide an owner-wide approvals list/badge and inline decisions without requiring controller access. Distinguish committed, applied, cancelled, and unknown approval results.
4. Render recorded indicators, gaps, truncated replay, unknown start outcome, offline/history, incompatibility, and revoked-access states. Keep network effects outside rendering and restore terminal state on errors.

## Acceptance criteria

- [x] A user can list machines, create a terminal/Codex session, interact, detach, and reattach through the SDK.
- [x] Observe mode blocks controller input but still allows eligible owner approvals.
- [x] Capabilities drive available actions; destructive termination and control takeover have explicit confirmation.
- [x] Recording/reconnect errors remain visible, and quitting/crashing restores terminal raw mode/cursor state.

## Verification

- Use deterministic SDK fixtures for UI state transitions and ratatui buffer checks, then manually exercise keyboard/navigation and terminal restoration. Run full connected workflows in 027.

Run the relevant formatting, lint, dependency-boundary, and supported-platform checks once code exists. Record real results and any unavailable environment; do not report planned or mocked external validation as completed.

## Boundaries and handoff

No Electron, web client, desktop notifications, or direct gateway/native-agent calls from widgets. Do not implement adoption before its later adapters exist.

**Deliver:** A callable TUI entry point and documented key bindings for integration into the existing CLI in 025.


## Remote recorded-history view

`qgenie-remote remote --gateway ORIGIN --token-file PATH [--ca-bundle PATH]` uses only the
client SDK for owner machine/session metadata and protected stream/history reads. A private
caller-owned token is injected; the TUI implements no OAuth or credential storage. Network
and decode jobs are bounded and independent of keyboard/signal handling. Original gaps and
redacted text are visible, control/bidi escapes are neutralized in history presentation, and
displayed history has explicit line/byte bounds. Stale selections cannot apply a different
stream's response. The existing terminal/signal guards are shared with local mode. Real keyboard/redaction/gap/quit/SIGTERM verification passes through the owned synchronous recording fixture. Live PTY/agent attach/control, owner approval actions and historical resume now have
subsequent actual keyboard evidence below.


## Live remote terminal evidence

The owned synchronous recording fixture also runs a real remote keyboard flow through two
gateway compositions and the actual daemon. It proves scoped folder browsing/new PTY start,
private persisted start identity, observe/control/input/release, discarded read-only keys,
detach and reattach, explicit cancellation/confirmation of termination, and lookup of the
same ended session after TUI restart without another child. SIGTERM and quit restore terminal
modes. Native state and recording warnings refresh separately from output/input. Input loss
requires explicit reacquisition; no queued input is replayed after reconnect. See README for
keys. Agent/approval UI and advertised-agent selection now also pass below; platform qualification remains separate.

The machine view now supports F2 rename, F3 label create/update/remove and F6 confirmed
remote-access revocation through the SDK. Names/keys/values have their server byte bounds;
requests submit once, uncertain replies stay uncertain and no mutation is resent. Cancelled
rename/remove/revoke actions have no effect. Revocation leaves a real scoped native PTY
running and locally controllable; its capture remains durable after remote access is gone.
The owned synchronous keyboard fixture verifies these flows and terminal restoration.

`/` filters machines by name/hostname/labels and sessions by cwd/state/agent or command label.
Enter applies, Esc restores the earlier filter, and Ctrl-U clears it. Filtered indexes are
cached outside rendering. Hidden/empty rows cannot remain action targets, and session
selection stays within the selected machine. Actual keyboard proofs cover matching labels,
typing `q` inside a filter, empty filtered machine/session lists, disabled hidden actions,
clearing filters and the existing live-terminal workflow. The latest expanded pipeline passes
in 44.27s; the independent primary-volume-loss/held-content proof is being finalized.

The remote live pane now also has structured message/tool rendering, scrollback, explicit
truncation/unconfirmed-tail labels and a bounded prompt/steer editor. It never feeds agent
text to the terminal parser; streamed messages are replaced by their completed body rather
than duplicated. Prompt/steer submits once through the SDK input cursor and clears unsent
text on loss, release and detach. Unit checks cover completion/coalescing and terminal/bidi
escape isolation. Actual public agent launch/approval UI and agent keyboard evidence are
now passing in the expanded owned synchronous fixture: advertised pinned Codex selection,
public creation, observe/control/prompt, structured checkpoint, cancellation/confirmation of
termination, owner approval inbox/cancel/confirm/application, and terminal restoration.
The source for these sessions is the public SDK and current native inventory. Historical
resume now also passes actual keys: new identity, saved thread, fresh cwd, partial-history
checkpoint and explicit termination. The complete fault/platform matrix remains in 027.

F7 inside an observing live pane opens the same owner approval inbox without taking control.
The composed keyboard proof selects the exact machine/session IDs, cancels then confirms an
offered answer, reconciles native application, returns to observe mode and verifies zero
controller leases. Returning installs a fresh output checkpoint; queued input is discarded.
Machine/session filtering accepts IDs in addition to names, labels, cwd, state and agent.
The [MVP evidence report](../docs/validation/mvp-evidence.md) records the remaining gates.

The machine list displays hostname, OS/architecture, CLI version, native agent versions and
last seen time. An owner-wide pending-approval badge refreshes through current SDK reads and
change hints, with a single ten-second job, at most 32 pages and explicit partial/unavailable
labels. It never authorizes a choice. No controller lease or native attachment is acquired.

Supporting peers now receive live session state over `session-changes-v1`, without periodic session-list requests. Pushed heads wake bounded output continuations. Folder, recovery and machine-action dialogs drain unsolicited events while their SDK calls run, then return to fresh metadata/checkpoints. Session selection follows identity across pushed list reordering; the real keyboard fixture navigates explicitly when running and ended sessions share a cwd.
