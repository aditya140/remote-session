# 024 — Implement the TUI machine, session, terminal, and approval flows

- **Status:** Not started
- **Milestone:** M0 minimal TUI / M1 full TUI
- **Workstream:** TUI
- **Depends on:** [023](023-implement-rust-client-sdk.md)
- **Index:** [Task plan](README.md)

## Goal and repository context

Deliver the first-release user interface using the client SDK, with visible control, recording, and reconnect state.

**Read first:** [PRD.md](../PRD.md) §§5.2, 5.4, CL-1; [components.md](../components.md) §5 TUI layout; [architecture.md](../architecture.md) §§6.3–6.6.

## Owned scope

- `crates/qgenie-remote-tui/ including state/update/effects/views/widgets`

Paths describe proposed implementation locations; they do not exist yet unless another task creates them. Follow [the subcrate standard](../subcrate-standard.md), add only useful modules, and coordinate shared manifests/exports and migrations as described in the index.

## Implementation

1. Build machine/session list, filtering, inventory/presence, rename/label/revoke actions, scoped new-session folder selection, and advertised-agent choice.
2. Implement structured transcript/tool/diff display, PTY pane, observe/attach, explicit takeover confirmation, detach via Ctrl-] d, interrupt, terminate, and historical resume as distinct actions.
3. Provide an owner-wide approvals list/badge and inline decisions without requiring controller access. Distinguish committed, applied, cancelled, and unknown approval results.
4. Render recorded indicators, gaps, truncated replay, unknown start outcome, offline/history, incompatibility, and revoked-access states. Keep network effects outside rendering and restore terminal state on errors.

## Acceptance criteria

- [ ] A user can list machines, create a terminal/Codex session, interact, detach, and reattach through the SDK.
- [ ] Observe mode blocks controller input but still allows eligible owner approvals.
- [ ] Capabilities drive available actions; destructive termination and control takeover have explicit confirmation.
- [ ] Recording/reconnect errors remain visible, and quitting/crashing restores terminal raw mode/cursor state.

## Verification

- Use deterministic SDK fixtures for UI state transitions and ratatui buffer checks, then manually exercise keyboard/navigation and terminal restoration. Run full connected workflows in 027.

Run the relevant formatting, lint, dependency-boundary, and supported-platform checks once code exists. Record real results and any unavailable environment; do not report planned or mocked external validation as completed.

## Boundaries and handoff

No Electron, web client, desktop notifications, or direct gateway/native-agent calls from widgets. Do not implement adoption before its later adapters exist.

**Deliver:** A callable TUI entry point and documented key bindings for integration into the existing CLI in 025.
