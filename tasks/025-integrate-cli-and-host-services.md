# 025 — Integrate CLI commands and independent host services

- **Status:** Complete for the selected Linux standalone development scope; external release qualification remains separately tracked
- **Milestone:** M0 terminal demo / M1 CLI integration
- **Workstream:** CLI and service composition
- **Depends on:** [009](009-implement-verified-codex-adapter.md), [014](014-implement-host-dispatch-and-filesystem-policy.md), [016](016-implement-session-replay-and-snapshots.md), [017](017-implement-fenced-control-leases.md), [018](018-implement-durable-approvals.md), [020](020-implement-recording-spool.md), [021](021-implement-transcript-storage-and-access.md), [024](024-implement-tui-workflows.md)
- **Index:** [Task plan](README.md)

**Scope:** The user selected Linux development completion in [ADR 0007](../docs/adr/0007-standalone-preview.md). See [current progress](PROGRESS.md) and [external release gates](../docs/validation/external-release-gates.md). Original criteria that require unrun environments or waived corporate inspection remain unchecked; this status does not claim full release qualification.

## Goal and repository context

Compose the reusable crates inside the real QGenie executable and prove connector restart/pause cannot kill its separately managed session owner.

**Read first:** [PRD.md](../PRD.md) §§5.1, 5.4, SE-6, SE-11; [components.md](../components.md) §§5–6 and §4 contract 9; [subcrate-standard.md](../subcrate-standard.md) §§3–5.

## Owned scope

- `Host service.rs, status.rs, doctor.rs integration interfaces`
- `Existing CLI command/composition paths verified in 001 (outside this checkout)`
- `docs/integration/remote-commands.md and Linux/macOS service fixtures`

Owned paths define this task’s implementation boundary; current source and executed evidence are recorded below. Follow [the subcrate standard](../subcrate-standard.md), add only useful modules, and coordinate shared manifests/exports and migrations as described in the index.

## Implementation

1. Wire remote TUI and up/down/status/logs/doctor/machines/attach/run/revoke/unenroll/config commands plus hidden host/sessiond roles into the existing CLI.
2. Reuse its service installers to give connector and sessiond independent systemd user units/cgroups and launchd lifetimes; define update deferral, explicit session-owner upgrade, idle exit, logout, and reboot semantics.
3. Implement idempotent up, --no-service, JSON/noninteractive modes, safe stdin-based credential handoff where needed, pause via down --keep-sessions, and distinct explicit termination with grace when keep-sessions is absent.
4. Implement doctor checks for scoped auth, proxy/CA/DNS/WSS, service/IPC state, agent availability, relay guard, compatibility, and recording backlog without leaking secrets.
5. Document enrollment consent, recording/retention, service prerequisites, command exit codes, and unsupported conditions. Keep legacy telemetry daemon behavior separate.

## Acceptance criteria

- [x] The actual CLI launches the TUI and scriptable flows using shared crates from one compatible revision.
- [ ] Restarting/upgrading/stopping the connector with active PTY and Codex sessions preserves processes and local recording on Linux and macOS.
- [x] Pause blocks remote access, resume reconnects/uploads retained spool content, and revoke/unenroll do not terminate sessions.
- [x] Repeated up does not duplicate services or machine enrollment; status/doctor and machine-readable failures match actual state.

## Verification

- Run command smoke tests and real systemd --user/launchd drills with active PTY and pinned Codex. Kill the connector/service cgroup, pause/resume, and separately test explicit session termination and reboot/lost-state handling.

Run the relevant formatting, lint, dependency-boundary, and supported-platform checks once code exists. Record real results and any unavailable environment; do not report planned or mocked external validation as completed.

## Boundaries and handoff

The user authorized a new standalone CLI/TUI (ADR 0007). Complete this task in `apps/qgenie-remote`; the missing corporate CLI source is not a prerequisite. Real service-lifetime and platform proofs remain required. Windows/Electron service supervision is deferred.

**Deliver:** Working qgenie commands, service-lifetime proof, diagnosis output, and documented package linkage for 027/028.

The 2026-10-18 owned synchronous fixture additionally executes `host up --no-service` twice
against an existing owner and PTY, verifies reactivation and exactly the original session,
then pauses its exact scoped connector and resumes foreground capture/upload. The complete
native/CLI/TUI recording proof passes in 35.33s. Process mode remains explicitly independent
of a service manager; it does not qualify actual systemd/launchd lifetime or logout/upgrade.
