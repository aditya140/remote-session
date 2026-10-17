# 017 — Implement durable host and controller fencing

- **Status:** Not started
- **Milestone:** M1 control correctness
- **Workstream:** Control authority
- **Depends on:** [012](012-implement-routing-and-presence.md), [015](015-implement-session-runtime-and-start-journal.md)
- **Index:** [Task plan](README.md)

## Goal and repository context

A replaced host connection or old controller must not retain mutation rights after a takeover or Redis failure.

**Read first:** [PRD.md](../PRD.md) SE-8, SE-11; [components.md](../components.md) §4 state authority and contract 6; [architecture.md](../architecture.md) §8.

## Owned scope

- `Gateway leases.rs and store/leases.rs plus generation migrations`
- `Sessiond control.rs and store/fences.rs; narrow routing/dispatch integration`

Paths describe proposed implementation locations; they do not exist yet unless another task creates them. Follow [the subcrate standard](../subcrate-standard.md), add only useful modules, and coordinate shared manifests/exports and migrations as described in the index.

## Implementation

1. Use the decision from 002 for durable conditional host/lease generations; keep Redis lease entries as caches.
2. Persist and activate the current generation in sessiond before acknowledging a new writable controller. Reject obsolete input, resize, interrupt, and other mutations requiring a fence.
3. Implement explicit takeover, renewal, expiry, disconnect, and generation recovery; audit transitions and avoid a database call per keystroke.
4. Keep owner approval rights independent of the lease. Specify explicit termination authorization separately, including external-session confirmation.

## Acceptance criteria

- [ ] Only one activated controller generation can mutate a session, including after reconnect, failover, or replacement.
- [ ] Delayed input from an old controller/host is rejected after takeover; an unacknowledged activation cannot create two controllers.
- [ ] Redis cache loss cannot grant access, resurrect a stale lease, or reset generations.
- [ ] Owner approval remains possible without control, while revoked principals and unsupported adapter actions are denied.

## Verification

- Run two-controller and two-host races, dropped activation acknowledgments, process restart, delayed input, expiry, and Redis failover tests against real Postgres/SQLite state.

Run the relevant formatting, lint, dependency-boundary, and supported-platform checks once code exists. Record real results and any unavailable environment; do not report planned or mocked external validation as completed.

## Boundaries and handoff

No approval winner selection, vendor-native control emulation, or automatic takeover without confirmation.

**Deliver:** Durable fencing and lease operations with race fixtures for SDK, TUI, and MVP validation.
