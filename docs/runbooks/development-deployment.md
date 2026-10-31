# Four-gateway development deployment

For real local user sign-in, use the [Keycloak development guide](../development/local-stack.md).
This runbook describes the default synthetic CI profile. The shared deployment
also accepts `--identity keycloak` with a separate private state directory.

This is an isolated **synthetic development environment**. The separate fixture issuer does
not authenticate people or a real tenant. Use no production credentials. It exercises the
real gateway/host/native PTY and recording paths with an actual same-host synchronous standby.

Prerequisites: a local Linux Docker engine with Compose v2, Python 3, OpenSSL and room for a
Rust release build. The initial compilation can take several minutes. Container limits reserve
about 6 GiB; this configuration is not a capacity qualification. Native macOS services and
retained-directory equivalence need separate execution evidence.

From the checkout:

```sh
python3 deploy/compose/stack.py init
python3 deploy/compose/stack.py build
python3 deploy/compose/stack.py up
python3 deploy/compose/stack.py status
python3 deploy/compose/stack.py monitor-smoke
```

Put options before the command. `--build-ca /absolute/ca-bundle` mounts an enterprise build
CA as a BuildKit secret. TLS verification stays enabled. Predefined Docker proxy settings
are forwarded; host-local proxy aliases receive build-only address mappings. Use
`--docker-host` to select an explicit engine. `--build-target gateway` or `--build-target fixture-identity` retries one
selected image after a partial build; the default builds both. The pinned installed toolchain builds musl without downloading lint
components. The runtime contains the static binary, CA trust and distroless files; its
default build target excludes the fixture identity binary.

`init` writes owned private files under ignored `deploy/compose/secrets`. Repeated calls
preserve the same project/ports/credentials and refuse implicit rotation. The bootstrap
container copies files to separate runtime volumes with non-root ownership and `0400` modes.
Gateway UID/GID 10001 reads only its runtime credentials; the edge gets only its TLS key and
certificate, and migration credentials are absent from gateway mounts. PostgreSQL uses UID
70 from its pinned Alpine image. Secrets are absent from images and `.env` files.

No database, Redis, PgBouncer, metrics or collector port is published. HTTPS 8443 and the
explicit fixture issuer's 9443 bind only to 127.0.0.1. Source server certificates last seven
days. Automatic renewal, public exposure and production publishing are outside this stack.
All images are pinned, root filesystems are read-only, processes have separate limits, and
Docker `local` logs rotate at three 10 MiB files per container.
Redis uses a separate CA-signed TLS certificate and password. Explicit `NO_PROXY` entries
keep only private service names and loopback traffic inside the deployment; configured
external HTTPS proxies and certificate verification remain active.

`up` orders initialization, primary/streaming standby, Redis/PgBouncer, fixture issuer and
edge, migration and actual synchronous-state qualification. It starts four gateways and the
bounded lifecycle scheduler after that boundary. Readiness needs current primary schema
access; Redis loss keeps the documented ready/degraded behavior with independently expiring
authority. Protected operations keep their own synchronous commit/recovery checks.

`/app/gateway probe` checks its own loopback health listener. Private `/metrics` exposes
readiness/draining, connection/pending counts and cross-node request counts without content
or identity labels. HAProxy health-checks four nodes, supports 65-second ordinary idle
connections and long WSS tunnels, and treats affinity solely as a load hint. Authorization
headers are never captured in access logs. OTel scrapes private metrics with bounded memory
and batches. A separate private OTLP/HTTP pipeline exports aggregate operator samples to
the pinned Prometheus evaluator. All 13 alert rules run there; the API and collector remain
unpublished. Monitoring storage is ephemeral and bounded independently of product data.
Use `stack.py monitor` to keep the host aggregate probe scheduled every 30 seconds, adding
`--native-state` for the active private spool. `monitor-smoke` recreates this ephemeral pair
and verifies evaluator fixtures, private ingestion and all four gateway scrapes. See
[operational telemetry](operations.md#telemetry-alerts-and-assignments) for limits and commands.
A production notification receiver and owner remain unconfigured.

To exercise actual native ownership:

```sh
cargo build --locked -p qgenie-remote
python3 deploy/compose/stack.py --client-bin target/debug/qgenie-remote smoke
python3 deploy/compose/stack.py restore-smoke
python3 deploy/compose/stack.py fault-smoke
```

The smoke fixture owns temporary enrollment/native/client state and a real PTY. It checks
start/input and executes local input while Redis and then PostgreSQL are stopped. It waits
for the recovered connector's actual activated route, SIGKILLs that hosting gateway and
reconciles the original start key without relaunch. It then checks four rolling gateway
recreations, the same native session, actual cross-node dispatch, confirmed termination and
verified/redacted protected history including both outage inputs. This deliberately interrupts
only the owned development project's dependencies and gateway. Mutations
are never retried. Native owner and connector are cleaned up independently. Record the actual
run result in PROGRESS.md; the presence of a script does not establish its proof.
`stack.py roll` applies additive migrations, recreates one gateway at a time, checks actual
readiness and verifies each container uses the image selected before the rollout. It then
updates the lifecycle scheduler. Rebuild/select the compatible image before rolling; a
plain container restart does not install a rebuilt image. Schema compatibility with the
old nodes is required throughout the additive migration. A failed check stops the rollout.
Private diagnostics are retained in `secrets/last-smoke.log`. `fault-smoke` verifies the
published port set and blocked administrative paths, stops Redis to check explicitly
degraded readiness and resubscription, then stops PostgreSQL to check live/unready behavior
and recovery. Run it only against the owned development project; it deliberately interrupts
that project's dependencies and restarts them in `finally` cleanup.

Other operations:

```sh
python3 deploy/compose/stack.py roll
python3 deploy/compose/stack.py backup
python3 deploy/compose/stack.py restore-smoke
python3 deploy/compose/stack.py down
```

`roll` restarts nodes sequentially and waits for each actual readiness. Native ownership is
external to the stack. `backup` writes a private custom-format logical dump. `restore-smoke`
uses a generated isolated database, sets quarantine/new epoch, checks schema/quarantine and
drops that temporary database. It never switches live authority. This checks the logical
tools, rather than qualifying off-host recovery, hold preservation or failure domains. The
separate `tests/e2e/recording_durability.py` drills primary-volume destruction/held content on
its owned resources. Backup targets, restore/recovery reconciliation, physical fault domains
and operating ownership still require their own evidence.

`down` preserves persistent database/secret volumes. Keep identical project/port options on
subsequent commands. Do not regenerate inputs or remove volumes as a credential-rotation
procedure. To discard synthetic data intentionally, stop only this project, explicitly remove
its named volumes through Compose, then remove its private input directory. No helper
automatically deletes persistent volumes. Host user-service operations are documented in
[remote commands](../integration/remote-commands.md); a gateway or connector restart never
authorizes a native-owner upgrade.
