# 007 — Implement deterministic action-level policy

- **Status:** Not started
- **Milestone:** M0 foundation
- **Workstream:** Policy
- **Depends on:** [004](004-implement-protocol-and-conformance.md)
- **Index:** [Task plan](README.md)

## Goal and repository context

Gateway and host checks need consistent action semantics, especially where owner approval rights differ from controller rights.

**Read first:** [PRD.md](../PRD.md) PO-1, SE-8–9, SE-12b; [architecture.md](../architecture.md) §8; [components.md](../components.md) §4 contract 4.

## Owned scope

- `crates/qgenie-remote-policy/`

Paths describe proposed implementation locations; they do not exist yet unless another task creates them. Follow [the subcrate standard](../subcrate-standard.md), add only useful modules, and coordinate shared manifests/exports and migrations as described in the index.

## Implementation

1. Define typed actions, caller-supplied identity/access facts, local restrictions, adapter capabilities, and allow/deny decisions with stable reasons.
2. Require ownership/current access for operations, controller authority for input/resize/interrupt, and owner plus host-policy approval for approval answers without requiring a lease or writable attachment.
3. Apply roots only to browsing, new-session cwd, and future file transfer. Preserve supported control of existing same-user sessions outside roots.
4. Compose gateway and host decisions without weakening the host's restrictions; represent separate explicit termination requirements and external-session confirmation.

## Acceptance criteria

- [ ] A table-driven suite covers owner, other user, revoked principal, controller, observer-owner approver, and unsupported adapter actions.
- [ ] Outside-root existing-session input and approvals are allowed when all other requirements pass; out-of-root browsing/start is denied.
- [ ] Missing or stale access facts fail closed, and terminal/agent/approval restrictions return stable denial reasons.
- [ ] The crate performs no filesystem, environment, network, token-validation, or database I/O.

## Verification

- Run policy tests over the operation matrix from 002, including denial composition and capability limits.

Run the relevant formatting, lint, dependency-boundary, and supported-platform checks once code exists. Record real results and any unavailable environment; do not report planned or mocked external validation as completed.

## Boundaries and handoff

Filesystem canonicalization belongs to 014; identity freshness and lease activation belong to gateway/sessiond tasks.

**Deliver:** Pure policy decisions and requirement types that both gateway and host can evaluate with their own trusted facts.
