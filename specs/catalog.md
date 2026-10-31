# Contract catalog

Status: Contract definitions with partial executable coverage. See [model boundaries](models.md)
for the laws now implemented and the requirements that remain outside their proofs.
The order below groups the contracts. It does not change release priorities.

## First modeling group

### AUTH: Action authorization

Sources: PRD PR-4, SE-8, SE-9, SE-11, SE-12b, PO-1; the
[operation matrix](../docs/contracts/operation-matrix.md).

Define a pure decision over an action and trusted facts. Cover positive permission,
denial precedence, controller independence for approvals, root restrictions, and
termination confirmation. The policy crate owns no mutable authority.

Use the complete [authorization definition](authorization.md) as the first model input.
Compare every action and Boolean-fact combination with `qgenie-remote-policy`.
Verify fact acquisition separately at the gateway, host, and sessiond boundaries.

### LIFE: Session lifetime

Sources: PRD SE-6 and SE-11; [ADR 0003](../docs/adr/0003-session-lifetime-and-ipc.md).

Model attachment state, connector state, sessiond state, and native session evidence
separately. Sessiond owns native execution. Include detach, client exit, connector
pause or crash, revocation, explicit termination, native exit, and sessiond loss.

- `LIFE-001`: Detach, client exit, connector loss, and revocation issue no native termination effect.
- `LIFE-002`: Explicit termination, turn interruption, and historical resume are distinct operations.
- `LIFE-003`: Offline presence does not establish native exit. Insufficient recovery evidence produces loss or uncertainty.

Do not interpret session survival as immunity to native exit, reboot, or sessiond crash.
Under a surviving sessiond and supported native runtime, network lifecycle changes
must leave local execution available. Check this contract with process and service tests.
Keep real systemd, launchd, logout, and suspend qualification explicit.

### START: Durable start and reconciliation

Sources: PRD SE-10; [ADR 0002](../docs/adr/0002-state-authority.md);
the [lost start reply scenario](../conformance/scenarios/start-lost-ack.json).

Model the scoped key `(owner, machine, key)`, parameter fingerprint, allocated session,
launch intent, and native correlation evidence. Sessiond owns the durable start journal.
Distinguish received, accepted, launching, running, failed, unknown, and lost states.

- `START-001`: A scoped key binds one fingerprint and allocated session identity. Changed parameters conflict.
- `START-002`: Recovery preserves that key. A timeout or missing gateway row does not authorize another native launch.
- `START-003`: Acceptance precedes native creation. An ambiguous creation remains uncertain until native evidence resolves it.
- `START-004`: Read-only lookup does not launch work. Capacity handling preserves required deduplication evidence.

Specify when a serialized owner can continue an accepted operation. Do not claim
exactly-once native execution from a database transaction alone. Progress requires
available journal storage and a responsive adapter. Compare crash-boundary traces
with the sessiond journal and the client's recovery behavior.

### CONTROL: Activated control and input

Sources: PRD SE-8 and SE-11; [ADR 0002](../docs/adr/0002-state-authority.md).

Model the proposed holder, host generation, controller generation, native activation,
lease deadline, access deadline, and input acknowledgment. PostgreSQL owns the proposal.
Sessiond owns persisted activation and admission of native mutations.

- `CONTROL-001`: Only the activated holder with current generations and valid deadlines can submit controlled mutations.
- `CONTROL-002`: A superseded generation cannot regain control through a delayed acknowledgment, retry, or restart.
- `CONTROL-003`: Takeover requires explicit confirmation. Reconnect does not silently reacquire control.
- `CONTROL-004`: An uncertain input is not automatically resent. Output cursors do not establish input acceptance.

Model proposal, activation, and confirmation as separate steps. Include lease expiry,
failed renewal, competing takeover, counter exhaustion, and lost replies. Progress
requires current access and successful activation. Compare gateway, sessiond, and SDK
traces, including rejection immediately before the native effect.

### APPROVAL: Decision winner and native application

Sources: PRD SE-9; [ADR 0002](../docs/adr/0002-state-authority.md);
the [lost application reply scenario](../conformance/scenarios/approval-lost-application-ack.json).

Model pending request identity, bound preflight, committed winner, delivery obligation,
native application journal, and application evidence. PostgreSQL owns the winner,
audit, and outbox transaction. Sessiond owns native application evidence.

- `APPROVAL-001`: Each request has at most one committed remote winner. Cancellation or timeout can leave no winner.
- `APPROVAL-002`: A winner, its audit, and its delivery obligation commit atomically. Redelivery preserves the decision identity.
- `APPROVAL-003`: A committed winner does not imply native application. Preserve applied, already-applied, cancelled, and unknown results.
- `APPROVAL-004`: Current policy and access still apply before native submission. Lost replies do not permit blind resubmission.

Use AUTH for owner eligibility without a control lease or attachment. Define progress
as continued delivery and reconciliation under valid access, available storage, and
responsive peers. Do not require eventual application after cancellation or policy change.
Compare winner races, policy changes, journal recovery, and lost native acknowledgments.

### ACCESS: Revocation and authority expiry

Sources: PRD ID-6 and PO-5; [ADR 0002](../docs/adr/0002-state-authority.md) and
[ADR 0009](../docs/adr/0009-revocation-completion.md).

