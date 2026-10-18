# 026 — Build a reproducible four-gateway development deployment

- **Status:** Complete for the selected Linux standalone development scope; external release qualification remains separately tracked
- **Milestone:** M0 deployment
- **Workstream:** Deployment
- **Depends on:** [012](012-implement-routing-and-presence.md)
- **Index:** [Task plan](README.md)

**Scope:** The user selected Linux development completion in [ADR 0007](../docs/adr/0007-standalone-preview.md). See [current progress](PROGRESS.md) and [external release gates](../docs/validation/external-release-gates.md). Original criteria that require unrun environments or waived corporate inspection remain unchecked; this status does not claim full release qualification.

## Goal and repository context

Run the first end-to-end slice on one dev server with realistic gateway routing and rolling restarts.

**Read first:** [architecture.md](../architecture.md) §§9.1–9.4; [components.md](../components.md) §8 deployment contract; [PRD.md](../PRD.md) §9 M0.

## Owned scope

- `deploy/images/, deploy/compose/ and deployment scripts`
- `docs/runbooks/development-deployment.md; CI integration-job wiring`

Owned paths define this task’s implementation boundary; current source and executed evidence are recorded below. Follow [the subcrate standard](../subcrate-standard.md), add only useful modules, and coordinate shared manifests/exports and migrations as described in the index.

## Implementation

1. Build the non-root gateway image with CA trust and serve/migrate/jobs entry points; pin images and use service-readable secret files without committing real secrets.
2. Run four gateway instances, HAProxy, Redis, Postgres, PgBouncer, and OTel. Resolve host-network port collisions through explicit per-instance ports or the recorded alternative from 002.
3. Configure affinity hashing, private administrative/data ports, TLS/WSS timeouts, schema-compatible readiness, drain/stop grace, resource limits, and log rotation.
4. Provide documented startup, migration, shutdown, backup/restore smoke, and rolling-restart commands; keep images independent of Compose-specific APIs.
5. Add CI startup checks and a fixture-identity path isolated to tests; real enrollment must still use the configured remote identity provider.

## Acceptance criteria

- [x] One documented command sequence starts all four healthy gateways without port conflicts or unreadable secrets.
- [x] The stack supports cross-node routing and a rolling gateway restart while reconnecting peers retain host-owned sessions.
- [x] Only the intended edge is reachable externally; database, metrics, and gateway administration stay private.
- [x] Invalid configuration/schema/secrets fail clearly; Redis/Postgres outages produce the agreed ready/degraded behavior.

## Verification

- Validate Compose configuration, build/run the stack, exercise health and TLS/WSS paths, rotate one gateway, and perform a test database backup/restore. Record any host/certificate prerequisites.

Run the relevant formatting, lint, dependency-boundary, and supported-platform checks once code exists. Record real results and any unavailable environment; do not report planned or mocked external validation as completed.

## Boundaries and handoff

This is the single-server dev deployment. Three-server failover topology, production image publishing, automated certificate renewal, Kubernetes, and public-internet exposure are not included.

**Deliver:** A reproducible integration environment and scripts used by 027/028; later migrations are discovered through the gateway's normal migrate command.

## Executed evidence (2026-10-18)

The pinned musl/distroless images built and ran as UID 10001. The documented initialization,
migration and startup sequence produced four healthy gateways, a real synchronous standby,
TLS/password Redis, transaction-pooled recording connections, isolated HTTPS fixture identity,
HAProxy, lifecycle jobs and OTel. Actual standalone CLI/native enrollment, persisted-key
start, controlled input, cross-node dispatch, four sequential gateway restarts, native session
survival, original-key lookup, termination and verified redacted history passed twice.

The isolated logical backup/restore checked matching schema 15 and recovery quarantine.
`fault-smoke` verified the published port set, blocked private edge paths, explicit invalid
configuration failure, Redis degraded readiness and resubscription, and primary-loss
liveness/unreadiness/recovery. Existing real gateway-process tests separately cover incompatible
schema and private-secret failures. Startup/connected/restore/fault checks are wired into CI;
the hosted CI job itself has not been observed here. Strict workspace/all-feature Clippy passes.

See [the runbook](../docs/runbooks/development-deployment.md) and ADR 0008 for private bridge
namespaces, generated secret ownership, verified proxy/CA behavior and exact commands. This
completion covers task 026's development scope; production identity, off-server durability,
certificate renewal, service-manager platforms and beta-scale qualification remain separate.

## Verified image rollout

The rolling command now applies additive migrations and recreates one gateway at a time;
a plain restart retained the old image after a rebuild. Each node passes its actual
readiness probe and its running image must match the image selected before rollout. A
failed check stops the rollout; the lifecycle scheduler updates afterward.

The rebuilt gateway image is **19,769,055 bytes**, with image ID
`sha256:f0c896346a5081c714d364083fbf5859cc82d8966d13fef28b47851bf44645b8`.
All four non-root gateways are healthy on that image. The actual CLI/native smoke passes
through recreation with the same native PTY/start identity, new input after reconnect,
cross-node dispatch, confirmed termination and redacted protected history.
The [sanitized rollout artifact](../docs/validation/development-roll-2026-10-18.json) records
the running images and qualification boundary. Managed-agent deployment is not claimed by
this PTY smoke; its separate composed native/SDK/TUI evidence remains in task 027.

The private pinned Prometheus evaluator and OTel operator exporter also pass
`monitor-smoke`: all 13 alert rules, four gateway scrapes, original-elapsed directory metrics
and aggregate ingestion. The host probe is an explicit operator process; it receives no
notification destination. See [alert evidence](../docs/validation/operational-alerts-2026-10-18.json).
