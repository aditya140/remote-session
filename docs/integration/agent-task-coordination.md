# Decision: defer automatic agent task coordination

Status: **No-go for a runtime orchestration engine.** This is a research decision
for issue #7. It is not a first-release feature or a claim of pilot validation.

## User task and evidence

A developer may ask several agents to review a change, or run a test agent beside
a development server. The developer needs exact results, visible attention
states, and a way to recover uncertain starts.

The current implementation provides named launch profiles, exact prompt receipts,
a work inbox, and explicit any/all waits. These functions establish the manual
workflow. No pilot usage records were supplied for this decision. We do not yet
know how often developers need automatic delegation or messages between agents.

Keep the manual workflow for the pilot. Measure its remaining problems before
adding another state machine. Do not add a new crate, scheduler, queue service,
implicit child creation, or automatic prompt delivery in this change.

| Candidate pilot task | Current workflow | Remaining question |
| --- | --- | --- |
| Parallel code and test review | Start two named profiles. Submit each prompt once. Save each exact turn receipt. Use an all wait and the work inbox. | Does manual assignment or collecting results cause repeated errors? |
| Server plus test agent | Start independent sessions. Wait for a fresh readiness literal after the captured boundary. Observe the test agent's exact turn. | Does the developer need durable dependencies beyond the current explicit waits? |
| Specialist follow-up | Open the result and submit a new prompt with explicit control. | Would a recorded message inbox save work without hiding its source or granting authority? |

These are proposed pilot tasks. We have not measured their frequency or benefit.
A running process, terminal title, or inferred text match cannot establish that an
agent task succeeded. Missing history and incomplete scans remain unknown.

## What Herdr demonstrates

The inspected source is Herdr revision
[`7663c204468e9cdc6b62f5e89aea938be72f6373`](https://github.com/herdrdev/herdr-core/tree/7663c204468e9cdc6b62f5e89aea938be72f6373).
Its [parent and child tests](https://github.com/herdrdev/herdr-core/blob/7663c204468e9cdc6b62f5e89aea938be72f6373/crates/herdr-core/tests/m6.rs)
cover inferred parents, explicit parents, scope, and children that outlive a parent.
Its normal policy ends children with their parent.
Its [notes implementation](https://github.com/herdrdev/herdr-core/blob/7663c204468e9cdc6b62f5e89aea938be72f6373/crates/herdr-core/src/notes.rs)
types marked child-status notes into a parent agent. It avoids blocked dialogs.
Its [wait contract](https://github.com/herdrdev/herdr-core/blob/7663c204468e9cdc6b62f5e89aea938be72f6373/crates/herdr-core/src/api/types.rs)
uses child activity and unseen news for any/all waits.

We inspected these sources. We did not build or execute Herdr. These mechanisms
suggest useful UX, but they do not establish our authorization or survival policy.
QGenie must keep durable sessions alive when a parent UI exits, a client disconnects,
a connector pauses, or remote access is revoked.

## Proposed boundary if pilot evidence supports a prototype

A task association would be metadata between independently owned sessions. The
native owner would remain the sole lifecycle authority. PostgreSQL would remain
the durable authorization authority. The work inbox would remain a projection.
No parent record could grant permission to a child session.

Use this explicit identity in a future design:

- Record the gateway origin, owner ID, machine ID, and native session ID for each
  endpoint. A display name or reusable profile name is not an identity.
- Record the parent task ID and child task ID as relationship references. Reject
  cross-owner relationships and cycles. Start with one parent per task.
- Persist the original start key and launch fingerprint before sending a child
  start. On an unknown reply, look up that same key. Do not generate a new key.
- Record the exact native turn ID from the prompt receipt. Do not derive it from
  recent events, screen text, or an idle process.
- Scope a message to its sender, receiver, exact session lifetimes, and optional
  turn IDs. Include its original message ID and source event cursor for deduplication.

A proposed message inbox would display data with provenance. Agent text, terminal
output, names, and status summaries could not become instructions to the transport.
Do not type a note into a terminal or submit it as a prompt automatically. A future
explicit send action would require current authorization and control. It would use
the existing uncertain-input rules. Message delivery acknowledgment would mean
stored or displayed, not that a tool ran or a turn succeeded.

| Action | Required authority and effect |
| --- | --- |
| Observe child state | Current authorization for that child. No control lease. |
| Cancel a wait or close the parent UI | Stop the owned observation task only. |
| Interrupt a child turn | Explicit request with the child's current control lease. |
| Terminate a child process | Separate confirmed termination with its existing audited authorization. |
| Approve a child request | Independent approval rights and the existing single winning decision. |
| Revoke parent access | Remove remote authority. Preserve native child lifetime and recording. |

An association could never cascade termination. A future explicit multi-session
termination action would need its own product review. It is not part of this design.
Keep the current single-user gateway trust model.

## Proposed bounds and recovery tests

These limits are prototype proposals, not implemented limits: 16 active associated
tasks per group, depth 2, 64 pending messages per receiver, and 4 KiB per message.
Use explicit capacity errors. Do not evict unseen messages silently. Apply a
configured retention policy to acknowledged messages. Bound all background work,
and give each task an owner and shutdown path. Record message content and explicit
loss under the existing recording and redaction policy before claiming delivery.
The storage and retention model requires review before implementation.

A prototype must pass these acceptance cases:

1. Lose a child-start reply after native acceptance. Reconcile the original key
   and prove that only one native process exists.
2. Disconnect the parent, pause its connector, and revoke access to a referenced
   machine. Prove that child processes survive. Stop remote actions only in the
   revoked scope; do not invent a cascading permission change.
3. Cancel an any/all wait. Prove that no child prompt, interrupt, approval, or
   termination was sent. Preserve results already observed.
4. Deliver duplicate or out-of-order messages. Display the original provenance
   once. Reject cross-owner, wrong-lifetime, cyclic, and over-depth references.
5. Fill every proposed bound. Report capacity or explicit recording loss. Prove
   that unrelated sessions still accept valid input.
6. Include approval-like instructions and terminal controls in message text.
   Render them as data. Do not answer approvals or acquire control.
7. Replace a child after a real process loss. Assign a distinct native session ID.
   Keep the old history and failed or unknown task result visible.

## Evidence required to revisit

Run a proposed two-week pilot with at least three developers and record at least
20 uses of the workflows above. Record manual steps, uncertain-start recoveries,
missed attention, and time spent collecting results. Avoid recording credentials
or unnecessary prompt content in the pilot report.

Revisit if at least two developers repeatedly need the same missing coordination
step and the simpler workflow cannot address it. These thresholds are a proposed
review rule, not measured evidence. Compare a message-inbox prototype with a full
scheduler before choosing implementation scope.

Issue #7 remains open for pilot evidence and a later go/no-go review. This PR
completes the bounded design investigation. It does not implement delegation.

Related implementation: [prompt receipts](remote-commands.md),
[launch profiles](launch-profiles.md), [observation waits](observation-waits.md),
and [work inbox](work-inbox.md). The laws in [the architecture](../../architecture.md)
and the [PRD](../../PRD.md) remain authoritative.
