# 006 — Implement secure same-user local IPC

- **Status:** Complete for the selected Linux standalone development scope; external release qualification remains separately tracked
- **Milestone:** M0 foundation
- **Workstream:** Transport / IPC
- **Depends on:** [004](004-implement-protocol-and-conformance.md)
- **Index:** [Task plan](README.md)

**Scope:** The user selected Linux development completion in [ADR 0007](../docs/adr/0007-standalone-preview.md). See [current progress](PROGRESS.md) and [external release gates](../docs/validation/external-release-gates.md). Original criteria that require unrun environments or waived corporate inspection remain unchecked; this status does not claim full release qualification.

## Goal and repository context

The host connector must communicate with an independently running session owner without linking its implementation or exposing administrative commands remotely.

**Read first:** [components.md](../components.md) §4 contract 2; [architecture.md](../architecture.md) §§4.1, 8; [subcrate-standard.md](../subcrate-standard.md) §§5–6.

## Owned scope

- `crates/qgenie-remote-transport/src/ipc/ and IPC feature wiring`
- `IPC peer/permission integration tests`

Owned paths define this task’s implementation boundary; current source and executed evidence are recorded below. Follow [the subcrate standard](../subcrate-standard.md), add only useful modules, and coordinate shared manifests/exports and migrations as described in the index.

## Implementation

1. Implement Linux/macOS Unix-domain connections with private directories/socket permissions and SO_PEERCRED/getpeereid checks as appropriate.
2. Negotiate the independent IPC version and carry remote actor, action, policy facts, and current generations using task 004 types.
3. Keep local privileged lifecycle commands distinct from the remotely dispatched operation set. Bound payloads, queues, and pending requests.
4. Handle stale sockets, reconnect, cancellation, and incompatible peers without deleting another live process's socket. Reserve a documented Windows boundary without shipping ConPTY/pipe support yet.

## Acceptance criteria

- [x] Same-user peers can negotiate and exchange bounded requests; foreign users and insecure socket locations are refused.
- [x] A remote-dispatch request cannot invoke local administrative lifecycle operations.
- [x] Malformed frames, incompatible versions, stale socket recovery, and interrupted reads fail explicitly.
- [x] Gateway-only builds exclude local platform IPC dependencies; connector and sessiond remain separate runtime roles.

## Verification

- Run Unix IPC integration tests on Linux and macOS, including permission and foreign-user checks in a suitable test environment; report unsupported test environments explicitly.

Run the relevant formatting, lint, dependency-boundary, and supported-platform checks once code exists. Record real results and any unavailable environment; do not report planned or mocked external validation as completed.

## Boundaries and handoff

No session registry, process supervisor, Windows implementation, or authorization authority in transport.

**Deliver:** Secure IPC endpoints and tests. Coordinate only transport manifest/lib exports with 005; keep ws and ipc implementation files separate.

## Current evidence

Every SDK/daemon dispatch connection negotiates IPC major 1 before a typed DispatchFrame exchange. Local admin lifecycle requests remain on the independent admin socket/schema. Tests pass for same-user peers, private permissions, oversized headers, incompatible versions, interrupted reads, live-socket preservation, stale-socket recovery, and exclusive listener ownership. Real separate-process and TUI keyboard checks pass after negotiation.

Native authority uses IPC 1.1 with an explicitly acknowledged `native-authority-v1` capability. Required-feature negotiation rejects older peers before an RPC is sent. Sessiond rejects authority frames following a legacy handshake while preserving the exact minor-zero welcome/local flow.

`bash tests/e2e/foreign_uid_ipc.sh` passed on Linux: an isolated, network-disabled container runs actual UID 0/1001 peers and proves directory permissions plus independent server/client peer-credential rejection. Fixture-only permission/ownership changes allow the credential checks to be exercised separately. The container has no host mounts or privileged flag and is removed afterward. CI includes this proof. macOS execution remains unrun. Unauthenticated remote frames are refused; activated remote dispatch now composes through tasks 014/017.
