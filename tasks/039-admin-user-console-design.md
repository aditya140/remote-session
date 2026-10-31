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
