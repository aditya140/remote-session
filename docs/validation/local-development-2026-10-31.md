# Local deployment execution report

Date: 2026-10-31. Platform: Linux x86-64 in the managed workspace.

## Goal and setup

The task was to deploy the application locally, use real local user authentication,
exercise the full user journey, and document a repeatable development setup.
The ordered plan is [task 037](../../tasks/037-local-development-end-to-end.md).
The commands for daily use are in the [local development guide](../development/local-stack.md).

The devcontainer previously started only PostgreSQL and Redis. The existing full
deployment used a synthetic issuer that did not authenticate a person. The new
optional profile adds Keycloak 26.8.1 and two local users. It retains the four
gateways, HAProxy edge, PostgreSQL primary and synchronous standby, Redis,
PgBouncer, recording lifecycle worker, OTel, and Prometheus.

The `tools/dev.py` entry point generates private configuration and supplies build,
startup, login, host, client, status, and shutdown commands. Keycloak uses its own
database and database role. Its realm, users, sessions, and signing keys persist.
Only HTTPS 8443 and 9443 are published, on loopback. Native session ownership stays
outside the host connector. The host uses explicit no-service mode in this shell.

The gateway still validates issuer, tenant, audience, subject, object, scope,
signature, and token times. Local claim mappers supply its existing contract.
The development directory adapter checks actual Keycloak user state through a
service account with read-user permission. It does not synthesize enabled users.
Production Entra behavior and the synthetic CI profile remain available.

## Repairs found during execution

1. The managed checkout gave public configuration files mode `0600`. Non-root
   containers could not read them. The launcher now makes a fixed list of public
   source files readable. It does not change secret permissions.
2. Keycloak rejected a wildcard callback port. The local realm now registers
   the native client's loopback IP redirect without a port. Keycloak accepts the
   CLI's temporary port under that registration.
3. The dedicated Keycloak client needed explicit subject and not-before claims.
   It also needed the requested offline refresh scope and user permission.
   The seed now supplies these. Per-token issue and expiry times remain native
   Keycloak claims. ID tokens do not gain remote API permission or audience.
4. Keycloak required S256 for device authorization. The host now supports an
   opt-in `device_pkce` setting. It keeps the verifier in memory and supplies it
   on polling. Existing providers keep the previous request shape by default.
5. The directory adapter initially rejected a percent-encoded Graph selection.
   It now parses the query and rejects duplicate or unexpected selections.
   Only a real missing user from Keycloak becomes missing-user evidence.
6. Docker uses the `vfs` driver in this workspace. Large copied build layers
   exhausted the 32 GiB filesystem. A local image build now uses Cargo cache
   mounts with the same release settings and runtime image. CI retains its
   exported dependency layers. The Keycloak profile skips the unused fixture image.
7. A new client could receive an owner-wide session update before its connection
   handshake finished. This caused an intermittent CLI connection error during
   the developer-command check. The gateway now transmits `ConnectionReady`
   before it publishes the client to event routing. The routing regression checks
   the exact handshake order while a host publishes session updates. It fails
   against the old gateway binary at the expected readiness assertion. The
   original failed CLI connection sent no start request and created no session.
8. Internal gateway probes could pass before HAProxy admitted a backend after
   restart. An immediate `host doctor` then received HTTP 503. The Keycloak
   startup path now waits for the protected API through the public HTTPS edge.
   It requires the API to reject an anonymous request with HTTP 401. The restart
   check uses the same surviving native session and original start key.

## Executed checks

| Check | Result and scope |
| --- | --- |
| Workspace tests, all features | 220 passed, zero failed, 23 ignored environment proofs. |
| Workspace Clippy, all targets and features | Passed with warnings denied. |
| Gateway handshake regression | Failed against the old binary. Passed against the fixed binary with concurrent session broadcasts and exact handshake-order checks. |
| Gateway PostgreSQL/Redis suite after the fix | 26 passed, zero failed. The separate synchronous durability proof was excluded from this invocation. Gateway Clippy also passed again. |
| Rust formatting | Passed. |
| Python tool tests | 10 passed. Includes private initialization, provider separation, strict directory data, and encoded queries. |
| Device OAuth regression tests | Two passed. One database-dependent test was explicitly ignored in this focused invocation. Both default and opt-in request shapes were checked. |
| Synthetic four-gateway deployment | Passed with the actual CLI, native owner, and host connector. |
| Synthetic deployment failures | Passed Redis outage, PostgreSQL outage, hosting-gateway SIGKILL, and four sequential gateway replacements. The same native PTY and start key survived. |
| Protected history | Passed actual output, gateway redaction, confirmed termination, and retained history after recovery. |
| Monitoring | All 13 alert rules passed evaluator firing/recovery fixtures. Four private gateway scrapes and aggregate ingestion passed. |
| Backup | Restored a logical backup into an isolated quarantined database. No live authority was replaced. |
| Native Codex acceptance | All five phases passed with the pinned 0.159.2 executable and a controlled model fixture. |
| Real-user Keycloak journey | Passed browser PKCE, device login, wrong-password rejection, enrollment, owner isolation, allowed roots, TUI keys, pause/reconnect, and revocation. |
| Real-user recording and recovery | Passed the deployment outage/rolling/history proof with real OAuth access and refresh tokens. |
| Keycloak disablement and outage | Passed disabled-account denial on fresh verification, failed authorization during provider loss, recovery, and persistent signing keys. |
| Final clean seed | Passed all 12 end-to-end checks in 148.738 seconds after recreating the database and credentials from the final generator. No manual realm edits were needed. |
| Final gateway image | Repeated all 12 checks after the handshake fix. Passed in 140.567 seconds. |
| Developer entry point | Passed real `login`, `host` diagnostics, `client` TUI observation/detach, and `down`/`up`/`host`. The same native session and original start key survived the full restart. |

