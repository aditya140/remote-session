# 004 — Implement public protocol types and conformance fixtures

- **Status:** Not started
- **Milestone:** M0 foundation
- **Workstream:** Protocol
- **Depends on:** [002](002-resolve-runtime-and-protocol-contracts.md), [003](003-bootstrap-rust-workspace-and-ci.md)
- **Index:** [Task plan](README.md)

## Goal and repository context

Host, gateway, SDK, and adapters need a single typed contract before they can be built independently.

**Read first:** [architecture.md](../architecture.md) §7 and §§6.1–6.6; [components.md](../components.md) §4 contracts 1–8; [subcrate-standard.md](../subcrate-standard.md) §§3, 6–7.

## Owned scope

- `crates/qgenie-remote-proto/ including separately versioned ipc types`
- `conformance/json/, conformance/binary/, conformance/scenarios/`
- `Schema export entry point in xtask/`

Paths describe proposed implementation locations; they do not exist yet unless another task creates them. Follow [the subcrate standard](../subcrate-standard.md), add only useful modules, and coordinate shared manifests/exports and migrations as described in the index.

## Implementation

1. Implement typed IDs, request/reply correlation, capabilities, versions, stable errors, machine/session/filesystem operations, approvals, recording receipts/gaps, and authority generations from 002.
2. Implement JSON and binary PTY validation: fixed endianness, reserved flags, size limits, malformed/truncated frames, and a representation that preserves u64 values in future JavaScript clients.
3. Define canonical challenge bytes without key storage or signing implementation. Separate local administrative IPC types from exported public gateway operations.
4. Publish fixtures for starts, replay/snapshot boundaries, input acknowledgments, approvals, recording, and version negotiation; export deterministic public JSON Schema.

## Acceptance criteria

- [ ] Golden fixtures round-trip without changing IDs, sequence values, or unknown-field behavior.
- [ ] Unsupported versions/capabilities and invalid or oversized frames fail with stable errors.
- [ ] Values above JavaScript's safe integer range are preserved by the chosen wire representation.
- [ ] The crate has no runtime, database, OAuth, keyring, TUI, or native-agent dependency; public schemas exclude privileged IPC.

## Verification

- Run protocol unit and fixture tests plus schema export twice and compare output. Test invalid/truncated binary frames and N-2 negotiation fixtures without inventing real historical CLI versions.

Run the relevant formatting, lint, dependency-boundary, and supported-platform checks once code exists. Record real results and any unavailable environment; do not report planned or mocked external validation as completed.

## Boundaries and handoff

No transport I/O, database rows in public APIs, or TypeScript SDK implementation.

**Deliver:** Published public types, IPC types, fixture vectors, and schema output. These are the shared inputs for parallel tasks 005–008, 010, and 023.
