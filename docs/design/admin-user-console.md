# Admin and user console proposal

Implementation: See [the browser console guide](../development/browser-console.md)
for current behavior, local setup, and verification. This document retains the
approved design proposals. It does not establish that every proposal is available.

Status: design review. No new production endpoint or browser authentication path
has been implemented. The prototype uses synthetic data and local simulation.

Open [the interactive options](../../design/admin-console/index.html). Select an
option and switch between Admin console and My workspace. Select navigation items,
filter inventory, search visible rows, or open an action dialog. No action contacts
a service. This design extends the PRD's later browser and admin UI scope.

## Revision 3: device and execution visibility

The [device and telemetry design](device-agent-telemetry.md) extends this proposal
with per-user device and server inventory, connection timestamps, per-session
harness/model observations, aggregate telemetry, explainable activity signals,
and a confirmed owner stop flow. It defines current data sources and missing
instrumentation. The current prototype includes these views in all three styles.
The user workspace uses this inventory as its main view and keeps approvals
separate. Production routes and execution remain proposed.

## Recommendation and visual options

| Option | Admin experience | User experience | Tradeoff |
| --- | --- | --- | --- |
| A: Studio — recommended | A charcoal sidebar, open metric strip, lilac activity histogram, fleet records, and a focused attention rail. | Work inbox, exact task outcomes, a separate approval card, machines, and history. | Best starting point for daily work. Deep incident analysis opens a focused view. |
| B: Signal | A dark monitoring canvas with an integrated latency plot, gateway health tracks, and a timestamped event stream. | A terminal-focused split view with exact outcomes, tool events, and an independent approval panel. | Best for an operator on call. Governance work takes more navigation. |
| C: Atlas | Warm surfaces, horizontal navigation, a fleet relationship map, and a subject inspector beside the audit trail. | A project list with session detail, independent approval decisions, and recording warnings. | Best for investigations. Persistent case storage would add scope and is deferred. |

Use A as the shared product shell. Offer B as the System health workspace and C as
the Audit workspace. Build one navigation and permission system. Do not implement
three products. Keep the visual options selectable until the design review ends.
The user screen keeps the same information order across options so a color or
density preference does not change action semantics.

