# Implement the Herdr follow-up issues as separate pull requests

Status: In progress. Authorized on 2026-10-31.

## Goal

Deliver one reviewable pull request for each of GitHub issues 1 through 8.
Work in issue order. Use a stacked branch when a change depends on an earlier
pull request. Keep each diff limited to its issue. Do not merge the pull requests
as part of this task.

## Sequence

1. Preserve the native prompt turn ID through a correlated receipt. Add exact
   send-and-wait behavior, capability checks, and failure/recovery tests.
2. Add local launch profiles and durable, scoped session names and labels. Reuse
   existing launch validation and stable session identities.
3. Add a bounded work inbox across machines. Keep task outcome, process lifetime,
   approvals, freshness, and incomplete coverage distinct.
4. Add fresh-output and explicit any/all target waits. Reuse observation and
   exact task receipts. Do not introduce implicit task creation or cancellation.
5. Generate command documentation and check released interface compatibility.
   Retain current schema/conformance checks and measure added CI work.
6. Audit the requested reliability scenarios. Add focused tests for gaps and fix
   defects demonstrated by those tests. Preserve existing bounded queues and
   authority fences.
7. Complete the deferred agent-coordination research as a design PR. Record
   evidence, missing pilot evidence, lifecycle decisions, and implementation gates.
8. Complete the deferred native-owner replacement research as a design PR. Record
   the handoff state, failure cases, old/new binary proof, and a go/no-go decision.

## Verification and delivery

- Read the issue and affected contracts before each change.
- Record the issue-specific design and acceptance checks before implementation.
- Run focused behavior tests, formatting, and relevant compilation/lint checks.
- Run broader checks when an interface or dependency change requires them.
- Generate affected schemas and document unsupported or unknown outcomes.
- Commit, push, and create each pull request with its issue and test evidence.
- State the base PR for dependent changes. Keep research gates explicit.
- Check GitHub CI and repair failures before declaring delivery complete.

The local development stack and its native demo session belong to the previous
deployment proof. Preserve them. Use isolated test state and disposable services
for implementation checks.

## Issue 1 design

- Add a distinct prompt-with-receipt input and an optional turn ID in its reply.
  Negotiate support at the client, gateway, connector, and native owner.
  Reject unsupported receipt requests before the native mutation.
- Preserve the ID from the native `turn/start` response. Do not infer the ID
  from observed activity. Store it with the existing input acknowledgment in
  one SQLite statement. Duplicate input returns the same receipt.
- Keep the existing prompt API for older peers. The new CLI prompt command
  requests a receipt. Add `--wait` and a bounded timeout. Release control before
  the existing read-only exact-turn wait. Print the receipt before waiting.
- Keep uncertain input uncertain. Do not resubmit it or guess its turn ID.
- Test receipt persistence, duplicate input, old peer refusal, exact correlation,
  and fast completion. Generate the wire schema and run applicable checks.

The SQLite migration adds a nullable receipt column. The previous native owner
rejects the new schema version. Downgrade requires a pre-migration state backup.
The network protocol remains additive for clients that use the old prompt API.

### Issue 1 implementation and evidence

The receipt now passes through the native owner, SQLite input journal, connector,
gateway, SDK, and CLI. The CLI releases control before the exact-turn observer.
An interrupted wait emits `cancelled` and leaves native work running.

Local Linux checks completed:

- Formatting, the public schema check, and the all-target workspace check passed.
- Clippy passed for the default workspace and the managed Codex application.
- The workspace run passed 222 tests and skipped 19 environment-gated tests. The
  new conformance fixture initially omitted the protocol `body` wrapper. After
  correction, all 11 tests in that suite passed. No runtime test failed.
- The managed session owner suite passed 71 tests. Eight gated tests were skipped.
- The explicitly selected pinned Codex owner test passed. It checks the exact
  completed turn ID, duplicate receipts, and a single native application.
- The full CLI suite passed ten tests. It checks fast completion, concurrent
  activity, observation recovery, old peers, lost or missing receipts, and SIGTERM.
