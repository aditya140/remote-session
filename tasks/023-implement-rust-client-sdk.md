# 023 — Implement the Rust client SDK and reconnect state

- **Status:** Not started
- **Milestone:** M0 client / M1 complete operations
- **Workstream:** Rust client
- **Depends on:** [005](005-implement-websocket-transport.md)
- **Index:** [Task plan](README.md)

## Goal and repository context

Provide a UI-independent client that can be developed against protocol fixtures while gateway and host implementations proceed.

**Read first:** [components.md](../components.md) §§2, 4–6; [architecture.md](../architecture.md) §§6.3–6.6, 7; [PRD.md](../PRD.md) SE-7–10, CL-1.

## Owned scope

- `crates/qgenie-remote-client/`

Paths describe proposed implementation locations; they do not exist yet unless another task creates them. Follow [the subcrate standard](../subcrate-standard.md), add only useful modules, and coordinate shared manifests/exports and migrations as described in the index.

## Implementation

1. Accept injected credentials and typed configuration; implement REST/WSS machine/session subscriptions, scoped directory browsing, session start/attach/control, owner approvals, and protected history.
2. Track request correlation and not_sent/unknown/rejected outcomes. Reconcile uncertain starts with the persisted key; never blindly start again or replay unacknowledged input.
3. Implement output deduplication, gap/tail-loss detection, snapshot/live cutover, truncated-history handling, and explicit controller reacquisition/takeover.
4. Surface approval committed/application states, recording gaps, revocation, minimum-version errors, and caller-owned cancellation. Bound pending requests and subscription queues.

## Acceptance criteria

- [ ] The SDK is usable without ratatui, native adapters, or copied OAuth code.
- [ ] Fixture-backed dropped/duplicated/out-of-order/tail frames reconcile correctly; reconnect never resends terminal keystrokes automatically.
- [ ] Owner approvals work without a control lease; stale-controller and revoked states surface as actionable typed errors.
- [ ] Slow subscribers, cancellation, token refresh, and compatibility rejection have bounded resources and documented behavior.

## Verification

- Run public-API tests against a fixture-driven REST/WSS server using 004's vectors. Real gateway/sessiond validation is performed in 027, so this task need not wait for those implementations.

Run the relevant formatting, lint, dependency-boundary, and supported-platform checks once code exists. Record real results and any unavailable environment; do not report planned or mocked external validation as completed.

## Boundaries and handoff

No widget rendering, credential storage implementation, or TypeScript SDK.

**Deliver:** Stable Rust APIs, SDK examples, and error/reconnect fixtures consumed independently by 024 and CLI composition.
