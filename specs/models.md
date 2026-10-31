# Executable model boundaries

The suite contains fourteen small Bend models. Each directory has a `Model.bend`,
a `LAWS.bend`, and a `PROOF.bend`. The laws constrain model behavior. They do not
establish that a database, operating system, adapter, or Rust implementation behaves
like the model. Use the [catalog](catalog.md) for requirement sources and authorities.

All models are pure. They contain no foreign implementation, unsafe definition, or
open proof. `Common.bend` defines natural-number comparisons and supporting proofs.
Natural-number identities are opaque symbolic values. They do not imply that an
implementation can use arbitrary-size counters or convert a UUID to a JavaScript number.

The trace proofs quantify over arbitrary finite event lists. They are not bounded
enumerations. Positive transition laws establish that permitted work can proceed.
The suite does not yet prove eventual progress under a fair scheduler.

## Authorization

Entry: [authorization/LAWS.bend](authorization/LAWS.bend).

The input is one action and eleven trusted Boolean facts. The result is an allowance
or stable denial code. The model has no state. Eleven named laws cover approval and
root independence, revocation, non-ownership, freshness, capability, input control,
and termination confirmation. The full permission equivalence in `AUTH-001` is checked
by exhaustive comparison, not by a separate Bend theorem in this revision.

The conformance runner checks all 18,432 inputs against the actual Rust policy function.
It compares allowance, error code, public outcome, and retryability. It also evaluates
the independent permission definition from [authorization.md](authorization.md).
Authentication, fact acquisition, and error-message wording remain outside this model.

## Start recovery

Entry: [start/LAWS.bend](start/LAWS.bend).

One model instance represents one durably accepted `(owner, machine, key)` record.
It contains an immutable fingerprint and session identity plus a launch phase.
`Accepted` is the initial phase. Dispatch moves it to `Launching` and emits one launch.
Native evidence can resolve launch success or definitive failure. A crash during an
ambiguous launch or after a running observation produces `Unknown`. Lookup and lost
replies do not launch work.

The laws preserve identity and bound launch effects over every finite event sequence.
They also prove that an equal fingerprint matches and that lookup is read-only.
The model trusts durable record creation, key uniqueness, serialized ownership, and
native evidence. It does not prove the database index, fingerprint hash, or process
discovery implementation. It does not permit relaunch after a crash or native loss.

Compare future traces with sessiond's [start owner](../crates/qgenie-remote-sessiond/src/runtime/start.rs)
and the SDK's [start recovery](../crates/qgenie-remote-client/src/remote_start.rs).

## Approvals

Entry: [approval/LAWS.bend](approval/LAWS.bend).

Each request starts as `Pending`. A valid commit creates `Won` and one atomic
winner/audit/outbox effect. Cancellation can close a request without a winner.
The native application state is separate: queued, submitted, applied, cancelled,
or unknown. The delivery permission fact represents current access and host preflight.

The laws preserve an established winner over every finite event sequence. A separate
trace law bounds native submission to at most one in this conservative adapter model.
An unknown application can become applied through native evidence. It cannot submit again.

The atomic effect is a required transaction boundary, not proof of SQL atomicity.
Request identity, hashes, preflight expiry, native cancellation evidence, and durable
journal recovery must be supplied by the integration. Duplicate delivery returns
existing application evidence; the model does not add a distinct wire-level
`already_applied` response state. Native `applied` evidence is trusted.

Compare future traces with [gateway approvals](../crates/qgenie-remote-gateway/src/approvals.rs)
and [sessiond approvals](../crates/qgenie-remote-sessiond/src/approvals.rs).

## Control

Entry: [control/LAWS.bend](control/LAWS.bend).

A lease contains a holder, host generation, controller generation, activation flag,
and lease/access validity facts. Admission compares all identities and validity facts.
Reconnect clears activation. Activation requires a newer numeric controller generation
and explicit confirmation. A separate helper exposes that decision as Boolean guards.

The laws cover inactive and expired-access denial, eligible-holder success, ignored
stale activation, equal-generation takeover denial, confirmation, reconnect, and no
automatic resend of uncertain input. A proposed lease represents a validated activation
proposal; validation and persistence are external obligations. This model does not yet
encode PostgreSQL proposal/sessiond acknowledgment/gateway confirmation as three states.

Compare future traces with [gateway controllers](../crates/qgenie-remote-gateway/src/controllers.rs)
and [native control](../crates/qgenie-remote-sessiond/src/runtime/control.rs).

## Session lifetime

Entry: [lifetime/LAWS.bend](lifetime/LAWS.bend).

The model maps lifecycle events to effects. Network events retain execution. Explicit
confirmed termination emits a kill. Interruption, native exit, and owner loss remain
separate effects. A trace law proves that any sequence of network lifecycle events
emits no kill. A native process can still exit independently.

This is an effect contract. It does not model OS process handles, service cgroups,
logout, or reboot. Those assumptions still require the native tests in ADR 0003.

## Access expiry

Entry: [access/LAWS.bend](access/LAWS.bend).

A grant starts with an already established deadline in a single local clock domain.
Access requires a healthy clock, no revocation, and a time strictly before that deadline.
Delay, retry, token refresh, directory success, and Redis hints cannot move the deadline.
Revocation and clock failure remove authority. A trace law preserves the original deadline.

Creating a new grant from a primary read and sessiond nonce is outside this model.
The implementation must establish the ten-second maximum and directory budget before
constructing the grant. The 15-minute Entra requirement and ADR 0009 completion-reporting
algorithm are defined in the catalog but are not proved by this deadline model.