Model durable disable flags, directory evidence, authoritative-read age, native grant
nonce, deadlines, and revocation status. PostgreSQL owns durable access decisions.
Model clocks that account for suspension and refuse authority after clock failure.

- `ACCESS-001`: The accepting gateway denies affected access after revocation commit. Other authority expires within the stated budget.
- `ACCESS-002`: Delay, retries, token refresh, and Redis hints cannot extend the original authority budget.
- `ACCESS-003`: Directory verification cannot clear manual revocation. Failed verification cannot refresh permission.
- `ACCESS-004`: Completed revocation means prior authority cannot admit remote I/O. It does not mean processes ended or peers acknowledged.

Keep denial deadlines separate from conservative completion reporting. Define the
10-second remote enforcement bound and the 15-minute directory-disablement requirement
with their distinct starting events and assumptions. Follow ADR 0009 for completion
after restart, observation eviction, or clock failure. Validate timing in deployment
tests; a modeled clock does not qualify a real tenant or suspended machine.

## Subsequent modeling group

These domains have selected executable models, except identity enrollment. Their
boundaries below describe the wider contract; they do not declare its laws complete.

| Domain | State and authority | Required contract | Sources and implementation evidence |
| --- | --- | --- | --- |
| REPLAY | Sessiond owns stream identity, head, floor, snapshot boundary, and retained suffix. The client owns its rendered cursor. | Apply a snapshot through S and subsequent events above S. Detect final-tail loss. Report truncation or overflow and resynchronize. Keep recording history separate from live replay. | PRD SE-7; failure walkthroughs; sessiond replay and client rendering tests. |
| RECORD | Sessiond owns spool and gap evidence. PostgreSQL owns stored chunks, receipt ledger, and recovery epoch. | Retries preserve immutable identity and content. Reclaim only from an applicable receipt or recorded loss transition. Storage failure does not stop execution. Metadata failure exposes uncertainty. Restore invalidates unsupported availability claims. | PRD PO-4; [ADR 0005](../docs/adr/0005-recording-and-retention.md); spool, ingest, disk-full, and restore tests. |
| HISTORY | PostgreSQL owns read authorization, audit, hold coverage, retention eligibility, and deletion coordination. | Release only authorized history. Audit admin reads before releasing content. Preserve content covered by a committed hold before purge. Overlapping holds remain effective. Retry does not reset content age. | PRD PO-6 through PO-8; ADR 0005; application-role RLS, pooled-context, and hold/purge race tests. |
| IDENTITY | The credential store owns installation identity. PostgreSQL owns enrollment and key bindings. Verified identity evidence binds the remote actor. | Enrollment retry preserves identity. Hostname changes do not change ownership. Key loss requires explicit recovery. Invalid audiences, foreign owners, and foreign OS users cannot cross their boundaries. | PRD ID-1 through ID-5 and ID-8; ADR 0002 and ADR 0003; identity, enrollment, and native IPC tests. |
| PROTOCOL | Protocol definitions own valid operations and encodings. Each receiving component owns validation and bounded admission. | Invalid or oversized frames cause no dispatch. Numeric identities preserve precision. Capabilities constrain operations. Public remote messages cannot invoke local administration or a generic native tunnel. | PRD SE-2a and compatibility requirements; [ADR 0004](../docs/adr/0004-wire-contract.md); protocol fixtures, schema checks, and transport tests. |
| CLIENT | The SDK owns request and reconciliation state. The TUI owns selection, confirmation, and displayed evidence. | Show observation separately from control. Bind confirmation to the intended target. Do not display uncertain mutations as success. Do not act on hidden selections or silently resend input. | PRD CL-1, SE-8 through SE-11; SDK recovery tests and actual TUI interaction tests. |

### Client workflow laws

Sources: the accepted [daily workflow](../tasks/033-implement-daily-remote-workflow.md)
and [remote command contracts](../docs/integration/remote-commands.md).

- `CLIENT-006`: Only temporary access or capacity loss permits observation reconnection.
  Authentication, authorization, and protocol failures stop recovery.
- `CLIENT-007`: A caller-owned credential channel requires a live publisher, a valid
  deadline, and no sign-in requirement. Losing any requirement denies local credential use.
- `CLIENT-008`: A terminal event proves success only with an explicit completed outcome.
  Failed, interrupted, and legacy unknown outcomes remain distinct.
- `CLIENT-009`: A targeted wait uses the newest matching terminal evidence. Events for
  another turn cannot satisfy the target. Missing evidence cannot prove completion.

See [model boundaries](models.md) for the symbolic proofs, bounded SDK comparisons,
and implementation behavior that remains outside these models.

## Definition decisions still needed

Before writing each stateful model, define its failure alphabet and trace observation points.
Include failures before and after durable acceptance and native effects where applicable.
Document which native adapter evidence the model trusts. Keep unsupported native
reconnection explicit; an abstract recovery transition must not invent that capability.

Choose concrete model bounds and their relationship to implementation limits. Define
how a Rust event maps to a model event, including internal steps with no public output.
The current models use finite-trace induction for selected safety laws. Timed system
composition and eventual-progress reasoning remain to be designed. Keep these separate
from the checked local deadline-preservation laws.

Keep native service survival, real identity-provider behavior, production durability,
capacity, and latency as separate qualification obligations. The
[external release gates](../docs/validation/external-release-gates.md) remain applicable.
