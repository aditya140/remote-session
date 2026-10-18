# 007 — Implement deterministic action-level policy

- **Status:** Complete for the selected Linux standalone development scope; external release qualification remains separately tracked
- **Milestone:** M0 foundation
- **Workstream:** Policy
- **Depends on:** [004](004-implement-protocol-and-conformance.md)
- **Index:** [Task plan](README.md)

**Scope:** The user selected Linux development completion in [ADR 0007](../docs/adr/0007-standalone-preview.md). See [current progress](PROGRESS.md) and [external release gates](../docs/validation/external-release-gates.md). Original criteria that require unrun environments or waived corporate inspection remain unchecked; this status does not claim full release qualification.

## Goal and repository context

Gateway and host checks need consistent action semantics, especially where owner approval rights differ from controller rights.

**Read first:** [PRD.md](../PRD.md) PO-1, SE-8–9, SE-12b; [architecture.md](../architecture.md) §8; [components.md](../components.md) §4 contract 4.

## Owned scope

- `crates/qgenie-remote-policy/`

Owned paths define this task’s implementation boundary; current source and executed evidence are recorded below. Follow [the subcrate standard](../subcrate-standard.md), add only useful modules, and coordinate shared manifests/exports and migrations as described in the index.

## Implementation

1. Define typed actions, caller-supplied identity/access facts, local restrictions, adapter capabilities, and allow/deny decisions with stable reasons.
2. Require ownership/current access for operations, controller authority for input/resize/interrupt, and owner plus host-policy approval for approval answers without requiring a lease or writable attachment.
3. Apply roots only to browsing, new-session cwd, and future file transfer. Preserve supported control of existing same-user sessions outside roots.
4. Compose gateway and host decisions without weakening the host's restrictions; represent separate explicit termination requirements and external-session confirmation.

## Acceptance criteria

- [x] A table-driven suite covers owner, other user, revoked principal, controller, observer-owner approver, and unsupported adapter actions.
- [x] Outside-root existing-session input and approvals are allowed when all other requirements pass; out-of-root browsing/start is denied.
- [x] Missing or stale access facts fail closed, and terminal/agent/approval restrictions return stable denial reasons.
- [x] The crate performs no filesystem, environment, network, token-validation, or database I/O.

## Verification

- Run policy tests over the operation matrix from 002, including denial composition and capability limits.

Run the relevant formatting, lint, dependency-boundary, and supported-platform checks once code exists. Record real results and any unavailable environment; do not report planned or mocked external validation as completed.

## Boundaries and handoff

Filesystem canonicalization belongs to 014; identity freshness and lease activation belong to gateway/sessiond tasks.

**Deliver:** Pure policy decisions and requirement types that both gateway and host can evaluate with their own trusted facts.

## Completion evidence

The pure policy crate implements typed actions and caller-supplied facts. Gateway and host callers must each authorize; a host denial cannot be weakened by a gateway allowance. Stable errors distinguish stale access, missing control, unsupported capability, and policy denial.

The policy tests passed for owner and other-user access, revocation, stale facts, foreign OS users, observer-owner approval without a lease, unsupported actions, outside-root existing input/approval, denied out-of-root browsing/start, and separate termination confirmation. External termination requires an additional external-session confirmation fact.

The crate reads no filesystem, environment, network, token, or database state. Filesystem enforcement, current identity authority, and durable lease activation remain with their owning tasks.
