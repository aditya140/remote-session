# 010 — Build the gateway application, durable store, and audit API

- **Status:** Complete for the selected Linux standalone development scope; external release qualification remains separately tracked
- **Milestone:** M0 gateway foundation
- **Workstream:** Gateway foundation
- **Depends on:** [004](004-implement-protocol-and-conformance.md), [007](007-implement-policy-engine.md)
- **Index:** [Task plan](README.md)

**Scope:** The user selected Linux development completion in [ADR 0007](../docs/adr/0007-standalone-preview.md). See [current progress](PROGRESS.md) and [external release gates](../docs/validation/external-release-gates.md). Original criteria that require unrun environments or waived corporate inspection remain unchecked; this status does not claim full release qualification.

## Goal and repository context

Gateway domain tasks need a shared application composition, PostgreSQL migration convention, and durable audit facility without relying on Redis for authority.

**Read first:** [components.md](../components.md) §§2, 4–5; [architecture.md](../architecture.md) §§4.2, 5, 9.2; [PRD.md](../PRD.md) PO-3.

## Owned scope

- `crates/qgenie-remote-gateway/src/{main,lib,app,config,state,shutdown,error}.rs`
- `Gateway store foundations, audit.rs, api/audit.rs, base migrations, and health endpoints`

Owned paths define this task’s implementation boundary; current source and executed evidence are recorded below. Follow [the subcrate standard](../subcrate-standard.md), add only useful modules, and coordinate shared manifests/exports and migrations as described in the index.

## Implementation

1. Create a thin serve/migrate/jobs command composition, typed configuration, secret-safe diagnostics, Postgres pools, migration execution, and schema compatibility checks.
2. Introduce only needed base identity/session metadata and append-only audit storage. Assign later migrations to their domain tasks; keep SQLite schemas separate.
3. Expose a transaction-aware audit writer so revocation, lease, and approval commits can include their audit event atomically. Restrict audit reads to authenticated platform admins.
4. Implement liveness/readiness/drain hooks and caller-configured structured telemetry. Make health states honor task 002's authority and degraded-service contract.

## Acceptance criteria

- [x] Fresh migration and supported-version upgrade work; incompatible schemas cause a clear readiness failure.
- [x] Domain changes and their audit records can commit or roll back together, with application roles unable to rewrite audit history.
- [x] Configuration supports service-readable secret files and does not leak credentials in errors or logs.
- [x] Shutdown marks the service unready before draining; no unauthenticated domain or audit API is exposed.

## Verification

- Run PostgreSQL integration tests for migrations, transaction rollback, role privileges, audit access, configuration parsing, and readiness transitions.

Run the relevant formatting, lint, dependency-boundary, and supported-platform checks once code exists. Record real results and any unavailable environment; do not report planned or mocked external validation as completed.

## Boundaries and handoff

Enrollment/authentication, routing, transcript content, and destructive jobs remain with their task owners; no all-purpose repository framework.

**Deliver:** Gateway composition, migration rules, authenticated-principal interface, audit transaction hooks, and minimal running health endpoints.

## Completed evidence

The gateway provides typed/redacted file configuration, PostgreSQL pools/role validation, embedded/checksummed transactional migrations with supported schema range, transaction-aware audit writes, admin-only audited reads, bounded HTTP work, health, and SIGINT/SIGTERM drain. Real PostgreSQL 17 tests pass for fresh/idempotent migration, checksum/future-schema failures, domain/audit commit and rollback, application privilege denials, commit-ordered audit cursors, authentication/admin boundaries, actual connection loss/recovery, and drain readiness. A separate real gateway process passed liveness/readiness, unauthenticated audit rejection, and clean SIGTERM exit. CI includes the PostgreSQL test. `serve` currently rejects all tokens by default until task 011 installs verified identity; the jobs composition validates its binding but task 022 registers cleanup. See [gateway configuration and verification](../crates/qgenie-remote-gateway/README.md).
