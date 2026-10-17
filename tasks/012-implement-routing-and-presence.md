# 012 — Implement gateway routing, inventory, and push presence

- **Status:** Not started
- **Milestone:** M0 terminal slice
- **Workstream:** Gateway routing
- **Depends on:** [005](005-implement-websocket-transport.md), [010](010-implement-gateway-foundation-and-audit.md), [011](011-implement-gateway-identity-and-enrollment.md)
- **Index:** [Task plan](README.md)

## Goal and repository context

Connect clients to hosts through any gateway replica while treating Redis as disposable routing and presence infrastructure.

**Read first:** [PRD.md](../PRD.md) PR-1–3; [architecture.md](../architecture.md) §§4.2–4.3, 6.2–6.3; [components.md](../components.md) §4.

## Owned scope

- `Gateway edge connection/limit modules and routing/{local,redis,presence}.rs`
- `Gateway session metadata projections and machine/user subscription handlers`

Paths describe proposed implementation locations; they do not exist yet unless another task creates them. Follow [the subcrate standard](../subcrate-standard.md), add only useful modules, and coordinate shared manifests/exports and migrations as described in the index.

## Implementation

1. Route locally when possible and through Redis sharded pub/sub otherwise; fan out session output per node and maintain user-wide inventory/session/presence subscriptions.
2. Authorize the session-to-machine-to-owner chain for every operation and validate host provenance for updates. Treat X-QGR-Affinity only as an untrusted routing hint.
3. Implement heartbeat TTL presence, initial snapshot plus push updates, replacement of older host sockets, and hooks for durable host generations activated in 017.
4. Handle Redis outages with explicit same-node/cross-node behavior, rebuildable subscriptions/keys, bounded queues, and resync signals for at-most-once delivery.

## Acceptance criteria

- [ ] Two gateway processes route an authenticated client to its host regardless of affinity; another user's IDs never provide access.
- [ ] Online presence arrives within 5 seconds and unclean disconnect is reflected within 30 seconds; inventory and session counts update by push.
- [ ] Slow observers cannot block controllers, and lost routed frames trigger recovery rather than implied delivery.
- [ ] Redis loss preserves durable metadata and local processes; same-node access is still bounded by authoritative access freshness from 019.

## Verification

- Run two-node integration tests with Redis disconnect/failover, forged affinity and IDs, host replacement, slow observers, and snapshot/subscription races.

Run the relevant formatting, lint, dependency-boundary, and supported-platform checks once code exists. Record real results and any unavailable environment; do not report planned or mocked external validation as completed.

## Boundaries and handoff

No session process ownership, durable output in Redis, approval authority, or Redis-only controller leases.

**Deliver:** Authenticated routing/subscription APIs and presence events. Host-generation enforcement is completed by 017 before the integrated MVP is accepted.
