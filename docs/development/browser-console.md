# Browser console

The gateway serves the admin and owner console at `/console/`. The application
uses the approved revision 4 design with actual API data. It has no Node runtime,
external asset service, or separate web server. The design prototypes under
`design/admin-console` remain separate from this application.

## Implemented views

- Owners can inspect their enrolled devices, presence, connection timestamps,
  native sessions, harness versions, requested model configuration, and protected
  recording history. They can confirm termination of one owned session.
- Platform admins can inspect users and their devices and sessions. They can read
  audit events, submit access revocations and legal-hold changes, and read protected
  recording history with the existing reason and audit requirements.
- Telemetry shows actual bounded inventory, committed start-event buckets,
  coverage warnings, and explainable device-enrollment and session-start bursts.
  Health reports the accepting gateway's actual component state.

Unknown native state is excluded from confirmed active totals. Installed harnesses
are not counted as running agents. A configured model is not an effective model
claim. Old or unsupported sessions retain unknown metadata. Usage counters,
scheduled telemetry evaluations, adaptive historical baselines, group policy
editing, and full browser terminal interaction are not implemented sources.
The UI shows unavailable states for those features. It does not draw sample data.
Server totals count the existing owner label `device_class=server`.
`device_class=workstation` is the other supported class. A missing label stays
unknown. The console does not infer device class from an operating system.

## Local sign-in

For a new local development deployment, use the existing local stack procedure:

```sh
python3 tools/dev.py up
```

Open `https://localhost:8443/console/`. The Keycloak profile provisions a separate
`qgr-browser` public client with S256 PKCE and an exact callback URI. Alice has
the local browser admin role. Bob remains an ordinary owner. Read their generated
passwords from the private `target/dev-stack/users.json` file. Do not commit that
file or copy its contents into logs.

This setup does not silently rotate an existing deployment's generated secrets
or change an existing Keycloak realm. If the local stack predates browser support,
create a separate project and state directory for validation. The implementation
proof uses `target/console-stack`, project `qgr-console-e2e`, edge port 8543, and
identity port 9543. The original local demo remains untouched.
Before you open this separate console, trust its own
`target/console-stack/certificates/ca.pem` in your development browser. The browser
proof installs this CA temporarily and removes that new trust entry after the test.

```sh
python3 deploy/compose/stack.py --state target/console-stack --project qgr-console-e2e --identity keycloak --edge-port 8543 --identity-port 9543 --docker-host unix:///var/run/docker.sock init
python3 deploy/compose/stack.py --state target/console-stack --project qgr-console-e2e --identity keycloak --edge-port 8543 --identity-port 9543 --docker-host unix:///var/run/docker.sock --build-cache mounts build
python3 deploy/compose/stack.py --state target/console-stack --project qgr-console-e2e --identity keycloak --edge-port 8543 --identity-port 9543 --docker-host unix:///var/run/docker.sock up
```

Builds behind a managed proxy must use the configured build CA as described in
[the local application guide](local-stack.md). Keep TLS verification enabled.
Trust only the generated development CA in the test browser. These local accounts
and certificates are development fixtures, not production identity settings.

## Gateway browser configuration

Enable the console sign-in service with `--browser-config-file` or
`QGR_BROWSER_CONFIG_FILE`. The gateway still requires the existing identity,
directory, and recording purposes for the applicable actions. Example configuration:

```json
{
  "public_origin": "https://remote.example.test",
  "authorization_endpoint": "https://identity.example.test/authorize",
  "token_endpoint": "https://identity.example.test/token",
  "client_id": "registered-browser-client",
  "scope": "openid api://registered-audience/Remote.Control",
  "encryption_key_file": "/run/qgr/browser-key",
  "ca_bundle": "/run/qgr/identity-ca.pem"
}
```

Register the exact callback `https://remote.example.test/console/auth/callback`.
Endpoints and callback use HTTPS. Generate a private 32-byte random encryption
key and mount the same key on every gateway. The file must not be world-readable.
The configured identity verifier remains responsible for issuer, audience, scope,
tenant, expiration, directory identity, and platform-admin claims.

Browser access tokens and one-time PKCE verifiers are encrypted in PostgreSQL.
Cookie identities are opaque and stored as hashes. Cookies are Secure, HttpOnly,
and SameSite=Lax. Session lifetime cannot exceed the access token lifetime or one
hour. Refresh tokens are not stored. After expiration, sign in again. Native work
continues while browser access is unavailable.

Use a callback-safe edge configuration. Authorization codes, cookies, bearer
headers, and reason text must not enter access logs. The development edge disables
request access logging. Static assets apply a content security policy that permits
only local scripts. Privileged API responses use private, no-store caching.

Logout deletes the shared session and fences an already-open browser WSS through
its independent authority gate. Browser session evidence has a four-second maximum
proof deadline and a two-second refresh interval. Cookie authentication cannot
be used for host enrollment, machine challenges, native uploads, or host WSS.
Bearer CLI clients keep their current authentication behavior.

## Database and rollout

