# 004 — Implement public protocol types and conformance fixtures

- **Status:** Complete for the selected Linux standalone development scope; external release qualification remains separately tracked
- **Milestone:** M0 foundation
- **Workstream:** Protocol
- **Depends on:** [002](002-resolve-runtime-and-protocol-contracts.md), [003](003-bootstrap-rust-workspace-and-ci.md)
- **Index:** [Task plan](README.md)

**Scope:** The user selected Linux development completion in [ADR 0007](../docs/adr/0007-standalone-preview.md). See [current progress](PROGRESS.md) and [external release gates](../docs/validation/external-release-gates.md). Original criteria that require unrun environments or waived corporate inspection remain unchecked; this status does not claim full release qualification.

## Goal and repository context

Host, gateway, SDK, and adapters need a single typed contract before they can be built independently.

**Read first:** [architecture.md](../architecture.md) §7 and §§6.1–6.6; [components.md](../components.md) §4 contracts 1–8; [subcrate-standard.md](../subcrate-standard.md) §§3, 6–7.

## Owned scope

- `crates/qgenie-remote-proto/ including separately versioned ipc types`
- `conformance/json/, conformance/binary/, conformance/scenarios/`
- `Schema export entry point in xtask/`

Owned paths define this task’s implementation boundary; current source and executed evidence are recorded below. Follow [the subcrate standard](../subcrate-standard.md), add only useful modules, and coordinate shared manifests/exports and migrations as described in the index.

## Implementation

1. Implement typed IDs, request/reply correlation, capabilities, versions, stable errors, machine/session/filesystem operations, approvals, recording receipts/gaps, and authority generations from 002.
2. Implement JSON and binary PTY validation: fixed endianness, reserved flags, size limits, malformed/truncated frames, and a representation that preserves u64 values in future JavaScript clients.
3. Define canonical challenge bytes without key storage or signing implementation. Separate local administrative IPC types from exported public gateway operations.
4. Publish fixtures for starts, replay/snapshot boundaries, input acknowledgments, approvals, recording, and version negotiation; export deterministic public JSON Schema.

## Acceptance criteria

- [x] Golden fixtures round-trip without changing IDs, sequence values, or unknown-field behavior.
- [x] Unsupported versions/capabilities and invalid or oversized frames fail with stable errors.
- [x] Values above JavaScript's safe integer range are preserved by the chosen wire representation.
- [x] The crate has no runtime, database, OAuth, keyring, TUI, or native-agent dependency; public schemas exclude privileged IPC.

## Verification

- Run protocol unit and fixture tests plus schema export twice and compare output. Test invalid/truncated binary frames and N-2 negotiation fixtures without inventing real historical CLI versions.

Run the relevant formatting, lint, dependency-boundary, and supported-platform checks once code exists. Record real results and any unavailable environment; do not report planned or mocked external validation as completed.

## Boundaries and handoff

No transport I/O, database rows in public APIs, or TypeScript SDK implementation.

**Deliver:** Published public types, IPC types, fixture vectors, and schema output. These are the shared inputs for parallel tasks 005–008, 010, and 023.

## Completion evidence

The proto crate now separates the public `wire::Envelope` from the local preview request/reply types and independently versioned `ipc::DispatchFrame`. Administrative shutdown cannot deserialize as dispatch and does not appear in the public schema.

The public contract includes machine/inventory/filesystem operations, PTY and agent launch, replay/snapshots, mutation generations, input acknowledgments, approval decision/application distinctions, recording batches/receipts, loss ranges, and canonical UTC capture timestamps. The recording digest binds the exact compressed bytes. Token storage/signing remains outside this crate.

Protocol tests passed for typed IDs, full-u64 decimal encoding, binary endianness, every truncated input header, malformed/oversized JSON, duplicate fields, version/capability rejection, canonical challenge bytes, launch fingerprint changes, missing targets/fences, replay order, and administrative schema exclusion. Twelve public JSON fixtures and a binary fixture round-trip. Two shared failure scenarios are stored under conformance/scenarios.

The schema exporter ran and its independent check produced identical output. Strict protocol Clippy and the no-default-feature build passed. The crate has no runtime, database, OAuth, keyring, UI, or native-agent dependency. Tests establish contracts, not any vendor or tenant capability.
