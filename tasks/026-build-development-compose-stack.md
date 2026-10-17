# 026 — Build a reproducible four-gateway development deployment

- **Status:** Not started
- **Milestone:** M0 deployment
- **Workstream:** Deployment
- **Depends on:** [012](012-implement-routing-and-presence.md)
- **Index:** [Task plan](README.md)

## Goal and repository context

Run the first end-to-end slice on one dev server with realistic gateway routing and rolling restarts.

**Read first:** [architecture.md](../architecture.md) §§9.1–9.4; [components.md](../components.md) §8 deployment contract; [PRD.md](../PRD.md) §9 M0.

## Owned scope

- `deploy/images/, deploy/compose/ and deployment scripts`
- `docs/runbooks/development-deployment.md; CI integration-job wiring`

Paths describe proposed implementation locations; they do not exist yet unless another task creates them. Follow [the subcrate standard](../subcrate-standard.md), add only useful modules, and coordinate shared manifests/exports and migrations as described in the index.

## Implementation

1. Build the non-root gateway image with CA trust and serve/migrate/jobs entry points; pin images and use service-readable secret files without committing real secrets.
2. Run four gateway instances, HAProxy, Redis, Postgres, PgBouncer, and OTel. Resolve host-network port collisions through explicit per-instance ports or the recorded alternative from 002.
3. Configure affinity hashing, private administrative/data ports, TLS/WSS timeouts, schema-compatible readiness, drain/stop grace, resource limits, and log rotation.
4. Provide documented startup, migration, shutdown, backup/restore smoke, and rolling-restart commands; keep images independent of Compose-specific APIs.
5. Add CI startup checks and a fixture-identity path isolated to tests; real enrollment must still use the configured remote identity provider.

## Acceptance criteria

- [ ] One documented command sequence starts all four healthy gateways without port conflicts or unreadable secrets.
- [ ] The stack supports cross-node routing and a rolling gateway restart while reconnecting peers retain host-owned sessions.
- [ ] Only the intended edge is reachable externally; database, metrics, and gateway administration stay private.
- [ ] Invalid configuration/schema/secrets fail clearly; Redis/Postgres outages produce the agreed ready/degraded behavior.

## Verification

- Validate Compose configuration, build/run the stack, exercise health and TLS/WSS paths, rotate one gateway, and perform a test database backup/restore. Record any host/certificate prerequisites.

Run the relevant formatting, lint, dependency-boundary, and supported-platform checks once code exists. Record real results and any unavailable environment; do not report planned or mocked external validation as completed.

## Boundaries and handoff

This is the single-server dev deployment. Three-server failover topology, production image publishing, automated certificate renewal, Kubernetes, and public-internet exposure are not included.

**Deliver:** A reproducible integration environment and scripts used by 027/028; later migrations are discovered through the gateway's normal migrate command.
