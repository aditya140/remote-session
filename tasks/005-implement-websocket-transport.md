# 005 — Implement bounded WSS transport with proxy support

- **Status:** Complete for the selected Linux standalone development scope; external release qualification remains separately tracked
- **Milestone:** M0 foundation
- **Workstream:** Transport / WSS
- **Depends on:** [004](004-implement-protocol-and-conformance.md)
- **Index:** [Task plan](README.md)

**Scope:** The user selected Linux development completion in [ADR 0007](../docs/adr/0007-standalone-preview.md). See [current progress](PROGRESS.md) and [external release gates](../docs/validation/external-release-gates.md). Original criteria that require unrun environments or waived corporate inspection remain unchecked; this status does not claim full release qualification.

## Goal and repository context

Both hosts and clients use outbound HTTPS/WSS through corporate proxies. Provide common connection plumbing without embedding session retry or authorization rules.

**Read first:** [architecture.md](../architecture.md) §§4.2–4.3, 6.6–7; [PRD.md](../PRD.md) §7 Networking; [subcrate-standard.md](../subcrate-standard.md) §§3–4.

## Owned scope

- `crates/qgenie-remote-transport/src/{config,codec,connection,queues,heartbeat,backoff,error}.rs`
- `crates/qgenie-remote-transport/src/ws/ and WSS integration tests`

Implementation lives in `crates/qgenie-remote-transport/src/ws/` and `tests/wss.rs`. Follow [the subcrate standard](../subcrate-standard.md), add only useful modules, and coordinate shared manifests/exports and migrations as described in the index.

## Implementation

1. Implement client/server feature boundaries, validated frames, bounded queues with explicit byte/count limits, and prioritized control, agent-event, and bulk-output lanes.
2. Honor HTTPS_PROXY, NO_PROXY, and configured corporate CA trust without disabling TLS verification; support cancellation and graceful close.
3. Implement configurable ping/pong deadlines with heartbeat at most 25 seconds, jittered reconnect primitives, and typed close/error outcomes.
4. Specify slow-observer disconnection/resync behavior and ensure a slow connection cannot stall other connections. Expose metrics hooks; do not install global tracing.

## Acceptance criteria

- [x] A client connects through a test CONNECT proxy using a trusted test CA; untrusted certificates are rejected.
- [x] Idle-timeout, dropped pong, oversized frame, saturated queue, and shutdown paths have bounded resource use.
- [x] Control traffic remains responsive during bulk output, and queued approvals/mutations are never silently dropped.
- [x] The transport never blindly retries starts, approvals, or terminal input and does not make authorization decisions.

## Verification

- Run transport contract tests with direct/proxied TLS endpoints, bypass rules, a 60-second proxy idle timeout, slow readers, and cancellation.

Run the relevant formatting, lint, dependency-boundary, and supported-platform checks once code exists. Record real results and any unavailable environment; do not report planned or mocked external validation as completed.

## Boundaries and handoff

Local IPC is owned by 006. Client replay and operation outcomes are owned by 023.

**Deliver:** WSS connection APIs, queue policies, backoff primitives, and error classifications usable with fake authenticated peers.

## Completed evidence

Real rustls TLS tests cover direct connections, HTTP and HTTPS CONNECT with a trusted test CA, untrusted rejection, proxy credentials/redaction, NO_PROXY, and proxy errors. A real 65-second test through a 60-second-idle proxy passed. Bounded priority/byte/count tests, stalled writer deadlines, dropped pong, oversized/malformed input, slow-observer isolation, owner cancellation, and graceful close passed. Strict all-feature Clippy and independent client/server feature builds passed on Linux; macOS execution remains CI evidence. Transport does not reconnect or retry operations; receipts distinguish not-sent from unknown and do not claim native application. See the [transport API and limits](../crates/qgenie-remote-transport/README.md).
