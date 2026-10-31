# Decision: defer live native-owner replacement

Status: **No-go for a live native-owner handoff in this release.** This is the
research decision for issue #8. It does not add a holder process or claim that an
old binary can transfer live sessions to a new binary.

## User task, current behavior, and missing evidence

A developer may need a native-owner security fix while a long-running task is
active. Connector replacement already preserves native sessions. Replacing the
native owner has a different failure boundary.

The current service installation path defers a native-owner update while sessions
are active. The composed Linux test checks the deferred result and unchanged
native service unit. A local user can accept disruption through the existing
explicit policy. Normal client disconnect, connector pause, and access revocation
preserve the native session. These facts do not prove native-owner replacement.

No developer report or pilot measurement establishes how often deferred updates
block necessary work. No released old/new native binary pair was supplied or
qualified. Keep the current policy. Record the duration of deferred updates,
the reason for urgency, and the cost of waiting before funding a prototype.

| Option | Benefit | Cost and present evidence |
| --- | --- | --- |
| Defer native-owner updates until sessions finish | Keeps one owner and the current recovery rules. | Can delay an urgent update. Current Linux service-install behavior is tested. |
| Explicitly stop work and update | Makes interruption visible and operator-controlled. | Loses active process state. A fresh process is a new lifetime. |
| Keep a permanent PTY holder and replace its higher-level core | Could move some upgrades outside the process-handle owner. | Adds another protocol, service lifetime, failure boundary, and state split. Our current native owner also owns agent and durable state. |
| Transfer a live native owner | Could preserve active work during an owner update. | Requires a tested ownership protocol for every state below. No implementation or old/new proof exists. |

## What Herdr establishes in source

