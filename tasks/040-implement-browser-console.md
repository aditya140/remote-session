# Implement the admin and user browser console

Status: implemented and validated locally. The user approved the revision 4 design.

## Plan before implementation

1. Add authenticated, bounded admin and owner inventory and telemetry read APIs.
   Preserve current device, native session, recording, and access authorities.
2. Add shared browser sign-in sessions with OIDC authorization code and PKCE.
   Keep tokens encrypted on the server. Require CSRF and origin checks.
3. Serve the approved console from the gateway. Replace every fixture with actual
   API data, explicit unknown fields, or an unavailable-source state.
4. Connect owner session stop through the existing correlated WSS protocol.
   Preserve confirmation, independent rights, uncertain outcomes, and other sessions.
5. Integrate local Keycloak configuration, private browser key provisioning, and
   the Docker build. Use an isolated deployment for schema and browser tests.
6. Test API role and owner isolation, browser authentication, cross-origin/CSRF,
   native stop behavior, and actual browser navigation. Run relevant workspace checks.
7. Record implementation evidence and remaining external qualification limits.

Implementation agents own the API module, browser identity module, frontend
assets, protected recording decoding, model metadata, and browser deployment proof.
The main agent owns composition, deployment, migrations, and integration validation. No production release is claimed from local development.


## Completed implementation

1. Added gateway-hosted static assets in `apps/qgenie-console`. The admin and owner
   views use actual authorized API responses. Missing sources show unavailable states.
2. Added bounded owner and admin inventory, start-event plots, review signals, and
   sanitized accepting-gateway health. Audit-derived reads commit an audit first.
3. Added shared, encrypted browser OIDC sessions. Added PKCE, single-use callback
   state, secure cookies, CSRF, origin checks, and existing-socket logout fencing.
4. Connected confirmed owner termination to the existing WSS authority. The browser
   preserves 64-bit generations and uncertain outcomes without an automatic resend.
5. Added bounded browser-readable recording history through existing protected
   readers. Added configured-model metadata to native acceptance and public inventory.
6. Added schema 18, separate local browser-client provisioning, a private encryption
   key, and an existing-state guard. The original local demo remains unchanged.
7. Added frontend contract checks to the existing Linux workspace CI job.

## Executed verification

- `cargo test --workspace --locked` passed: 254 tests. Nineteen environment-specific
  tests were ignored by this command. Separate proofs below cover selected fixtures.
- `cargo clippy --workspace --all-targets -- -D warnings` passed.
- Managed-Codex all-target Clippy passed.
- The disposable PostgreSQL and Redis gateway suite passed: 36 tests. It includes
  browser session sharing, CSRF and origin guards, role isolation, bounded metadata,
  stale observations, transparent activity thresholds, and health projections.
- The Chromium frontend contract suite passed after the final asset formatting.
  It covers roles, configured and effective models, metadata and recording XSS,
  exact termination, 64-bit generations, cancel, unknown-result reload, other-session
  survival, recording gaps, preserved admin reason drafts during background refresh,
  and desktop, tablet, and mobile layouts.
- Formatting, dependency boundaries, and public, REST, and recording schema
  freshness checks passed. The Python tooling suite passed: 14 tests.
- The law kernel passed 115 laws, rejected 32 mutations, and compared 22,712 Rust
  cases. The sanitized report is `target/validation/console-laws.json`.
- The pinned real Codex acceptance and recovery test passed. It checks that the
  configured model survives native persistence and public projection. Preview
  schema compatibility passed. No released N-2 qualification is claimed.

The final real Keycloak browser proof passed ten checks in 18.615 seconds. It used
actual Alice and Bob browser sessions. It verified the exact image on each of four
gateways and forced requests across different gateways. It enrolled a real native
host and verified its committed connection timestamp in the owner device-list API.
It started two PTYs, stopped only the selected PTY, and verified the other process
and displayed ended state. Admin user and device navigation passed. Logout closed
an already-open authenticated WSS in 2.083 seconds. The second native process kept
running until fixture cleanup. The report is
`target/console-stack/last-browser-console-e2e.json`.

The full composed native recording proof passed all five phases in 166.387 seconds.
It checks real native PTYs, the pinned Codex process, managed-start crash boundaries,
protected decoded HTTP reads, and receipt survival after primary volume loss.
Decoded reads check owner and admin isolation, required admin reason, redaction,
explicit gaps, and the existing privileged audit contract. Owner self-reads add no
privileged audit. Each successful admin read commits one. The report is
`target/validation/console-native.json`.

All four isolated gateways run the final image:
`sha256:47c61eda35e17a5b7b89d15050b396568c65d6c55fc4c09166ce025d811b8a97`.
The final gateway all-target Clippy check passed after the recording test fixes.
The original deployment remains healthy. Owned native proof fixtures were cleaned up.
A sanitized, committed [validation record](../docs/validation/browser-console-2026-11-01.json)
contains the final browser checks, image IDs, native phases, and qualification limits.


## Issues found during implementation

- Device-list APIs originally omitted committed last-connection timestamps.
  All three machine-list handlers now use the same audited enrichment as snapshots.
  The PostgreSQL contract checks the timestamp and owner scope on both admin routes.
- The first nested Docker attempt exhausted the workspace disk. Generated caches
  and completed disposable fixtures were cleared. Source, deployment secrets,
  databases, the original demo, and native owner state were preserved.
- The browser proof now waits for each gateway before a routing fault. It selects
  the explicit admin workspace. Its logout assertion uses bounded read polling
  because the application CSP correctly rejects string evaluation.
- The recording HTTP contract now receives its audit inspection connection from
  the fixture. It does not assume optional metadata audit configuration. This
  change does not increase production database privileges. The HTTP contract checks
  that an owner self-read does not create a privileged admin audit. An admin read
  must add one committed read audit before it returns content.


## Remaining qualification and unavailable sources

This completes the approved browser companion implementation and local Linux proof.
It does not complete production Entra, production fault-domain, representative-scale,
or other native-platform release gates. Hosted GitHub CI was not verified here.
Effective runtime model identity, normalized token usage, scheduled rule evaluations,
adaptive baselines, organization policy editing, and full browser terminal interaction
remain unavailable. The application labels those limits. See the
[browser console guide](../docs/development/browser-console.md).
