# Contract failure walkthroughs

Status: design review evidence for task 002. These are written state transitions, not executed application tests. Tasks 004 and 015–028 must convert the applicable cases into fixtures and actual fault tests.

## Dropped start acknowledgment

1. The client persists key K and sends validated parameters with fingerprint F.
2. Sessiond commits `(owner, machine, K, F, session S)` before native launch.
3. Sessiond creates the correlated native session, then commits its native identity and running state.
4. The acknowledgment is lost. The client reports `unknown` and queries K.
5. Sessiond returns S. The client does not allocate another start key or repeat native creation.

If the crash occurs after native creation but before its identity is committed, sessiond uses the recorded correlation identity to search. If that adapter cannot prove presence or absence, the result remains `outcome_unknown`. A gateway row missing S does not prove that no session exists. Reusing K with another fingerprint returns a conflict.

Expected invariant: at most one known correlated launch, or explicit uncertainty. A timeout never proves a safe new launch. Owners: 004, 015, 023, 027.

## Stale controller and lost activation reply

1. Controller A uses generation 7. PostgreSQL conditionally proposes generation 8 for confirmed takeover by B.
2. Sessiond commits generation 8 before acknowledging its activation.
3. A delayed input from A arrives with generation 7. Sessiond rejects it without writing to the PTY.
4. The activation reply to B is lost. B has no writable-success response yet.
5. The gateway reconciles generation 8, verifies that it remains current, and reports control to B.

If generation 9 superseded it, the old activation acknowledgment cannot grant B control. Redis loss cannot reset any generation. After sessiond restart, new access/activation is required above the persisted generation floors. An expired access delegation refuses mutation even if a lease projection remains present.

Expected invariant: only the activated holder can mutate. No timeout rolls control back to an older generation. Owners: 012, 017, 023, 027.

## Lost approval delivery and native acknowledgment

1. Sessiond journals native approval A. Two owner clients receive it; neither needs a controller lease.
2. The gateway checks current access and obtains a host-policy preflight for each proposed answer.
3. One conditional transaction commits decision D, its audit record, and its outbox row. The other answer receives `already_resolved`.
4. Delivery of D is lost. The worker redelivers the same D after restart.
5. Sessiond journals D before native submission. A repeated delivery finds that journal entry.

If native submission succeeds but its acknowledgment is lost, report `outcome_unknown` until native lookup proves an outcome. Do not submit another answer based only on a missing reply. If host policy changed or the native request was cancelled before application, report the corresponding cancellation. Keep D as the historical winner. Revocation never turns a pending answer into approval.

Expected invariant: one committed decision with durable delivery; no unsupported exactly-once native-execution claim. Owners: 009, 018, 023, 027.

## Filled spool and lost recording receipt

1. Gateway ingestion is unavailable. Sessiond keeps local execution, live replay, and independent recording capture.
2. The recording content budget fills. Sessiond commits a gap for the oldest unacknowledged range before reclaiming its bytes.
3. Recent output enters the reclaimed space. The TUI and local status show a recording warning.
4. Ingestion recovers. The host uploads stable chunk and gap identities.
5. PostgreSQL commits a chunk and receipt, but the reply is lost. The host keeps that chunk and retries the same identity.
6. The gateway returns the same equivalent receipt. Only then does the host reclaim the matching chunk.

If metadata persistence also fails, report durability uncertainty. On recovery, disclose the interval from the last durable frontier. Do not describe an unwritten marker as durable. If a database restore lost acknowledged data, expose recovery loss and epoch reconciliation rather than claiming that a prior receipt still proves availability.

Expected invariant: storage failure does not stop execution; retained history never conceals known or uncertain loss. Owners: 020, 021, 027, 028.

## Legal hold racing partition deletion

1. A retention worker identifies a monthly partition whose entire interval is older than the cutoff.
2. A platform admin starts a hold for user U. Both paths acquire the PostgreSQL recording-lifecycle lock before changing preservation authority.
3. If the hold commits first, the final retention transaction sees it and preserves covered content plus metadata/audit references before dropping the partition.
4. If the partition drop commits first, the hold can cover only content still retained or otherwise recoverable. Its response must not claim recovery of already purged content.
5. A crash after held-copy commit but before drop leaves protected content. A retry rechecks coverage and deduplicates copies.

Ingest takes the same lock. Late covered content goes to held storage even after ordinary partition retirement. Releasing one hold removes only its coverage. Another overlapping hold still prevents deletion. Hold state and admin reads remain absent from owner responses.

Expected invariant: no content covered by an already committed hold is lost to a later purge. A new hold cannot retroactively preserve content that was already purged. Owners: 021, 022, 027.

## Missed revocation hint and stale directory verification

1. A gateway commits a revocation and its audit event, then immediately blocks affected local access.
2. Suppress every Redis invalidation hint. Other gateways continue their primary-state checks at most every 5 seconds.
3. A cached result expires no later than 10 seconds from the start of its authoritative read. Failed checks cannot extend it.
4. Sessiond's nonce-bound access renewal also expires within the remaining authority horizon. A delayed renewal cannot add another full 10 seconds to an old cache result.
5. User and machine sockets close without termination commands. Local processes and pending native requests survive.

For Entra, stop authoritative directory verification. A successful prior check expires at most 840 seconds after its request began. Gateway enforcement adds at most 10 seconds. This is a design budget of 850 seconds; the actual integration and disablement drill remain unverified. A later successful directory result cannot clear manual revocation.

Expected invariant: no Redis hint or valid token can extend stale authority. Owners: 011, 019, 027.

## Lost final output and snapshot cutover

1. A client has rendered cursor 120. Sessiond emits 121, but the network drops it and the session becomes idle.
2. The client's next head reconciliation reports head 121. It requests the missing suffix.
3. During attach, sessiond captures a replay boundary and registers bounded live delivery without an unobserved interval.
4. If 121 is retained, replay it once. If the cursor is below the floor, report truncation and provide a PTY snapshot at S.
5. Render the snapshot as state through S, then apply only events above S. An overflow invalidates the attachment and requires resync.

Expected invariant: no hidden tail loss or duplicate rendering at snapshot/live cutover. Input acknowledgment remains a separate cursor and never triggers automatic input resend. Owners: 016, 023, 027.

## Connector pause and service update

1. Start a real PTY and a verified Codex session under sessiond.
2. Stop the connector service, including its service-manager cgroup or job.
3. Sessiond continues both sessions and recording in its independent service lifetime.
4. Resume the connector. Activate a fresh host generation, reconcile session state, and upload retained spool data and gaps.
5. Defer sessiond binary replacement until it is idle, unless a local user explicitly accepts a disruptive update.

Expected invariant: network/service lifecycle does not imply process termination. This walkthrough is not a Linux or macOS service proof. Owners: 025, 027.