We inspected Herdr at revision
[`7663c204468e9cdc6b62f5e89aea938be72f6373`](https://github.com/herdrdev/herdr-core/tree/7663c204468e9cdc6b62f5e89aea938be72f6373).
Its [holder](https://github.com/herdrdev/herdr-core/blob/7663c204468e9cdc6b62f5e89aea938be72f6373/crates/herdr-core/src/holder.rs)
owns PTYs and process reaping. It continuously reads output, retains a 1 MiB tail
per terminal, and limits core output backlog to 8 MiB. The holder and core have
different ownership boundaries. Replacing the core can leave the holder alive.

The [upgrade tests](https://github.com/herdrdev/herdr-core/blob/7663c204468e9cdc6b62f5e89aea938be72f6373/crates/herdr-core/tests/upgrade.rs)
include recovery from the installed executable. The old/new release test requires
`HC_OLD_BIN` and returns without running when it is absent.
The [upgrade script](https://github.com/herdrdev/herdr-core/blob/7663c204468e9cdc6b62f5e89aea938be72f6373/scripts/upgrade-test.sh)
builds that old binary from a selected revision. We did not execute these tests.
Their source does not prove that our `sessiond` can hand off its native authority.

## State that a future design must account for

| State | Current authority | Required handoff property |
| --- | --- | --- |
| PTY master, child process group, wait/reap rights | Native owner | Preserve the same child process and terminal. Transfer or retain exactly one effective mutation and reaping authority. |
| Pending input and partial output | Native owner and bounded adapter queues | Fence old writes. Account for queued bytes and partial UTF-8/control sequences. Do not resend an uncertain input. |
| Terminal parser and replay cursor | Native session state | Preserve parser state, event sequence, retained floor, and explicit gaps. A new screen cannot silently replace missing history. |
| Start keys and input receipts | Native SQLite journal | Keep original identities and acknowledgment outcomes. Permit one writer. Recover the original key after a lost reply. |
| Recording capture, spool, and receipts | Native recording worker and gateway commit authority | Preserve reserved sequences, unfinished fragments, pending chunks, loss ranges, and verified receipt identity. Duplication is idempotent; omission is explicit. |
| Controller lease and generation | Native authority with gateway authorization | Preserve original expiry and advance the fence. Old-owner messages and old controller credentials cannot regain authority. |
| Remote access challenge and connector generation | Existing gateway and native authority rules | Revalidate current authorization. A successful handoff does not extend access or undo revocation. |
| Approval request and decision | Gateway durable decision plus native application journal | Preserve exact native request identity, the winning decision, application outcome, and uncertain delivery. A control lease cannot approve. |
| Structured Codex process and local transport | Managed adapter in native owner | Account for local WSS requests, response correlation, private endpoints, native buffers, and child parentage. A PTY descriptor alone is insufficient. |
| Session identity and public history | Original native lifetime and gateway records | Keep identity only for verified continuation of the same process. A restart after loss receives a distinct session ID. |

The issue #6 spool fix releases the owner's lock explicitly after SQLite closes.
A duplicate lock descriptor can no longer retain ownership after normal close.
A future handoff cannot treat possession of a duplicated descriptor as authority.
It must acquire and fence ownership through an explicit protocol.

## Proposed Linux-only experiment, if the need is demonstrated

Start with a PTY-only experiment outside production launch paths. Do not include
Codex until its separate transport and process-ownership design is reviewed.
Keep the current owner authoritative while a successor prepares. The successor
must not write input, resize, decide approvals, mutate SQLite, or acknowledge
capture during preparation.

A future protocol must have one durable authority epoch and an exclusive mutation
gate. Preparation does not change that epoch. Fence the old owner before the new
owner activates. Never let a timeout or missing acknowledgment authorize both
owners. The new owner must read the committed epoch and verify current rights.
The old owner may resume only if no successor committed authority and it still
owns the exclusive gate. Otherwise it remains fenced.

This is a required invariant, not a complete handoff algorithm. Linux descriptor
passing does not by itself transfer process parentage or reaping rights. A
retained holder may be necessary. That would change the architecture and needs
its own product decision. Do not hide a permanent new service inside an upgrade
helper. Bound preparation time, retained descriptors, buffers, and rollback work.

| Failure point | Required safe result before a prototype can pass |
| --- | --- |
| Successor fails during preparation | Old owner remains the only authority. No mutation or input is repeated. |
| Old owner fails before a durable transfer | Recover only state that can be verified. Report lost or unknown if process continuity is not established. |
| Descriptor delivery succeeds but activation does not | The prepared successor cannot act. Keep or release descriptors under a bounded recovery rule. |
| Authority commits but the ready acknowledgment is lost | Read the committed epoch. Old owner stays fenced. Never activate two owners or resubmit a prompt. |
| Old owner sends a late reply or close | Reject its stale epoch. Do not disconnect or mutate the successor. |
| Successor fails after authority commits | Preserve the committed fence. Recover acknowledged journals and explicit loss. Do not roll back to a stale writer. |
| Both versions fail | Use normal owner-loss recovery. A new child is a distinct lifetime; do not claim seamless continuation. |
| Recording storage or agent transport cannot transfer safely | Abort preparation before activation, or report the exact continuity failure. Never silently claim complete capture. |

## Schema and rollback constraints

The original preview uses native schema 7. Prompt receipts introduce schema 8.
An old binary that rejects the newer schema cannot safely resume after migration.
A future handoff must either keep a compatible schema epoch for both versions or
refuse the transition before activation. Qualify forward and rollback paths with
the actual binaries. Do not restore an old SQLite backup over acknowledged input,
start identities, approval outcomes, or recording receipts.

Gateway client N-2 compatibility does not imply native database compatibility.
A release manifest must name each binary revision, supported native schema,
recording schema, and transition direction.

## Required evidence before changing the decision

1. Record a repeated developer need for urgent updates during active work. Compare
   the measured disruption with the current deferred-update policy.
2. Review the exact Linux ownership and fencing algorithm. Identify who owns each
   state in the table at every phase. Bound all queues and retained resources.
3. Build immutable old and new binaries. Launch a real PTY with the old binary.
   Update to the new binary and prove process identity, input, resize, output,
   recording, and cursor continuity. A missing binary is an unrun test, not a pass.
4. Inject each failure above before and after every durable boundary. Test lost
   replies, delayed old messages, connector replacement, revocation, and disk full.
5. Show that old controllers cannot regain authority. Preserve successful input
   receipts and exact start keys without duplicate native effects.
6. Qualify structured Codex separately with its pinned real native binary and
   approval flows. Do not infer it from a shell PTY test.
7. Run native Linux service-manager tests. Treat macOS launchd and Windows service
   behavior as separate release gates with separate evidence.

Issue #8 remains open for demonstrated need and real binary qualification. This
PR supplies the decision, state inventory, and failure criteria. No runtime
handoff support is added.

See [the owner and connector architecture](../../architecture.md),
[ADR 0003](../adr/0003-session-lifetime-and-ipc.md),
[interface compatibility checks](interface-checks.md), and
[external release gates](../validation/external-release-gates.md).