PostgreSQL schema 18 adds browser session/state tables and audit lookup indexes.
The app role can read and delete its browser session records but gains no new
transcript or audit privileges. Tokens use a private key outside the database.
Native SQLite does not require a version bump for optional requested-model data.

Back up the database and apply the existing migration procedure. Old gateway
binaries reject schema 18. Roll out a compatible gateway set with the same browser
key and origin. Rotating the key invalidates old browser sessions; coordinate the
change across nodes. Do not restore an old database over acknowledged native input,
start identities, approval outcomes, or recording receipts.

## API and authorization

The new inventory endpoints live under `/v1/me` and `/v1/admin`. New lists are
cursor-paged with a maximum of 100 items. Snapshot projections are bounded to 500
rows per entity and expose incomplete coverage. Audit-derived series limit the
window to 24 hours and the series to 120 buckets, with an explicit event cap.
Per-device last connection uses indexed committed activation records. Missing old
history is not reconstructed from the current heartbeat.

Admin metadata uses the existing privileged governance admission path. Remote
kill-switch state must not remove an admin's ability to inspect governance.
Owner inventory and all native actions retain current remote-access checks.
Platform-admin metadata access does not grant another user's control or stop rights.

The new read endpoints use `GET`. Cookie requests use the current browser session.
Bearer requests retain existing identity checks.

| Path | Access | Result |
| --- | --- | --- |
| `/v1/me` | Verified user | Current identity and capabilities. |
| `/v1/me/overview` | Owner | Owned inventory snapshot and coverage. |
| `/v1/me/machines` | Owner | Owned device list and last connection. |
| `/v1/me/sessions` | Owner | Owned native session list. |
| `/v1/admin/overview` | Platform admin | Bounded platform inventory and coverage. |
| `/v1/admin/users` | Platform admin | User list. |
| `/v1/admin/users/{user}` | Platform admin | One user's inventory snapshot. |
| `/v1/admin/users/{user}/devices` | Platform admin | One user's device list. |
| `/v1/admin/users/{user}/sessions` | Platform admin | One user's session list. |
| `/v1/admin/machines` | Platform admin | Device list. |
| `/v1/admin/machines/{machine}/sessions` | Platform admin | One device's session list. |
| `/v1/admin/sessions` | Platform admin | Native session list. |
| `/v1/admin/telemetry/summary` | Platform admin | Inventory, harness, and source-coverage totals. |
| `/v1/admin/telemetry/series` | Platform admin | Audited start-event buckets. |
| `/v1/admin/telemetry/runs` | Platform admin | Explicit unavailable state for scheduled evaluations. |
| `/v1/admin/activity-signals` | Platform admin | Fixed-threshold review and stale-observation signals. |
| `/v1/admin/health` | Platform admin | Accepting-gateway component state. |

Lists accept `limit` and `after`. The maximum list limit is 100. The series accepts
`hours` from 1 to 24 and `buckets` from 1 to 120. The console reuses existing audit,
revocation, legal-hold, and recording-stream endpoints for their current purposes.

The browser terminates sessions through the existing owner-authorized WSS protocol.
It resolves the exact session and current host generation, confirms the request,
and sends one operation. A lost reply stays unknown. Read-only reconciliation
checks the original session; it does not resend the command. Other native sessions
and device access remain unchanged. Offline hosts cannot establish a successful stop.

Browser-readable recording endpoints are:

- `POST /v1/console/sessions/{session}/history` with the existing owner query.
- `POST /v1/console/admin/sessions/{session}/history` with the existing admin query.

They reuse existing protected readers. Admin reads commit a reasoned read audit
before returning content. Owner self-reads keep the existing owner isolation and
do not create a privileged admin-read audit. They decode only bounded, validated
chunks and retain explicit loss ranges.
The frontend renders recording text as text. It never executes terminal controls
or treats recording content as HTML.

## Verification

Run the frontend contract checks with a trusted local Chromium installation:

```sh
python3 -m pip install -r apps/qgenie-console/tests/requirements.txt
python3 apps/qgenie-console/tests/browser_contract.py --chromium /usr/bin/chromium
```

These intercepted contracts do not establish deployment behavior. The real browser
proof signs in through Keycloak, checks admin/owner isolation and CSRF, moves a shared
session across gateway failures, enrolls a native host, starts two real PTYs, stops
one through the browser, and verifies that the other remains running. It also
checks logout of an already-open browser WSS.

```sh
python3 tests/e2e/browser_console.py --state target/console-stack --project qgr-console-e2e --edge-port 8543 --identity-port 9543 --client-bin target/debug/qgenie-remote --install-browser-ca
```

`--install-browser-ca` adds only the owned fixture CA to the browser trust database
and removes only the newly added entry afterward. Passwords and token files remain
private. The proof records a sanitized JSON result and screenshots in the owned
state directory. [Task 040](../../tasks/040-implement-browser-console.md) records
actual executed results. The committed [validation record](../validation/browser-console-2026-11-01.json)
contains the final checks and image IDs. Production Entra registration and infrastructure gates
remain separate from local Keycloak qualification.
