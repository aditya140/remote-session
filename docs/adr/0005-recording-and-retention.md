# ADR 0005: Protect persistent recordings before release

Status: selected implementation and release gate. Dates, staffing, and infrastructure qualification are not established.

## User task and problem

Users need recorded history and visible loss when storage fails. Platform admins need audited access and legal holds. The PRD says recording is mandatory but puts its storage protections in M2. A release must not persist real content while deferring those protections.

## Decision and reason

Keep recording mandatory. Treat tasks 020, 021, and 022 as a release dependency for any milestone that persists real session content. Include owner/admin access, audited admin reads, retention, and hold preservation with that content. M0 may prove the contract with disposable synthetic fixtures. It must not expose unprotected real-user recordings as an alpha feature.

This resolves implementation ordering. It does not confirm an earlier delivery date or approve a new schedule. Keep the original milestone estimates as planning context. Task 027 must report the protection gate separately from terminal/Codex demonstration results.

## Independent capture and storage

Sessiond records normalized agent events and PTY output/resizes/timing through a separate spool from live replay. Raw terminal keystrokes are not stored separately. A no-echo password therefore does not appear through an input-recording path. Echoed output can still contain secrets; reuse verified host streaming redaction and writer-side redaction.

Record immutable chunks with these fields:

- Chunk, session, machine, and owner identities; schema/codec version; stream and stream epoch.
- Inclusive output sequence boundaries and per-record capture timestamps. Preserve record and fragment order.
- Stable minimum/maximum capture time; compressed and uncompressed byte counts; SHA-256 digest of the exact compressed binary bytes (before base64 transport encoding).
- Redacted content plus an explicit relation to any loss range. No tokens or key material in metadata.

Use UTC content time to choose the partition and retention age. Validate it against session lifetime and a configured clock-skew bound; initially permit at most 5 minutes of future skew. An invalid clock produces an ingest error and warning. Do not replace capture time with upload time or silently clamp it forward. Delayed uploads may contain old content; old age does not make it new.

The initial batch target is 2 seconds or 64 KiB compressed, whichever comes first. Bound uncompressed content to 1 MiB per chunk. Split a larger normalized record into identified, ordered fragments before compression. Bound decompression memory, CPU work, and concurrent batches. The uploader must not share an unbounded queue with live output.

## Spool and loss ranges

Start with a configurable 256 MiB per-host content budget and a separate preallocated 16 MiB metadata reserve. These are development defaults that task 028 must measure. Reject invalid zero budgets. Cap active sessions and coalesce adjacent gaps so content saturation cannot create unlimited metadata.

Persist a chunk before offering it for upload. Persist its identity, capture boundaries, and digest once. An upload retry cannot change them. Readiness or gateway connectivity must not control local capture.

At the content cap, first commit a gap covering the oldest unacknowledged content, then reclaim that content and admit recent output. The gap identifies session, stream, sequence interval, content-time interval, reason, and durable gap ID. Merge adjacent gaps conservatively without claiming retained content as complete. Upload and acknowledge gap metadata through the same idempotent receipt discipline as content.

Crash recovery must distinguish a committed gap with remaining content from a deleted chunk with its committed gap. A committed gap is conservative evidence of possible loss; later recovery of the original content can add evidence without deleting loss history. Reclaim a gap record only after its durable receipt.

A completely failed disk can prevent even the metadata reserve from being written. Report `recording_durability_unknown` locally and on reconnect. Preserve the last durable capture frontier. On recovery, mark the uncertain interval explicitly through the next established frontier. Do not claim a gap was durable when its write failed. Keep draining the process and keep available remote control responsive. Total storage failure cannot provide a loss-free metadata guarantee.

Recording warnings persist across reconnect. Alert on backlog, content eviction, failed gap persistence, and recovery uncertainty. Recording resumes when storage recovers; earlier gaps remain visible. Users and orgs cannot disable capture through configuration or compile features.

## Idempotent ingest and receipts

Authenticate the enrolled machine and verify the session-to-machine-to-owner relationship on every batch. Ignore a submitted owner ID as authority. Validate size, codec, digest, sequence boundaries, capture times, and schema before committing.

Use a stable batch identity independent of partition location. Maintain a unique receipt ledger entry for `(machine_id, session_id, chunk_id)` in the same transaction as stored content or a gap. A duplicate with identical immutable fields returns the existing equivalent receipt. A duplicate with different content or boundaries returns `idempotency_conflict` and never overwrites history.

The receipt includes receipt/chunk identity, digest, capture/sequence boundaries, database recovery epoch, and disposition. Return `stored` only after content and ledger commit with `synchronous_commit=on`. A policy-expired late upload can return a committed `expired` disposition without recreating an expired partition. It must not claim stored content. The host reclaims only the exact matching batch after validating the authenticated receipt and allowed disposition.

