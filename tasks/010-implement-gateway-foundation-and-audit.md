# 010 — Build the gateway application, durable store, and audit API

- **Status:** Not started
- **Milestone:** M0 gateway foundation
- **Workstream:** Gateway foundation
- **Depends on:** [004](004-implement-protocol-and-conformance.md), [007](007-implement-policy-engine.md)
- **Index:** [Task plan](README.md)

## Goal and repository context

Gateway domain tasks need a shared application composition, PostgreSQL migration convention, and durable audit facility without relying on Redis for authority.

**Read first:** [components.md](../components.md) §§2, 4–5; [architecture.md](../architecture.md) §§4.2, 5, 9.2; [PRD.md](../PRD.md) PO-3.

## Owned scope

- `crates/qgenie-remote-gateway/src/{main,lib,app,config,state,shutdown,error}.rs`
- `Gateway store foundations, audit.rs, api/audit.rs, base migrations, and health endpoints`

Paths describe proposed implementation locations; they do not exist yet unless another task creates them. Follow [the subcrate standard](../subcrate-standard.md), add only useful modules, and coordinate shared manifests/exports and migrations as described in the index.

## Implementation

1. Create a thin serve/migrate/jobs command composition, typed configuration, secret-safe diagnostics, Postgres pools, migration execution, and schema compatibility checks.
2. Introduce only needed base identity/session metadata and append-only audit storage. Assign later migrations to their domain tasks; keep SQLite schemas separate.
3. Expose a transaction-aware audit writer so revocation, lease, and approval commits can include their audit event atomically. Restrict audit reads to authenticated platform admins.
4. Implement liveness/readiness/drain hooks and caller-configured structured telemetry. Make health states honor task 002's authority and degraded-service contract.

## Acceptance criteria

- [ ] Fresh migration and supported-version upgrade work; incompatible schemas cause a clear readiness failure.
- [ ] Domain changes and their audit records can commit or roll back together, with application roles unable to rewrite audit history.
- [ ] Configuration supports service-readable secret files and does not leak credentials in errors or logs.
- [ ] Shutdown marks the service unready before draining; no unauthenticated domain or audit API is exposed.

## Verification

- Run PostgreSQL integration tests for migrations, transaction rollback, role privileges, audit access, configuration parsing, and readiness transitions.

Run the relevant formatting, lint, dependency-boundary, and supported-platform checks once code exists. Record real results and any unavailable environment; do not report planned or mocked external validation as completed.

## Boundaries and handoff

Enrollment/authentication, routing, transcript content, and destructive jobs remain with their task owners; no all-purpose repository framework.

**Deliver:** Gateway composition, migration rules, authenticated-principal interface, audit transaction hooks, and minimal running health endpoints.
