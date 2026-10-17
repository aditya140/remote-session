# 015 — Implement sessiond and durable session-start reconciliation

- **Status:** Not started
- **Milestone:** M0 terminal slice
- **Workstream:** Session runtime
- **Depends on:** [006](006-implement-secure-local-ipc.md), [007](007-implement-policy-engine.md), [008](008-implement-adapter-contract-and-pty.md)
- **Index:** [Task plan](README.md)

## Goal and repository context

Sessiond, rather than the connector or a viewing client, must own processes and know whether an idempotent start actually launched.

**Read first:** [PRD.md](../PRD.md) SE-6, SE-10–11; [components.md](../components.md) §4 contracts 3, 5, 9; [subcrate-standard.md](../subcrate-standard.md) §§4–5.

## Owned scope

- `crates/qgenie-remote-sessiond/src/{runtime,ipc,registry,lifecycle,start,supervisor,error}.rs`
- `Sessiond store/{sessions,starts}.rs and base SQLite migrations`

Paths describe proposed implementation locations; they do not exist yet unless another task creates them. Follow [the subcrate standard](../subcrate-standard.md), add only useful modules, and coordinate shared manifests/exports and migrations as described in the index.

## Implementation

1. Implement private SQLite state, a session registry, bounded event distribution, and supervised adapter handles under a caller-owned runtime.
2. Persist scoped idempotency keys and request fingerprints before launch; model received, durable acceptance, native launch, running, failed, and outcome-unknown stages.
3. Correlate native IDs and recover interrupted starts. Reusing a key with changed parameters is a conflict; uncertainty never becomes a fresh uncorrelated launch.
4. Separate detach, interrupt, explicit termination, and historical resume. Reconcile actual processes/native sessions after restart and report lost where recovery cannot be proved.
5. Expose hooks for event sequencing, control fences, approvals, replay, and recording owned by later tasks without embedding their state machines here.

## Acceptance criteria

- [ ] Concurrent duplicate starts create at most one known process/session, or report explicit uncertainty where native launch cannot be reconciled.
- [ ] Crash injection before/after journal commit, native launch, and acknowledgment yields a documented recoverable result.
- [ ] Client/connector loss does not end sessions; dropping a committed start future does not implicitly kill its child.
- [ ] SQLite files are private and durable state survives restart; a reboot does not falsely report dead sessions as running.

## Verification

- Run real PTY plus deterministic fake-adapter tests across each crash point, same-key conflicts, disconnect, shutdown, and reboot/lost-state reconciliation.

Run the relevant formatting, lint, dependency-boundary, and supported-platform checks once code exists. Record real results and any unavailable environment; do not report planned or mocked external validation as completed.

## Boundaries and handoff

No user-service installation, gateway metadata authority, replay persistence, recording spool, lease state machine, or approval journal in this task.

**Deliver:** Session lifecycle API, durable start lookup/reconciliation, registry, and event hooks. Tasks 016–020 own distinct extensions.