Compare future traces with [revocation propagation](../crates/qgenie-remote-gateway/src/revocation/propagation.rs)
and the native authority boundary. A fresh valid grant must use a separate creation
transition; retries of an old grant cannot represent it.

## Replay

Entry: [replay/LAWS.bend](replay/LAWS.bend).

The model starts with a contiguous rendered cursor in one verified stream epoch.
A snapshot establishes that cursor. The next sequence applies, an earlier sequence
is a duplicate, and a forward gap requires resynchronization. Overflow also requires
resynchronization. Head comparison detects an omitted final event.

The laws cover symbolic snapshot and sequence boundaries. They do not prove terminal
screen reconstruction, subscription cutover atomicity, stream identity validation,
or real network delivery. Compare future traces with [client replay](../crates/qgenie-remote-client/src/remote_replay.rs)
and [sessiond replay](../crates/qgenie-remote-sessiond/src/runtime/remote_replay.rs).

## Recording

Entry: [recording/LAWS.bend](recording/LAWS.bend).

A chunk starts retained. Identity, digest, and capture time remain unchanged on upload
retry. A reclaimable receipt requires matching identity, matching recovery epoch,
and durable acceptance. Capacity pressure distinguishes a committed gap from uncertain
loss when metadata cannot be persisted. Storage events never request process termination.

Receipt validation and gap durability are trusted facts supplied by storage owners.
The model does not compute hashes, write gaps, or prove synchronous replication.
Its single-chunk state does not establish range coverage across a complete spool.
Compare future traces with [spool reclamation](../crates/qgenie-remote-sessiond/src/recording/spool/reclaim.rs).

## Retention and legal holds

Entry: [retention/LAWS.bend](retention/LAWS.bend).

One retained item has an expiry fact and coverage from two distinct holds. Place,
release, purge, and upload retry are serialized lifecycle transactions. The laws cover
preservation by a committed hold, overlapping coverage, eligible purge, non-retroactive
hold placement, and unchanged age on retry. They distinguish both orders of a hold/purge race.

The model is a two-hold instance. General hold sets, monthly partition boundaries,
held copies, referenced metadata, and backup expiry remain outside its proof scope.
The expiry fact and transaction serialization must be established by the database.
Compare future traces with [holds](../crates/qgenie-remote-gateway/src/transcripts/holds.rs)
and [retention](../crates/qgenie-remote-gateway/src/transcripts/retention.rs).

## Bounded admission and counters

Entry: [bounds/LAWS.bend](bounds/LAWS.bend).

A budget contains used and remaining units. One request either consumes one remaining
unit or rejects without change. Total capacity is invariant for any finite request count.
An exhausted counter does not reset its used value to zero.

Instantiate the initial total with the applicable queue or counter limit. The proof
uses natural numbers. Fixed-width conversion, simultaneous count/byte limits, queue
release, and concurrent admission require separate implementation checks.

## Capability negotiation

Entry: [capabilities/LAWS.bend](capabilities/LAWS.bend).

One capability is available only at the intersection of client, gateway, host, and
adapter support. A missing required capability rejects negotiation. Laws include
successful agreement and missing-participant denials.

This model applies per capability. It assumes the flags describe verified support.
Version ordering, full capability sets, and downgrade policy remain governed by ADR 0004
and the Rust negotiation tests.

## Protocol

Entry: [protocol/LAWS.bend](protocol/LAWS.bend).

The model defines a typed token encoding for input and output messages. It preserves
symbolic session identity, sequence, host generation, and controller generation.
Round-trip equality holds for every modeled message. Truncated input, invalid framing,
and absent tags produce no dispatch.

This is an abstract token codec. It is not the production JSON, decimal-string, UUID,
base64, or binary PTY codec. Numeric precision in those encodings still needs the
[Rust protocol conformance suite](../crates/qgenie-remote-proto/tests/conformance.rs).
Public administration is excluded from this model's message type; production schema
exclusion remains an integration check.

## Client confirmation and outcomes

Entry: [client/LAWS.bend](client/LAWS.bend).

A confirmation binds a target and action. Execution requires that target to remain
visible and both bindings to match. A law denies every changed target under an explicit
inequality premise. Other laws deny hidden selections and changed actions, permit a
matching confirmation, preserve unknown outcomes, and forbid automatic input resend.

The model does not establish TUI keyboard handling or atomic selection snapshots.
The SDK and TUI must preserve the modeled binding until dispatch.

## History privacy

Entry: [privacy/LAWS.bend](privacy/LAWS.bend).

The owner projection contains content and omits hold state and admin-read count.
A two-observation law proves that changing either hidden value cannot change the
projection when content is equal. Other laws deny a foreign owner and require committed
audit plus applicable case context before an admin can receive held content.

This proves projection-level confidentiality only. It does not prove SQL row isolation,
timing confidentiality, or identical content availability after retention. Authentication,
RLS, reason validation, and audit commit remain integration obligations.

## Composition and remaining evidence

Authorization facts, activated control, access deadlines, and storage receipt facts
must be connected to their producing authorities. The suite does not yet prove this
composition. Only authorization has an automated comparison with the current Rust code.
Stateful Rust trace comparisons, general hold sets, full codecs, identity enrollment,
eventual-progress proofs, and deployment qualification remain separate work.

Do not infer coverage of an entire catalog requirement from a related law name.
Use each checked theorem's actual statement and the boundaries above.
