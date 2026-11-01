# Admin and user console design

Status: design review. Production implementation has not started.

## Design steps

1. Read the PRD, architecture, gateway routes, audit model, and recording APIs.
2. Inspect Beautiful UI and identify relevant interaction patterns.
3. Create three browser prototypes with separate admin and user views.
4. Map existing APIs and proposed APIs. Define authority, freshness, and failure states.
5. Validate keyboard navigation, simulated actions, and narrow layouts.
6. Present the options and recommend a first implementation scope.

## Scope

All prototype data is synthetic. Prototype actions change local display state only.
A browser UI extends the current TUI-first release scope. It needs an explicit
implementation phase after design review. Admin monitoring does not grant session
control. Recording reads require the existing audited access path.

## Delivered design and evidence

The [proposal](../docs/design/admin-user-console.md) includes three visual options,
admin and user journeys, a permission matrix, an inventory of current routes,
proposed routes, browser authentication, freshness contracts, and implementation
phases. The [prototype](../design/admin-console/index.html) runs without external
assets or network calls. Six desktop screenshots and a visual comparison are in
`design/admin-console/previews`. The design branch is `design/admin-user-console`.

Headless Chromium and Playwright checked all six desktop views and six narrow
layouts at 390 pixels. The check exercised inventory filters, required revocation
confirmation, user navigation, Escape in dialogs, and keyboard search. No browser
script errors occurred. The first narrow-layout check exposed horizontal overflow;
the corrected grid and navigation now pass. Relative document links and whitespace
checks passed. These are prototype checks, not production permission tests.

The Beautiful UI site was accessible through the web research tool at its `www`
URL. The direct executor request was denied by network policy. No network policy
was changed. The reference informed interaction patterns; no source assets were
copied. Production UI, APIs, browser identity, and deployment remain proposed.

## Revision 2: response to visual feedback

The user rejected the first visual pass and requested designeer.xyz as another
reference. The live domain was unavailable through both the executor policy and
the web reader. Indexed references described its curated design-resource model.
We inspected the maker's Product Hunt page, Tremor, and Refactoring UI. We did not
claim to inspect the live Designeer gallery or treat another domain as a verified
replacement.

Studio now uses a charcoal navigation rail, open metrics, a lilac histogram, and
an attention column. Signal uses a unified dark monitoring canvas, gateway tracks,
and an event stream. Atlas uses top navigation, a fleet map, and an investigation
inspector. User screens use task cards, a terminal detail layout, or project rows.
The original prototype is preserved under `design/admin-console/archive/v1.html`.

The six desktop screens and six mobile layouts passed Chromium checks. Filters,
required confirmation, role navigation, Escape, and keyboard search passed.
Direction switching and all admin navigation views also passed. No script errors
occurred. A mobile Atlas navigation overflow was corrected before final capture.
The review package and images now contain revision 2. The API and authority
proposal retains its design-only scope.

## Revision 3 plan: user, device, and execution visibility

1. Inspect current machine presence, session identity, and agent inventory fields.
2. Add admin user inventory and a user-to-device-to-session drill-down.
3. Separate connected devices, enrolled servers, current process state, turn state,
   model identity, harness identity, and observation freshness.
4. Add aggregate telemetry and explainable unusual-activity examples with evidence.
5. Add owner device/session views and an explicit confirmed session stop flow.
6. Document source gaps, API additions, metric definitions, and authorization.
7. Check the prototype on desktop and mobile, including stop and offline cases.

These steps extend the design prototype. Production API and runtime behavior
remain proposed until implementation. Prototype actions cannot stop a real agent.

### Revision 3 delivered and checked

The prototype now has admin Users and Telemetry & signals views. A user row opens
its device inventory and agent sessions. Device detail shows harness version,
reported model or unknown, native process state, task state, connection history
fields, and last-seen freshness. Server counts are an explicit device subset.
The owner workspace shows only its own fixture resources and targeted stop controls.

The [device and telemetry proposal](../docs/design/device-agent-telemetry.md)
defines the additional APIs, missing collection fields, count semantics, model
provenance, aggregate coverage, explainable activity rules, and owner stop authority.
Installed harnesses are not counted as running agents. Stale sessions remain
unknown and stay outside confirmed active totals. Token totals are partial and
session-scoped. Activity signals do not automatically revoke or terminate work.

Chromium checks passed six desktop and twelve narrow views. They exercised user
and device navigation, owner-visible scope after admin navigation, missing model
metadata, cancel, exact stop confirmation, preservation of unrelated sessions,
unknown stop reconciliation without resend, offline controls, aggregate reporting
coverage, signal evidence, and keyboard dialogs. No script errors or external API
calls occurred. A fixture check first used a non-unique heading locator; it now
selects the visible content heading. The final proof passed.

New previews include admin user detail, aggregate telemetry, owner devices and
agents, and stop confirmation. The review package includes the new API proposal.
All runtime interactions remain simulated; no production routes or telemetry jobs
were added, and no real native session was stopped.