The [native Codex artifact](local-managed-codex-2026-10-31.json) records the five
phases. The public session phase took 70.02 seconds. The primary-volume-loss phase
took 27.88 seconds. The total runner took 233.322 seconds, including additional
compilation and waiting for another Cargo check. These are test durations, not a
capacity benchmark.

The [final Keycloak artifact](local-keycloak-2026-10-31.json) records each real-user
check and the remaining external gates. The setup was first repaired through
disposable instances. The clean-seed proof used newly generated configuration and
empty application and Keycloak databases. The final gateway proof reused that seed
without manual configuration changes. Earlier private state and test backups were
retained in ignored directories.

The [developer-command artifact](local-dev-commands-2026-10-31.json) records the
helper checks. The final local stack remains running. Alice's `local-development`
machine has a live shell with the marker `LOCAL_DEV_READY`. Its saved session and
start identity are in private `target/dev-stack/demo-session.json`. Use
`tools/dev.py client` with the documented options to inspect it. The native owner
and connector run as separate processes. The browser CA remains trusted in the
current user's NSS database.

The cold local release compilation took 7 minutes 45 seconds while other checks
ran. The second image reused the compiled binaries; its Cargo step took 0.67
seconds. The warm release rebuild after the handshake fix took 3 minutes 58 seconds.
The final gateway image is 19,687,135 bytes and runs as UID/GID `10001:10001`.
Its image ID in this execution is
`sha256:f6edb57d3663896779203cb4614d200a0758bbffbf19e7ab3b7b8e38a5deb6e5`.

## Commands used

The image build used the local Docker socket, the configured proxy, and verified
build CA trust:

```sh
python3 deploy/compose/stack.py --docker-host unix:///var/run/docker.sock \
  --build-cache mounts --build-ca /etc/ssl/certs/ca-certificates.crt build

python3 tools/dev.py --client-bin target/debug/qgenie-remote \
  --docker-host unix:///var/run/docker.sock --skip-build up

python3 tests/e2e/local_keycloak.py --client-bin target/debug/qgenie-remote \
  --docker-host unix:///var/run/docker.sock
```

Chromium trusted the generated CA through the current user's NSS database. No
test disabled HTTPS certificate verification. The browser test used real login
forms and consent controls. The actual CLI generated its own PKCE callback and
device proof. A temporary browser opener directed its login URL to Chromium.

The synthetic profile used separate state and ports:

```sh
python3 deploy/compose/stack.py --state target/dev-fixture \
  --project qgr-local-fixture --edge-port 10443 --identity-port 11443 \
  --docker-host unix:///var/run/docker.sock --client-bin target/debug/qgenie-remote smoke
```

The same prefix ran `up`, `monitor-smoke`, `restore-smoke`, and `down`.
The native Codex runner used:

```sh
CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_INCREMENTAL=0 \
CARGO_BUILD_JOBS=2 QGR_CODEX_BINARY=/tmp/qgr-ci-codex/node_modules/@openai/codex-linux-x64/vendor/x86_64-unknown-linux-musl/bin/codex \
python3 tests/e2e/recording_durability.py --require-managed-codex \
  --report target/validation/local-managed-codex-2026-10-31-retry.json
```

## Failed attempts and limits

The first layered build was cancelled before storage ran out. A later overlapping
deployment attempt and Clippy check encountered a full filesystem. The work
preserved usable binaries and private data, removed regenerable Rust build output,
and reclaimed only immutable Docker build-cache records created by this task.
Application images and Cargo cache mounts were retained. Both checks then ran
again successfully. The temporary synthetic stack was stopped after its proof.

The first native acceptance attempt failed at a correlated remote-start deadline
during concurrent cold builds. A new isolated fixture passed the complete runner.
The cause of that first timeout was not conclusively established. The failed
mutation was not automatically resent. Earlier Keycloak attempts exposed the
configuration errors listed above. Test-driver repairs covered consent-button
selection, mixed diagnostic output, and selecting the test machine in the TUI.
One browser callback was interrupted by a concurrent Docker network removal.
The final browser proof runs without another stack creating or removing networks.

This proves local Linux development behavior. It does not qualify corporate Entra,
macOS services, logout/reboot supervision, production fault domains, or beta scale.
The disabled-user check forces fresh gateway verification; it does not time the
entire cached-directory freshness window. Native Codex uses a real executable with
a fixture model service. No external model API or paid inference account was configured.

Private credentials, tokens, screenshots, raw diagnostics, native state, and
recordings remain in ignored local directories. They are not included in Git.
