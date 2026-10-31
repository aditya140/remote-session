# Fix GitHub Actions

Status: Completed.

## Goal and sequence

Make the existing Rust, platform, deployment, and specification checks pass on hosted
runners. Preserve behavioral assertions and report each platform independently.

1. Read the failing hosted jobs and identify the first failing command in each job.
2. Replace deprecated macOS clock bindings without changing suspension or failure semantics.
3. Make private test directories independent of the runner umask. Reproduce the recording
   acceptance failure under the hosted runner permissions and rerun the composed proof.
4. Update action runtimes, pin runner images, set timeouts and permissions, and stop one
   platform failure from cancelling another platform. Add a stable aggregate CI result.
5. Validate locally, push the fixes, and inspect the new hosted runs. Resolve additional
   failures until the checks pass. Record the run links and remaining qualification limits.

## Initial evidence

Run [41214716384](https://github.com/aditya140/remote-session/actions/runs/41214716384)
failed on `c14917a`. The macOS Clippy job rejected deprecated `libc` Mach time bindings.
The PostgreSQL job failed in the recording fixture while opening a private identity.
The Linux matrix job was cancelled by fail-fast. The deployment job passed.
The separate [specification run](https://github.com/aditya140/remote-session/actions/runs/41214716393)
also passed.

## Changes and local evidence

- The macOS elapsed clock uses `mach2` bindings. The dependency is optional and
  macOS-only. The clock still counts sleep and rejects a failed or backward clock.
  A native smoke test checks that the clock advances. The macOS cross-target
  Clippy check passes with warnings denied.
- The recording failure reproduces with `umask 0022`. Private identity, native
  state, and start-journal fixtures now request `0700` when they create directories.
  Product permission checks remain in place.
- The first hosted rerun passes macOS Clippy and then exposes the same fixture
  defect in saved setup. Inspection also found it in client start-journal and
  native approval tests. Those fixtures now use explicit private permissions.
- The next macOS run exposed a Linux-only hostname read in `host setup`.
  Setup now calls `gethostname` and checks the returned bytes. It no longer needs
  the Linux `/proc` filesystem to create a host configuration.
- The native Codex test used a one-second timer to detect an escaped tool.
  A deliberate runner delay reproduced a false failure: the marker already existed
  before termination. The fixture now releases the tool only after termination.
  It also tests a delay longer than the old timer. The ordinary sandboxed case
  passes locally with the same binary hash as the npm fixture used by GitHub.
  Hosted CI still found a real escape. An explicitly approved unsandboxed tool
  reproduces it locally because the tool starts a separate process group.
  Two native shutdown signals also leave that descendant running.
- The foreign-user IPC proof assumed that its first write would succeed. The server
  can reject the foreign peer before that write. The fixture accepts that disconnect
  and retains the independent server-side and client-side permission assertions.
  Its Docker harness now uses the local daemon explicitly and builds with `--locked`.
- The macOS PTY burst reached the five-second fixture deadline and cleanup then
  stalled. Native phase output later measured 4.83 seconds for a passing 512 KiB
  burst. The bulk-read fixture now allows 30 seconds. All byte-count, content,
  frame-size, resize, and subsequent-input assertions remain. Cleanup continues
  draining output while it reaps the terminated child and has a ten-second limit.
  The workflow bounds the adapter lifecycle step to four minutes.
- The macOS roots fixture compared canonical output with `/var` input paths.
  macOS resolves those paths through `/private/var`. The assertion now compares
  canonical paths, as the filesystem contract requires.
- The inherited-terminal recording fixture assumed Linux EOF behavior. On macOS,
  session-leader exit closes the terminal stream despite the inherited descriptor.
  The fixture requires complete captured output on macOS and retains the durable
  recovery-loss assertion on Linux. Workspace tests use `--no-fail-fast` so a
  failing crate does not hide results from later crates.
- A later database run missed the offline update after a host disconnected.
  Background presence refresh can replace the bytes that disconnect cleanup uses
  for comparison. Cleanup can also race a refresh already in flight. The repair
  removes local registration first, clears by the unique connection generation,
  and clears a late refresh if its connection is no longer registered. A Redis
  regression check preserves a newer connection while clearing old refreshes.
  The full PostgreSQL/Redis integration test passes locally with this change.
- The malformed-frame CLI test reproduced an extra reconnect locally. A command
  can observe the transport queue closing before the SDK records its protocol
  failure. The repair records the terminal transport result before returning
  that command error. The deterministic regression fails without the repair.
  All 38 SDK tests, seven CLI workflow tests, and 100 repeated malformed-frame CLI
  invocations pass. Workspace Clippy and the specification suite also pass.
- Workflow actions use verified commit pins and Node 24 runtimes. Jobs use fixed
  runner images, timeouts, read-only tokens, and independent matrix results.
  Superseded runs cancel. Dependency caches survive test failures. The `CI` job
  requires every Rust workflow job to pass. Both workflows run for every change.
- Actionlint and the Python tool tests pass. The specification suite passes all
  115 laws, 32 mutation checks, 18,432 authorization comparisons, and 4,274 SDK
  comparisons. The checked-in report is refreshed after the final source changes.

The first local recording rerun reached a later correlated-response timeout while
another build used the same machine. A separate run passed the session workflow,
native Codex approval delivery, managed start crash checks, and primary-volume
loss. The [sanitized result](../docs/validation/ci-recording-2026-10-31.json) records
all five passing phases under `umask 0022`. The timeout did not reproduce in that run. The test assertions and deadlines were not changed.
The repair-branch run passes all jobs. A fresh run on `main` exposed another
intermittent routing-fixture failure, described below.

## Native cleanup repair plan

1. Keep the approved unsandboxed tool as a regression. Release its marker gate
   only after termination. Check sandboxed tools separately.
2. On Linux, retain kernel process descriptors for the owned tree. Stop each
   parent before discovering its children. Validate each child against that
   retained parent. Bound the traversal by time and process count.
3. Check the original authorization before each signal. Terminate children before
   the app-server. Resume any processes stopped by an incomplete attempt. Do not
   signal saved or unverified process IDs.
4. Verify denied and cancelled requests, protocol disconnection, native cleanup,
   and owner reaping. Keep macOS native-agent qualification open.
5. Run hosted CI again and publish the complete results.

The Linux repair passes all seven adapter integration tests, including both native
sandbox modes and failed-startup reaping. The unsandboxed test also disconnects
native IPC before it requests termination. Three owner tests check denied authority,
partial-stop recovery, preservation of an existing stop, cancelled callers, and an
unrelated live owner. Kernel process descriptors prevent PID reuse from redirecting
a signal. The traversal uses `/proc`
parent identities because some supported test kernels omit the optional `children`
files. All 19 adapter unit tests and 75 session-owner tests pass. The latter
includes actual managed-agent and process-crash checks; the two child-only test
entry points run through their parent fixtures. Linux and macOS cross-target
Clippy checks pass. Privileged and already reparented external processes remain
outside this proof.

## Initial hosted evidence

Verified implementation commit: `00a27fe3d857400bb970ce36ed41b5f949e4b601`.
Documentation commit `d286917` changes this report and the progress record only.

| Check | Result |
| --- | --- |
| [Linux](https://github.com/aditya140/remote-session/actions/runs/41223632662/job/124603131386) | Pass. Workspace, features, Clippy, schemas, CLI/TUI/history, foreign-user IPC, and full recording storage. |
| [macOS](https://github.com/aditya140/remote-session/actions/runs/41223632662/job/124603131309) | Pass. Workspace, features, Clippy, schemas, native PTY, recording, CLI/TUI, and history. |
| [Database and native integration](https://github.com/aditya140/remote-session/actions/runs/41223632662/job/124603131085) | Pass. PostgreSQL/Redis, enrollment, revocation, recording durability, native Codex, and transaction pooling. |
| [Development deployment](https://github.com/aditya140/remote-session/actions/runs/41223632662/job/124603131329) | Pass. Image build, monitoring, smoke, baseline, restore, and fault recovery. |
| [Aggregate CI](https://github.com/aditya140/remote-session/actions/runs/41223632662/job/124606378960) | Pass. Every required Rust workflow job succeeded. |
| [Specification laws](https://github.com/aditya140/remote-session/actions/runs/41223632599) | Pass. All laws, mutation checks, and Rust comparisons. |

The run preserves the [synchronous acceptance report](https://github.com/aditya140/remote-session/actions/runs/41223632662/artifacts/13051576346)
and the [specification report](https://github.com/aditya140/remote-session/actions/runs/41223632599/artifacts/13050499767).
The artifacts have the workflow's 14-day retention period. Job logs also record the results.

These checks do not qualify real enterprise identity, native user services,
production fault domains, or the pilot workload. Those release gates remain open.

## Main-branch recovery follow-up

The fresh `main` run on `d286917` failed while waiting for an attach request at a
replacement host after Redis publisher loss. Host `ConnectionReady` confirms local
registration. It precedes route-hint publication, which can recover independently.
The fixture sent its new cross-node attach before it observed that discovery state.

Repair sequence:

1. Reproduce a delayed route hint after the replacement host registers.
2. Make the fixture wait for the public online inventory before it sends the one
   attach request. Preserve the deadline and avoid replaying an uncertain request.
3. Check the full PostgreSQL/Redis proof and repeat it. Run hosted CI again.

A controlled missing hint and 750 ms Redis write pause reproduces an immediate
`NotFound` reply before host dispatch. This pause is below the existing two-second
broker budget. With the public-presence wait, the same full routing proof passes
in 68.24 seconds. The test sends one attach request and keeps its eight-second
forwarding deadline. A concurrent client read now reports an early error directly.
Gateway Clippy also passes. Two fresh runs of the exact CI database command
pass all 26 tests in 68.72 and 69.03 seconds. The final hosted run below also passes this fixture correction.

## TUI startup follow-up

The routing correction passes on hosted Linux. The recording job on `24688d6`
then times out while requesting control in a newly created terminal. The displayed
terminal remains in observe mode. Before changing behavior, trace the initial
session metadata, capability updates, and F2 handling. Reproduce the ordering,
repair the responsible boundary, and rerun the composed native recording proof.

A local trace confirms `Running → Accepted → Running` delivery for the new
terminal. The old `Accepted` update has `input=false`. The TUI replaces the current
capabilities and silently ignores F2 during that interval.

Repair sequence:

1. Deliver both stale startup states to the actual TLS-connected TUI. Use a visible
   ordered event as a barrier before the test sends one F2. Establish failure on
   the original code.
2. Apply lifecycle state and capabilities together. Ignore metadata from an earlier
   startup phase. Continue to apply current capabilities and terminal outcomes.
3. Remove diagnostic instrumentation. Run the CLI/TUI regressions, Clippy, and the
   composed native recording proof. Push the repair and verify hosted CI.

The ordered TLS/TUI regression fails on the original code at F2 and passes after
the repair. All seven CLI workflow tests pass. Formatting and managed-feature
Clippy pass. The complete recording proof also passes: native start and control,
approvals, managed-start crash recovery, synchronous receipts, and primary-volume
loss. The sanitized local report is `target/validation/ci-tui-recording-after.json`.

## Deployment dispatch follow-up

The deployment job on `24688d6` passes monitoring and dependency-loss recovery,
then fails its cross-node dispatch assertion. The probe calls `sessions`, which
reads the gateway's durable projection. That command does not dispatch to the host.
Earlier native operations can mask this defect when they use different gateways.

Repair sequence:

1. Use the original start-key lookup for each probe. Verify the returned session
   identity. This read requires actual native dispatch and does not replay a start.
2. Keep the measured cross-node counter, bounded attempts, and token-affinity
   changes. Preserve the deployment assertion.
3. Run Python validation and verify the actual four-gateway deployment in CI.

## Inventory byte-budget follow-up

The macOS all-feature owner tests on `18ed4f5` expose a serialized page above the
512 KiB limit. The inventory code sums each record but omits JSON array brackets
and separators. Random IDs and platform path lengths can place a page near that
boundary. The same omission exists in local and remote pages and legacy lists.

Repair sequence:

1. Make the existing large-history fixture place records exactly at the byte
   boundary. Reproduce the overflow on Linux without depending on path length.
2. Include array brackets and separators in all four inventory budgets. Preserve
   pagination cursors, page limits, and the overload result for legacy lists.
3. Run the owner tests and Clippy. Verify the hosted platform and deployment jobs.

The adjusted large-history fixture reproduces the overflow on the original code.
With the repair, all 70 non-ignored Linux owner tests pass. Five native tests retain
their existing separate qualification path. The two crash-helper entry points are
excluded from direct execution and still run through their parent fixtures.
Formatting and all-feature owner Clippy pass. The final hosted run below verifies
the repair on both Linux and macOS.

## Final main-branch evidence

Verified implementation commit: `8a005aef311f2799c0cb471029493b12f34698bd`.
Both workflows complete successfully on `main`. This final documentation update
changes only this task report and the progress record.

| Check | Result |
| --- | --- |
| [Linux](https://github.com/aditya140/remote-session/actions/runs/41227751216/job/124615451898) | Pass. Workspace, features, Clippy, schemas, CLI/TUI/history, foreign-user IPC, and full recording storage. |
| [macOS](https://github.com/aditya140/remote-session/actions/runs/41227751216/job/124615451766) | Pass. Workspace, features, Clippy, schemas, native PTY, recording, CLI/TUI, and the exact page-boundary regression. |
| [Database and native integration](https://github.com/aditya140/remote-session/actions/runs/41227751216/job/124615451821) | Pass. PostgreSQL/Redis, enrollment, revocation, recording durability, native Codex, and transaction pooling. |
| [Development deployment](https://github.com/aditya140/remote-session/actions/runs/41227751216/job/124615451692) | Pass. Image build, monitoring, actual cross-node dispatch, capacity baseline, restore, and fault recovery. |
| [Aggregate CI](https://github.com/aditya140/remote-session/actions/runs/41227751216/job/124619485928) | Pass. Every required Rust workflow job succeeded. |
| [Specification laws](https://github.com/aditya140/remote-session/actions/runs/41227751221) | Pass. Laws, mutation checks, and 18,432 authorization plus 4,274 SDK comparisons. |

The final run preserves the [synchronous acceptance report](https://github.com/aditya140/remote-session/actions/runs/41227751216/artifacts/13052418455)
and the [specification report](https://github.com/aditya140/remote-session/actions/runs/41227751221/artifacts/13053180563).
The artifacts have a 14-day retention period. The deployment log confirms the
original-key lookup crossed gateway nodes and returned the same native session.
The database job passes all 26 integration tests both directly and through
transaction pooling. It also passes all six explicitly selected native adapter
contracts and the full recording/managed-start/primary-loss proof.

No identified CI failure remains open. External release qualification remains
separate, as listed above and in the release-gate document.
