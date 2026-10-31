# Daily remote workflow implementation

Date: 2026-10-31. Platform: Linux x86-64. Toolchain: Rust 1.99.1.
Base revision: `73c509f4007fc7368da4409c930ebad80d5938a1`.
Scope: [task 033](../../tasks/033-implement-daily-remote-workflow.md).

The accepted local implementation is complete. The separate enterprise deployment and
pilot gates remain open. The implementation follows the selected ideas from the
[Herdr Core review](herdr-core-review-2026-10-31.md). It adds no runtime dependency, crate,
database migration, or native owner replacement.

## Implemented sequence

1. Add setup and sign-in. `host setup` validates roots and atomically creates private
   configuration without replacing an existing file. `remote setup` saves connection
   settings. Browser PKCE and device login reuse the existing OAuth implementation.
   The application owns serialized refresh and keeps user credentials in memory.
   The SDK receives credential updates. Machine identity remains separate.
2. Add bounded reads. `remote read` renders a safe text checkpoint. `remote events`
   emits bounded JSON records with explicit checkpoint, limit and timeout information.
   Both paths use existing native observation APIs and never acquire control.
3. Add activity and waits. `remote activity` separates typed agent activity from process
   lifetime and durable approval status. `remote wait` targets one turn or native exit.
   The TUI displays activity and per-session approval counts. Its `!` filter selects
   pending approvals, failed or uncertain lifetime, and recording warnings on the current page.
4. Add observation recovery. The TUI retains its output cursor and clears controller state,
   queued input and confirmations before reconnecting. Read-only CLI commands can repeat
   observations. Recovery uses current credentials and six attempts with capped backoff.
   Authentication, authorization and protocol failures stop recovery. No mutation is resent.
5. Update command, crate and task documentation. Generate the changed public schema.
   Retain old event fixtures and add a fixture for the new native turn outcome.

## Correctness repairs found during implementation

- The Codex adapter discarded the native final turn status. Successful, failed and
  interrupted turns all became the same completion event. The event now retains an optional
  typed outcome. A targeted wait reports success only for the requested turn with an explicit
  completed outcome. Old events remain readable but cannot prove success.
- Long CLI operations did not drain unsolicited connection events. The CLI now drains them
  while preserving each operation's original result. This avoids unnecessary queue exhaustion.
- A truncated checkpoint could make an ended process look like proof that a turn did not
  finish. Waits now preserve the uncertain result. Confirmed process exit can be observed
  without requiring a still-available native output checkpoint.
- Transport protocol failures were reported as generic disconnections. The SDK now retains
  that distinction so observation recovery stops. Explicit access-denial messages also remain
  available after closure. A lost WSS refresh acknowledgment can use a new authenticated
  connection; it cannot cause an application action to repeat.
- Setup publication now distinguishes replacement of client preferences from creation of
  initial host configuration. Concurrent setup cannot overwrite an existing host file.

## Executed checks

The counts below overlap. Do not add them as a unique test count.

| Check | Result |
| --- | --- |
| `cargo test --workspace --locked` | 211 passed; 18 ignored by the suite configuration. |
| Final client, TUI, application-unit and `client_workflow` tests | 52 passed. These include the final recovery error-classification repairs. |
| Application unit and `client_workflow` tests with `managed-codex` | 8 passed. |
| Sessiond library tests with `managed-codex` | 70 passed; 7 ignored. |
| Adapter library tests with `codex` | 16 passed. |
| Actual pinned Codex approval, stream, steer, interrupt and resume proof | Passed against `/opt/codex/bin/codex` with a synthetic local provider. It asserts completed and interrupted native outcomes. |
| Protocol conformance tests | 9 passed, including exact round trips for 32 network fixtures. |
| Workspace check and strict Clippy | Passed. |
| Strict Clippy for the application, adapters and sessiond with managed Codex | Passed. |
| Formatting, dependency boundaries and all three generated schema checks | Passed. |
| Workspace Rust documentation with warnings denied | Passed. |
| Python tool tests | 3 passed. |
| Actual local keyboard and process-survival smoke | Passed, including SIGTERM terminal restoration. |
| History-capacity smoke | Passed with 4,100 seeded records, restart, pagination, key reconciliation and new work. |

The CLI integration tests use actual subprocesses and verified TLS/WSS. They cover saved
setup, two device sign-ins without persisted user credentials, safe readable output, exact
turn matching, timeout, lost lifetime, truncated history, cursor recovery, and protocol failure.
The peer rejects application mutations from read-only commands.

The keyboard recovery test opens the real TUI in a PTY. It selects the attention filter,
observes output, requests control once, enters an unsent draft, and drops the connection.
It verifies retained output, removal of the draft, observation-only recovery, no second
lease request, and terminal restoration on SIGTERM.

OAuth tests use a verified TLS fixture to rotate refresh credentials and deny the next
exchange. Failure stops access without retry. WSS tests cover injected rotation, denied
rotation and a lost refresh acknowledgment. These are synthetic identity proofs.

Logs remain under `target/validation/daily-workflow-*.log`. Test sources remain in the
repository; the logs are local build output. Build-cache cleanup was needed
to complete checks within the workspace disk limit. No source or state journal was removed.

## Remaining qualification

This run did not execute real enterprise sign-in, corporate network deployment, an actual
systemd user-service installation, or the developer pilot. It did not rerun the full
synchronous PostgreSQL/Redis recording deployment fixture. Relevant ignored tests retain
their documented environment prerequisites.

The separate host resource budget remains open. The earlier measurement was 32.53 MB
aggregate RSS and 0.57 percent idle CPU, against 30 MB and 0.5 percent targets. This change
does not claim a new measurement or close that gate. See [task 030](../../tasks/030-complete-linux-release-readiness.md)
and [external release gates](external-release-gates.md).

Launch profiles and additional session names still follow pilot feedback. Agent delegation,
file operations, new adapters and native owner replacement remain later work.
