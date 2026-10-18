# 014 — Implement the host connector and local filesystem enforcement

- **Status:** Complete for the selected Linux standalone development scope; external release qualification remains separately tracked
- **Milestone:** M0 terminal slice
- **Workstream:** Host connector
- **Depends on:** [006](006-implement-secure-local-ipc.md), [007](007-implement-policy-engine.md), [013](013-implement-host-enrollment-and-credentials.md)
- **Index:** [Task plan](README.md)

**Scope:** The user selected Linux development completion in [ADR 0007](../docs/adr/0007-standalone-preview.md). See [current progress](PROGRESS.md) and [external release gates](../docs/validation/external-release-gates.md). Original criteria that require unrun environments or waived corporate inspection remain unchecked; this status does not claim full release qualification.

## Goal and repository context

Keep one outbound WSS connection, enforce local restrictions, and forward authorized operations to sessiond without owning the processes.

**Read first:** [PRD.md](../PRD.md) PR-4, PO-1, SE-5, SE-12b; [components.md](../components.md) §§4–6; [architecture.md](../architecture.md) §§4.1, 8.

## Owned scope

- `Host {host,connector,dispatch,ipc,filesystem,inventory}.rs`
- `Host policy/{paths,agents}.rs and policy/filesystem tests`

Owned paths define this task’s implementation boundary; current source and executed evidence are recorded below. Follow [the subcrate standard](../subcrate-standard.md), add only useful modules, and coordinate shared manifests/exports and migrations as described in the index.

## Implementation

1. Implement reconnect, challenge/token refresh, inventory reporting, gateway compatibility, and remote dispatch over secure IPC. Preserve machine identity on reconnect.
2. Canonicalize and safely resolve allowed roots and requested paths; reject traversal, symlink escapes, unsafe ownership, and path-substitution races for browsing/new-session cwd.
3. Evaluate local agent, terminal, and approval policy for every operation; send trusted facts and generation context to sessiond for its own checks.
4. Support external-session operation facts without treating an outside-root cwd as a denial reason. Expose relay/inventory diagnostics and connection/recording-warning status.

## Acceptance criteria

- [x] The connector has one outbound gateway connection and does not open an inbound remote listener or own child agent/PTY processes.
- [x] Filesystem tests reject traversal, symlink escape, and race attempts while allowing configured roots.
- [x] Existing same-user control/approval outside roots is allowed when capability, access, lease, and other policy checks pass.
- [x] Restart, denied access, and disconnected IPC return explicit outcomes without terminating local sessions or retrying ambiguous mutations blindly.

## Verification

- Run connector integration tests with fake sessiond and gateway peers plus real temporary filesystem/symlink cases. Verify process ownership remains outside the connector.

Run the relevant formatting, lint, dependency-boundary, and supported-platform checks once code exists. Record real results and any unavailable environment; do not report planned or mocked external validation as completed.

## Boundaries and handoff

Control-generation activation is 017; native approval application is 018; service lifecycle is 025. No file upload/download in MVP.

**Deliver:** A policy-enforcing network-to-IPC bridge, inventory events, and scoped directory browsing.

## Current evidence and remaining integration

Host filesystem operations hold directory descriptors opened component by component with no symlink following; ownership/permissions, traversal, replacement, pagination, and changed-directory cursors are tested. Four bounded blocking workers serve listings. Sessiond now resolves cwd independently under its own pinned roots and retains that descriptor through native exec, so a remote bridge can carry the requested path without trusting the connector's earlier lookup. Actual PTY tests prove both cwd and relative executable stay bound to the selected inode after substitution. Public PTY start now preserves the native pinned-directory and original-authority checks through the protected recording preflight.

The actual connector integration test uses verified HTTPS/WSS, PostgreSQL, and Redis. It proves scoped browsing, refresh without advancing the host generation, host replacement/reconnect, and cancellation. One connector owns one outbound connection; it owns no native child. Gateway host dispatch carries authenticated actor/client/generation context. Native activation, control, original access and recording now compose through the independent IPC bridge.

Public PTY start passes the actual persisted-key SDK/WSS/gateway/activated connector/native process path in the owned synchronous recording harness. Same-key retry produces one child/start audit; changed parameters are NotSent; unready storage cannot launch. The connector automatically uploads real native capture and end metadata. A real shell moves outside its allowed start root: browsing that parent is denied while capture, take-control, resize, interrupt, input, release and confirmed owner termination still work. Policy separately permits eligible owner approvals without roots or a controller lease; real managed SDK/TUI approval application passes. This does not adopt an unrelated external session.
