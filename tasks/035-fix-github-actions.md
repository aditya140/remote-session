# Fix GitHub Actions

Status: In progress.

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
Final hosted results remain pending.


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
unrelated live owner. Kernel process
descriptors prevent PID reuse from redirecting a signal. The traversal uses `/proc`
parent identities because some supported test kernels omit the optional `children`
files. All 19 adapter unit tests and 75 session-owner tests pass. The latter
includes actual managed-agent and process-crash checks; the two child-only test
entry points run through their parent fixtures. Linux and macOS cross-target
Clippy checks pass. Privileged and already reparented external processes remain
outside this proof.

## Hosted evidence

Implementation commit: `0d578cd63c3a13cd763800d2fb4781188452b304`.

- [macOS job](https://github.com/aditya140/remote-session/actions/runs/41222097362/job/124598646703):
  all workspace, feature, schema, Clippy, PTY, recording, CLI, TUI, and history checks pass.
- [Specification workflow](https://github.com/aditya140/remote-session/actions/runs/41222097496):
  all laws, mutation checks, and Rust comparisons pass.
- Linux passes. The database job exposes the native cleanup failure above.
  The deployment job also passes in the
  [Rust workflow](https://github.com/aditya140/remote-session/actions/runs/41222097362).

These checks do not qualify real enterprise identity, native user services,
production fault domains, or the pilot workload. Those release gates remain open.
