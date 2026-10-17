# 014 — Implement the host connector and local filesystem enforcement

- **Status:** Not started
- **Milestone:** M0 terminal slice
- **Workstream:** Host connector
- **Depends on:** [006](006-implement-secure-local-ipc.md), [007](007-implement-policy-engine.md), [013](013-implement-host-enrollment-and-credentials.md)
- **Index:** [Task plan](README.md)

## Goal and repository context

Keep one outbound WSS connection, enforce local restrictions, and forward authorized operations to sessiond without owning the processes.

**Read first:** [PRD.md](../PRD.md) PR-4, PO-1, SE-5, SE-12b; [components.md](../components.md) §§4–6; [architecture.md](../architecture.md) §§4.1, 8.

## Owned scope

- `Host {host,connector,dispatch,ipc,filesystem,inventory}.rs`
- `Host policy/{paths,agents}.rs and policy/filesystem tests`

Paths describe proposed implementation locations; they do not exist yet unless another task creates them. Follow [the subcrate standard](../subcrate-standard.md), add only useful modules, and coordinate shared manifests/exports and migrations as described in the index.

## Implementation

1. Implement reconnect, challenge/token refresh, inventory reporting, gateway compatibility, and remote dispatch over secure IPC. Preserve machine identity on reconnect.
2. Canonicalize and safely resolve allowed roots and requested paths; reject traversal, symlink escapes, unsafe ownership, and path-substitution races for browsing/new-session cwd.
3. Evaluate local agent, terminal, and approval policy for every operation; send trusted facts and generation context to sessiond for its own checks.
4. Support external-session operation facts without treating an outside-root cwd as a denial reason. Expose relay/inventory diagnostics and connection/recording-warning status.

## Acceptance criteria

- [ ] The connector has one outbound gateway connection and does not open an inbound remote listener or own child agent/PTY processes.
- [ ] Filesystem tests reject traversal, symlink escape, and race attempts while allowing configured roots.
- [ ] Existing same-user control/approval outside roots is allowed when capability, access, lease, and other policy checks pass.
- [ ] Restart, denied access, and disconnected IPC return explicit outcomes without terminating local sessions or retrying ambiguous mutations blindly.

## Verification

- Run connector integration tests with fake sessiond and gateway peers plus real temporary filesystem/symlink cases. Verify process ownership remains outside the connector.

Run the relevant formatting, lint, dependency-boundary, and supported-platform checks once code exists. Record real results and any unavailable environment; do not report planned or mocked external validation as completed.

## Boundaries and handoff

Control-generation activation is 017; native approval application is 018; service lifecycle is 025. No file upload/download in MVP.

**Deliver:** A policy-enforcing network-to-IPC bridge, inventory events, and scoped directory browsing.
