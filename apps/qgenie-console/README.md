# QGenie Remote browser console

The gateway serves this application at `/console/`. The application uses local
JavaScript and CSS assets. It has no frontend build step or external CDN dependency.

Sign in through `/console/auth/login`. The gateway stores upstream credentials in
its browser session. The browser receives an opaque cookie and a CSRF token.
Bearer credentials are not stored in browser storage or URL parameters.

The authenticated owner view lists enrolled devices and native sessions. A
platform admin can switch to the administrative inventory, telemetry, audit, and
governance views. The server independently enforces every API permission.

The UI reads bounded `/v1/me/overview` or `/v1/admin/overview` snapshots. It displays
partial coverage and unknown states. It distinguishes requested model configuration
from reported effective models. Missing model, version, usage, profile, and history
data are not replaced with demonstration values.

Native session termination uses the existing gateway WebSocket protocol. The UI
checks the original session, current host generation, and termination capability.
It requires explicit confirmation. External sessions require another confirmation.
No controller lease is acquired for owner termination. A missing reply stays
unknown. A non-secret display record in session storage preserves that state over
reload. Reconciliation only reads the original session; it does not resend a stop.
Other sessions and device access continue.

Recording discovery and every content page use the existing protected read path.
Admin reads require a reason. The gateway verifies and decodes bounded compressed
chunks before the browser receives normalized records. The browser inserts content
as text. Explicit recording gaps remain visible.

The audit and existing revocation/hold APIs back the governance views. The console
does not present a simulated policy-save action. Organization policy editing,
statistical anomaly baselines, and scheduled rule jobs remain unavailable until
the corresponding services exist. Activity history and threshold signals use
committed audit events.

Run the browser contract checks with Playwright and Chromium:

```sh
python3 apps/qgenie-console/tests/browser_contract.py
```

These checks intercept HTTP and WebSocket responses. They verify browser contract,
rendering, uncertain-outcome, and text-safety behavior. They do not prove a deployed
native session was stopped. The repository's real Keycloak/native deployment test
provides that separate evidence.
