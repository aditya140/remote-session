# 006 — Implement secure same-user local IPC

- **Status:** Not started
- **Milestone:** M0 foundation
- **Workstream:** Transport / IPC
- **Depends on:** [004](004-implement-protocol-and-conformance.md)
- **Index:** [Task plan](README.md)

## Goal and repository context

The host connector must communicate with an independently running session owner without linking its implementation or exposing administrative commands remotely.

**Read first:** [components.md](../components.md) §4 contract 2; [architecture.md](../architecture.md) §§4.1, 8; [subcrate-standard.md](../subcrate-standard.md) §§5–6.

## Owned scope

- `crates/qgenie-remote-transport/src/ipc/ and IPC feature wiring`
- `IPC peer/permission integration tests`

Paths describe proposed implementation locations; they do not exist yet unless another task creates them. Follow [the subcrate standard](../subcrate-standard.md), add only useful modules, and coordinate shared manifests/exports and migrations as described in the index.

## Implementation

1. Implement Linux/macOS Unix-domain connections with private directories/socket permissions and SO_PEERCRED/getpeereid checks as appropriate.
2. Negotiate the independent IPC version and carry remote actor, action, policy facts, and current generations using task 004 types.
3. Keep local privileged lifecycle commands distinct from the remotely dispatched operation set. Bound payloads, queues, and pending requests.
4. Handle stale sockets, reconnect, cancellation, and incompatible peers without deleting another live process's socket. Reserve a documented Windows boundary without shipping ConPTY/pipe support yet.

## Acceptance criteria

- [ ] Same-user peers can negotiate and exchange bounded requests; foreign users and insecure socket locations are refused.
- [ ] A remote-dispatch request cannot invoke local administrative lifecycle operations.
- [ ] Malformed frames, incompatible versions, stale socket recovery, and interrupted reads fail explicitly.
- [ ] Gateway-only builds exclude local platform IPC dependencies; connector and sessiond remain separate runtime roles.

## Verification

- Run Unix IPC integration tests on Linux and macOS, including permission and foreign-user checks in a suitable test environment; report unsupported test environments explicitly.

Run the relevant formatting, lint, dependency-boundary, and supported-platform checks once code exists. Record real results and any unavailable environment; do not report planned or mocked external validation as completed.

## Boundaries and handoff

No session registry, process supervisor, Windows implementation, or authorization authority in transport.

**Deliver:** Secure IPC endpoints and tests. Coordinate only transport manifest/lib exports with 005; keep ws and ipc implementation files separate.