- The isolated composed proof passed all five phases. They cover the synchronous
  database pair, the public session pipeline, managed Codex, native start failure
  boundaries, and loss of the primary database volume. The public pipeline includes
  the new CLI send-and-wait flow through the real connector and gateways.

The composed report is local at `target/validation/issue-1-native.json`. Native
platform evidence is Linux only. Hosted CI remains a separate delivery check.

## Issue 2 design

- Save at most 64 private local launch profiles. Each profile binds an HTTPS
  gateway origin and stable machine ID to a directory and shell argv or agent
  task. Reject unknown fields. Do not store authentication settings in a profile.
- Run a profile through the existing start journal and launch path. Print the
  original start key. Recheck current ownership, allowed roots, and agent support.
  A profile is a convenience setting, not a permission boundary.
- Keep session names and labels in one PostgreSQL metadata column. Native host
  projections cannot overwrite it. Use bounded, audited owner mutations. Keep
  session IDs, launch keys, recordings, and native process state unchanged.
- Resolve names by exact gateway/owner/machine scope. Reject duplicate names.
  Add CLI management and filtering. Show and filter metadata in the TUI.
- Negotiate metadata mutations. Default missing metadata to empty for old records
  and clients. Add the PostgreSQL migration and update the generated schema.
- Test profile persistence, invalid imports and scope mismatch, existing launch
  recovery, metadata authorization and persistence, ambiguous names, host updates,
  and CLI/TUI presentation. Keep all database tests separate from the local demo.

### Issue 2 implementation and evidence

Profiles now use private local storage and the existing native start identity.
Names and labels use one audited PostgreSQL column. Host projections cannot
replace this metadata. CLI and TUI listing and filters include the metadata.

The isolated gateway suite passed all 26 selected tests. It checks concurrent
label updates, owner and machine scope, ambiguous names, host projection updates,
and unchanged session identity. The real CLI profile tests passed. They check
file permissions, strict profile fields, invalid input, scope mismatch, and
explicit replacement. The client test checks old-peer refusal before any frame
is sent and reads a session record without the new metadata field.

The full workspace suite, workspace Clippy, managed application Clippy,
formatting, and all three schema freshness checks passed. The composed report
at `target/validation/issue-2-native.json` passed all five phases. It includes a
real profile launch, reconciliation by the original launch key, and CLI metadata
changes read through the SDK. The composed run took 160 seconds on this Linux
workspace. It does not establish production or native macOS qualification.

PostgreSQL schema 17 requires the documented migration procedure. The original
local development deployment and its native demo session remain unchanged.

## Issue 3 design

- Add a work inbox to the existing TUI. Read the current owner machine inventory,
  session pages, typed agent checkpoints, and approval summary. Add no new server
  index, lifecycle authority, or wire operation.
- Read at most 16 sessions or 16 machines per page. Continue by stable machine
  and session IDs. Run one native read at a time. Bound each checkpoint and page.
- Keep lifetime, typed task activity, approvals, recording warnings, and freshness
  separate. Failed turns remain actionable in running native processes. Partial
  scans and unavailable activity cannot establish that no work needs attention.
- Keep completion acknowledgments in the open gateway TUI instance. Scope each
  key to owner, machine, session, and exact successful turn. Limit the set to
  4096 keys. Preserve it through navigation and reconnects. Clear it on TUI exit.
  It cannot suppress failures, approvals, recording warnings, or unknown state.
- Use the existing approval view and session/history navigation. Each page owns
  a separate observation connection. Cancellation closes owned reads and never
  terminates native work or repeats a mutation.
- Test multiple machines and pages, failed turns in live processes, scoped
  completion acknowledgment, incomplete approval coverage, and interrupted reads.

### Issue 3 implementation and evidence

The TUI work inbox now scans owner sessions across machines and pages. It keeps
process state, typed task outcome, approvals, recording warnings, and observation
coverage separate. Session/history and approval navigation reuse the existing
views. Completion acknowledgments remain local to the open gateway TUI.

