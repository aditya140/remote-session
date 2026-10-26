# 029 — Stabilize the core developer journey

Status: complete for Linux code repairs and fixture validation. External pilot gates remain open.
Authorized by user direction on 2026-10-26.

## Goal

Implement the repairs identified in the [repository review](../docs/validation/repository-review-2026-10-24.md).
A developer must be able to discover an allowed directory, start a shell or supported
Codex session, interact, answer approvals, disconnect, reattach, and terminate.
Completed history must not prevent new work or hide active sessions.

Keep the first release limited to Linux, single-owner machines, shells, and the pinned
Codex adapter. Preserve protected recording, access control, retention, and holds.
Do not add agents, platforms, clients, or a new deployment architecture in this task.
Preserve the review fixes already present in the working tree.

## Execution rules

Document each contract before its implementation. Update the evidence below after
running its checks. A checkbox means executed acceptance, not only code written.
Keep uncertain start identities. Never reclaim history by permitting an old start key
to launch again. Keep host execution independent of client and connector lifetime.
Changes stay in the existing crates. Additive protocol changes need schema updates
and compatibility checks. Do not silently truncate a list and describe it as complete.

## Step 1 — Establish scope and external gates

Actions:

- Inspect the current implementation and retain the existing user changes.
- Record the Linux release journey and the unresolved external checks.
- Check whether this environment has enterprise credentials and a real user-service manager.

Acceptance:

- [x] The goal and ordered implementation steps exist before product changes.
- [x] Available and unavailable external checks are recorded with evidence.

Real Entra, the corporate proxy, user logout/login, production fault domains, and a
human dogfood cohort cannot be replaced with synthetic tests. Missing access does not
block independent code repairs. It does block claims that those release gates passed.

## Step 2 — Repair session inventory and runtime lifecycle

Actions:

- Add bounded, cursor-based session history reads through the owning layers.
- Keep live process limits separate from historical record counts.
- Read archived metadata from durable storage without loading all history into memory.
- Preserve same-key lookup and parameter-conflict behavior after archival and restart.
- Ensure connector updates and approval discovery do not require one unbounded history frame.
- Keep the TUI and CLI able to navigate history without hiding current sessions.

Acceptance:

- [x] More than 4,096 completed records do not prevent a new start.
- [x] Page boundaries preserve scope, order, and complete traversal.
- [x] Active sessions remain discoverable with a large history.
- [x] Original keys still reconcile after restart; conflicting parameters do not launch.
- [x] Recording and approval state remain independent of metadata eviction.

## Step 3 — Repair client journal lifecycle

Actions:

- Keep a compact immutable identity journal; query native authority for confirmed results.
- Retain immutable key/fingerprint records for conflict detection.
- Bound pending work and recovery pages rather than lifetime successful starts.
- Preserve private permissions and crash-safe persistence.
- Support existing journal files without discarding uncertain submissions.

Acceptance:

- [x] Successful historical starts do not exhaust pending-start capacity.
- [x] Cancellation and lost acknowledgments preserve the original identity.
- [x] Conflicting reuse remains rejected after journal maintenance and restart.

## Step 4 — Complete allowed-root discovery

Actions:

- Define an authorized, read-only operation that returns the configured allowed roots.
- Route it through the existing gateway and host authorization checks.
- Start the creation picker at the root selection when no valid cwd is known.
- Preserve host and native directory enforcement, including symlink checks.

Acceptance:

- [x] A machine with no sessions and a restricted root supports first-session creation.
- [x] Multiple roots are selectable.
- [x] Unauthorized callers cannot discover roots; outside-root browsing stays denied.
- [x] Older peers fail explicitly or retain a documented manual-path fallback.

## Step 5 — Verify the complete supported journey

Actions:

- Add focused regressions for history, journals, discovery, and compatibility.
- Run workspace tests, formatting, Clippy, boundaries, and generated-schema checks.
- Run the available process/TUI smoke checks.
- Run available composed fixtures where dependencies support them.
- Record external checks as unrun when their prerequisites are absent.

Acceptance:

- [x] Required local checks pass, with ignored tests counted separately.
- [x] Disconnect and reattach preserve execution and terminal restoration.
- [x] No fix weakens start deduplication, scope checks, approvals, or recording.
- [x] Evidence states the exact tested configuration and remaining release gates.

## Step 6 — Simplify touched paths and publish operating status

Actions:

- Extract cohesive handlers where the repairs would otherwise enlarge dense dispatch loops.
- Reuse existing UI helpers; do not introduce a generic handler framework.
- Update CLI/API documentation, the review findings, and task progress.
- Document the next real-environment pilot and measurements: first-session success,
  reattach success, approval latency, total host memory, and recording volume.

Acceptance:

- [x] Documentation describes implemented behavior and compatibility limits.
- [x] The final status separates code completion from external qualification.

## Contract decisions and executed evidence

2026-10-26: The environment status reports connected, current observations, and no
configured enterprise secrets, runtime variables, or outbound identities. Real tenant
validation is unavailable through this environment configuration. Product implementation
has not started at the time this plan was written.

Before step 2 implementation: SQLite remains the start-key authority. Historical
metadata stays durable and is loaded on demand. The resident map owns active/recent
execution state; finished entries can leave that map only after their metadata is
persisted. Add explicit session pages and single-session reads. Legacy complete-list
operations retain their explicit overflow error. New clients use bounded pages.
Live inventory excludes terminal history; history remains separately navigable.
Pages use stable ID ordering and count/byte bounds. They are traversal cursors, not a
transactional snapshot across concurrent starts. Restart a traversal to see inserts
before its cursor. Connector live updates must not wait for a complete history scan.

