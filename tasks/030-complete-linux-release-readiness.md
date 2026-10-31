# 030 — Complete the Linux release journey

Status: in progress. Overall completion is the goal.
Authorized by user direction on 2026-10-26.

## Goal and completion rule

Complete the supported Linux journey from first enrollment through repeated daily use.
Keep the selected scope: one owner, shell sessions, and pinned Codex. Task 029 repaired
the core journey. This task completes the remaining implementation and qualification.
Do not add platforms, adapters, or a deployment framework to close a checklist.

Overall completion requires executed evidence for each applicable requirement. A passing
fixture establishes its stated boundary. It does not establish enterprise deployment,
real user services, or human pilot success. Keep external gates open until they run.
Preserve all existing working-tree changes. Document contract changes before editing code.

## Ordered steps

1. Establish one current completion record. Link the earlier evidence and list remaining
   work. Separate code repairs, reproducible checks, and external qualification.
2. Simplify native request dispatch. Extract concrete connection and administration
   handlers. Make capability checks readable. Preserve authorization, errors, resource
   limits, independent process ownership, and committed shutdown after a lost reply.
   Check capability denial through the actual IPC boundary.
3. Close regression coverage gaps. Run the new history-capacity smoke check in CI.
   Inspect whether CI actually exercises the pinned managed agent. State any missing
   prerequisite instead of presenting an optional skipped phase as full coverage.
4. Resolve the host-footprint finding with measured evidence. Compare the deployed
   release build with the earlier debug-build baseline. Measure both native and connector
   processes over a useful idle interval. Fix demonstrated avoidable costs. Keep an unmet
   target open. Do not weaken recording or combine process owners to reduce the number.
5. Run checks for the completed changes. Record exact builds, commands, results, and
   limits. Update the review and the current completion record.
6. Qualify enterprise access and real Linux services. Then run a small developer pilot
   through creation, approvals, disconnect, reattach, termination, and repeated history.
   Use the external gate document for identity, proxy, logout, fault-domain, and operating
   prerequisites. This step requires an appropriate deployment and assigned operators.

## Acceptance

- [x] The ordered plan exists before this task's product changes.
- [x] Native dispatch has readable capability checks and concrete handlers.
- [x] Actual IPC denies requests that lack required capabilities.
- [x] CI includes historical-capacity regression coverage and accurately identifies agent coverage.
- [x] Host memory and idle CPU have current evidence for a release build.
- [ ] Aggregate host memory and idle CPU meet the PRD budget under representative use.
- [x] Relevant formatting, build, lint, contract, and process checks pass.
- [x] Current documentation identifies all remaining release gates and measured limits.
- [ ] Real enterprise access, Linux services, and the pilot have executed evidence.

## Existing evidence and external prerequisites

- [Task 029](029-stabilize-core-journey.md): lifecycle, journals, directory discovery,
  pagination, first-session creation, and composed shell/Codex repairs.
- [Core journey result](../docs/validation/core-journey-2026-10-26.json): actual Linux
  processes, pinned Codex with a synthetic provider, and same-host synchronous PostgreSQL.
- [Repository review](../docs/validation/repository-review-2026-10-24.md): repaired issues,
  remaining readability work, and the host-footprint finding.
- [External release gates](../docs/validation/external-release-gates.md): required access
  and evidence. The current environment has no configured enterprise credentials and no
  operating user-service manager. These checks remain unrun.

## Decisions and evidence

Before step 2: Refactor inside sessiond. Use ordinary functions and a concrete capability
check. Do not add handler traits, registration tables, or another state machine. Keep
capability checks before dispatch. Keep long-lived change subscriptions and bounded
recording reads separate from ordinary requests. Preserve all current capability names.

Before step 4: The previous footprint sample used debug binaries and a two-second CPU
interval. It is evidence about that build, not the release binary. Compare build profiles
before changing architecture. Report aggregate RSS for both host processes in bytes;
the PRD threshold is 30,000,000 bytes and 0.5 percent of one CPU core.

During step 2 review: The connection future owns admission permits while a blocking
native operation runs. Aborting that future can release capacity before the work ends.
Move these permits into the blocking worker. Perform capability admission before recording
capacity admission. Keep database-backed session-kind lookup off the async executor.

Before step 3: CI must install the exact supported Codex version and require its composed
phase. Keep the optional shell-only runner for local diagnosis, but add an explicit
require-managed switch for acceptance. Include the actual native adapter contract and
the history-capacity smoke check. Version checks must fail before claiming agent coverage.

Before resource instrumentation: Reuse the composed fixture. It already owns the real
session daemon and connector. Allow an explicit built host binary. Sample both processes
for 60 seconds after PTY completion with no active sessions. Verify process identity and
recording-enabled authority before and after the sample. Include a binary hash and exact
threshold results in the sanitized report. Keep load-generator and gateway costs separate.

The first release-build composed attempt stopped in the TUI harness before resource
measurement. The harness treated a partially rendered creation header without a busy
footer as ready. It sent Enter before directory discovery completed. Before repairing
the harness, require the explicit directory-ready message before sending that key.

