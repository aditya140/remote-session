# ADR 0009: Conservative revocation completion from bounded authority expiry

Status: selected for the Linux standalone development scope; real-process proofs in task 019.

## Decision

`completed` means that authority predating the committed revocation can no longer admit
remote I/O. It does not mean native processes were terminated, approvals were answered or
every disconnected peer acknowledged a broadcast. Running local sessions remain alive.

The existing access contract bounds gateway socket authority to ten seconds from the
primary read's start. Native grants receive only that remaining budget, anchored at their
earlier sessiond-issued challenge. Every actual native write retains that original grant;
gateway and native clocks include suspension and fail closed on failure/regression.
Delayed queues, retries, reconnection and token refresh cannot add another budget.

After committing or reading the exact revocation row from the primary, a gateway records
a local suspend-aware observation. A full `MAX_ACCESS_LIFETIME_MS` later, every authority
window that could have preceded that observation is expired. New primary reads observe
the durable disable flags. Therefore the API may conservatively report `completed`
without waiting for acknowledgments from disconnected hosts. Redis delivery supplies no
completion evidence. UTC `committed_at` remains display/identity metadata only.

The constant is shared by public grant validation, gateway gates, host IPC and native
authority, with a compile-time upper bound of ten seconds. Completion is generally later
than actual denial: it is a conservative report, not permission to delay enforcement.

## Bounded state and recovery

Gateway clones share at most 4096 observations, keyed by authenticated actor and request.
Repeated polls preserve the first observation. Changed durable row identity resets it.
Restart, a different gateway's first observation and bounded cache eviction wait a new
full horizon. No persisted wall-clock time restores completion. Every response still
reads the primary row or completes the original idempotent transaction; SQL failure cannot
manufacture completion. Clock/lock failure refuses the response, and clock regression
permanently prevents completion until process restart.

This status describes the recorded revocation operation. Current access always uses the
current primary flags and directory evidence; status grants no authority. There is no
public unrevocation API, and privileged recovery/restore remains a separate procedure.

## Qualification

Deterministic tests cover polling, elapsed suspension, regression, changed identity,
eviction and restart. Actual two-process gateway tests suppress Redis hints, measure
denial and require a separate node's pending-to-completed transition. The native connector
proof uses the signed admin API and preserves a real local PTY across machine revocation.
Real Entra disablement, macOS/suspend execution and production fault-domain qualification
remain external release gates under the user's Linux development scope.
