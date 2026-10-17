# 009 — Validate and implement the pinned Codex adapter

- **Status:** Not started
- **Milestone:** M0 proof / M1 Codex
- **Workstream:** Adapters / Codex
- **Depends on:** [001](001-inspect-existing-qgenie-integration.md), [008](008-implement-adapter-contract-and-pty.md)
- **Index:** [Task plan](README.md)

## Goal and repository context

The proposed app-server commands, shared-daemon behavior, and relay-disable settings are assumptions until checked against the Codex version QGenie actually distributes.

**Read first:** [PRD.md](../PRD.md) SE-2, SE-2a, SE-5; [architecture.md](../architecture.md) §§4.1, 4.7; [components.md](../components.md) §8.

## Owned scope

- `crates/qgenie-remote-adapters/src/codex/`
- `docs/integration/codex-capabilities.md and sanitized native protocol fixtures`

Paths describe proposed implementation locations; they do not exist yet unless another task creates them. Follow [the subcrate standard](../subcrate-standard.md), add only useful modules, and coordinate shared manifests/exports and migrations as described in the index.

## Implementation

1. Pin and inspect the actual distributed Codex version and generated native schemas. Prove supported launch/connect, thread start/resume, prompt/steer, interrupt, streamed items, approval, and reconnect operations.
2. Implement the verified subset behind task 008's interface and reuse task 001's endpoint-injection/launch integration. Record unsupported features as blockers or explicit capability limits.
3. Guard Unix sockets by ownership/permissions and verified loopback endpoints; refuse non-loopback listeners. Confirm the real relay-disable setting and detect/report relay re-enablement.
4. Normalize native events and approval outcomes, preserving native IDs for reconciliation. Distinguish requested, applied, cancelled, already-applied, and uncertain outcomes as supported by evidence.

## Acceptance criteria

- [ ] A reproducible pinned-version proof demonstrates all claimed MVP capabilities; fake fixtures alone are not treated as vendor validation.
- [ ] Managed launch uses QGenie endpoints without exposing provider credentials to the gateway or clients.
- [ ] Non-loopback or wrong-owner endpoints fail; the verified managed configuration disables the vendor relay.
- [ ] Disconnected/restarted native sessions do not create false success or exactly-once claims; the capability matrix records limits.

## Verification

- Run native contract tests against the pinned binary plus fixture tests for schema changes, reconnect, approval cancellation, endpoint rejection, and relay configuration drift.

Run the relevant formatting, lint, dependency-boundary, and supported-platform checks once code exists. Record real results and any unavailable environment; do not report planned or mocked external validation as completed.

## Boundaries and handoff

External live-session adoption, Claude/Pi, and claims about an uninspected Codex version are deferred. Missing binary/source access blocks native proof, not unrelated PTY work.

**Deliver:** Verified capability matrix, adapter implementation, and relay diagnostics consumed by sessiond, host doctor, and MVP tests.
