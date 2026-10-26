# Repository review — 2026-10-24

## Assessment

The project is over-scoped for an initial dogfood release. Its main process boundaries
are justified by the written requirements. A wholesale rewrite or crate merge would
not resolve the main problems.

The intended user task is clear: enroll a machine without inbound networking, start
or observe a terminal or coding agent from another machine, answer approvals, and
reattach without losing execution. The PRD also requires enterprise identity,
revocation that preserves execution, single-controller input, mandatory recordings,
audited history, retention, and legal holds. These requirements make this a remote
execution service with compliance storage, not only a terminal relay.

[ADR 0007](../adr/0007-standalone-preview.md) explicitly authorizes the standalone
application and Linux development scope. Corporate integration, platform execution,
and production qualification are separate gates. Their absence is not an unauthorized
implementation choice. It does limit what the implementation can establish.

This review follows requirements, CLI/TUI flows, client cancellation, session storage,
root enforcement, routing, and selected tests. It is not a complete security audit.

## Follow-up — 2026-10-26

[Task 029](../../tasks/029-stabilize-core-journey.md) documents the repair sequence and
implementation. The descriptions below record the problems found during this review.
The follow-up adds paged inventory, a compact identity journal, root discovery, and
finished-state reclamation. It also fixes a background-refresh navigation race and
preserves warnings during archival. Verification and external gates are in task 029.

[Task 030](../../tasks/030-complete-linux-release-readiness.md) continues toward overall
Linux completion. Its [BeautifulRust review](beautiful-rust-review-2026-10-26.md) adds
connector pagination, native cancellation, launcher ownership, and recording-worker
lifetime repairs. It also adds release-build resource measurements and mandatory native
agent coverage in CI. The resource budget and external qualification remain explicit gates.

## Findings

### 1. Session history eventually blocks ordinary use — high priority, fixed in task 029

The native owner retains finished sessions and loads them again after restart.
Both PTY and managed-agent starts refuse new work at 4,096 total sessions, including
finished sessions. The client start journals also stop accepting new identities at
4,096 files. The remote journal limit applies across machine scopes in that directory.

Inventory fails earlier. Native and gateway session lists refuse inventories above
1,024 entries or 512 KiB. Local session listing also refuses metadata above 512 KiB.
There is no session-list cursor. The connector uses the same complete native list to
publish session changes and poll approvals. These limits therefore affect discovery
and update paths, not only a history screen.

Evidence:

- `crates/qgenie-remote-sessiond/src/runtime/start.rs`: `start_guarded`.
- `crates/qgenie-remote-sessiond/src/runtime/managed.rs`: managed start capacity.
- `crates/qgenie-remote-sessiond/src/runtime/authority.rs`: `Message::SessionList`.
- `crates/qgenie-remote-sessiond/src/runtime.rs`: `Runtime::list`.
- `crates/qgenie-remote-sessiond/src/store.rs`: `Store::load`.
- `crates/qgenie-remote-gateway/src/routing/store.rs`: `session_projection`.
- `crates/qgenie-remote-client/src/remote_start.rs`: `persist`.
- `crates/qgenie-remote-host/src/connector/{changes,approvals}.rs`.

This review reproduced the local failure with the actual CLI and daemon. A disposable
database contained 4,096 synthetic completed session records and no running sessions.
After restart, `list --json` failed with the local list-limit error. A new `/bin/true`
start failed with `session registry capacity reached`. This was a seeded-history test,
not a 4,096-process soak. The remote limit is established by code inspection.
The sessiond README documents the cap, but documentation does not provide a recovery
workflow.

Repair: live inventory now uses separate indexed queries. History uses bounded pages.
SQLite retains start keys and loads historical metadata on demand. A compact client
journal retains immutable identities. The CLI regression now passes with 4,100 seeded
completed records, a restart, original-key reconciliation, and a new live session.
This is a history-capacity regression, not a process or fleet soak.

### 2. A first session requires knowledge the picker does not supply — medium priority, fixed in task 029

The remote TUI starts the creation picker at the selected session's cwd, or `/` when
no session is selected. A host restricted to a home or project directory rejects `/`.
The machine contract has no allowed-roots field, and the browsing API requires a path.
The user must press `p` and supply a known allowed path.

Evidence: `crates/qgenie-remote-tui/src/remote.rs` creation branch,
`crates/qgenie-remote-tui/src/remote_start.rs`,
`crates/qgenie-remote-proto/src/wire.rs` (`Machine`, `FsList`), and
`crates/qgenie-remote-host/src/filesystem.rs` (`Roots::resolve`).

Repair: authorized `fs.roots` discovery supplies the creation picker. A selected valid
directory remains selected. Without one, the picker offers allowed roots. Older peers
retain manual path entry. Host and native directory enforcement remain in place.

### 3. Cancellation defeats the local start-worker bound — medium priority, fixed here

`Client::start` held a borrowed semaphore permit in the calling future. It launched
blocking journal work that can wait on an exclusive file lock. Cancelling the caller
released the permit while the blocking worker continued. Repeated cancellation could
admit more workers than the configured limit.

The worker now owns the permit until it finishes. The regression test holds the
journal lock, cancels the start, and checks that capacity remains reserved until the
worker can finish. The remote start path already used this ownership pattern.

### 4. Terminal input drops common editing keys — medium priority, fixed here

