# Rust review: QGenie Remote

This review applies BeautifulRust to the nine product crates and the standalone CLI.
The SDK and protocol expose library contracts. The gateway, host, session owner, and TUI
implement one application through internal crate boundaries. Rust 1.99.1 and edition 2024
are pinned. The existing process and authority boundaries have concrete requirements.
The first correction is active inventory pagination in the connector. The review also
found a process ownership defect in native launchers and unnecessary idle polling.
Normal Clippy, workspace tests, a pedantic audit, and library documentation checks ran.

Source: [BeautifulRust](https://github.com/morganlinton/BeautifulRust), revision
`a2b22682fa69b5e2d41a5d041663df7d51aac521`. The project installs all three skill definitions
and the five reference digests under `.agents/skills`. Their Git blob hashes match upstream.
The [review skill](../../.agents/skills/rust-review/SKILL.md) defines this report format.

## Must fix

### 1. Active inventory assumes one page

Locations at review: `crates/qgenie-remote-host/src/connector/changes.rs:55` and
`crates/qgenie-remote-host/src/connector/approvals.rs:28`.

**Rule:** Preserve observable behavior when refactoring (BeautifulRust, section 4).
Validate input contracts (C-VALIDATE, Rust API Guidelines). Test pagination properties
rather than one small example (BeautifulRust, section 11).

**Problem:** Both consumers require `next: None` on the first active-session page. The
native owner limits metadata pages to 512 KiB, independently of the session count.
A valid page can therefore have a continuation below the active-session count limit.
The connector rejects that reply. Metadata following can restart repeatedly, and
approval discovery cannot reach the remaining sessions. Combining all pages into one
outbound frame would create another oversized-frame failure.

**Fix:** Read all bounded active pages through one native helper. Validate cursor progress
and scope. Preserve the page boundaries when publishing metadata. Flatten only the session
IDs needed for approval rotation. Keep the original grant on every read.

```rust
let pages = client.active_session_pages(&access).await?;
for sessions in pages {
    publish(&sender, client.scope().machine, sessions).await?;
}
```

Status: repaired. The regression failed with `Error::Protocol` before the change and
passes afterward. It reads 18 large records across three native pages. The combined
metadata exceeds one network frame, while each published page remains valid.
Bound a traversal to 128 sessions and 8 MiB.
The native owner permits 64 active sessions. The traversal allowance permits some
concurrent turnover while preventing an unlimited scan. An exceeded bound fails explicitly.

### 2. Native admission must outlive a cancelled caller

Location: `crates/qgenie-remote-sessiond/src/server.rs`, data and administrative handlers.

**Rule:** Resources follow ownership (BeautifulRust, sections 3 and 7). Dropping an async
future does not stop a blocking worker. Preserve cancellation and failure semantics.

**Problem:** The former connection task held admission permits outside `spawn_blocking`.
Cancellation could release capacity while SQLite or native work continued.

**Fix:** Move permits into the worker. Return them with the response so successful callers
also retain admission while writing a reply.

```rust
let (response, _permit, _recording_permit) = tokio::task::spawn_blocking(move || {
    // Perform bounded native work while these permits remain owned here.
    (response, permit, recording_permit)
}).await?;
write_dispatch(&mut stream, &response).await?;
```

Status: repaired before this skill was installed. Four IPC regressions pass. They cover
missing capabilities, cancellation during data and administrative writes, and shutdown
after a lost reply. The snippet shows the ownership shape; it omits dispatch details.

### 3. A version-matching launcher can break native ownership

Locations: `crates/qgenie-remote-adapters/src/codex/endpoint.rs`, `connect_retained`, and
`crates/qgenie-remote-adapters/src/codex/owner.rs`, `ProcessOwner`.

**Rule:** Resources follow ownership, BeautifulRust sections 3 and 7. Validate input
contracts, C-VALIDATE. A successful version probe does not establish process identity.

**Problem:** The npm executable starts a separate native child. The adapter accepts its
same-user socket but retains the launcher's PID. Termination and owner-death handling
therefore cover a different process. The initial npm-based proof left native app-server
processes alive after cleanup. The controlled regression reproduces the accepted mismatch.

**Fix:** On Linux, compare socket peer credentials with the still-owned child PID on
connection and reconnection. Reject a mismatch before native initialization. Signal the
private process group before reaping its leader during explicit cleanup. Use the actual
native executable in CI. Preserve wrappers that replace themselves with `exec`.

```rust
if peer.pid().and_then(|pid| u32::try_from(pid).ok()) != Some(owner_pid) {
    return Err(CodexError::Configuration);
}
```

Status: repaired. The regression fails before the change and passes afterward. All five
actual Codex adapter checks pass, including tool-descendant termination, retained directory
identity, stream controls, resume, and original-grant startup. This is Linux evidence.

### 4. Sealed recording encoders consume lifetime capacity

Location: `crates/qgenie-remote-sessiond/src/recording/worker.rs`, `run`.

**Rule:** Bound retained state and give resources explicit lifetimes, BeautifulRust
sections 3 and 7. Test boundary behavior, section 11.

**Problem:** The worker retains every sealed stream encoder. Its 4,096-entry bound
therefore limits lifetime recordings. The earlier disk-side history fix does not release
this in-memory state. The regression fails on registration 4,097 without a worker restart.

**Fix:** Release encoder state after queue drain and durable sealing. Retain immutable
epoch identity and sealed status in SQLite. Return a closed capture on a repeated sealed
registration. Preserve the 64 active-stream bound and the durable outstanding-work bound.

```rust
streams.retain(|_, stream| !stream.sealed);
```

Status: repaired. The passing regression completes 4,100 registrations, then
checks a new recorded stream, refusal to write through the old capture, and immutable
epoch identity. The full managed session-owner suite also passes.

## Should fix

### 5. Name operation modes at SDK call sites

Locations: `crates/qgenie-remote-client/src/lib.rs:117`,
`crates/qgenie-remote-client/src/remote_control.rs:58`, and
`crates/qgenie-remote-gateway/src/routing/edge.rs:107`.

**Rule:** C-CUSTOM-TYPE, Rust API Guidelines; domain types, BeautifulRust section 4.

**Problem:** Calls such as `session_page(None, 64, true)` and
`terminate_current(machine, session, true, false)` hide the meaning of each flag.
The gateway handshake also accepts four boolean configuration arguments. These calls
are easy to misread during changes to authorization or confirmation behavior.

**Fix:** Introduce narrow domain types at these shared interfaces. Keep the existing
JSON fields and protocol semantics. Migrate Rust callers together or keep a compatibility
wrapper if the SDK has external users. A capability set with independent flags does not
need a state enum.

```rust
pub enum SessionSelection {
    Active,
    All,
}

pub enum TerminationConfirmation {
    ManagedSession,
    IncludingExternalSession,
}
```

Status: recommended API cleanup. Do this after the correctness repair. Do not add a
generic options framework or silently change confirmation requirements.

### 6. Put failure and cancellation contracts beside SDK methods

Locations: `crates/qgenie-remote-client/src/lib.rs:117` and `:142`,
`crates/qgenie-remote-client/src/live.rs:383`, and
`crates/qgenie-remote-client/src/remote_start.rs:51`.

**Rule:** C-FAILURE and C-QUESTION-MARK, Rust API Guidelines.

**Problem:** README files explain uncertain starts and paged history, but the new public
methods omit error sections. A caller reading generated API documentation cannot readily
see that cancelling a start does not undo a launch, or that ID cursors are not snapshots.

**Fix:** Document errors, cancellation, bounds, and reconciliation where callers use them.

```rust
/// Reads one page in session-ID order.
/// Concurrent starts can insert IDs before the cursor; restart traversal to see them.
///
/// # Errors
/// Returns an error for an invalid page limit, unavailable authority, or malformed reply.
```

Status: targeted documentation repaired for changed session-page, start, and recovery
journal APIs. The review also found that the older-gateway fallback accepted an invalid
page limit. The SDK now rejects limits outside 1..=256 before that fallback. Its real TLS
regression checks zero and 257 without sending a request. Expand documentation for other
APIs when those interfaces change.

### 7. Make TUI transitions readable before adding actions

Locations: `crates/qgenie-remote-tui/src/remote_start.rs:233` and
`crates/qgenie-remote-tui/src/remote.rs`, the main `tokio::select!` block.

**Rule:** Make invalid states unrepresentable, BeautifulRust sections 1 and 4;
Google Rust Style Guide, readable control flow.

**Problem:** The creation picker combines `editing`, `choosing_root`, `submitted`, and
`loaded`. Large branches inside a macro mix input decoding, transitions, network jobs,
and presentation. The recent discarded-navigation repair and partial-render test race
show the cost of reasoning about readiness in several places. This is a maintenance
finding, not proof that another launch is duplicated.

**Fix:** Extract named input and completion handlers. Represent mutually exclusive picker
modes directly. Keep the selected path and immutable start key explicit. Keep a submitted
start separate from cancellable reads.

```rust
enum PickerMode {
    LoadingRoots,
    ChoosingRoot,
    EditingPath,
    Browsing,
    Submitted,
}
```

Status: recommended follow-up. Preserve real keyboard coverage and uncertain-start tests.
Do not introduce an event bus or a reusable UI state-machine framework.

### 8. Preserve private diagnostic categories when sanitizing errors

Locations: `crates/qgenie-remote-gateway/src/transaction.rs`, `database_error`, and
`crates/qgenie-remote-client/src/start_journal.rs`, `Journal::open`.

**Rule:** C-GOOD-ERR, Rust API Guidelines; error context, BeautifulRust section 5.

**Problem:** Public errors correctly hide secrets. However, several boundaries also
discard the original failure category without a private diagnostic. A permission issue,
full disk, lock timeout, and damaged journal can look alike to an operator.

**Fix:** Keep the current sanitized wire error. Record a bounded operation name and
allowlisted error category locally. Do not log SQL text, paths containing user data,
arguments, recordings, connection strings, or tokens.

```rust
tracing::warn!(
    operation = "authority.commit",
    sqlstate = error.as_database_error().and_then(|cause| cause.code()).as_deref(),
    "database operation failed"
);
```

Status: recommended operational improvement. Review the diagnostic sink and retention
before adding logs. Returning raw source errors to remote callers is not the fix.

## Consider

The release host sample exceeded both footprint targets. Two avoidable costs were visible
in code: a five-millisecond empty recording-worker loop and a 100-millisecond connector
inventory loop. Explicit producer wakeups now drive recording work. Native notifications
drive inventory reads, with scheduled metadata and history reconciliation. Timed recording
flushes, recovery, and native authorization checks remain active. The capacity report
records the repeated measurement; a passing functional proof does not close this budget.

The initial pedantic audit reports 999 unique warning locations across production code, tests,
and examples. Duplicate diagnostics from library and test targets were removed. The
largest group is 311 missing error-documentation sections. These are audit observations,
not 999 defects. The workspace already centralizes its lints. Do not enable the full
BeautifulRust lint set as an error gate before selecting useful rules and fixing their
existing findings.

Keep these deliberate choices:

- Independent capability flags describe a set. They are not mutually exclusive states.
- `Arc` and owned background tasks enforce real process and cancellation lifetimes.
- Public wire structs are passive serialization data. Making all fields private or adding
  `non_exhaustive` everywhere would cause churn without fixing the reviewed defect.
- Native effect barriers intentionally hold a lock across the admitted OS call. This
  serializes replacement against an in-flight effect. Removing the lock to satisfy a
  generic callback rule would weaken fencing.
- Recording shutdown joins its worker to finish owned persistence. Review blocking
  teardown at async call sites; do not detach required capture merely to shorten `Drop`.

The nine crates are justified by their responsibilities. The main excess is breadth of
release scope and dense orchestration inside modules. More crates, more traits, or a
blanket rewrite of error types would increase the current maintenance cost.

## What is already good

- Typed identities and explicit unknown outcomes prevent unsafe mutation retries.
- Separate connector and native owners preserve sessions across remote disconnection.
- Concrete modules and pure policy checks follow the dependency graph without a generic
  handler framework.
- Actual process, TLS, storage, cancellation, and recovery proofs test observable behavior.

## Verification and sequence

The final code checks ran on Linux with Rust 1.99.1. Ignored tests remain separate from
passed tests. The native and composed commands explicitly select their required proofs.

| Check | Result |
| --- | --- |
| `cargo fmt --all -- --check` | Passed |
| `cargo check --workspace --all-targets --locked` | Passed |
| Workspace Clippy, default and managed-Codex feature sets, with `-D warnings` | Passed |
| `cargo test --workspace --locked` | 193 passed, 18 ignored |
| Session-owner tests with `--features managed-codex` | 71 passed, 8 ignored |
| Actual pinned Codex adapter contracts with `--ignored` | 5 passed |
| Release CLI local TUI and 4,100-record history-capacity smoke checks | Passed |
| Composed remote, managed-start, recording, and primary-volume-loss proofs | Passed |
| Final 60-second release host budget | Not met: 32.53 MB RSS and 0.57% CPU |
| `cargo doc --workspace --no-deps --features qgenie-remote/managed-codex --locked` | Passed |
| `cargo xtask boundaries` and public, REST, and recording schema checks | Passed |
| Upstream skill verification | All 9 source files match pinned Git blobs |

The initial pedantic audit completed successfully with warnings. It informs this review;
it is not a new error gate. The CI workflow was checked locally. No GitHub Actions run is
claimed. The [final composed result](linux-completion-2026-10-26.json) and
[capacity report](capacity.md) record the release journey and host measurements.

Use this sequence next:

1. Close the measured host resource budget with profiling and repeatable evidence.
2. Qualify real enterprise access and Linux user services, then run the small developer pilot.
3. Simplify SDK mode arguments and TUI transitions as the pilot identifies affected calls.
4. Add safe diagnostic categories before assigning production operators.

Keep the current platform and adapter scope until the supported journey meets its gates.
