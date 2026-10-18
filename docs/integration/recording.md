# Recording integration status

The standalone preview remains a local replay demonstration. Public remote effects remain disabled. Protected recording upload/history APIs are opt-in and require isolated credentials, current directory proof and qualified lifecycle/durability state. Persistent real-user release has not been qualified. ADR 0005 requires tasks 020–022 together before releasing persistent content. These components use disposable synthetic verification.

## Native component

Sessiond owns separate recording metadata/content files, independent of replay. Its recording worker accepts at most 64 queued events (2 MiB source bytes) through nonblocking handoff; four RPC slots are separate. Only that owned blocking worker performs sequence reservation, compression, persistence, cap eviction and receipt reclamation. Viewer/connector lifetime does not own capture.

Recording sequences commit before encoding/exposure. Full-u64 counters, immutable stream epochs, binary output, fragments, source capture/elapsed time and exact compressed-byte SHA-256 survive retries. Chunks use bounded JSONL/zstd, one UTC capture month, at most 1 MiB raw and 64 KiB compressed. The normalized contract has no raw-input variant. Shared schema is `conformance/schema/recording-v1.json`.

Host streaming literal rules retain possible secret prefixes through output, resize, flush and chunk boundaries. EOF masks incomplete prefixes. This is an explicit standalone integration point, not verified corporate redactor compatibility. The user waived the original CLI source. Corporate integration cannot be claimed from these sample rules.

Default budgets are 256 MiB content plus physically preallocated 16 MiB SQLite metadata free-list pages. The reserve cannot guarantee successful writes under total filesystem/journal failure. Gaps commit before unacknowledged content deletion; unoffered adjacent gaps can be atomically superseded with a new ID. Offered IDs/bounds never change. No gap is reclaimed before its own exact authenticated receipt.

Content receipts match the original compressed digest, capture/sequence bounds, stream/session/machine identities and an independently authenticated backend recovery epoch. Gap receipts have their own typed ID and canonical sorted-key JSON digest. A receipt cannot establish its own trusted epoch. Unknown/missing acknowledgments retain content. Corrupt/missing files and reserved-but-uncommitted records become conservative durable loss. Failed storage sets locally visible durability uncertainty; reconciliation establishes explicit loss before resuming. Native EOF is distinct from recording-worker shutdown.

## Gateway component

Codec preparation has sixteen actual blocking permits held through cancellation. It validates size/CPU boundaries, a single zstd frame, hash, duplicate-free normalized JSON, sequence/fragment order, exact source capture bounds, UTC month and explicit five-minute session/future skew. Upload time never replaces original age.

Writer literal redaction is a separate bounded backstop. It masks known literals across record fields and conservatively masks potential chunk-edge prefixes/suffixes. Excessive overlapping match work masks the entire text and records saturation. It preserves original metadata/digest for receipt identity, clears unprotected upload bytes, and computes a separate hash for stored redacted bytes. The sealed protected value is preparation, not an authorization or committed receipt.

Schema 9 isolates ingest, owner read, admin read and lifecycle credentials. Purpose checks reject privileged, table-owner, mixed and unexpected-grant identities. FORCE RLS protects normal content, receipts, held data and case membership. Missing transaction context denies reads; owner context is transaction-local. Direct partition access is not granted. Admin normal reads require actor/reason; held rows additionally require matching case coverage. Coverage is maintained by lifecycle authority, never an owner or ingest caller. The protected HTTP composition is described below.

## Verified and remaining

Real native PTY/no-echo, SQLite cap/stall/write-failure/reopen/process-exit and isolated UID-1000 ENOSPC tests pass. Actual application-role PostgreSQL tests pass owner isolation, size-one pool context reset, direct partition/held-row denial, wrong/mixed role rejection and held case membership. The schema-11 gateway suite also passed through actual PgBouncer 1.24.1 transaction pooling, including two separate frontends reusing one backend with deliberately stale session context. These are component proofs, not complete remote recording.

Schema 10 commit components now bind current machine/key/session/owner under ordered primary locks, preserve stream/fragment identity and atomically persist content/gap plus per-recovery-epoch receipts. Duplicates commit a fresh write barrier; production requires remote-apply qualification. PostgreSQL 17+ bounds active SQL at three seconds with two-second statement/lock budgets. PostgreSQL disables these timers during synchronous COMMIT, so an owned, pool-bounded supervisor separately uses the original three-second transaction budget, then allows up to one second each for protocol cancellation and response drain. Canceled frontends are retired. A canceled/local-only commit never produces a receipt or admin page. The production method also requires schema 12 lifecycle protection, while only tests can qualify the synthetic single-server path. The disposable synchronous-pair proof passes production methods and primary-volume loss. Deployed standby/backup restoration and full native recovery remain unqualified. Schema 13 uses the original native-observed process end when present. Native SQLite schema 5 preserves that observation across restart and inventory retries cannot rewrite it. Unknown/legacy ends retain the first final projection upper bound; repeated final inventory does not extend it. This observation is not a completeness certificate or an exact kernel exit timestamp.

Schema 11 adds bounded owner/admin history components, transaction-ID-bound RLS context, scoped full-u64 keyset cursors and protected held gaps. Admin reason, exact released chunk/gap IDs and applicable case commit in the same transaction before any page is returned. Audit failure denies the result. Owners cannot see held-only copies or administrative context. Recovery-epoch changes invalidate existing cursors explicitly. Tests use actual signed owner/admin identities and isolated database roles. A history page does not certify native capture completeness; late backfill can require refreshing pagination.