All 41 client tests and all six TUI tests passed. The new WSS tests cover two
machines and multiple pages, a failed turn in a running process, incomplete
approval coverage, exact and scoped completion acknowledgments, duplicate
acknowledgments, and connection loss. The new focused tests took 0.22 seconds.
Workspace Clippy, formatting, and the Python syntax check passed.

The five-phase Linux deployment proof passed. The real keyboard test opens the
work inbox, displays scanned sessions, selects an entry, and reads its protected
history. Existing native approval, controlled prompt, start recovery, and durable
recording proofs also passed. The local report is
`target/validation/issue-3-native.json`. No production qualification is claimed.

## Issue 4 design

- Add `wait-output` for a bounded literal after an explicit event cursor. Parse
  retained PTY events from zero to establish escape parser state, but discard
  text at or before the selected boundary. Never match checkpoint screen text.
- Reuse the bounded VTE parser and output cursor. Keep parser and match state
  through reconnects. Report gaps, missing parser history, and resource limits.
  Limit literals to 512 bytes and total decoded output to 8 MiB.
- Add `wait-many` over 1 to 16 explicit machine/session/turn or exit targets.
  Reject duplicates. Poll at most two targets concurrently and visit all pending
  targets in each round. Reuse typed outcomes and approval summaries.
- Define any as the first resolved outcome, including failure or attention.
  Define all as all resolved outcomes. Preserve exact per-target outcomes when
  another target times out or the user cancels. Unresolved any targets remain null.
- Give each new command one cancellation handler. Emit one final result. Do not
  acquire control, submit prompts, start sessions, approve work, or terminate it.
- Test old text, terminal controls, Unicode and chunk boundaries, gaps, bounded
  memory, fast completion, mixed outcomes, reconnects, and cancellation.

### Issue 4 implementation and evidence

The CLI now supports fresh literal output waits and bounded explicit any/all
waits. The client preserves parser state through reconnects. It reports gaps and
resource limits instead of claiming a match from a checkpoint. Each command owns
its cancellation handler and leaves native work running.

The workspace suite passed 238 tests with 19 environment-gated tests ignored.
This run used two test threads. An earlier default-parallel run exposed a spool
lock release race during concurrent process creation. Issue 6 will add a focused
regression and fix. The CLI suite passed 15 tests, then the added checkpoint-only
regression passed independently. Workspace and managed application Clippy passed.
The law check passed 115 laws, rejected 32 mutations, and compared 22,712 Rust
cases. The generated comparison harness now uses the workspace dependency patches.

The complete Linux deployment proof passed all five phases in 137 seconds.
The real CLI matched native PTY output without acquiring control or stopping the
session. The fixture now accounts for both intentional observer attachments.
The local report is `target/validation/issue-4-native.json`. External release
gates remain unrun.

## Issue 5 design

- Generate help and CLI argument contracts from the real Clap definitions.
  Parse documented shell examples without dispatch or shell expansion.
- Pin the original preview revision and schema digests. Compare existing fields,
  result types, commands, and required arguments. Retain the latest three released
  baselines when releases exist. Do not claim N-2 evidence from a preview.
- Keep schema freshness and behavioral capability tests. Add negative checker
  tests so a changed contract cannot silently pass.
- Run the focused CLI test for prose-only changes. Keep deployment jobs skipped.
  Run full push CI on main and release tags. Run feature CI once per PR update.
- Test routing failures and intentional skips. Measure local warm check cost.

### Issue 5 implementation and evidence

The actual Clap definitions generate the help reference and CLI shape. The test
parsed 45 shell examples without dispatch. The checker accepted the pinned
preview baseline and rejected removed fields, changed types, new constraints,
new required values, removed commands, and renamed flags in negative tests.
The old-peer WSS test accepted the historical prompt acknowledgment without a
turn field and refused a receipt request before sending any input.

