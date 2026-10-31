# 033 — Implement the daily remote workflow

Status: complete for local implementation and validation. Authorized on 2026-10-31.
The overall release goal remains open in task 030.

## Goal

Implement the accepted sequence from the Herdr Core review: simple setup and client
sign-in, readable output and events, activity summaries and targeted waits, then
automatic observation recovery. Preserve Linux, one owner, shell, and pinned Codex.
Overall completion remains the goal. A real enterprise deployment and developer pilot
need their own executed evidence; local fixtures cannot close those gates.

## Ordered implementation steps

1. Save non-secret client connection settings and provide browser/device sign-in.
   Reuse the existing scoped OAuth implementation. Refresh credentials only in the
   running client. Preserve explicit token-file injection for automation. Provide
   host setup through the existing host configuration and service paths.
2. Add readable `remote read` and bounded JSON `remote events` commands through the
   existing observer/checkpoint APIs. Reads must never take control or send input.
3. Derive a separate activity summary from typed events and durable approvals. Add
   targeted `remote wait` with explicit completion, approval, timeout, loss, and
   uncertain-history results. Expose attention in the TUI without conflating agent
   activity and process lifetime.
4. Reconnect client observation with bounded backoff and current credentials. Preserve
   cursor ordering. Require a new explicit control request after connection loss.
   Never resend input, starts, or approval choices. Stop on denied access or sign-in
   requirements. Keep the UI responsive while a connection is unavailable.
5. Check relevant API compatibility and command documentation. Run focused regressions,
   workspace checks, and the supported Linux workflow checks. Record evidence and limits.

## Decisions before product changes

- Keep new behavior in existing crates and concrete modules. Do not add a runtime,
  workflow engine, generic event bus, or Herdr dependency.
- The application owns interactive OAuth and refresh. The client SDK accepts supplied
  credentials and expiry updates. It does not acquire host-execution dependencies.
- Persist only gateway, CA, and OAuth configuration selectors. User access and refresh
  credentials stay in memory. A new client invocation can sign in through existing SSO.
  No credential-store prerequisite is needed to use this first standalone client flow.
- Token refresh is serialized. A failed or uncertain exchange requires sign-in rather
  than retrying a possibly consumed refresh token. A returned rotation replaces it.
- Keep explicit `--token-file` behavior compatible. Do not silently send an existing
  credential to a different configured gateway.
- Use existing structured Codex events and approval records. Terminal output does not
  grant permissions. Unknown and stale information remain visible.
- Preserve Codex's final turn outcome. Review found that normalization discarded failed
  and interrupted status. Add an optional typed outcome to the existing completion event.
  Old events remain readable but cannot prove successful completion. Keep native lifetime
  and turn outcome separate. Regenerate schemas and test old-event compatibility.
- A targeted wait follows an explicit turn or exit. Timeout and cancellation preserve
  native execution. A checkpoint that cannot prove an old turn does not prove success.
- Readable terminal output comes from a checkpoint and replay. Never scroll a native
  application to collect text. Preserve bounded output and visible truncation.
- Read and wait commands use bounded reads on one owned connection. They drain live
  traffic while each read runs. Waits poll at one-second intervals and stop at an
  explicit deadline. Only observations can restart after connection loss.
- Recovery makes at most six connection attempts with capped backoff. It stops on
  authentication, authorization, and protocol failures. The user can retry explicitly
  after that budget. A recovered terminal retains its output cursor and discards control.
- Launch profiles and additional session naming follow pilot feedback, as accepted in
  task 032. Agent delegation, file operations, new adapters, and owner replacement remain
  later work. They do not block this implementation.

## Acceptance

- [x] Setup and browser/device sign-in remove routine token-file handling for people.
- [x] Client credentials refresh without entering machine state or diagnostic output.
- [x] Read and event commands remain bounded, read-only, and cancellable.
- [x] Activity and waits report observed state and preserve unknown outcomes.
- [x] The TUI exposes work that needs attention.
- [x] Client observation recovers without repeating actions or restoring control.
- [x] Tests cover credential rotation/failure, output bounds, targeted waits, and reconnect.
- [x] Documentation describes the commands, persistence, and remaining qualification.

## Execution record

The steps above were documented before product changes. Each stage is implemented in the
existing application, client, host, TUI and protocol modules. The
[validation report](../docs/validation/daily-remote-workflow-2026-10-31.md) records the ordered
changes, defects found, checks and limits. The
[command guide](../docs/integration/remote-commands.md) describes setup and daily use.

The workspace suite passed 211 tests with 18 configured ignores. Final focused checks passed
52 tests. Managed Codex checks and the actual pinned native turn-outcome proof passed.
The real keyboard test verifies observation recovery, cleared drafts, no automatic control
request, and terminal restoration. Strict linting, schemas and documentation checks passed.

## External prerequisites

The current environment has no configured enterprise credentials or assigned pilot host.
Continue implementation and synthetic integration checks independently. Task 030 retains
the real identity, corporate networking, Linux user-service, pilot, and resource gates.
