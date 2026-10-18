# ADR 0006: Make the development deployment explicit

Status: selected implementation contract. No application stack is running or verified.

## User task and problem

Developers need four gateways on one server to test routing and rolling restart. Identical ports collide under host networking. Root-only secret files cannot be read by a non-root gateway. Exact schema equality prevents compatible rolling upgrades. Treating every Redis outage as total failure conflicts with local routing.

## Decision and reason

Use four explicitly named development gateway services with host networking and distinct ports. Use private health endpoints and non-root-readable secret mounts. Publish a schema compatibility range per binary. Keep local routing available during Redis failure only while durable access authority remains fresh.

| Service | HTTP/WSS private port | Health/metrics private port | Node identity |
| --- | --- | --- | --- |
| Gateway 1 | 8081 | 9091 | `dev-gw-1` |
| Gateway 2 | 8082 | 9092 | `dev-gw-2` |
| Gateway 3 | 8083 | 9093 | `dev-gw-3` |
| Gateway 4 | 8084 | 9094 | `dev-gw-4` |

HAProxy terminates TLS on port 443 and hashes the untrusted affinity hint for routing only. Enumerate all four backends and their matching readiness ports. For a single-server setup, bind gateway and data ports to loopback. A later multi-server deployment must use private addresses and firewall rules. Do not use `--scale gateway=4` with identical host-network ports.

PostgreSQL, Redis, PgBouncer, and OTel ports remain private. No host agent receives an inbound remote listener. WSS clients and connectors honor the configured proxy/CA trust. Test identities belong to an isolated test harness, not a production authentication bypass.

## Configuration and secrets

Process entry points read configuration. Libraries receive typed configuration. Require explicit public origin, unique node ID, distinct listeners, database connection, and issuer/audience. Fail on invalid or conflicting values without logging secrets. Do not silently use an inference audience.

Support secret-file settings for database and Redis credentials and signing secrets. If a direct secret and its file setting are both supplied, reject the ambiguity. Bound secret-file reads to 64 KiB and reject unexpected file types or unsafe permissions.

The gateway runs as UID 10001. A mounted secret must be readable by that UID or its dedicated group. Suitable layouts include owner UID 10001 with mode `0400`, or a root-owned file with mode `0440` and the gateway's dedicated group. A root-owned `0400` file is insufficient. Compose file-backed secrets do not universally honor requested UID/mode metadata; the deployment script must check actual in-container readability without printing content.

Do not place real credentials in `.env`, images, Git, CLI arguments, diagnostics, or validation artifacts. Separate migration, authority, ingest, read, admin, and lifecycle database credentials according to their responsibilities. Give only the migration entry point elevated schema privileges.

## Health and failure behavior

| Condition | Liveness | Readiness | Service behavior |
| --- | --- | --- | --- |
| Initialized; PostgreSQL current; schema compatible; Redis available | Live | Ready | Normal routing and APIs. |
| Redis unavailable; PostgreSQL current; local routing initialized | Live | Ready with `degraded` detail | Same-node traffic and durable APIs continue. Cross-node requests return typed unavailable/unknown outcomes as appropriate. Resubscribe and resync on recovery. |
| PostgreSQL unavailable | Live while process functions | Unready immediately | No new authority changes or admissions. Close existing affected access before its 10-second freshness deadline. Local host execution continues. |
| Schema outside supported range | Live | Unready | Refuse domain operations. Run explicit compatible migrations. |
| Required secret invalid/unreadable | Startup fails | Unready | Return a redacted configuration error. |
| Directory verification stale for a principal | Live | Global readiness can remain ready | Deny that user's and machines' access. Alert on verification age and broad provider failures. |
| Transcript storage unavailable | Live | Main routing can remain ready if access authority is healthy | Fail ingest/reads explicitly. Host spool, gaps, and warnings continue. If transcripts share the failed authority database, use the PostgreSQL-unavailable rule. |
| Draining | Live | Unready | Stop admissions, send `4503`, close within the drain budget. |

Do not return raw database/Redis errors, credentials, or principal details in health responses. Expose bounded diagnostic state on the private endpoint. Readiness must not continue to claim authority health after its check is stale.

Use 60 seconds for the default gateway drain budget and 75 seconds for container stop grace. Mark unready before draining sockets. Bound reconnect backoff and use jitter. Retrying the connection does not retry the last mutation.

## Migrations and jobs

Each binary declares a durable schema epoch and inclusive minimum/maximum supported schema revision. These constants ship with the binary; an environment override cannot make an incompatible schema safe. Version package, public wire, local IPC, native adapter, and database schema separately.

Use expansion, migration, then contraction. The expanded schema must support both old and new binaries during rolling replacement. Apply migrations once under a PostgreSQL migration lock. New domain migrations append unique identifiers. Do not rewrite an applied migration. Test fresh installation and the supported upgrade paths.

The `serve` process does not apply destructive schema changes on startup. The `migrate` command applies the selected migrations. The `jobs` command uses database-authoritative coordination for retention and holds. Redis leadership may reduce duplicate scheduling for disposable presence work, but cannot authorize a destructive operation.

Task 026 must pin actual image versions/digests and verify CA trust, non-root permissions, health checks, rolling restart, and backup/restore. Image size, fleet scale, and production availability remain measurements for task 028. One server is not highly available.
