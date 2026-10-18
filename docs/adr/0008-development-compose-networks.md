# ADR 0008: Isolate development services with Compose bridge networks

Status: implemented for task 026; runtime qualification is recorded in PROGRESS.md.

The single-server development deployment uses a private `backend` bridge and a separate
egress bridge. Each gateway has its own container namespace and listens on 8080/9091, with
explicit `development-1` through `development-4` node names. This replaces four host-network
port pairs for this Compose environment. Database, PgBouncer, Redis and collector endpoints
are unpublished. The edge publishes HTTPS on loopback 8443 and the isolated fixture issuer
on loopback 9443. Publishing off-host endpoints requires explicit configuration and certificates.

The gateway image is a pinned Rust 1.99.1 musl build on pinned distroless/static with CA trust,
UID/GID 10001, read-only root filesystem and no shell. The default Docker build target is the
gateway. A separate explicitly selected `fixture-identity` image contains a known synthetic
test signing key and an issuer/audience dedicated to development. The production gateway
contains no fixture authentication path. Real Entra/provider credentials must never be used
with that test image. Operator initialization mounts separate secret volumes into gateway,
edge, identity, Redis, PgBouncer, migration, lifecycle and PostgreSQL processes. Migration
credentials are absent from gateway mounts. Secrets are never baked into images or `.env`.

Protected development PTYs use an actual synchronous streaming standby, remote-apply commits
and the real bounded lifecycle process. Both database volumes share one physical host; this
does not qualify production failure domains or backup/restore operations. The stack helper
checks actual replication before setting the development database readiness flags. A logical
restore smoke uses a new isolated database, forces recovery quarantine/new epoch and never
switches the live authority.

HAProxy uses consistent hashing of an authorization header as a disposable affinity hint;
it never captures that header in logs. Gateway authentication, current primary facts, native
authority, Redis signatures and fencing remain independent of affinity. OTel scrapes private
gauges/counters without owner/session/content labels; its local debug exporter is replaced
with the operator's authenticated OTLP exporter for a deployed environment.
