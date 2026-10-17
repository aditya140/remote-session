# 011 — Implement gateway user and machine authentication

- **Status:** Not started
- **Milestone:** M0 enrollment / M1 identity
- **Workstream:** Gateway identity
- **Depends on:** [001](001-inspect-existing-qgenie-integration.md), [005](005-implement-websocket-transport.md), [010](010-implement-gateway-foundation-and-audit.md)
- **Index:** [Task plan](README.md)

## Goal and repository context

Enroll a machine under the right owner and authenticate user clients separately from machine connectors.

**Read first:** [PRD.md](../PRD.md) ID-1–5, ID-7–8; [architecture.md](../architecture.md) §§6.1–6.2, 8; [components.md](../components.md) §§4, 6.

## Owned scope

- `Gateway identity/{jwt,machines,challenge}.rs and api/machines.rs`
- `Gateway edge upgrade authentication and identity-specific migrations`

Paths describe proposed implementation locations; they do not exist yet unless another task creates them. Follow [the subcrate standard](../subcrate-standard.md), add only useful modules, and coordinate shared manifests/exports and migrations as described in the index.

## Implementation

1. Validate issuer, signature/JWKS, expiry, dedicated remote audience/scope, and admin role where needed. Reject inference tokens and machine tokens used as user credentials.
2. Implement owner-scoped machine enroll/list/rename/label APIs using persisted machine identity and proof of possession. Specify key-loss, rotation, cloned identity, and hostname-change behavior from 002.
3. Implement single-use nonce challenge verification, canonical signed bytes, replay/expiry protection, and machine tokens lasting no more than 15 minutes without user refresh tokens.
4. Authenticate WSS host/client upgrades and check durable access state before admitting a connection. Supply current-principal interfaces that 019 extends with bounded reconciliation.

## Acceptance criteria

- [ ] Repeated enrollment with the same valid identity returns the same machine; equal hostnames alone cannot merge unrelated machines.
- [ ] Wrong audience/issuer/scope, expired tokens, revoked keys, foreign ownership, and replayed challenges are rejected.
- [ ] Machine tokens cannot perform client/admin operations, and enrollment never stores user refresh tokens for host use.
- [ ] REST and WSS authentication use the same ownership/current-access semantics and stable public errors.

## Verification

- Run JWT/JWKS and challenge fixtures plus PostgreSQL API tests for idempotent enrollment, rename, key rotation/recovery, and cross-user denials. Validate the selected remote tenant configuration when available.

Run the relevant formatting, lint, dependency-boundary, and supported-platform checks once code exists. Record real results and any unavailable environment; do not report planned or mocked external validation as completed.

## Boundaries and handoff

User OAuth UI belongs to CLI integration/013. Revocation propagation and directory freshness are 019; identity APIs must not claim that work is already complete.

**Deliver:** Authenticated enrollment endpoints, principal contexts, challenge/token flow, and machine metadata operations for host/client consumers.
