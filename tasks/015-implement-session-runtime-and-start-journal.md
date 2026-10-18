# 015 — Implement sessiond and durable session-start reconciliation

- **Status:** Complete for the selected Linux standalone development scope; external release qualification remains separately tracked
- **Milestone:** M0 terminal slice
- **Workstream:** Session runtime
- **Depends on:** [006](006-implement-secure-local-ipc.md), [007](007-implement-policy-engine.md), [008](008-implement-adapter-contract-and-pty.md)
- **Index:** [Task plan](README.md)

**Scope:** The user selected Linux development completion in [ADR 0007](../docs/adr/0007-standalone-preview.md). See [current progress](PROGRESS.md) and [external release gates](../docs/validation/external-release-gates.md). Original criteria that require unrun environments or waived corporate inspection remain unchecked; this status does not claim full release qualification.

## Goal and repository context

Sessiond, rather than the connector or a viewing client, must own processes and know whether an idempotent start actually launched.

**Read first:** [PRD.md](../PRD.md) SE-6, SE-10–11; [components.md](../components.md) §4 contracts 3, 5, 9; [subcrate-standard.md](../subcrate-standard.md) §§4–5.

## Owned scope

- `crates/qgenie-remote-sessiond/src/{runtime,ipc,registry,lifecycle,start,supervisor,error}.rs`
- `Sessiond store/{sessions,starts}.rs and base SQLite migrations`

Owned paths define this task’s implementation boundary; current source and executed evidence are recorded below. Follow [the subcrate standard](../subcrate-standard.md), add only useful modules, and coordinate shared manifests/exports and migrations as described in the index.

## Implementation

1. Implement private SQLite state, a session registry, bounded event distribution, and supervised adapter handles under a caller-owned runtime.
2. Persist scoped idempotency keys and request fingerprints before launch; model received, durable acceptance, native launch, running, failed, and outcome-unknown stages.
3. Correlate native IDs and recover interrupted starts. Reusing a key with changed parameters is a conflict; uncertainty never becomes a fresh uncorrelated launch.
4. Separate detach, interrupt, explicit termination, and historical resume. Reconcile actual processes/native sessions after restart and report lost where recovery cannot be proved.
5. Expose hooks for event sequencing, control fences, approvals, replay, and recording owned by later tasks without embedding their state machines here.

## Acceptance criteria

- [x] Concurrent duplicate starts create at most one known process/session, or report explicit uncertainty where native launch cannot be reconciled.
- [x] Crash injection before/after journal commit, native launch, and acknowledgment yields a documented recoverable result.
- [x] Client/connector loss does not end sessions; dropping a committed start future does not implicitly kill its child.
- [x] SQLite files are private and durable state survives restart; a reboot does not falsely report dead sessions as running.

## Verification

- Run real PTY plus deterministic fake-adapter tests across each crash point, same-key conflicts, disconnect, shutdown, and reboot/lost-state reconciliation.

Run the relevant formatting, lint, dependency-boundary, and supported-platform checks once code exists. Record real results and any unavailable environment; do not report planned or mocked external validation as completed.

## Boundaries and handoff

No user-service installation, gateway metadata authority, replay persistence, recording spool, lease state machine, or approval journal in this task.

**Deliver:** Session lifecycle API, durable start lookup/reconciliation, registry, and event hooks. Tasks 016–020 own distinct extensions.

## Current evidence and remaining integration

Real Linux PTY tests prove sixteen concurrent same-key starts have one native effect. The output supervisor is reserved before durable acceptance and performs the spawn itself; losing the caller or reply leaves the committed child supervised. A private exclusive runtime lock remains held by the supervisor until the child is drained/reaped. A second runtime cannot concurrently recover the same journal.

Six actual helper-process exits cover before journal, accepted, launching, native launch, running commit, and before acknowledgment. Reopening preserves accepted keys, returns outcome-unknown/lost where native correlation cannot be established, and never launches a second child for the same key. These are test-only fault hooks. Input intent is durable before native delivery, and a lease expiring while SQLite is stalled is rechecked before any bytes are written. Persisted input deduplication uses an ephemeral keyed digest, not plaintext or a dictionary-testable keystroke hash.

SQLite schema 3 now persists immutable origin/machine/owner bindings and per-session scope. Same-key starts in different scopes remain separate; local keys are unchanged. Actual SQLite upgrade/restart and PTY marker tests prove scope/key retention, rejection of owner reassociation, and no implicit adoption of local history. Native launch now holds a descriptor-scoped cwd through exec, including relative executable resolution; path substitution and pre-exec permission changes are tested. Primary-child reaping has a bounded drain independent of inherited slave descriptors.

Native remote activation/freshness, original-authority mutations and managed Codex lifecycle now compose through the public SDK and two gateways. Platform execution remains in its owning tasks. Persistent PTY checkpoint/cutover evidence is recorded in 016.

Public PTY start now passes the actual persisted-key SDK/WSS/gateway/activated connector/native process path in the owned synchronous recording harness. Same-key retry produces one child/start audit; changed parameters are NotSent; unready storage cannot launch. The connector automatically uploads real native capture and end metadata. Remote controls and managed runtime now compose; see PROGRESS.md for current evidence.

Managed Codex now shares the PTY start lock and scoped SQLite key namespace. A configured
Linux host reserves its process/event owner and independent bounded replay worker before
acceptance, pins cwd through the native descriptor, commits the native thread identity before
initial prompt delivery and retains ownership across caller/connector loss. Actual pinned
Codex tests cover eight duplicate starts, cross-kind conflicts, input deduplication/controller
replacement, structured redacted capture, explicit termination and recovered thread/end
metadata. An uncertain thread/start is never retried as a fresh launch. The actual Linux owner-SIGKILL fixture observes native exit via a pidfd, reopens the
original journal as lost/unknown and verifies no duplicate launch. Managed children bind
parent-thread death using PR_SET_PDEATHSIG. Public structured SDK/TUI creation, prompt and
termination now pass. Confirmed-ended historical resume creates a new native instance using
the canonical private home and a newly selected descriptor-scoped cwd. Lost/unconfirmed
history cannot resume. Six actual managed crash boundaries now also pass: before journal, accepted, launching, native launched, identity/state committed and before caller ACK. Accepted keys reconcile to lost/unknown without another native launch; before-journal lookup remains unknown and launches nothing. The native process is observed through a parent-validated pidfd before each post-launch crash. Runtime IPC reconnect remains unsupported: protocol loss preserves explicit uncertainty and owner termination; platform qualification remains separate.

The three explicit managed proofs (six crash boundaries, owner SIGKILL and full caller-loss/deduplication/resume) pass together against actual pinned Codex 0.159.2. Crash hooks exist only in test builds and are controlled by an owned helper barrier; production builds include none.
