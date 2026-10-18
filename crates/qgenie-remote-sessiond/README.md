# qgenie-remote-sessiond

Local process authority, start journal, control, replay, and snapshots.

## Consumers and API

The standalone composition root and private IPC peers. Direct product dependencies: proto, transport, policy, adapters.

`Config::new(state_dir, roots)`, `Runtime::open`, and async `serve`.

## Configuration and platforms

Supports Linux/macOS Unix PTYs. The caller owns the Tokio runtime. The composition root sets a private umask. Roots and resource budgets are explicit configuration.

## State, cancellation, and limits

SQLite stores accepted starts before native launch. Same-key changes conflict. A missing reply does not create a new launch. The registry caps active sessions at 64 and total retained starts at 4096. Replay caps at 10,000 events/8 MiB. Per-session locks serialize generation changes and input. The server caps dispatch/admin connections at 64/4. A daemon crash can lose native processes; restart preserves key identity and reports lost/uncertain execution. Scoped PTY/managed capture and original-access IPC compose the independent spool, authenticated receipt reclamation and protected host uploader.

The native supervisor is reserved before acceptance and owns launch, drain, and reap independently of an RPC. An exclusive private runtime lock prevents a second owner from recovering the live journal. Six actual process-exit boundaries and concurrent duplicate starts are tested. Leases are rechecked after durable input intent and before native bytes; the input digest key exists only for this runtime.

Native cwd policy independently opens each path component from pinned root descriptors, rejects traversal/symlinks/unsafe ownership, and holds the selected inode through `fchdir`/exec. No canonicalize-then-reopen path is used for launch. At most 16 roots are allowed. Primary-child reaping is independent of PTY EOF; a descendant retaining the slave cannot strand the owner forever. The owner drains for at most 500 ms after primary exit and records an explicit gap if the descriptor remains open.

SQLite schema 3 adds immutable `(gateway origin, machine, owner)` native bindings and per-session remote scope. Trusted composition configuration admits at most 64 bindings. Scoped start namespaces include that tuple plus the original key; local/legacy keys retain their existing namespace. Enrollment never adopts historical local sessions or changes a durable machine's owner. Public native dispatch requires activation, current original access and action policy.

Schema 4 adds durable host floors/connection identity. Native IPC 1.1 requires `native-authority-v1`, issues bounded nonces (four per binding/256 total), and activates at most four unexpired access grants per binding. Lifetimes are at most ten seconds from original issue, including persistence and suspension. Linux uses `CLOCK_BOOTTIME`; macOS uses `mach_continuous_time`. Clock errors fail closed. Private controller activation additionally requires `native-controller-v1`. Query/activate/renew/release commit floors before ACK, require explicit takeover, preserve original deadlines on duplicates, and reject revival after release/restart. Public effects check the original access/controller authority at native I/O. Owner approvals require current access without a controller lease. Restart clears grants, never floors. Service startup allows a bounded two-second prior-owner lock handoff before failing, without recovering state before exclusive ownership succeeds.

SQLite schema 2 adds one bounded complete terminal checkpoint per session. Checkpoint, cursor, event, and necessary pruning commit atomically. Checkpoints are captured periodically and before pruning would remove their required tail. Live attachment captures state/cursor under the same session lock as output. A lost or incomplete persistent suffix fails with `history_truncated`; an unconfirmed native-owner-loss tail is never presented as a complete screen. Finished/historical sessions retain no in-memory grid.

Four concurrent terminal captures are admitted. Local list metadata is capped at 512 KiB and rejects larger replies until SDK pagination is integrated; status counts do not enumerate metadata. Snapshot IPC pages are at most 256 KiB, with at most 32 pages/eight MiB per capture. Each epoch is bound to client/session and a fixed cursor. Assembly state caps at 32 captures/32 MiB and expires after 30 seconds. The SDK has a 25-second assembly deadline. The [terminal codec](../../vendor/README.md) validates complete native state; it is independent of replay cursors and recording.

## Verification

```sh
cargo test --package qgenie-remote-sessiond
cargo clippy --package qgenie-remote-sessiond --all-targets -- -D warnings
```

Use the root README for the real-process and TUI smoke checks. Linux tests have run in the managed workspace. macOS execution is pending CI.

## Recording component

`RecordingEncoder` receives an immutable `RecordingIdentity`, durably reserved `CapturedRecord`s and bounded injected `RedactionRules`. It preserves source timestamps/sequence, arbitrary output bytes and fragments; normalized records have no keystroke variant. Its streaming literal rules are a disclosed standalone sample integration, not verified corporate redaction. It retains possible secret prefixes across flushes, and masks incomplete prefixes at EOF.

