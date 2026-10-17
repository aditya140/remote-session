# 005 — Implement bounded WSS transport with proxy support

- **Status:** Not started
- **Milestone:** M0 foundation
- **Workstream:** Transport / WSS
- **Depends on:** [004](004-implement-protocol-and-conformance.md)
- **Index:** [Task plan](README.md)

## Goal and repository context

Both hosts and clients use outbound HTTPS/WSS through corporate proxies. Provide common connection plumbing without embedding session retry or authorization rules.

**Read first:** [architecture.md](../architecture.md) §§4.2–4.3, 6.6–7; [PRD.md](../PRD.md) §7 Networking; [subcrate-standard.md](../subcrate-standard.md) §§3–4.

## Owned scope

- `crates/qgenie-remote-transport/src/{config,codec,connection,queues,heartbeat,backoff,error}.rs`
- `crates/qgenie-remote-transport/src/ws/ and WSS integration tests`

Paths describe proposed implementation locations; they do not exist yet unless another task creates them. Follow [the subcrate standard](../subcrate-standard.md), add only useful modules, and coordinate shared manifests/exports and migrations as described in the index.

## Implementation

1. Implement client/server feature boundaries, validated frames, bounded queues with explicit byte/count limits, and prioritized control, agent-event, and bulk-output lanes.
2. Honor HTTPS_PROXY, NO_PROXY, and configured corporate CA trust without disabling TLS verification; support cancellation and graceful close.
3. Implement configurable ping/pong deadlines with heartbeat at most 25 seconds, jittered reconnect primitives, and typed close/error outcomes.
4. Specify slow-observer disconnection/resync behavior and ensure a slow connection cannot stall other connections. Expose metrics hooks; do not install global tracing.

## Acceptance criteria

- [ ] A client connects through a test CONNECT proxy using a trusted test CA; untrusted certificates are rejected.
- [ ] Idle-timeout, dropped pong, oversized frame, saturated queue, and shutdown paths have bounded resource use.
- [ ] Control traffic remains responsive during bulk output, and queued approvals/mutations are never silently dropped.
- [ ] The transport never blindly retries starts, approvals, or terminal input and does not make authorization decisions.

## Verification

- Run transport contract tests with direct/proxied TLS endpoints, bypass rules, a 60-second proxy idle timeout, slow readers, and cancellation.

Run the relevant formatting, lint, dependency-boundary, and supported-platform checks once code exists. Record real results and any unavailable environment; do not report planned or mocked external validation as completed.

## Boundaries and handoff

Local IPC is owned by 006. Client replay and operation outcomes are owned by 023.

**Deliver:** WSS connection APIs, queue policies, backoff primitives, and error classifications usable with fake authenticated peers.