Before steps 3 and 4 implementation: Replace lifetime file-count limits in client
journals with an indexed SQLite identity journal. Retain keys and parameter digests;
keep legacy files as migration backups. Bound storage in bytes and recovery reads in
pages. Uncertain identities are never evicted to make room. The root-discovery API
returns at most the host's configured 16 roots after the same browse authorization.
The picker presents a root selection and retains manual path entry for older peers.

Before protocol compatibility repair: Add `session.updates` for partial host metadata.
Keep `session.snapshot_list` as the legacy complete-list contract. Negotiate paged
metadata support. Send partial events only to clients that negotiated it. Older clients
can request complete lists and receive explicit overflow errors. New native connectors
require paged metadata support from the gateway and session owner.

Before query-plan repair: Use separate live and history query predicates so live reads
can use partial indexes. Add PostgreSQL migration 0016 for owner/machine/ID traversal
and live rows. SQLite migration 0007 supplies the matching native indexes.

Implementation decision for step 3: A second client-side pending/confirmed state machine
was unnecessary. One compact indexed identity journal preserves every key. Native lookup
remains the authority for launch outcomes. This removes the lifetime file-count cap without
adding another reconciliation state. The database has a 64 MiB physical page budget.

Pilot sequence after local acceptance: First verify real tenant login and the corporate
proxy. Then verify user logout/login with the actual user manager. Next run the complete
journey with a small developer cohort. Measure first-session success, reattach success,
approval latency, total host memory, and recording bytes per session-hour. Keep expansion
work deferred until these results and an operator response to failures are recorded.

Before warning-persistence repair: Finished runtime state must not be the only copy of
a recording or replay warning. SQLite schema 7 stores the warning with session metadata.
Archival and restart preserve it independently of the recording worker's lifetime.

Composed validation found a navigation race: a background page refresh could consume the
busy slot when the user pressed `/`. The view then interpreted filter text as commands.
Before the repair: explicit navigation must cancel pending read jobs. Background refresh
work must not discard filter, history, or approval navigation keys.

Before supervisor cleanup: Move completed-thread reclamation from the PTY-only path into
the shared start lifecycle. Managed Codex starts must release finished supervisor handles
without requiring a later shell start. Join only threads that have already finished.

## Executed verification — 2026-10-26

The test environment is Linux x86_64, Rust 1.99.1, Docker 28.4.0, PostgreSQL 17.11,
Redis 7, and pinned Codex 0.159.2. Codex uses the existing synthetic provider fixture.
The selected environment reports no enterprise credentials. `systemctl --user
is-system-running` reports `offline`.

| Check | Result |
| --- | --- |
| `cargo test --workspace --locked` | 186 passed; 17 ignored; no failures. |
| `cargo test -p qgenie-remote-sessiond --features managed-codex --locked` | 66 passed; 8 ignored; no failures. |
| Workspace check and Clippy, all targets | Passed; Clippy treats warnings as errors. |
| Managed sessiond Clippy, all targets | Passed. |
| Formatting, dependency boundaries, all three schema checks, diff check | Passed. |
| Gateway PostgreSQL/Redis/TLS integration | Passed; includes 4,100 history records, live reads, scope checks, and migrations. |
| Actual host connector integration | Both tests passed; includes roots, replacement, refresh, and revocation. |
| `tests/e2e/local_tui_smoke.py` | Passed; includes detach, reattach, native survival, SIGTERM, and terminal restoration. |
| `tests/e2e/history_capacity_smoke.py` | Passed; 4,100 seeded rows, daemon restart, full CLI output, cursor pages, original key, and a new live child. |

The new component regressions also cover byte-limited pages, historical recording
warnings, finished supervisor cleanup, 4,101 remote identities, legacy journal import,
conflicting reuse, and reclaimed recording epochs. Existing cancellation and lost-reply
regressions still pass. Component tests exercise two roots and denied callers. The
composed TUI exercises root selection without a selected session.

Initial checks exposed obsolete file-journal assertions and missing test imports. Those
were corrected. The composed test exposed two product issues: root discovery discarded
a valid selected cwd, and background reads could drop the filter key. Both were repaired.
No failed attempt is counted as passing evidence.

Operational limits: session history remains durable and needs disk monitoring. The repair
does not delete replay or recording history. ID cursors are not chronological snapshots.
Filters apply to the displayed page. Legacy complete-list operations can still return
explicit overflow errors. The client identity database has a 64 MiB page budget. See the
[upgrade procedure](../docs/integration/remote-commands.md#inventory-pages-and-upgrade-order)
for protocol negotiation, migrations, and the shared-client-state restriction.

The final composed command passed with `QGR_CODEX_BINARY` selecting Codex 0.159.2:

```sh
python3 tests/e2e/recording_durability.py --report /tmp/core-journey-report.json
```

The [sanitized result](../docs/validation/core-journey-2026-10-26.json) records all phases.
The fixture starts with an empty native inventory, discovers the restricted allowed root,
and launches the first session there. It also exercises shell and agent TUI creation,
controlled input, approvals, reattachment, confirmed termination, historical resume,
redacted recording, lost acknowledgments, and preservation through connector replacement.
Three actual managed start/owner crash proofs passed. The final durability phase passed
paused replication, duplicate receipt reconciliation, primary-volume loss, held content,
audits, quarantine, and recovery-epoch checks. These are owned fixtures on one Linux host.
They do not establish real tenant access or production fault-domain durability.

All six implementation steps are complete. The next work is the external pilot sequence
above. Real Entra, the corporate proxy, real logout/login, production fault domains,
macOS, and a human dogfood cohort remain unrun. No production deployment was performed.