All 14 Python tool tests passed in 0.40 seconds. The warm CLI test took 0.09
seconds. The old-peer test took 0.13 seconds. Workspace Clippy and formatting
passed. CI routing tests require the focused documentation job on prose changes
and require the existing runtime jobs on code changes. Push triggers now select
main and release tags; feature PR updates have one run instead of two.

The repository has no release tags. N-2 released-client qualification remains
open. The preview revision, immutable file digests, and release procedure are
explicit. These checks do not replace real released-client or platform evidence.

## Issue 6 design

- Audit current transport deadlines, replacement generations, control leases,
  native lifetime, and recording tests. Add only missing composed scenarios.
- Reproduce the observed spool lock race with a duplicate file descriptor.
  Release the lock explicitly after SQLite closes. A descriptor inherited by a
  concurrently created child must not retain authority after its owner closes.
- Use the existing real deployment fixture for two native PTYs. Stall one SDK
  consumer until its bounded queue reports overload. Prove progress in the other
  PTY. Reconnect the first observer and close the old observer afterward.
- Verify final output before exit in native replay and protected history.
  Missing capture must be explicit. Keep all waits bounded and fixtures owned.
- Reconnect a real observing TUI and change its window size while an SDK
  controller holds the lease. Check native geometry. Then resize with the valid
  controller. There is no remote terminal theme action to exercise.
- Record existing versus new coverage and added local runtime. Do not infer
  native-owner handoff or other-platform qualification from these tests.

### Issue 6 implementation and evidence

The deterministic spool descriptor regression failed before the fix and passed
afterward. A private guard now unlocks only after SQLite closes. All nine selected
spool tests passed; two environment-specific spool tests remained ignored.

The new composed test proved progress with two real PTYs after one SDK observer
stopped reading. The bounded observer reported overload. Its replacement received
completion after the old observer closed. Native replay put final output before
exit, and protected paginated history contained the final output. This block took
6.51 seconds. Two real TUI observers resized their own windows without changing
native geometry. The current controller then resized the terminal successfully.

The first composed attempt used an invalid history page limit in the new fixture.
The fixture now uses the public four-item maximum. The final five-phase Linux
proof passed in 132 seconds: `target/validation/issue-6-native.json`. The native
pipeline phase took 83.37 seconds. The warm prior issue 4 pipeline took 71.30
seconds; this is a local comparison, not a controlled hosted CI benchmark.

The default-parallel workspace suite passed 242 tests with 19 environment-gated
tests ignored. The extended actual-process crash test then passed in 0.63 seconds.
It verifies distinct fresh lifetimes and preserved old lost or unknown records.
Workspace and managed application Clippy, formatting, 14 Python tool tests, and
the viewer script syntax check passed. The full coverage audit is
[terminal reliability coverage](../docs/validation/terminal-reliability-coverage.md).

GitHub later reported new workflow jobs as failed before any steps ran. Those
jobs have no available logs. The connector cannot read check annotations, and the
CLI could not read the run. The underlying hosted-start failure remains unknown.
These hosted failures are not reported as green local or hosted evidence.

## Issue 7 design

- Inspect Herdr parent, child, note, and wait contracts at the pinned revision.
- Compare proposed pilot tasks with the implemented receipt, profile, inbox, and
  wait workflow. Separate source evidence from missing developer usage evidence.
- Define independent session identity, association, provenance, authorization,
  cancellation, and termination. Propose bounds and recovery acceptance tests.
- Record a no-go decision for a runtime engine until a measured pilot supports
  the extra state. Keep the issue open for its stated entry condition.

### Issue 7 research result

The [coordination decision](../docs/integration/agent-task-coordination.md)
records a no-go for automatic delegation now. It includes three proposed pilot
workflows, exact identities and message provenance, explicit action authority,
proposed capacity bounds, and failure acceptance tests. Parent shutdown cannot
terminate durable children. Agent text cannot approve or grant control.

The referenced Herdr source was inspected at its pinned revision. It was not
executed. Developer pilot evidence was not supplied. The usage gate remains open;
no orchestration runtime or new crate was added. Relative documentation links and
the current CLI documentation check are the applicable checks for this PR.