The shared input encoder returned no bytes for Shift-Tab and Insert. Home and End
also ignored application cursor mode. The encoder now forwards these keys using the
appropriate terminal sequences. A regression test covers normal and application mode.
This change does not claim complete keyboard-protocol support.

### 5. Repeated UI escaping and dense dispatch code make review harder

Five TUI modules implemented almost identical text sanitizers. One intentionally
preserved newlines and tabs; the others rejected them. This review consolidates them
in `display.rs` with explicit `single_line` and `multiline` entry points. PTY output
still uses the terminal parser. A test checks controls, Unicode, and direction overrides.

Larger review obstacles remain. The session daemon places capability classification
and long boolean checks inside its connection loop. TUI event loops contain many
compressed statements inside `tokio::select!`. Protocol macros hide normal type
formatting. Passing rustfmt does not make these blocks easy to review.

Proposed action: extract named operation handlers and capability checks within the
existing crates. Keep side-effect ordering explicit. Avoid introducing a generic
handler framework or more crates.

## What to keep

- Keep the connector separate from sessiond. Pause and revocation must not kill execution.
- Keep PostgreSQL as the durable authorization authority. Redis routing hints cannot
  replace committed access decisions.
- Keep start identity reconciliation, controller generations, and distinct unknown
  outcomes. Blind retries can duplicate execution or input.
- Keep owner approvals independent of the control lease.
- Keep protected recording, explicit gaps, and hold preservation while those remain
  product requirements.
- Keep vendor code out of the gateway and policy decisions free of I/O. The crate
  boundaries serve these purposes. The number of crates alone is not the problem.

The code contains substantial recovery and fault tests. It is not an empty scaffold.
There are also four locally patched dependencies: terminal parser/state, PTY launch,
and PostgreSQL cancellation. [The patch documentation](../../vendor/README.md) gives
concrete reasons. These patches still create an upgrade obligation. Assign ownership
and pursue supported upstream APIs; do not remove patches without replacing their
behavior and rerunning the relevant proofs.

## Where the project became too complicated

### The release scope grew before the first-use experience was complete

Mandatory real-content recording pulled retention, legal holds, audited access,
synchronous receipts, and restore behavior into the early implementation. The PRD
also specifies multiple gateways, corporate identity, proxy behavior, and eventual
200,000-socket qualification. These are explicit product choices, not arbitrary
libraries that can be deleted during cleanup.

Proposed action: freeze feature growth and qualify one Linux developer journey with
shell and the supported Codex version. Keep the compliance requirements if they are
mandatory. If they are negotiable, revise the PRD and release gate explicitly before
simplifying storage or deployment. A smaller synthetic-only spike is different from
a release that persists real developer content.

### Local preview and remote product paths both require maintenance

The application has local `Client` operations and remote `GatewayClient`/`LiveClient`
operations, separate start journals, and separate local and remote TUI flows. This
made the standalone preview useful, but it also created duplicate lifecycle and UI
work. The cancelled-worker bug is an example of the two paths drifting.

Proposed action: retain local administration and diagnostics. Decide whether the local
interactive preview is a supported product surface. Share pure UI/input functions
where behavior is identical. Do not route local administration through the gateway
just to remove code.

### Evidence is narrower than the size of the implementation suggests

[The capacity report](capacity.md) records eight sockets, two terminals, and 200 echo
samples. It does not establish the 200,000-socket target or representative recording
storage. Its measured idle RSS is 27.50 MiB for sessiond plus 24.10 MiB for the connector.
Together these exceed the PRD's 30 MB host-footprint target. The report also explains
why its latency measurement does not establish pure gateway overhead.

[The external gates](external-release-gates.md) remain explicit: real Entra,
user-service lifetime, macOS, production fault domains, and full beta capacity.
The completion reports generally disclose these limits. However, leading with
“All 28 tasks complete” makes these qualifications too easy to miss.

Proposed action: lead release status with supported workflows and unqualified gates.
Use fresh evidence from the intended deployment before making performance or release
claims. Do not treat a completed task list as acceptance of the product requirements.

## Recommended order

1. Fix inventory pagination and define session/journal lifecycle rules.
2. Complete root discovery and test the first-session workflow with a new user.
3. Qualify real Linux user-service lifetime and identity in the intended environment.
4. Simplify the existing dispatch and UI functions without changing contracts.
5. Measure representative host footprint, recording volume, and reconnect behavior.
   Expand load testing only after these basic workflows are reliable.

## Verification for this review

Executed in the Linux workspace:

- `cargo test --workspace --locked`: 180 passed, zero failed, 17 ignored.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`: passed.
- `cargo fmt --all -- --check`: passed.
- `cargo xtask boundaries`: passed.
- `python3 tests/e2e/local_tui_smoke.py target/debug/qgenie-remote`: passed.
  This exercised keyboard input, detach, session survival, observe/control,
  reattach, quit, SIGTERM, and terminal restoration.
- The seeded 4,096-record local-history reproduction confirmed both listing and
  new-start refusal. The disposable daemon was stopped after the reproduction.
- `git diff --check`: passed.

The cancellation and terminal-key regression tests each failed against the original
implementation before the fixes. Both passed afterward. The shared display sanitizer
test also passed.

Ignored tests include database/native-environment proofs and isolated crash helpers.
They are not counted as passed. This review did not rerun the Compose deployment,
synchronous PostgreSQL fault matrix, pinned native-agent qualification, production
capacity, or external platform/tenant gates. Previous reports remain historical
records and are not counted as checks executed during this review.
