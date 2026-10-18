# 008 — Implement the adapter interface and PTY driver

- **Status:** Complete for the selected Linux standalone development scope; external release qualification remains separately tracked
- **Milestone:** M0 terminal slice
- **Workstream:** Adapters / PTY
- **Depends on:** [004](004-implement-protocol-and-conformance.md)
- **Index:** [Task plan](README.md)

**Scope:** The user selected Linux development completion in [ADR 0007](../docs/adr/0007-standalone-preview.md). See [current progress](PROGRESS.md) and [external release gates](../docs/validation/external-release-gates.md). Original criteria that require unrun environments or waived corporate inspection remain unchecked; this status does not claim full release qualification.

## Goal and repository context

Provide a real shell execution capability and a small native-adapter boundary for the durable session runtime.

**Read first:** [PRD.md](../PRD.md) SE-1, SE-6–7, SE-11; [components.md](../components.md) §4 contract 3 and §5 adapter layout; [architecture.md](../architecture.md) §§4.1, 6.6.

## Owned scope

- `crates/qgenie-remote-adapters/src/{adapter,commands,events,launch,registry,error}.rs`
- `crates/qgenie-remote-adapters/src/pty/ and PTY fixtures`

Owned paths define this task’s implementation boundary; current source and executed evidence are recorded below. Follow [the subcrate standard](../subcrate-standard.md), add only useful modules, and coordinate shared manifests/exports and migrations as described in the index.

## Implementation

1. Define launch preparation, bounded event receivers/command handles, native identifiers, proven capabilities, and explicit native approval-application outcomes.
2. Implement Linux/macOS PTY launch with supplied cwd, command, environment, size, input, resize, interrupt, termination, and exit reporting.
3. Maintain a terminal screen/scrollback model and snapshot interface; keep public sequencing and durable logs outside the adapter.
4. Bound blocking work and queues, define ownership/shutdown behavior, and expose captured output/resizes without separately recording raw input.

## Acceptance criteria

- [x] A real PTY fixture accepts input, resizes, reports output/exit, and snapshots ANSI/UTF-8 state correctly.
- [x] Interrupt and terminate are distinguishable actions; losing an observer does not implicitly terminate the process.
- [x] Native implementation details and credentials do not escape into the shared protocol or debug output.
- [ ] Linux/macOS tests pass and feature isolation keeps PTY dependencies out of gateway builds.

## Verification

- Run real-process PTY tests with resize, alternate screen, split UTF-8, no-echo input, output bursts, and explicit cleanup.

Run the relevant formatting, lint, dependency-boundary, and supported-platform checks once code exists. Record real results and any unavailable environment; do not report planned or mocked external validation as completed.

## Boundaries and handoff

No durable registry, public event sequence assignment, process service management, or Windows ConPTY.

**Deliver:** Adapter API plus a working PTY driver for 015. Codex work in 009 owns separate modules and consumes this interface.

## Current evidence

PreparedPty separates validation/capabilities from native launch. Host-only launch environment injection is redacted and bounded. Native exit facts no longer expose portable-pty types. Screen/scrollback ownership moved into adapters; ANSI colors, alternate screens, split UTF-8, resize, and snapshot restoration pass. Input/output/resize/native exit and no-echo/separate-observer process tests pass on Linux. Read chunks, write deadlines, sessiond workers, and native Codex command/event queues are bounded. Three additional real PTY proofs verify a caught interrupt leaves the shell usable until explicit termination, a lossless 512 KiB output burst stays within read bounds while resize/input work, and dropping the last output reader leaves native execution/control usable. Linux qualification passes; macOS execution remains unrun.
