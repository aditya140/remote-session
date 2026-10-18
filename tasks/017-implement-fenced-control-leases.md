# 017 — Implement durable host and controller fencing

- **Status:** Complete for the selected Linux standalone development scope; external release qualification remains separately tracked
- **Milestone:** M1 control correctness
- **Workstream:** Control authority
- **Depends on:** [012](012-implement-routing-and-presence.md), [015](015-implement-session-runtime-and-start-journal.md)
- **Index:** [Task plan](README.md)

**Scope:** The user selected Linux development completion in [ADR 0007](../docs/adr/0007-standalone-preview.md). See [current progress](PROGRESS.md) and [external release gates](../docs/validation/external-release-gates.md). Original criteria that require unrun environments or waived corporate inspection remain unchecked; this status does not claim full release qualification.

## Goal and repository context

A replaced host connection or old controller must not retain mutation rights after a takeover or Redis failure.

**Read first:** [PRD.md](../PRD.md) SE-8, SE-11; [components.md](../components.md) §4 state authority and contract 6; [architecture.md](../architecture.md) §8.

## Owned scope

- `Gateway leases.rs and store/leases.rs plus generation migrations`
- `Sessiond control.rs and store/fences.rs; narrow routing/dispatch integration`

Owned paths define this task’s implementation boundary; current source and executed evidence are recorded below. Follow [the subcrate standard](../subcrate-standard.md), add only useful modules, and coordinate shared manifests/exports and migrations as described in the index.

## Implementation

1. Use the decision from 002 for durable conditional host/lease generations; keep Redis lease entries as caches.
2. Persist and activate the current generation in sessiond before acknowledging a new writable controller. Reject obsolete input, resize, interrupt, and other mutations requiring a fence.
3. Implement explicit takeover, renewal, expiry, disconnect, and generation recovery; audit transitions and avoid a database call per keystroke.
4. Keep owner approval rights independent of the lease. Specify explicit termination authorization separately, including external-session confirmation.

## Acceptance criteria

- [x] Only one activated controller generation can mutate a session, including after reconnect, failover, or replacement.
- [x] Delayed input from an old controller/host is rejected after takeover; an unacknowledged activation cannot create two controllers.
- [x] Redis cache loss cannot grant access, resurrect a stale lease, or reset generations.
- [x] Owner approval remains possible without control, while revoked principals and unsupported adapter actions are denied.

## Verification

- Run two-controller and two-host races, dropped activation acknowledgments, process restart, delayed input, expiry, and Redis failover tests against real Postgres/SQLite state.

Run the relevant formatting, lint, dependency-boundary, and supported-platform checks once code exists. Record real results and any unavailable environment; do not report planned or mocked external validation as completed.

## Boundaries and handoff

No approval winner selection, vendor-native control emulation, or automatic takeover without confirmation.

**Deliver:** Durable fencing and lease operations with race fixtures for SDK, TUI, and MVP validation.

## Current native evidence

SQLite schema 4 persists each immutable binding's highest host generation and connection identity. IPC 1.1 explicitly negotiates `native-authority-v1`; a legacy local handshake cannot invoke native authority operations. Existing IPC 1.0 local clients remain compatible.

Native nonces retain their original issue time. Activation commits the floor before acknowledgment, never extends duplicate grants, rejects replaced/lower generations, and forgets grants on restart. A SQLite stall that consumes the remaining budget still fences the old host but cannot acknowledge fresh authority. Native component tests cover SQLite/Unix IPC, replacement/restart, bounded nonce admission, full-u64 floors, actor/scope filtering, and unsupported remote mutations. The integrated host activation and durable controller proposal proofs are described below.

Access and local control additionally use suspend-aware elapsed time (`CLOCK_BOOTTIME` on Linux, `mach_continuous_time` on macOS). Input rechecks its owner guard before every nonblocking write. Deterministic tests expire a grant/lease while its awake-time deadline remains future. Actual Linux suspend/resume and macOS execution remain qualification work in 019/027. Current native policy revision is startup revision 1; dynamic policy changes and durable approval preflight belong to 014/018.

Gateway schema 7 and the connector now compose native host activation with directory/current-access proof, original nonce preservation through cross-node queues, and PostgreSQL proposal confirmation after native acknowledgment. Real sessiond plus two gateway compositions/HTTPS/Graph/PG/Redis tests pass renewal, replacement, scoped reads, revocation and local-child survival. Local admin binding registration is independently framed and cannot adopt local sessions. The integrated proof uses one process for the gateway compositions; two actual gateway-process outage proofs are separate.

Public PTY/managed control, native effects and owner approvals now compose and are verified below. Platform and extended fault qualification are external release gates.

## Controller component proof

Private IPC additionally requires `native-controller-v1`. Native query/activate/renew/release preserves durable full-u64 floors, explicit takeover, original 15-second deadlines, and in-process input sequence state. Expired/released/restarted generations cannot reactivate. Real Unix IPC confirms the floor independently after acknowledgment; real PTY/SQLite-lock tests verify expiry and uncertainty fence the previous holder. PostgreSQL schema 8 provides immutable acquisition requests, proposal confirmation after native ACK, renewal-stage CAS, idempotent reconciliation, release, and transactional audit. Actual application-role PostgreSQL race/rollback/replacement/overflow tests pass. These component proofs are supplemented by the public WSS/native input composition below.


## Public controller and effect composition

The SDK negotiates `session-control-v1`. Gateway-only `host-controller-v1` RPCs query the
native floor, reserve the PostgreSQL proposal, activate native state, and confirm that exact
proposal before returning a lease. Native and primary original deadlines only shrink during
confirmation or duplicate reconciliation. Release also reconciles a final native session.
`native-effects-v1` separately authorizes private PTY input, resize, interrupt and termination.
The scope effect barrier waits for an admitted OS call before replacement ACK and releases
before store I/O. Each input intent preserves its HMAC identity and original nonce/lease;
expiry in SQLite prevents native bytes. Local IPC cannot impersonate a remote lease.

Actual two-gateway composition tests pass competing controllers, explicit takeover, stale
input denial, unchanged-generation renewal, release/reacquisition and no-echo remote input.
Locking the primary session table still allows native input/output before unlock. Signed
route hints retain original nonces; primary freshness remains independently watched. SDK
lost-reply and canceled-input fixtures require explicit reacquisition and send no second
input. Owner termination without control requires confirmation and a committed request audit.
These use two gateway compositions in one OS process plus a real daemon, not the complete
multi-process restart/failover matrix. Managed controls and lease-independent approvals now also pass; suspend/platform and the
complete process-fault matrix remain.
