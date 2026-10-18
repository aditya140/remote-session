# ADR 0002: Give each state one authority

Status: selected for implementation in task 002. No runtime proof exists yet.

## User task and problem

Users must reconnect, take control, answer approvals, and revoke access without duplicate execution or session loss. Redis delivery and connection success cannot establish those outcomes.

## Decision and reason

Use PostgreSQL transactions for gateway authority. Use sessiond's private SQLite journal and adapter evidence for local execution authority. Redis contains disposable routing, presence, and lease projections. This separates delivery from durable decisions and actual execution.

| Mutable state | Authority and commit acknowledgment | Recovery or stale authority | Saturation behavior |
| --- | --- | --- | --- |
| Owner, machine keys, manual disable state | PostgreSQL primary; change and audit commit together | Re-read durable state. Directory updates cannot clear manual disablement. | Reject changes that cannot be committed. |
| Directory enabled state and verification time | Validated directory integration; PostgreSQL stores its separate projection | Expired verification denies user and machine access. See freshness below. | Bound verification concurrency; expiry still denies access. |
| Host connection generation | PostgreSQL conditional increment; sessiond durably activates the generation | Replaced generations cannot mutate local sessions. | Reject generation exhaustion; never wrap. |
| Controller generation and holder | PostgreSQL conditional transition plus sessiond activation acknowledgment | Reconcile the same generation after lost replies. Redis cannot grant or restore a lease. | Reject takeover when activation cannot be recorded. |
| Running process and native session ID | Sessiond's SQLite journal plus actual adapter/process evidence | Reconcile native identity. Report `lost` or `outcome_unknown` when evidence is insufficient. | Reject new launches at registry/journal capacity; continue existing sessions. |
| Start idempotency | Sessiond's durable scoped key and request fingerprint | Query the same key. Do not infer absence from gateway metadata. | Preserve key tombstones; refuse new starts rather than evict deduplication evidence. |
| Output cursor, replay head/floor, snapshot boundary | Sessiond's sequence allocator and replay log | Report truncation or an explicit gap. Never substitute transcript history for live replay. | Evict the oldest entries and retain the newest suffix under the declared limits. |
| Input receipt | Sessiond's active-generation input journal and adapter result | Report the last accepted input ID separately from output. An uncertain native write remains uncertain. | Reject before acceptance when the bounded input queue is full. |
| Pending native approval and application result | Sessiond's approval journal and adapter evidence | Reconcile cancellations, local resolutions, and lost native replies. | Reject new remote decisions when safe journaling is unavailable. Never auto-approve. |
| One winning remote approval decision | PostgreSQL conditional update, audit, and outbox in one transaction | Retry delivery of the committed decision identity; query application state. | Refuse a new decision before commit if its durable delivery obligation cannot be stored. |
| Unacknowledged recordings and loss ranges | Sessiond's independent spool and gap journal | Follow [ADR 0005](0005-recording-and-retention.md). | Evict old unacknowledged content with explicit gaps. |
| Stored recordings and commit receipts | PostgreSQL ingest transaction and receipt ledger | Reconcile receipt identity and database recovery epoch. | Return retryable ingest failure; leave host execution independent. |
| Holds and permission to purge | PostgreSQL transactions and lifecycle locks | Retry the same transition after crash. Redis leadership grants no deletion authority. | Defer destructive work if authority or preservation cannot be established. |
| Inventory, session counts, and presence | Host reports and live authenticated connection; gateway/Redis projections | Rebuild snapshots and subscriptions. Offline projection does not mean a process ended. | Coalesce inventory and force a new snapshot after subscription overflow. |

Libraries receive these facts from the owning component. The pure policy crate does not discover authority through I/O.

## Enrollment identity

For Entra, persist the canonical user under issuer, tenant, and the verified `oid` directory object. Treat JWT `sub` as a potentially pairwise application subject; changing clients must not change machine ownership or bypass a user revocation. Bind legacy preview subjects to an object only from a verified token; unknown subjects cannot create a replacement owner while legacy identities remain unmapped.

Persist a random installation identity and signing key before the first enrollment request. Scope enrollment uniqueness to the authenticated owner and installation identity. Require proof of possession. Bind the registered machine to its OS-user identity and public key. Treat hostname and display name as mutable metadata.

An interrupted enrollment reuses the persisted identity. An equal hostname with another key must not merge records. A changed hostname retains the machine record. A revoked identity must not silently enroll again.

Key loss requires explicit recovery using user authentication. Recovery revokes the old key and records the replacement in an audited transaction. Do not infer identity from hostname during recovery. Intentional cloning requires a new installation identity and key. A copied private identity cannot be reliably distinguished from the original; a newer host generation fences the older connection. Report this protection limit.

Task 013 must report whether the inspected backend supports non-exportable Ed25519 signing, exportable credential storage, or a private-file fallback.

## Session start

Scope a start key to `(owner_id, machine_id, idempotency_key)`. Store a deterministic fingerprint of the validated launch parameters. Task 004 defines its canonical encoding. Exclude transport correlation IDs from this fingerprint.

