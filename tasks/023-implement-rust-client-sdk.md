# 023 — Implement the Rust client SDK and reconnect state

- **Status:** Complete for the selected Linux standalone development scope; external release qualification remains separately tracked
- **Milestone:** M0 client / M1 complete operations
- **Workstream:** Rust client
- **Depends on:** [005](005-implement-websocket-transport.md)
- **Index:** [Task plan](README.md)

**Scope:** The user selected Linux development completion in [ADR 0007](../docs/adr/0007-standalone-preview.md). See [current progress](PROGRESS.md) and [external release gates](../docs/validation/external-release-gates.md). Original criteria that require unrun environments or waived corporate inspection remain unchecked; this status does not claim full release qualification.

## Goal and repository context

Provide a UI-independent client that can be developed against protocol fixtures while gateway and host implementations proceed.

**Read first:** [components.md](../components.md) §§2, 4–6; [architecture.md](../architecture.md) §§6.3–6.6, 7; [PRD.md](../PRD.md) SE-7–10, CL-1.

## Owned scope

- `crates/qgenie-remote-client/`

Owned paths define this task’s implementation boundary; current source and executed evidence are recorded below. Follow [the subcrate standard](../subcrate-standard.md), add only useful modules, and coordinate shared manifests/exports and migrations as described in the index.

## Implementation

1. Accept injected credentials and typed configuration; implement REST/WSS machine/session subscriptions, scoped directory browsing, session start/attach/control, owner approvals, and protected history.
2. Track request correlation and not_sent/unknown/rejected outcomes. Reconcile uncertain starts with the persisted key; never blindly start again or replay unacknowledged input.
3. Implement output deduplication, gap/tail-loss detection, snapshot/live cutover, truncated-history handling, and explicit controller reacquisition/takeover.
4. Surface approval committed/application states, recording gaps, revocation, minimum-version errors, and caller-owned cancellation. Bound pending requests and subscription queues.

## Acceptance criteria

- [x] The SDK is usable without ratatui, native adapters, or copied OAuth code.
- [x] Fixture-backed dropped/duplicated/out-of-order/tail frames reconcile correctly; reconnect never resends terminal keystrokes automatically.
- [x] Owner approvals work without a control lease; stale-controller and revoked states surface as actionable typed errors.
- [x] Slow subscribers, cancellation, token refresh, and compatibility rejection have bounded resources and documented behavior.

## Verification

- Run public-API tests against a fixture-driven REST/WSS server using 004's vectors. Real gateway/sessiond validation is performed in 027, so this task need not wait for those implementations.

Run the relevant formatting, lint, dependency-boundary, and supported-platform checks once code exists. Record real results and any unavailable environment; do not report planned or mocked external validation as completed.

## Boundaries and handoff

No widget rendering, credential storage implementation, or TypeScript SDK.

**Deliver:** Stable Rust APIs, SDK examples, and error/reconnect fixtures consumed independently by 024 and CLI composition.


## Current gateway SDK

The UI-independent SDK now accepts injected tokens and canonical HTTPS/CA configuration.
Protected stream discovery/history, bounded independent stored-hash decoding and typed
owner/admin errors pass against the real protected gateway. An owned WSS driver provides
bounded correlated reads, explicit reauthentication and event delivery; out-of-order replies,
late duplicates, dropped responses, malformed snapshot order and slow subscribers have real
TLS fixture proofs. Actual two-gateway/native connector tests pass inventory, session reads,
browse and refresh. The SDK stores no OAuth credential and performs no automatic request or
input retry. Public start/control/approval operations and output recovery now compose through the
actual native/two-gateway fixture. Full platform/release qualification remains in task 027.

Public PTY start now passes the actual persisted-key SDK/WSS/gateway/activated connector/native process path in the owned synchronous recording harness. Same-key retry produces one child/start audit; changed parameters are NotSent; unready storage cannot launch. The connector automatically uploads real native capture and end metadata. PTY and managed controls, durable approvals and guarded historical resume now pass the composed Linux fixture; see PROGRESS.md for current evidence.

## Completed SDK evidence

Twenty-four client tests cover real TLS correlation/reordering/duplicates, PTY and agent
lost-start replies, input poisoning, token refresh during uncertain input, capability
rejection, owner-only approval reads and one-shot choices. The owned `RemoteObserver`
installs a checkpoint, performs bounded continuation reads, detects an idle final tail and
retains a final slow-consumer error outside its eight-update/16 MiB queue. Dropping it
cancels pending reads without input replay or native termination. Reconnect and controller
reacquisition remain explicit.

The [SDK README](../crates/qgenie-remote-client/README.md) documents the APIs and bounds.
Actual synchronous-storage/two-gateway/native tests additionally verify public creation,
structured observation, control, approval delivery and new-cwd historical resume. This
component status does not complete the platform, tenant or full process-fault gates in 027.

Pushed native head/session state now wakes output continuation immediately. Older metadata cannot rewind a cursor or release unverified bytes; the new causal fixture qualifies that behavior. `session_changes_available()` exposes negotiated push support, and idle output reconciliation stays bounded.