Schema 12 lifecycle components pass real PostgreSQL/PgBouncer proofs. They add revisioned holds, optional capture windows/cases, overlapping coverage, direct preservation of late uploads and bounded retirement. The final drop verifies current hold coverage and exact bytes under the ingest/hold lifecycle lock. The signed-admin hold endpoint and bounded lifecycle jobs command are composed and reject unqualified state; successful hold HTTP/job execution passes on the disposable synchronous pair.

Native epoch-bound reclamation and the asynchronous machine-only uploader are implemented. Remaining work includes full remote native capture composition, combined remote lifetime/fault evidence, deployed recovery/durability and lifecycle qualification, job scheduling and remote TUI/status warnings. Capture must remain active through gateway/storage failure and live control must never wait on spool/network work. No switch may disable mandatory recording.

Production qualification still requires synchronous durable commit/restore reconciliation, backup/hold/retention drills and capacity measurements. A development commit does not prove survival of a lost database volume. The PgBouncer harness is `tests/e2e/pgbouncer_recordings.py`; CI installs the broker and runs this proof, but an actual CI run has not been observed here.

Native spool scoped fetch/status/ack now filters the immutable original origin/owner/machine, including before marking a gap offered. Foreign bindings cannot acknowledge content or gaps, and cannot inspect another binding's queue counts. Scoped worker RPC and spool tests pass. Native PTY hooks and `recording-read-v1` IPC now compose scoped reads under original access; reclamation additionally requires the independent `recording-reclaim-v1` exchange below.

Scoped PTY runtime capture now registers before launch, records output/resize through nonblocking handoff and excludes raw/no-echo input. A failed initial recorder prevents that scoped child launch while preserving its start identity; existing sessions continue during later recording failure. Normal EOF seals capture, while inherited terminal cutoff creates an explicit durable gap without waiting for native restart. Cap/storage warnings survive restart and remain visible through cached status. Nine actual runtime/Unix IPC tests pass, including native end persistence and original remote projection. Recording reads release authority locks before worker I/O and recheck the original nonce afterward; expiry/replacement denies queued results. Four server recording slots and one connector recording slot preserve other native capacity. Legacy receipt-only IPC placeholders remain unsupported; the context-bound reclamation exchange is separate.

## Protected HTTP and uploader composition

`Gateway::enable_recordings` installs separate ingest (8 connections), owner (4) and admin
(4) pools. All must have the same independently generated database identity as authority;
partial startup closes every opened pool. Configure the three recording database URL files
and an explicit private `--recording-redaction-file` containing a JSON array of sample literal
strings (an empty array selects no extra gateway literals). Identity/directory configuration
is required. No public switch enables private synthetic durability.

Machine-only POST routes are `/v1/recordings/chunks`, `/v1/recordings/gaps` and
`/v1/recordings/sessions/{session}/authority`. Content uploads have an explicit 128 KiB
HTTP budget; ordinary requests keep 64 KiB. Session/machine/owner/lifetime are checked
before preparation and again under primary transaction locks. Owner history uses POST
`/v1/sessions/{session}/history`; the separate admin route is
`/v1/admin/sessions/{session}/history` with `AdminHistoryQuery`. Reasons are JSON bodies,
not query strings. Successful responses use `Cache-Control: private, no-store`.

The native owner issues an epoch challenge bound to the original access nonce, immutable
scope, connection and host generation. The connector exchanges that challenge over its
configured verified HTTPS origin with machine credentials. The authority response binds
current owner/machine/key and a backend recovery epoch after a fresh remote-apply barrier.
The uploader validates those identities before native binding and obtains the receipt in a
separate request. Receipt fields cannot establish or replace the native binding.

`recording-reclaim-v1` is independently negotiated. Native binding cannot change the epoch
of an existing grant, and a newer bound grant prevents older exchanges from becoming
current again. Receipt reclaim checks exact scope/digest/capture bounds after acquiring the
spool SQLite write lock, then validates the same original native grant and epoch immediately
before logical deletion. No authority/session lock spans storage I/O. The metadata deletion
commits before unlinking bytes. Expiry during lock waits, replacement, stale epochs and
replayed old exchanges retain content. Restart forgets every ephemeral epoch binding.

The connector owns at most one upload job independently of native capture and public
request work. It retries immutable spool items with bounded exponential backoff (1–30 s),
pauses when no fresh native grant exists, and never retries live input. Lost responses retain
local items; duplicate content/gap requests reconcile exact durable receipts. Connector exit
cancels its uploader without owning capture. The synchronous harness also runs a real
HTTPS/Unix-IPC spool recovery and lost-response test before the primary-volume-loss drill;
its completion markers prevent an empty test selection from passing.


Owner/admin POST `/v1/{admin/}sessions/{session}/history/streams` discovers currently visible
recorded streams through the same primary context and isolated read roles as content pages.
Pages are capped at 64 entries. Cursors bind session, backend recovery epoch, held scope and
case. Held-only streams are excluded from owner discovery, require a current matching case
on the admin path, and index metadata is released only after its exact returned stream IDs
and reason/case have a synchronous audit commit. Discovery does not certify capture
completeness. The SDK and remote owner TUI consume these paths.