`RecordingSpool::open(SpoolConfig::new(private_dir), now)` owns separate SQLite metadata, a process lock and compressed content files. Call register/reserve/commit/fetch/acknowledge on an owned blocking worker; never hold live session locks across these calls. Defaults are 256 MiB content plus a physically preallocated 16 MiB metadata free-list reserve. Limits include 64 active streams, 4096 total streams/reservations/gaps and 8192 content entries. Metadata journal/filesystem failure can still defeat persistence; `durability_unknown()` remains locally readable when SQLite fails. Reconcile writes conservative recovery loss before resuming and never reuses exposed sequences.

A group ends on a complete fragmented record. Storage failures can persist only part of a group; recovery conservatively marks its outstanding reservations. Gap metadata commits before cap eviction; only unoffered gaps may be atomically superseded/coalesced with new IDs. Fetch makes a gap immutable. Exact authenticated content/gap receipts must match stream epoch, digest, sequence/capture boundaries and recovery epoch; a receipt cannot supply its own trusted epoch. Reclaiming an absent batch is idempotent. Corrupt/missing content becomes durable loss. Warnings survive reconnect/reopen. Live replay is independent.

`cargo test -p qgenie-remote-sessiond --lib recording` exercises component faults and `bash tests/e2e/recording_disk_full.sh` proves actual ENOSPC plus separate live PTY control in an isolated networkless container. Native capture/read integration is described below. The synchronous fixture verifies protected upload/reclaim, history/lifecycle and primary-volume loss; production fault domains and operational qualification remain release gates.

`RecordingWorker::spawn` owns the blocking spool thread. `register` returns cloneable `RecordingCapture` producer handles with a bounded 64-event queue (up to 2 MiB uncompressed admission data); four RPC slots are separate. `capture` uses a short admission mutex and nonblocking try-send. It never waits for file I/O, compression or uploader work. Overflow and invalid admissions retain ordered loss obligations. The worker reserves sequences, coalesces loss, retries storage recovery, flushes at two seconds, and supports bounded fetch/receipt RPCs. Caller-supplied timestamps are original capture time. `finish` means native EOF; owner shutdown without that evidence leaves an uncertain tail. Only the owning native runtime should stop/join this worker after native drain. Tests include actual no-echo PTY output and SQL-stall handoff/recovery. Production durability qualification remains pending.

`Config::recording` supplies mandatory scoped-capture budgets and sample redaction rules. Native PTY output and successful resizes use nonblocking capture; inputs do not. Before native launch, recorder initialization failure returns the durable failed start without creating a child. Existing sessions continue through later spool failure. Complete EOF seals a stream; incomplete EOF/cutoff writes an explicit recovery-loss tail, with reopen reconciliation if storage remains unavailable. Worker warning state is cached and survives reconnect/restart. An absent metadata spool reports unknown completeness rather than claiming intact history. Unscoped preview sessions retain their original local scope.

Native `recording-read-v1` adds original-access-scoped fetch/status. Authority is checked before and after worker I/O; no authority/session lock spans that wait. Read capacity is four on the server. Fetch allows one immutable 64 KiB compressed item and exposes no global enrollment counts. The host bridge reserves at most one recording request, leaving other native request capacity available. Receipt IPC requires separately authenticated current recovery-epoch evidence and execution-time acknowledgment fencing; a receipt cannot authenticate its own epoch.

## Native changes and managed execution

`native-changes-v1` provides four concurrent scoped read-only streams and at most 64 scope
watch entries. Revisions coalesce without retaining output. Every notification write keeps
its original suspend-aware grant and native replacement barrier, including partial writes.
Idle checks and peer EOF cancel subscriptions. `ReadHead` returns exact floor/head/input
counters under the same scope and original authority. Host/gateway/SDK use these as hints;
rendered content still comes from bounded replay and checkpoints.

The optional `managed-codex` feature owns pinned Codex 0.159.2, typed structured replay,
durable start provenance and approval application. All six start crash boundaries, owner
SIGKILL and caller-loss recovery pass on Linux with the actual binary. Runtime IPC reconnect
is unsupported; a failed protocol preserves uncertainty and explicit termination rather
than relaunching or resending input. Confirmed-ended historical resume creates a new identity
and child using the saved thread/private home and a freshly validated cwd. See
[MVP evidence](../../docs/validation/mvp-evidence.md) for commands and unrun release gates.