Beautiful UI provides useful patterns for compact navigation, filtered records,
task status rows, and focused decision cards. We inspected
[Beautiful UI](https://www.beautifului.dev/) on 2026-10-31. This prototype adapts
those interaction ideas to QGenie. It does not copy the site's code or assets.
The site's chat and reasoning components are not required for governance work.

## Revision 2: visual direction

The first concept did not meet the user’s visual expectations. Revision 2 changes
the composition as well as the color palette. It reduces repeated card borders,
uses a more deliberate type scale, gives navigation consistent vector icons, and
makes each direction use a different information layout. User views now differ
by task cards, a terminal workspace, or a project list.

The user requested [designeer.xyz](https://designeer.xyz). Direct access and the
web reader could not open the live site. We used indexed descriptions and the
[maker’s product discussion](https://www.producthunt.com/products/designeer)
to understand its curated design-resource approach. We did not inspect its live
visual gallery or claim that another domain is its verified replacement.

The revision also draws from [Tremor](https://www.tremor.so/) for compact analytic
signals and [Refactoring UI](https://refactoringui.com/) for grouping through
spacing and contrast. The latter explicitly discusses reducing excess borders.
No library, commercial component, source code, or third-party image was copied.
Beautiful UI remains the reference for task and decision interactions.

The API proposal and permission model below keep their previous scope. The
prototype remains synthetic. The first version is retained in
[the design archive](../../design/admin-console/archive/v1.html).

## Core admin journeys

1. Open Overview. Distinguish reported process state, recording coverage, access,
   and observation freshness. Open the affected machine or signal.
2. Revoke machine, user, or global remote access. Show exact scope and affected
   remote connections. Require explicit confirmation. Keep the original request
   identity after a lost reply. Display committed denial separately from verified
   propagation. Do not terminate native sessions.
3. Search audit metadata. Select a subject and session. Request recording access
   with a reason and case reference where the existing held-record contract
   requires one. Commit the read audit before returning content. Audit pagination
   too. Do not preload transcript content into hover cards, search, or summaries.
4. Place, change, or release a legal hold with its original identity and revision.
   One platform admin is sufficient under the current PRD. Keep holds and admin
   reads invisible to the affected user. A case label is optional context; a case
   management service is not needed for the first implementation.
5. Review effective policy and a proposed change. Show the scope, old and new
   values, revision, and host delivery status. A saved policy is not proof that
   every disconnected host applied it. Local allowed roots remain host-enforced.
6. Inspect health signals with timestamps and source scope. Open the existing
   monitoring tools for deep metric and trace analysis. Do not expose private
   scrape ports or collect raw transcript text as telemetry.

## Core user journeys

- Open Work inbox to see owned sessions across machines. Keep process running,
  exact turn completion or failure, approval state, and recording coverage separate.
  Show incomplete scan coverage. Unknown is not an empty inbox.
- Browse owned machines and change their display metadata. Use the existing
  machine revocation path for self-service remote access removal.
- Read owned recording history and explicit gaps. Show content age and coverage.
  Do not expose legal holds, admin read events, or hidden investigation activity.
- Observe a session through the existing gateway. Acquire control explicitly
  before sending input or resize. Keep approval permission independent. Preserve
  the original start key and prompt receipt after uncertain outcomes.
- Use CLI-local launch profiles through the CLI initially. The preview profile
  cards describe a possible browser flow. They do not imply that local profiles
  are already synchronized to the gateway. Shared profiles require another design.

## Roles and authority

The current verified principal contains `user` and `platform_admin`. It does not
implement a complete operator or organization policy role system.

| Capability | Session owner | Platform admin | Future metadata-only operator |
| --- | --- | --- | --- |
| Own machines, sessions, and recordings | Yes, under current owner checks | Own resources through the owner path | Only own resources unless separately authorized |
| Fleet metadata and health | Own scope | Proposed global read views | Proposed limited grant; requires a separate role decision |
| Read another user's recording | No | Existing reason-gated audited path | No |
| Read privileged audit and legal holds | No | Existing audit read; hold listing proposed | No |
| Revoke user or global access | No | Existing revocation path | No |
| Change organization policy | No | Proposed versioned policy path | No |
| Input, resize, interrupt, or terminate another user's session | No | No new right from admin status | No |
| Answer another user's approval | No | No new right from admin status | No |

Start with the current owner and platform-admin roles. If operations staff need
independent metadata access, add a separately reviewed metadata-only role. Do not
silently grant transcript access with a dashboard role. Compute permissions on
the server. Hidden buttons are not authorization.

## Existing API inventory

These routes exist in `qgenie-remote-gateway/src/app.rs` and
`qgenie-remote-gateway/src/transcripts/api.rs`. They are HTTP paths, not new design.

| Current route | Use and limitation |
| --- | --- |
| `GET /v1/admin/audit?after=&limit=` | Platform-admin audit read. Current cursor is a counter; limit is at most 200. Server-side subject/action/time filters need an additive extension. |
| `POST /v1/admin/revocations` | Confirmed global, user, or machine denial with a request identity. |
| `GET /v1/admin/revocations/{request}` | Durable result and propagation state. Reconcile the original request. |
| `POST /v1/admin/holds` | Existing revisioned hold change. A hold inventory read is missing. |
| `POST /v1/admin/sessions/{session}/history/streams` | Audited protected stream discovery. Keep reason fields out of URL parameters. |
| `POST /v1/admin/sessions/{session}/history` | Audited protected recording page. Current history limit is at most four items. |
| `GET /v1/machines` and `GET /v1/machines/{machine}` | Owner machine discovery. Does not provide a cross-owner admin inventory. |
| `PATCH /v1/machines/{machine}` | Owner machine display metadata. |
| `POST /v1/machines/{machine}/revoke` | Owner self-service machine revocation. |
| `POST /v1/sessions/{session}/history/streams` and `/history` | Owner recording access. Preserve the bounded page contract. |
| `GET /v1/connect/client` | Current client WSS handshake. Session discovery, replay, control, approvals, start reconciliation, and metadata use negotiated wire messages. Browser authentication needs new work. |
| Private `GET /live`, `/ready`, `/metrics` | Deployment health and metrics. Not public browser endpoints. |

Keep these routes and their current contracts. New read endpoints must reuse
existing authorization, native routing, recording access, and audit services.
Do not build a second session lifecycle or copy native authority into a web table.

## Proposed API additions

All routes below are proposals. Prefer additive typed DTOs in the existing proto
crate and modules in the current gateway. A new server crate or orchestration
service has no demonstrated purpose.

| Priority | Proposed route | Purpose and authority |
| --- | --- | --- |
| 1 | `GET /v1/me` | Verified identity and effective UI capabilities. Never echo raw tokens. |
| 1 | `GET /v1/admin/overview` | Bounded aggregate access, recording, and fleet observations. Return independent source coverage and timestamps. Admin only. |
| 1 | `GET /v1/admin/users?cursor=&limit=&q=` | Minimal identity directory projection and effective access state. Admin only. Redact unnecessary profile data. |
| 1 | `GET /v1/admin/machines?cursor=&limit=&owner=&state=` | Cross-owner machine metadata and freshness. No recording content. Admin only. |
| 1 | `GET /v1/admin/sessions?cursor=&limit=&machine=&owner=&state=` | Reported session metadata. Never treat stale native state as confirmed live state. Admin only. |
| 1 | `GET /v1/admin/health` | Sanitized dependency and recording health summaries. No connection strings, private endpoints, or raw logs. Admin only. |
| 1 | `GET /v1/admin/holds?cursor=&limit=&user=` | Privileged hold inventory, revision, and preservation job progress. Never return this data to an owner route. |
| 1 | `GET /v1/admin/revocations?cursor=&limit=&target=` | List durable requests and their last verified propagation evidence. Preserve existing per-request status. |
| 1 | Extend `GET /v1/admin/audit` | Optional actor, subject, action, machine, session, and time filters. Preserve the existing unfiltered counter-cursor behavior. Bound filter ranges and indexed reads. |
| 2 | `GET /v1/admin/policies` and `GET /v1/admin/policies/{policy}` | Effective policy and immutable revision history. Distinguish current enforcement from proposed fields. |
| 2 | `POST /v1/admin/policies/{policy}/preview` | Validate a bounded diff and show estimated impact with coverage. No mutation. |
| 2 | `PUT /v1/admin/policies/{policy}` | Save a policy with `If-Match`, original request identity, and reason. Commit policy and audit together. Native delivery status is separate. |
| 2 | `GET /v1/admin/policies/{policy}/delivery` | Per-host reported revision, time, rejection, offline, or unknown state. No unsupported claim of complete rollout. |
| 2 | `GET /v1/me/sessions?cursor=&limit=&machine=` | Owner-scoped inventory adapter over current projection. Reuse wire session-page semantics. |
| 2 | `GET /v1/me/work?cursor=&limit=` | Owner work page. Reuse the bounded inbox model. Return scanned and unconfirmed scope. |
| 2 | `GET /v1/me/approvals?cursor=&limit=` | Owner-authorized approval summary. Approval application remains the existing separately authorized operation. |
| 3 | `POST /v1/browser/connect-tickets` | Short-lived, single-use, origin-bound WSS ticket. Scope to authenticated owner client access; do not grant host enrollment or cross-owner control. |

Do not add REST copies of prompt, input, resize, control, or termination now.
Use the current negotiated wire operations for browser interactivity after its
authentication and compatibility tests pass. A prompt request must not become a
retryable generic HTTP form submission.

Organization policy needs an enforcement design before priority 2 ships. Group
policy requires a verified directory membership source that is not established
by the current enabled-state check. Gateway policy may narrow permitted actions.
It cannot loosen host restrictions. Retention policy must preserve held records,
original capture age, and monthly deletion rules. Recording cannot be disabled.

## Response and mutation contracts

New inventory pages use an opaque stable cursor, a default limit of 25, and a
maximum of 100. Current recording and audit limits remain unchanged. Bind cursors
to the actor, filters, sort order, and expiry. Use deterministic ID tie-breakers.
Avoid exact unbounded total-count queries. Do not issue one native call per row
when rendering a fleet page.

A proposed overview response makes incomplete information explicit:

```json
{
  "observed_at": "2026-10-31T13:42:16Z",
  "coverage": {"reported_machines": 24, "known_machines": 26, "complete": false},
  "reported_running_sessions": 86,
  "unknown_machines": 2,
  "recording": {"warnings": 2, "explicit_gaps": 1},
  "revocations": {"pending": 1},
  "sources": [{"kind": "machine_projection", "observed_at": "2026-10-31T13:42:08Z"}],
  "request_id": "req_example"
}
```

These values are synthetic, not observations of the current deployment. Totals
need defined windows and grain. A graph cannot certify complete recording.
Raw session IDs and timing labels can be shown to operators; sensitive content
must stay behind the recording access path.

New mutations carry an original request ID, reason, and resource revision where
applicable. Preserve current revocation and hold DTOs. If those operations need
an additional reason field, add it compatibly and retain legacy request behavior.
Do not claim existing endpoints already require a field absent from their DTOs.

Return typed errors for denied access, revision conflict, capacity, stale
observation, and unavailable authority. After a lost mutation reply, show an
unknown result and reconcile its original identity. Do not issue a new request
identity. A successful save means durable commit; propagation may remain pending.
Bound reason text and exclude it from telemetry. Audit the authoritative outcome.

## Browser and operational design

Serve one frontend under the existing gateway origin with `/admin` and `/app`
routes. Start with a small React and TypeScript frontend if the design is selected.
Use native fetch and the existing wire contract; a general TypeScript SDK is not
a prerequisite. The reviewed prototype is standalone HTML with no dependencies.

Add browser OIDC authorization-code flow with PKCE. Keep upstream tokens in a
server-side browser session in the existing gateway. Use an opaque Secure,
HttpOnly, SameSite cookie, bounded expiry, and server-side revocation. Validate
Origin and CSRF tokens for mutations, including protected POST reads that produce
audit records. Do not put bearer tokens in local storage or URL query strings.

Browser WSS uses a short-lived single-use ticket only after priority 3. Bind it
to the expected origin and authenticated user. Limit replay and handshake work.
Recheck current authorization throughout the socket lifetime. Authenticated
frontend routes never replace backend checks. Return `private, no-store` for
privileged identity, inventory, audit, and recording responses.

Read overview and inventory from bounded PostgreSQL projections with indexed
filters. Use existing native checkpoints only for explicit owner/session detail
reads, with at most two concurrent native calls and a short deadline. Display
unknown state when a host cannot reply. Poll visible overview pages every ten
seconds, suspend hidden pages, and cancel obsolete requests. Avoid an unbounded
live event bus in the first version.

Metrics remain in Prometheus and the OTel pipeline. Add a small sanitized adapter
for defined aggregates only if deployment access is available. Allowlist metric
queries and bound ranges. Set a maximum 24-hour range and 120 buckets initially.
Return unavailable when a source is absent. Do not substitute invented values or
query metrics directly from the browser. Keep deep monitoring links deployment-
configured and role checked.

## States and accessibility

Every read view needs loading, empty, partial, stale, denied, expired login,
offline, and unavailable-authority states. Render an observed timestamp and an
explicit refresh action. Never convert a timeout into zero incidents.

Every privileged mutation needs confirmation, in-flight, committed/pending,
completed, conflict, denied, and unknown-reply states. Audit recording reads before
display. Keep gaps visible in rendered history. Sanitize terminal control bytes.

Use semantic tables, labeled form controls, visible keyboard focus, Escape to
close dialogs, and reduced-motion support. Status uses text as well as color.
At narrow widths, collapse navigation and keep table overflow within its card.
The prototype provides role switching for design review only. Production role
selection derives from the authenticated principal and cannot be toggled locally.

## Implementation order and acceptance gates

1. Approve the visual shell and browser scope. Add the browser session boundary
   and `/me`. Test real local Keycloak sign-in, expiry, logout, CSRF, and denied roles.
2. Add admin metadata read endpoints and the owner history browser. Reuse the
   existing audit, revocation, hold, and protected recording operations. Add bounded
   queries and reason-gated recording UI before returning any cross-owner content.
3. Deliver the overview, fleet, audit, holds, revocation, and health screens.
   Prove that revoked remote access preserves real native PTYs and Codex work.
   Verify committed versus completed propagation under missed broadcasts.
4. Add policy preview, durable versioned save, and host delivery only after the
   policy and directory source design is accepted. Test revision conflicts,
   disconnected hosts, host rejection, retention boundaries, and legal holds.
5. Add browser owner inbox and live observation. Qualify ticket replay protection,
   exact start recovery, capability refusal, lease authority, and independent
   approval rights before enabling any browser mutation of a native session.

Tests must prove owner isolation and platform-admin enforcement at the API and
RLS layers. Check direct API calls, not only navigation visibility. Verify that
recording audit failure returns no content. Verify that user responses and cache
entries never reveal holds or admin reads. Test bounded pages, request cancellation,
stale projections, missing metrics, and lost mutation replies. Exercise real
Keycloak and the existing composed native fixture. Measure query and CI costs.

The selected scope remains design-only until review. Screenshots and a tested
prototype establish visual and interaction proposals; they do not establish
production authorization, accessibility conformance, scale, or deployed API support.

## Repository evidence

- [Requirements](../../PRD.md): PO-1 through PO-8 and later browser scope.
- [Architecture](../../architecture.md): authorization, recording, and revocation.
- [Gateway routes](../../crates/qgenie-remote-gateway/src/app.rs).
- [Recording routes](../../crates/qgenie-remote-gateway/src/transcripts/api.rs).
- [Current REST DTOs](../../crates/qgenie-remote-proto/src/rest.rs).
- [Audited reads](../../crates/qgenie-remote-gateway/src/audit.rs).
- [Work inbox](../integration/work-inbox.md).

## Revision 4: expressive visual system

The user requested more icons, plots, visualization, and color, and explicitly
requested parallel agent work. Three agents designed isolated admin, telemetry,
and owner-workspace modules. The shared shell integrates them under one navigation
and permission model.

The admin module adds colored icon metrics, a labeled native-session and agent-turn
plot, a native-state coverage ring, owner cards, and device coverage tracks. The
user detail view adds device cards and a model-reporting ring. The telemetry module
adds start-event buckets, host and model coverage rings, owner execution bars,
partial usage reporting, and evidence cards. The owner workspace adds recognizable
device icons, workload bars, connection and model coverage rings, usage context,
and session cards with explicit stop controls.

Blue identifies primary navigation and devices. Teal marks confirmed activity.
Amber marks incomplete observations and review needs. Pink distinguishes agent
attention and usage context. Color always accompanies text or a labeled chart
legend. Unknown state stays visible. No color establishes an authorization right.

Plots identify their sample scope, units, coverage, and observation window.
Synthetic history is labeled as a design example. Hover details expose the exact
sample values in activity plots. Owner bars and device cards lead to existing
metadata views. No chart automatically revokes access, approves work, or stops a
session.

Local files under `design/admin-console/variants` hold the three view modules.
`visual-shell.css` holds the shared shell. `visual-integration.js` connects them
to the current prototype data and interactions. The review package includes these
files so it can run without external libraries or services. The production API
proposal remains separate from this visual design work.
