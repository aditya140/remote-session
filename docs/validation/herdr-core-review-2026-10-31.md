# Herdr Core: useful ideas for QGenie Remote

Reviewed on 2026-10-31. This is a source review, not a runtime qualification.

## Source and conclusion

The reviewed repository is [herdrdev/herdr-core](https://github.com/herdrdev/herdr-core),
revision `1d09234dac41a729d07212f70f8ef73c1e107f0f`, package version `0.1.15`.
It is cloned at `/workspace/herdr-core`, outside this repository. It uses Apache-2.0.
No upstream implementation code was copied into QGenie Remote.

Herdr Core is separate from the older Herdr terminal application. Core runs programs,
exposes terminal operations, and connects machines. It does not provide the older
application's pane layout or plugin marketplace. Do not attribute those features to Core.

Its strongest contribution is a small set of useful actions: start work, read its output,
wait for a result, see what needs attention, and reconnect. QGenie Remote already has
many supporting mechanisms. Expose them clearly before adding another runtime.

## Feature comparison

| Herdr Core function | QGenie Remote today | Recommendation |
| --- | --- | --- |
| Program-reported working, blocked, done, and error status | Session lifetime states, typed Codex turn/tool events, and an owner approval badge | Add a compact activity summary and a needs-attention view. Keep activity separate from process lifetime. |
| `read`, `wait`, and `events` commands | Raw `remote replay`, protected history, and a bounded observer SDK; no equivalent CLI wait/event commands | Add readable output and bounded event/wait commands using the existing SDK. |
| Automatic machine reconnect and viewer recovery | The host connector reconnects; the remote TUI and observer require explicit client recovery | Add automatic client observation recovery and clear sign-in-required states. |
| Named launch templates with a role and initial task | Agent picker, allowed directories, and configured Codex provider | Add small launch profiles after setup and sign-in work. Revalidate policy on every start. |
| Terminal names, labels, and combined machine addresses | Machine names/labels, stable session IDs, and per-machine session views | Add optional session names and useful filters when pilot usage supports them. Keep IDs authoritative. |
| Parent/child agent work, messages, and waits | No general agent-delegation workflow | Defer. This adds lifecycle and authority requirements beyond the current first release. |
| Core updates preserve programs through separate terminal holders | Connector updates preserve sessions; active native-owner updates are deferred | Keep our current process boundary. Reconsider deeper owner replacement only for a demonstrated need. |
| Generated command reference and released API compatibility checks | Generated schemas, current-schema checks, and protocol fixtures | Add command-document checks and a released contract baseline without changing protocols. |
| Non-PTY exec and remote file operations | These remain later PRD capabilities | Keep them later. Herdr's account-wide file access does not implement our allowed-root contract. |

## 1. Show which work needs attention

Herdr Core receives OSC 7501 status reports from terminal programs. Reports distinguish
working, done, blocked, and error. A blocked report can identify permission, question,
or authentication. Programs that do not report status have no status. Core does not infer
agent state from screen text. Its turn counter tracks transitions out of working.

Sources: [status implementation][hc-status], [status/event tests][hc-status-tests].

QGenie already has `TurnStarted`, `TurnCompleted`, tool events, and errors in
[the public event model](../../crates/qgenie-remote-proto/src/wire.rs). It also has
[an owner approval summary](../../crates/qgenie-remote-tui/src/remote_badge.rs).
The missing product layer is a consistent summary across sessions.

Proposed change: show active turn, pending approval, completed turn, and error separately
from running/exited/lost process state. Offer a needs-attention filter. Preserve unknown
and stale state after disconnects or incomplete scans. Derive Codex activity from its
typed events and durable approvals. Do not require another status protocol for Codex.
An OSC-based extension for generic terminals is a later capability, not approval authority.

Acceptance: a non-controller owner can locate and answer a real pending approval;
completion does not mark a live agent as exited; stale information is visibly stale.

## 2. Read output and wait for a specific result

`hc read` returns terminal text. `hc wait` waits for a status transition, process exit,
or new output that contains a requested string. Its timeout returns current information
and leaves the program running. A wait after a send follows that send's turn, including
a turn that finished before the wait began. A program without status reports is waited
on until exit; this is a wait rule, not a claim that the program reported working.

Sources: [wait implementation][hc-wait], [wait tests][hc-wait-tests],
[bounded run output][hc-run]. Tests cover fast transitions, timeouts, process exits,
and output produced before versus after the wait begins.

Proposed CLI additions, not implemented commands:

```text
qgenie-remote remote <connection options> read MACHINE SESSION --lines 100
qgenie-remote remote <connection options> wait MACHINE SESSION --turn TURN --timeout 180
qgenie-remote remote <connection options> events MACHINE SESSION --json
```

Build on [remote replay](../../crates/qgenie-remote-client/src/remote_replay.rs),
[the bounded observer](../../crates/qgenie-remote-client/src/observe.rs), and
[existing CLI dispatch](../../apps/qgenie-remote/src/remote_cli.rs). Start with a readable
checkpoint and event stream. Add wait once its turn/cursor contract is explicit.
Keep waits bounded and cancellable. Report timeout, lost, approval-needed, and completion
as different outcomes. Never report a missed event as success.

One upstream detail should not transfer: Core can read a full-screen application by
temporarily scrolling it and stitching the screen. That operation sends terminal input.
Our observer must use existing checkpoints/replay and report truncation. A read must not
acquire control or type into an application. See [the upstream read path][hc-api] and
[screen stitching][hc-stitch].

Acceptance: reading and waiting work without control; cancelling a wait leaves execution
alive; an already completed target turn returns promptly; memory remains bounded.

## 3. Restore observation after a dropped connection

Core supervises machine connections with backoff. It distinguishes unreachable,
needs-login, and needs-update. Viewer tests cover reconnecting the same viewer and
preventing a returning viewer from immediately taking another viewer's terminal size.
Waiting remote commands can resume waiting after a dropped connection.

Sources: [remote supervisor and wait recovery][hc-remote],
[remote reconnect tests][hc-remote-tests]. These tests use local simulated machine links;
they do not establish behavior through our corporate gateway or identity provider.

Proposed change: reconnect the QGenie client, reauthenticate, and restore observation
from a valid cursor or fresh checkpoint. Show connection state and recovery progress.
Keep controller reacquisition explicit. Discard uncertain input. Reconcile uncertain
starts using the original durable key. Core's label-based run lookup is not a substitute
for our durable start identity. Avoid a generic retry wrapper around all operations.

Acceptance: a dropped client connection recovers output once, without launching a second
session, replaying input, or taking control from another client. Expired credentials and
revocation stop recovery appropriately. This depends on the client sign-in work identified
in the earlier direction review.

## 4. Reuse launch profiles

Core templates combine a command, description, and opening instructions. They are loaded
on each use. The first message normally waits for a program status report. A timeout can
leave a trust dialog untouched and report that the message was not sent. The optional
quiet-output fallback is explicitly weaker evidence of readiness.

Sources: [template format][hc-templates], [template/readiness tests][hc-template-tests].

Proposed change: a small local profile can select machine, allowed directory, advertised
agent, and optional initial task. Use the existing structured Codex path and provider
injection. Validate current roots, agent availability, and authorization at each launch.
Do not store credentials in profiles. A profile described as read-only provides prompt
instructions; it does not create a filesystem permission boundary.

Acceptance: a profile reduces repeated setup, cannot bypass changed host policy, and
keeps the original start identity through cancellation or a lost reply. Do not add a
workflow engine or a new crate for profiles.

## 5. Make the interfaces easier to learn and maintain

Core generates its API document from a method table and Rust types. It also compares
the current API with a released baseline. Its command reference comes from executable
help, and checks validate documentation examples. This reduces interface drift.

Sources: [API definition][hc-api], [compatibility checks][hc-compat],
[command-document checks][hc-cli-docs].

QGenie's [schema command](../../xtask/src/main.rs) verifies that committed schemas match
current types. That check serves a different purpose from testing compatibility with a
released contract. Keep our protocols and capability negotiation. Add representative
released-client compatibility fixtures and help/example checks alongside real interface
changes. Do not replace the protocol with JSON-RPC merely to imitate Core.

## Features to study later

- **Agent delegation:** Core supports parent/child identity, messages, waits across
  children, and attention notes. Children normally end with their parent, with an opt-out.
  This is useful for an orchestration product, but it needs explicit lifecycle policy here.
  Do not make a client disconnect or an informal relationship terminate durable sessions.
  See [parent/child tests][hc-children] and [notes][hc-notes].
- **Native-owner updates:** Core places PTYs in separate holders. Its upgrade test checks
  an old binary against a new one when `HC_OLD_BIN` is supplied; it otherwise skips that
  proof. Our connector/sessiond separation already preserves work during connector updates.
  Replacing sessiond while preserving all live recording and authority state is a separate
  design, not a small refactor. See [upgrade tests][hc-upgrade].
- **Keep after process loss:** Core can replace a lost program with a fresh shell. It does
  not rerun the original task. QGenie should keep lost and unknown states explicit and use
  a separately confirmed new start. See [keep tests][hc-keep].
- **Herdr session adoption:** The PRD already places this later. The architecture's old
  Herdr command example does not describe Core's current `terminal.*` JSON-RPC API.
  Any future adapter must verify the selected product/version, controller rules, recording,
  and recovery semantics. Do not assume a drop-in socket integration.

Core trusts clients with the OS account's access and relies on SSH or the selected transport
for remote trust. Its attach protocol permits multiple active writers and uses recent viewer
activity to choose size. QGenie requires corporate gateway authorization, host policy, and
one active controller. Preserve those boundaries. See [Core security][hc-security] and
[the stream protocol][hc-schema].

## Recommended implementation order

1. Complete the existing setup/sign-in work. Preserve the real deployment and pilot gates.
2. Add readable `read` and bounded `events` commands using the existing SDK. These provide
   immediate value and a simpler way to diagnose the developer journey.
3. Add activity summaries and target-specific `wait`, with explicit freshness and outcome rules.
4. Complete automatic observation recovery through the same credential lifecycle. Test
   reconnects, approval interruptions, and unchanged native lifetime before the pilot.
5. Add launch profiles and session naming only where pilot feedback shows repeated friction.
6. Defer delegation, file operations, new adapters, and deeper runtime replacement.

Help/example checks and released compatibility tests accompany the affected steps.
This order extends task 030. It does not close its resource or real-environment gates.

## Verification and limits

The clone, upstream revision, package version, license, implementation, and selected test
assertions were inspected. The published OpenRPC files were parsed and checked for the
methods cited here. Local links and pinned upstream file references were checked against
the two checkouts. Repository whitespace checks pass.

Herdr Core was not built or run. Its build requires Zig 0.16 and the Ghostty submodule;
Zig was not available on PATH and submodules were not initialized. No agent was launched,
no paid model was invoked, and no real SSH deployment was tested. Upstream test assertions
describe coverage present in source, not test passes observed in this review. QGenie product
code was not changed, so its runtime suites were not rerun.

[hc-status]: https://github.com/herdrdev/herdr-core/blob/1d09234dac41a729d07212f70f8ef73c1e107f0f/crates/herdr-core/src/status.rs
[hc-status-tests]: https://github.com/herdrdev/herdr-core/blob/1d09234dac41a729d07212f70f8ef73c1e107f0f/crates/herdr-core/tests/m3.rs
[hc-wait]: https://github.com/herdrdev/herdr-core/blob/1d09234dac41a729d07212f70f8ef73c1e107f0f/crates/herdr-core/src/wait.rs
[hc-wait-tests]: https://github.com/herdrdev/herdr-core/blob/1d09234dac41a729d07212f70f8ef73c1e107f0f/crates/herdr-core/tests/m5.rs
[hc-run]: https://github.com/herdrdev/herdr-core/blob/1d09234dac41a729d07212f70f8ef73c1e107f0f/crates/herdr-core/src/api/run.rs
[hc-api]: https://github.com/herdrdev/herdr-core/blob/1d09234dac41a729d07212f70f8ef73c1e107f0f/crates/herdr-core/src/api.rs
[hc-stitch]: https://github.com/herdrdev/herdr-core/blob/1d09234dac41a729d07212f70f8ef73c1e107f0f/crates/herdr-core/src/stitch.rs
[hc-remote]: https://github.com/herdrdev/herdr-core/blob/1d09234dac41a729d07212f70f8ef73c1e107f0f/crates/herdr-core/src/remote.rs
[hc-remote-tests]: https://github.com/herdrdev/herdr-core/blob/1d09234dac41a729d07212f70f8ef73c1e107f0f/crates/herdr-core/tests/remote.rs
[hc-templates]: https://github.com/herdrdev/herdr-core/blob/1d09234dac41a729d07212f70f8ef73c1e107f0f/docs/templates.md
[hc-template-tests]: https://github.com/herdrdev/herdr-core/blob/1d09234dac41a729d07212f70f8ef73c1e107f0f/crates/herdr-core/tests/m7.rs
[hc-compat]: https://github.com/herdrdev/herdr-core/blob/1d09234dac41a729d07212f70f8ef73c1e107f0f/crates/herdr-core/tests/compat.rs
[hc-cli-docs]: https://github.com/herdrdev/herdr-core/blob/1d09234dac41a729d07212f70f8ef73c1e107f0f/crates/herdr-core/tests/cli_docs.rs
[hc-children]: https://github.com/herdrdev/herdr-core/blob/1d09234dac41a729d07212f70f8ef73c1e107f0f/crates/herdr-core/tests/m6.rs
[hc-notes]: https://github.com/herdrdev/herdr-core/blob/1d09234dac41a729d07212f70f8ef73c1e107f0f/crates/herdr-core/src/notes.rs
[hc-upgrade]: https://github.com/herdrdev/herdr-core/blob/1d09234dac41a729d07212f70f8ef73c1e107f0f/crates/herdr-core/tests/upgrade.rs
[hc-keep]: https://github.com/herdrdev/herdr-core/blob/1d09234dac41a729d07212f70f8ef73c1e107f0f/crates/herdr-core/tests/keep.rs
[hc-security]: https://github.com/herdrdev/herdr-core/blob/1d09234dac41a729d07212f70f8ef73c1e107f0f/docs/security.md
[hc-schema]: https://github.com/herdrdev/herdr-core/blob/1d09234dac41a729d07212f70f8ef73c1e107f0f/schema/README.md