Upload cancellation or a missing acknowledgment leaves the batch unacknowledged. Retry with the same identity. Bound retries with backoff; do not block the live session. Gap receipts obey the same rule. Receipt tombstones must outlive any accepted retry window. Retiring them must also make old retries ineligible for fresh insertion.

## Database recovery and durability limits

A commit receipt proves durability under the configured PostgreSQL durability boundary. A single-server development commit survives a gateway/process restart; it does not prove survival of a lost data volume. Asynchronous replication can lose acknowledged rows after standby promotion. Receipt identity cannot reconstruct bytes that the host already reclaimed.

Production must establish a durable boundary before receipt-based reclamation: a tested synchronous standby or another verified zero-loss durable commit path. If that boundary is unavailable, ingest cannot return a reclaimable `stored` receipt. The operator must not silently downgrade this rule during an outage.

Every restore or promotion establishes a new recovery epoch. Stop ingest and destructive jobs until the operator reconciles receipt-ledger coverage, hold state, and authority generations. Fence restored stale connections. Retry retained spool content against the new epoch. If a disaster restore predates a reclaimable receipt, record explicit recovery loss ranges from available evidence and report them; do not pretend the old receipt proves current content availability.

Tasks 021/026/028 must document and drill the actual durability boundary, recovery procedure, and backup target. No standby or backup configuration has been verified in this checkout.

## Protected historical reads

Use separate database roles for migrations, gateway authority, ingest, owner history reads, admin history reads, and retention/holds. Application roles must not be table owners, superusers, or `BYPASSRLS`. Ingest cannot read arbitrary transcript content. Use a narrow database function or restricted ledger role for receipt lookup.

Apply explicit RLS and grants to normal partitions, held content, receipt metadata, and referenced history metadata. A copied table does not inherit a safe security policy automatically. RLS tests must use actual application roles rather than a development database administrator.

For owner reads, begin a transaction, set transaction-local authenticated user context, query the bounded page, then end the transaction. Missing context denies reads. PgBouncer transaction pooling must never retain one request's identity for the next request.

Platform-admin reads require the verified role and a nonempty reason. Reads of held content also require the applicable case reference. Buffer only a bounded page. Commit the admin-read audit event before releasing that page's bytes. Audit failure denies the read. Streaming and pagination require an audited bounded scope for each released page.

Owners and admins can read eligible transcripts. Other users, managers, sharees, and support roles cannot. Owner responses must not expose legal-hold flags, admin reasons, or admin-read events. Do not notify the owner about an admin read.

## Holds and monthly retention

Use PostgreSQL locks and transactions for ingest, hold placement/change/release, and partition retirement. Use one transaction-scoped global recording-lifecycle advisory lock initially. All these paths must acquire it before changing hold coverage or partition membership. It is a deliberately simple correctness boundary. Task 028 measures contention; a later finer-grained design needs equivalent race proofs.

Use bounded batches. A partition can have `active`, `retiring`, and `retired` states. Starting retirement does not authorize deletion. While holding the lifecycle lock, copy every actively held row and its required references to protected held storage, verify preservation, and commit the drop or retirement transition. A hold committed before the final drop must participate in its preservation check. A crashed worker resumes the same state; another worker cannot bypass that check. Long copies must release the lock between bounded batches and recheck hold coverage before final deletion.

Held storage includes explicit many-to-many hold coverage, keyed independently of a single hold. Releasing one hold cannot delete content still covered by another. Hold changes preserve newly covered existing content and future ingest. Late content whose ordinary partition is retired goes directly to held storage if an active hold covers it; otherwise it receives the policy-expired disposition. Public receipts always follow ordinary expiry even when a private hold preserves a copy, so receipts do not disclose hold state to the affected user. An expired disposition makes no promise of owner-visible availability.

Preserve referenced session/machine metadata and audit events for held users. Soft deletion must not break these references. Audit history is append-only to application roles. Only audited retention operations can retire eligible history.

At each nightly run, compute `cutoff = database_now - retention_days`. A monthly partition with half-open interval `[start, end)` is eligible only when `end <= cutoff`. At the 90-day default, rounding can retain content for roughly 120 days plus job scheduling effects. Neither uploads nor retries reset age. Hold windows use original content time.

Only platform admins can place, change, or release a hold. Case references are optional for preservation, as specified by PO-8; a held read requires an applicable assigned case reference. One admin is sufficient. An API success means the hold is durably active and deletion is fenced; any bounded backfill continues under that protection. The release operation rechecks all remaining coverage before cleanup.

Backups can contain data beyond live-database expiry. Task 028 must record backup expiry and restore-time hold/retention reconciliation. Do not invent legal retention periods or claim that deleting a live partition erases all backups.

## Verification owners

Task 020 tests cap/write failures and crash recovery. Task 021 tests ingest deduplication, receipts, RLS, pooled identity isolation, and audited reads. Task 022 tests hold/ingest/purge races and overlapping coverage. Tasks 027/028 retain unrun durability, backup, and infrastructure proofs as release blockers.