The user added BeautifulRust to this task. Install the three upstream skills at a pinned
revision in `.agents/skills`. Read the review and best-practice guidance. Run a pedantic
lint audit and library documentation check. Write a prioritized review with concrete
locations, named rules, and fixes. Preserve the current product requirements when a
general style rule conflicts with a deliberate process or wire contract.

Before the SDK boundary repair: Validate the public 1..=256 page limit before selecting
the older-gateway fallback. Compatibility must not accept invalid arguments. Document
the fallback's complete-list behavior and the original-key cancellation contract.

The release sample passes the composed functional proof, but measures 32,579,584 bytes
and 1.72 percent CPU across both host processes. Before the idle repair: replace the
recording worker's five-millisecond empty polling with explicit wakeups for capture,
commands, completion, and shutdown. Keep timed flush and recovery work. Wait for native
change notifications between connector reads, with the existing three-second metadata
refresh and scheduled history traversal as reconciliation deadlines. Keep the native
suspend-aware authorization checks unchanged. Repeat the same release measurement.

The npm Codex launcher forks the native executable. A successful version probe does
not establish process ownership. Before the ownership repair: on Linux, require the
native socket peer PID to equal the launched child PID on connection and reconnection.
Continue to accept wrappers that use `exec`. On explicit cleanup, signal the private
process group before reaping its leader. This also cleans a rejected forking launcher.
Use only the still-owned child's ID; never adopt a PID from disk or from a replacement
socket. Test rejection and cleanup with an actual native executable before the repair.

The memory review found a second lifetime limit: the recording worker retains every
sealed encoder and refuses registration after 4,096 streams. Before this repair: release
sealed encoder state after durable sealing and queue drain. Keep epoch identity and closed
state in SQLite. A repeated registration for a sealed epoch must return a closed capture,
as before, and must not create a new encoder or recording sequence. Keep the 64 active
stream bound and the durable outstanding-work bound. Prove 4,100 completed registrations
through the actual worker without a restart, then verify a new capture and old-epoch denial.

## Repairs completed

- Native dispatch now uses concrete handlers and one capability check. Four real IPC
  regressions cover capability denial, cancellation during database work, and lost replies.
- CI runs the seeded history-capacity check and requires the pinned native Codex phases.
  The local installation uses the same npm package and actual native executable as CI.
- The connector consumes bounded active pages and preserves publication frame bounds.
  Its large-record regression failed before the repair. Scope and cursor rejection pass.
- The SDK validates page bounds before the legacy fallback and documents start cancellation.
- Linux Codex connection and reconnection verify the socket peer PID. Explicit cleanup
  signals only the still-owned process group. The launcher regression failed before the
  repair. Five actual pinned adapter proofs pass afterward.
- Recording work uses explicit wakeups. Idle inventory uses notifications and reconciliation
  deadlines. No recording or authority requirement was removed.
- The worker releases durably sealed encoders. Its lifetime regression failed on registration
  4,097 and now completes 4,100 lifecycles plus a new recorded stream. The managed session-owner
  suite passes after this repair.

The [BeautifulRust review](../docs/validation/beautiful-rust-review-2026-10-26.md) records
these findings and the lower-priority API, TUI, and diagnostic recommendations. The three
upstream skills are installed at a pinned revision. All nine installed source files match
their upstream Git blob hashes. The port skill is installed but was not needed for this task.

## Final local evidence and remaining work

The [final Linux result](../docs/validation/linux-completion-2026-10-26.json) passes the
public session pipeline, pinned Codex, managed-start crash boundaries, and synchronous
primary-volume-loss proofs. The release binary also passes the real-keyboard local TUI
smoke and 4,100-record history-capacity check. The runner cleaned its owned Docker resources.
No task-owned native app-server remained running after the final checks.

Formatting, workspace/all-target checking, strict Clippy with default and managed feature
sets, API documentation, dependency boundaries, and all three schema checks pass.
Workspace tests report 193 passed and 18 ignored. The managed session-owner suite reports
71 passed and 8 ignored. The actual native adapter suite explicitly runs five ignored
proofs and passes all five. The composed runner separately executes its ignored proofs.
The [review](../docs/validation/beautiful-rust-review-2026-10-26.md) records commands and limits.
GitHub Actions itself was not run in this environment.

Overall completion remains open. The final 60-second release sample measures 32,526,336
bytes aggregate RSS and 0.5667 percent of one core. Both exceed the PRD thresholds.
Idle CPU fell from 1.7167 percent in the earlier release sample. The
[capacity report](../docs/validation/capacity.md) preserves both measurements and the memory
diagnostic. Profile remaining timer work and resident code pages before another repair.
Do not merge process owners or weaken recording to satisfy this number.

Real enterprise credentials, an operating Linux user-service manager, and an assigned
pilot host remain unavailable in this environment. The deployment-location question is
pending. These conditions prevent the real-environment qualification step. API mode types,
TUI transitions, and safe diagnostic categories remain ranked follow-up recommendations;
they are not claimed as implemented.
