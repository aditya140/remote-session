# 025 — Integrate CLI commands and independent host services

- **Status:** Not started
- **Milestone:** M0 terminal demo / M1 CLI integration
- **Workstream:** CLI and service composition
- **Depends on:** [009](009-implement-verified-codex-adapter.md), [014](014-implement-host-dispatch-and-filesystem-policy.md), [016](016-implement-session-replay-and-snapshots.md), [017](017-implement-fenced-control-leases.md), [018](018-implement-durable-approvals.md), [020](020-implement-recording-spool.md), [021](021-implement-transcript-storage-and-access.md), [024](024-implement-tui-workflows.md)
- **Index:** [Task plan](README.md)

## Goal and repository context

Compose the reusable crates inside the real QGenie executable and prove connector restart/pause cannot kill its separately managed session owner.

**Read first:** [PRD.md](../PRD.md) §§5.1, 5.4, SE-6, SE-11; [components.md](../components.md) §§5–6 and §4 contract 9; [subcrate-standard.md](../subcrate-standard.md) §§3–5.

## Owned scope

- `Host service.rs, status.rs, doctor.rs integration interfaces`
- `Existing CLI command/composition paths verified in 001 (outside this checkout)`
- `docs/integration/remote-commands.md and Linux/macOS service fixtures`

Paths describe proposed implementation locations; they do not exist yet unless another task creates them. Follow [the subcrate standard](../subcrate-standard.md), add only useful modules, and coordinate shared manifests/exports and migrations as described in the index.

## Implementation

1. Wire remote TUI and up/down/status/logs/doctor/machines/attach/run/revoke/unenroll/config commands plus hidden host/sessiond roles into the existing CLI.
2. Reuse its service installers to give connector and sessiond independent systemd user units/cgroups and launchd lifetimes; define update deferral, explicit session-owner upgrade, idle exit, logout, and reboot semantics.
3. Implement idempotent up, --no-service, JSON/noninteractive modes, safe stdin-based credential handoff where needed, pause via down --keep-sessions, and distinct explicit termination with grace when keep-sessions is absent.
4. Implement doctor checks for scoped auth, proxy/CA/DNS/WSS, service/IPC state, agent availability, relay guard, compatibility, and recording backlog without leaking secrets.
5. Document enrollment consent, recording/retention, service prerequisites, command exit codes, and unsupported conditions. Keep legacy telemetry daemon behavior separate.

## Acceptance criteria

- [ ] The actual CLI launches the TUI and scriptable flows using shared crates from one compatible revision.
- [ ] Restarting/upgrading/stopping the connector with active PTY and Codex sessions preserves processes and local recording on Linux and macOS.
- [ ] Pause blocks remote access, resume reconnects/uploads retained spool content, and revoke/unenroll do not terminate sessions.
- [ ] Repeated up does not duplicate services or machine enrollment; status/doctor and machine-readable failures match actual state.

## Verification

- Run command smoke tests and real systemd --user/launchd drills with active PTY and pinned Codex. Kill the connector/service cgroup, pause/resume, and separately test explicit session termination and reboot/lost-state handling.

Run the relevant formatting, lint, dependency-boundary, and supported-platform checks once code exists. Record real results and any unavailable environment; do not report planned or mocked external validation as completed.

## Boundaries and handoff

The existing CLI repository is a real external dependency. A local harness helps development but cannot substitute for completing that integration. Windows/Electron service supervision is deferred.

**Deliver:** Working qgenie commands, service-lifetime proof, diagnosis output, and documented package linkage for 027/028.