| Stage | Durable evidence | Allowed recovery |
| --- | --- | --- |
| `received` | None | A failure before durable acceptance can return `not_sent`. |
| `accepted` | Key, fingerprint, allocated session ID, and launch intent committed in SQLite | The same key returns this record. Only its serialized owner can continue launch. |
| `launching` | Launch intent and correlation identity committed before native creation | Find the correlated native process/session. Do not repeat an ambiguous native creation. |
| `running` | Native identity and running transition committed | Return the existing session ID. Rebuild gateway metadata as a projection. |
| `failed` | Evidence that launch failed without a surviving native session | Return the recorded failure. A deliberate new attempt uses a new key. |
| `outcome_unknown` | Native creation might have happened, but cannot be proved or excluded | Show uncertainty. Reconcile the original key; do not create another process. |
| `lost` | A previously known session has no recoverable native process after restart/reboot | Preserve history and report loss. Historical resume is a separate start operation. |

Reject the same key with a different fingerprint as `idempotency_conflict`. Keep tombstones for any key that could otherwise be replayed. Journal compaction must preserve deduplication or explicitly retire the machine identity. Dropping a client future after acceptance does not cancel the launch or terminate its process.

After a missing reply, the SDK first performs same-key lookup. A definitive sessiond result may permit continuing the original accepted operation. A timeout or absent gateway projection does not permit a new launch.

## Control activation and fencing

Allocate monotonically increasing host and controller generations in PostgreSQL. Attach these generations to every remote mutation that requires them. Sessiond serializes activation and mutation for each session and persists the activated generations.

1. Commit a proposed generation and audit transition in PostgreSQL using a conditional update.
2. Deliver the activation to the current host connection. Sessiond persists the generation before it acknowledges activation.
3. Confirm that the proposed generation remains current in PostgreSQL. Only then report writable control to the client.
4. If the reply is lost, query or redeliver the same activation. A lower generation never restores control. If another proposal supersedes it, reject the old acknowledgment.

Use a 15-second controller lease with renewal at most every 5 seconds. Renewal cannot change the holder or generation. Reconnect does not silently reacquire control. Takeover requires explicit confirmation and a new generation. Sessiond checks the holder, generations, local deadline, action capability, and current policy before input, resize, or interrupt.

Access freshness also bounds every remote mutation. A valid controller lease cannot extend an expired access delegation. On restart, sessiond preserves generation floors and accepts no remote mutation until fresh activation and access checks complete.

Approvals and explicit owner termination require the current host generation and current access, but do not require controller ownership. Termination is a separate audited action with confirmation; external-session termination requires an explicit external-session confirmation fact.

## Access freshness

New connections and durable authority changes consult the PostgreSQL primary. Each gateway refreshes connected-principal access at most every 5 seconds. Measure cache age from the monotonic start of the authoritative read, not from a delayed response. No cached permission survives 10 seconds from that start. Use a deadline that includes scheduling, reads, and socket closure.

After a revocation commit, the accepting gateway denies affected actions immediately. Redis invalidation accelerates other gateways. A missed hint cannot extend the 10-second deadline. PostgreSQL failure makes the gateway unready; existing access closes before its freshness budget expires. User disablement includes enrolled-machine tokens.

Bound in-flight remote authority at sessiond as well. Use a sessiond-issued nonce and local monotonic deadline for each host access renewal. The gateway response must bind that nonce, the current host generation, and the remaining access-authority lifetime. That lifetime is at most 10 seconds minus the age of the gateway's authoritative read. Also cap it by the remaining directory-verification lifetime. Never add a fresh 10-second delegation to a nearly expired cache entry. Sessiond applies that remaining lifetime from the earlier local time when it issued the nonce. Network delay must shorten validity. A delayed response cannot restart the deadline. Refresh at most every 5 seconds. Restart invalidates outstanding nonces. This avoids a database read for each keystroke and prevents delayed queued mutations from surviving access expiry.

Store directory verification separately from manual global/user/machine disable flags. The design limit is 840 seconds from the start of the last successful authoritative directory verification, with at most 10 additional seconds for gateway enforcement. This leaves 50 seconds inside the 900-second requirement. No new connection may use expired directory evidence. Task 019 must validate a real mechanism, its detection cadence, and request deadlines. Token expiry alone provides no directory evidence.

## Approval decision and application

Before choosing a winner, ask sessiond to validate the pending native request, proposed answer, and current host policy. Bind the preflight result to the approval ID, request hash, proposed decision hash, policy revision, current host generation, and a short nonce deadline no greater than 10 seconds. A policy cache with unknown freshness is insufficient.

The gateway conditionally commits one decision, its actor, audit event, and delivery obligation. A committed response means only that this transaction won. It does not mean that the native agent applied the answer.

Sessiond journals the decision ID before application and rechecks current policy and native pending state. A policy change after preflight can yield `cancelled` with reason `policy_changed`; it must not cause an invalid answer to be applied. The committed decision remains in history. Another answer does not overwrite it. This is not an atomic transaction across PostgreSQL, host policy, and the native agent.

Report native application as `applied`, `already_applied`, `cancelled`, or `outcome_unknown`. A native reply lost after submission is uncertain until the adapter proves the result. Duplicate delivery returns the journaled result or performs native lookup. Do not repeat an uncertain native answer unless the verified vendor contract proves it safe.

Owner clients can answer without a lease or attachment. Revocation refuses new answers and prevents unapplied queued answers from bypassing current access. Pending requests otherwise follow explicit native cancellation or configured timeout policy. Revocation does not approve or terminate anything.

## Verification owners

Tasks 011/013 verify enrollment. Tasks 015–018 verify starts, cursors, fencing, and approvals. Task 019 proves freshness bounds. Task 027 verifies the combined failure cases. The [operation matrix](../contracts/operation-matrix.md) and [walkthroughs](../contracts/failure-walkthroughs.md) are the shared behavioral inputs.
