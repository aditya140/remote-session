# 020 — Implement the durable recording spool and explicit loss gaps

- **Status:** Complete for the selected Linux standalone development scope; external release qualification remains separately tracked
- **Milestone:** M0 recording proof; release mapping decided in 002
- **Workstream:** Host recording
- **Depends on:** [001](001-inspect-existing-qgenie-integration.md), [015](015-implement-session-runtime-and-start-journal.md)
- **Index:** [Task plan](README.md)

**Scope:** The user selected Linux development completion in [ADR 0007](../docs/adr/0007-standalone-preview.md). See [current progress](PROGRESS.md) and [external release gates](../docs/validation/external-release-gates.md). Original criteria that require unrun environments or waived corporate inspection remain unchecked; this status does not claim full release qualification.

## Goal and repository context

Recording must remain enabled independently of bounded live replay, while storage failures cannot halt session execution.

**Read first:** [PRD.md](../PRD.md) PO-4; [architecture.md](../architecture.md) §4.4; [components.md](../components.md) §4 contract 8 and §7 recording proposal.

## Owned scope

- `Sessiond recording/{spool,chunk,gaps}.rs and recording migrations`
- `Recording event hooks and private spool files`

Owned paths define this task’s implementation boundary; current source and executed evidence are recorded below. Follow [the subcrate standard](../subcrate-standard.md), add only useful modules, and coordinate shared manifests/exports and migrations as described in the index.

## Implementation

1. Create immutable chunk identities/boundaries with stable capture time, sequence ranges, stream metadata, redacted content, and replay-independent storage budgets.
2. Record normalized agent events and terminal output/resizes/timing. Reuse QGenie redaction before content leaves the host; never separately store raw no-echo keystrokes.
3. Reclaim uploaded content only after the durable commit receipt defined in 002, or the explicit cap policy. At the cap discard oldest unacknowledged content, reserve space for coalesced durable gap metadata, and admit recent output.
4. Continue recording locally during pause/disconnection; expose backlog, gaps, rate/quota warnings, and receipt/gap upload interfaces.

## Acceptance criteria

- [x] Postgres/gateway outages and a full spool do not stop session output, control, or process execution.
- [x] Lost or duplicate receipts preserve correct reclamation; crashes cannot erase acknowledged gap information or rewrite chunk capture times.
- [x] Known secrets are redacted and terminal password/no-echo input is absent from recorded raw input.
- [x] Warnings survive reconnect and recovered recording retains earlier loss markers instead of presenting complete history.

## Verification

- Run disk-cap, disk-write failure, restart, duplicate receipt, long-disconnect, pause/resume, redaction, and no-echo PTY scenarios with a fake uploader.

Run the relevant formatting, lint, dependency-boundary, and supported-platform checks once code exists. Record real results and any unavailable environment; do not report planned or mocked external validation as completed.

## Boundaries and handoff

No PostgreSQL content store, retention job, or live replay replacement. Mandatory recording cannot be disabled through a feature flag.

**Deliver:** Spool fetch/ack APIs, immutable batch fixtures, and persistent gap/warning events for 021 and the TUI.

## Component implementation and evidence

Sessiond now owns an independent private spool database/content directory and bounded streaming encoder. Default content/metadata budgets are 256 MiB/16 MiB; SQLite physically allocates metadata pages and retains its free list. This reserve cannot guarantee writes through total disk failure. Full-u64 recording sequences commit before encoding/exposure, independently of live replay. Immutable zstd JSONL chunks preserve binary output, ordered fragments, original capture times, stream epochs and compressed-byte SHA-256. The normalized shared contract has no keystroke/input variant.

Injected literal redaction rules are bounded, omit secret Debug/serialization, and keep possible prefixes across event, resize and timer/chunk boundaries. EOF masks incomplete secret prefixes. This is an explicit sample integration point, not a claim of corporate redactor compatibility. The user waived the external CLI source; do not request it to continue.

Spool tests pass lost/duplicate/exact receipt checks, authenticated recovery-epoch mismatch, immutable identity, metadata bounds, durable/coalesced gap-before-eviction, offered gap identity stability, actual process-exit reservation recovery, corrupt/missing chunks, and write-failure reconciliation. `tests/e2e/recording_disk_full.sh` fills an isolated 4 MiB tmpfs to actual ENOSPC as UID 1000: uncertainty is visible, a separate live PTY accepts control/output, and recovery commits explicit loss. This is isolated disk-failure component evidence; separate composed runtime tests described below cover capture integration. Protected upload/history composition is now implemented; production real-user qualification remains separate.

The owned capture worker now has a separate 64-event (at most 2 MiB input) queue and four RPC slots. Its nonblocking producer handoff tracks admission order and persists explicit overflow loss after earlier queued records. SQLite stall/recovery, lost/duplicate fake uploader ACKs, detached viewer lifetime, orderly drain, missing-native-EOF tail uncertainty and real no-echo PTY output pass. Batch boundaries follow capture months even through UTC regression. Native PTY hooks and scoped read-only IPC now work as described below. Remaining: remote status/TUI composition and full remote process fault boundaries. Protected ingest/holds/retention (021–022) still gate release.

## Native runtime composition

Scoped native PTY starts now register capture before native launch. Output and successful resizes enter the nonblocking producer; raw input never does. Configuration has budgets/rules and no disable switch. Recording initialization failure before launch leaves a durable failed start with no child; existing running sessions and unscoped preview remain independent. No connector/viewer owns capture. Complete native EOF seals the stream, while inherited-descriptor cutoff/read failure persists an explicit conservative tail gap before freeing its active slot. Storage failure preserves reopen reconciliation. Ordinary preview history is never adopted into an enrollment.

Seven composed native tests pass real PTY output/no-echo/redaction/resize, live control/output while recording SQLite is locked, prelaunch storage failure, inherited-tail gaps, cap warnings across restart, actual Unix capability/scope checks and original access expiry/replacement during recording I/O. Native read-only `recording-read-v1` IPC rechecks original authority after worker work without holding authority/session locks through I/O. Spool reads have a four-request server budget; the connector allows at most one recording request within its four-slot native budget. Legacy receipt-only IPC remains unsupported. Context-bound reclamation requires independently authenticated epoch evidence, as implemented below.

Warnings are cached by the worker and exposed without SQLite work in session-list paths. Existing spool metadata reconciles at native startup. Missing metadata is reported as unknown completeness; an old receipt cannot reconstruct missing bytes. Public upload/history, independent backend epoch binding, native acknowledgment fencing and gateway/uploader composition now pass the proofs below; production qualification is external.


## Authenticated reclamation and upload

`recording-reclaim-v1` independently binds a native-issued challenge and original access
nonce to machine-authenticated HTTPS backend epoch evidence. The receipt cannot establish
that binding. Exact receipt metadata and current native grant/epoch are rechecked after
acquiring SQLite's write lock, immediately before logical deletion. Expiry, replacement and
old epoch receipts retain content. The single-job connector uploader uses immutable retries
and 1–30 second backoff, independently of capture. Actual HTTPS + sessiond process tests on
the synchronous pair pass lost ACK retention/retry, recovered gap upload and foreign-binding
isolation. The legacy receipt-only IPC remains unsupported. See the recording integration
document for the original-grant and recovery-quarantine prerequisites.

Public PTY start now passes the actual persisted-key SDK/WSS/gateway/activated connector/native process path in the owned synchronous recording harness. Same-key retry produces one child/start audit; changed parameters are NotSent; unready storage cannot launch. The connector automatically uploads real native capture and end metadata. Public remote control/input and managed runtime now pass the composed proof; see PROGRESS.md for current evidence.
