# 012 — Implement gateway routing, inventory, and push presence

- **Status:** Complete for the selected Linux standalone development scope; external release qualification remains separately tracked
- **Milestone:** M0 terminal slice
- **Workstream:** Gateway routing
- **Depends on:** [005](005-implement-websocket-transport.md), [010](010-implement-gateway-foundation-and-audit.md), [011](011-implement-gateway-identity-and-enrollment.md)
- **Index:** [Task plan](README.md)

**Scope:** The user selected Linux development completion in [ADR 0007](../docs/adr/0007-standalone-preview.md). See [current progress](PROGRESS.md) and [external release gates](../docs/validation/external-release-gates.md). Original criteria that require unrun environments or waived corporate inspection remain unchecked; this status does not claim full release qualification.

## Goal and repository context

Connect clients to hosts through any gateway replica while treating Redis as disposable routing and presence infrastructure.

**Read first:** [PRD.md](../PRD.md) PR-1–3; [architecture.md](../architecture.md) §§4.2–4.3, 6.2–6.3; [components.md](../components.md) §4.

## Owned scope

- `Gateway edge connection/limit modules and routing/{local,redis,presence}.rs`
- `Gateway session metadata projections and machine/user subscription handlers`

Owned paths define this task’s implementation boundary; current source and executed evidence are recorded below. Follow [the subcrate standard](../subcrate-standard.md), add only useful modules, and coordinate shared manifests/exports and migrations as described in the index.

## Implementation

1. Route locally when possible and through Redis sharded pub/sub otherwise; fan out session output per node and maintain user-wide inventory/session/presence subscriptions.
2. Authorize the session-to-machine-to-owner chain for every operation and validate host provenance for updates. Treat X-QGR-Affinity only as an untrusted routing hint.
3. Implement heartbeat TTL presence, initial snapshot plus push updates, replacement of older host sockets, and hooks for durable host generations activated in 017.
4. Handle Redis outages with explicit same-node/cross-node behavior, rebuildable subscriptions/keys, bounded queues, and resync signals for at-most-once delivery.

## Acceptance criteria

- [x] Two gateway processes route an authenticated client to its host regardless of affinity; another user's IDs never provide access.
- [x] Online presence arrives within 5 seconds and unclean disconnect is reflected within 30 seconds; inventory and session counts update by push.
- [x] Slow observers cannot block controllers, and lost routed frames trigger recovery rather than implied delivery.
- [x] Redis loss preserves durable metadata and local processes; same-node access is still bounded by authoritative access freshness from 019.

## Verification

- Run two-node integration tests with Redis disconnect/failover, forged affinity and IDs, host replacement, slow observers, and snapshot/subscription races.

Run the relevant formatting, lint, dependency-boundary, and supported-platform checks once code exists. Record real results and any unavailable environment; do not report planned or mocked external validation as completed.

## Boundaries and handoff

No session process ownership, durable output in Redis, approval authority, or Redis-only controller leases.

**Deliver:** Authenticated routing/subscription APIs and presence events. Host-generation enforcement is completed by 017 before the integrated MVP is accepted.

## Verified implementation

Two actual Linux gateway processes, verified HTTPS JWKS/WSS, PostgreSQL 17, and Redis 8 pass the routing fixture. Proofs cover cross-node read/reply/output, foreign IDs and token families, maximum JSON frames, bounded slow observers, inventory/counts/presence, host replacement, refresh, pub/sub interruption, a full finite Redis pause, same-node reads during outage, rebuild/recovery, access disable, and full-u64 exhaustion. Host messages in this test are protocol fixtures. Separate synchronous and development-stack proofs now exercise the real sessiond/connector, native control, directory freshness and process survival. Unsupported/unactivated mutations remain explicit not-sent. See [routing limits, recovery, and configuration](../docs/integration/gateway-routing.md).
