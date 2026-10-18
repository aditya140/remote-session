# ADR 0004: Define one bounded public wire contract

Status: selected for implementation. Task 004 owns Rust types, schemas, codecs, and executable fixtures.

## User task and problem

Clients must reconnect without confusing output cursors, input acknowledgments, request IDs, or control generations. Future JavaScript clients must preserve counters above the safe integer range. The illustrative binary input header does not carry control fences.

## Decision and reason

Use JSON text for typed control/events and binary messages for PTY bytes. Specify each identity and numeric field before implementing independent components. Validate the same contract at every network and IPC boundary.

## Identity and scalar encoding

| Value | Encoding and scope |
| --- | --- |
| Entity ID | A typed prefix plus a canonical lowercase UUID, such as `m_550e8400-e29b-41d4-a716-446655440000`. Types cannot be interchanged. Use UUIDv4 for newly allocated opaque identities. |
| Message ID | `msg_` UUID, unique for each transport envelope. Replies use `re` to identify the request envelope. |
| Request ID | `req_` UUID, stable for one logical SDK call across correlation/lookup. It does not confer mutation idempotency. |
| Start idempotency key | `idem_` UUID, persisted before sending. Scope and fingerprint follow [ADR 0002](0002-state-authority.md). |
| Approval and decision IDs | Separate `a_` and `d_` UUIDs. Stable IDs deduplicate delivery; an envelope ID does not replace them. |
| Recording chunk/receipt IDs | Separate `chunk_` and `receipt_` UUIDs. Bind immutable content and the recovery epoch as specified in [ADR 0005](0005-recording-and-retention.md). |
| Output/input sequence and generation | Canonical decimal string representing an unsigned 64-bit integer. Reject signs, whitespace, leading zeros except `0`, fractions, exponents, overflow, and JSON numeric values. |
| Version and terminal dimensions | Bounded JSON integers. Version components fit `u16`. Terminal rows/columns are each `1..=4096`. |
| Public timestamp | UTC RFC 3339 with milliseconds and `Z`; preserve capture time on retries. Do not use wall time for local lease deadlines. |
| Binary in JSON | Canonical padded base64. Do not use numeric byte arrays for opaque blobs. |

Counter examples are `"0"`, `"9007199254740993"`, and `"18446744073709551615"`. Counters never wrap. Zero represents the initial cursor or absent acknowledgment; active generations and emitted event sequences start at one. Exhaustion returns `capacity_exhausted` and requires explicit recovery.

Persist full counter values without narrowing to signed `BIGINT`: use checked `NUMERIC(20,0)` in PostgreSQL and fixed-width 8-byte big-endian blobs in SQLite. Do not order variable-width decimal text lexicographically. These are owning-store rules, not public database types.

## Envelope and versions

Each envelope contains `v`, `id`, `type`, and a typed `body`. It can contain `re`, `machine`, `session`, and `seq` only where the operation permits them. `v` is the negotiated major version. The handshake carries supported `(major, minor)` pairs, required/optional capabilities, CLI version, and bounded limits.

The initial public version is `1.0`. Select the highest mutually supported pair. Reject an unsupported major or missing required capability before operations begin. Select optional capabilities by intersection. Never advertise an unverified adapter capability. N-2 refers to the eventual CLI release compatibility matrix; there are no historical CLI versions to fabricate now. Task 004 must use synthetic versions for negotiation tests.

Accept additive unknown object fields and ignore them without side effects. Reject unknown operations, security-relevant enum values, duplicate JSON keys, invalid IDs, and invalid known fields. An unknown normalized event kind can appear only in a typed opaque event that clients render as unsupported content. Unknown fields never confer permission or activate a capability. Compatible additions must not add mandatory behavior to an existing message. Breaking semantics require a new major version.

Public schemas describe only public operations. IPC negotiation and administrative requests are separately versioned and excluded. Schema generation must be deterministic and consume the same Rust definitions used by codecs.

## Canonical start fingerprint

Fingerprint the fully validated start parameters after applying explicit protocol defaults: owner/machine, agent or shell kind, command and ordered arguments, cwd, initial prompt, terminal size, and public launch options. Include no provider credentials, access tokens, request IDs, or message IDs. Reject duplicate fields before canonicalization.

Encode the fixed typed object using RFC 8785 canonical JSON and compute SHA-256. Counter fields remain decimal strings. Preserve array order and exact string bytes. Do not normalize shell text or paths into an assumed semantic equivalence. Host filesystem validation precedes the durable accepted record. Task 004 must provide equal/defaulted and changed-parameter fingerprint fixtures.

## Binary PTY messages

Use one complete application message per WebSocket message. Reassemble WebSocket fragments within the declared limit before codec validation. Multi-byte integers use network byte order. Flags and reserved bytes must be zero in v1.

| Offset | Bytes | Meaning |
| --- | --- | --- |
| 0 | 1 | Type: `0x01` input, `0x02` output. |
| 1 | 1 | Flags, zero. |
| 2 | 2 | Reserved, zero. |
| 4 | 16 | Session UUID bytes, without the textual `s_` prefix. |
| 20 | 8 | Input sequence for `0x01`; session output sequence for `0x02`. |
| 28 | Remaining | Typed payload below. |

For output, the payload contains 1–32768 PTY bytes. The sequence belongs to the same session event stream as structured events and resizes.

For input, the payload starts with an 8-byte host generation and an 8-byte controller generation, then 1–32768 PTY bytes. The minimum input message is 45 bytes; the maximum is 32812 bytes. The maximum output message is 32796 bytes. Reject truncated fences, invalid types, empty bytes, and oversized messages. Do not interpret an input counter as an output cursor.

The gateway derives actor and connection identity from the authenticated socket and attachment. Internal routing retains that provenance. Sessiond checks the source holder, active generations, access deadline, and adapter capability before applying bytes. The binary header alone grants no permission. Input acknowledgments use typed JSON and report receipt/application outcome separately from output delivery.

## Sequencing and replay

Sessiond assigns one monotonically increasing output sequence per session across structured events, PTY output, resize events, and local approval-state events. Clients deduplicate by `(session_id, seq)`. Input sequences are scoped to `(session_id, controller_generation)`. They never share the output cursor.

An input acknowledgment means that sessiond accepted the identified input and reports its adapter-write result. It does not prove that a shell executed a command. Duplicate accepted input IDs return the recorded result while that generation is active. Reconnect never automatically resends input.

An attach request supplies its last contiguous rendered cursor. The response identifies `floor`, `head`, and a stream epoch. Sessiond registers a bounded live subscription while capturing the replay boundary. Replay covers the requested suffix through that boundary; live delivery begins after it. Buffer overflow invalidates the attachment and requires another attach.

For PTYs, a screen snapshot includes dimensions, modes, cursor, screen cells, bounded scrollback, and `snapshot_seq`. That snapshot already includes all output through its cursor. Apply only later output afterward. Structured replay that lacks earlier history must declare `history_truncated`; it cannot invent an equivalent complete transcript.

The standalone implementation uses `format: "vt100_state_v1"` and base64 `data` on snapshot pages. Its versioned, validated complete checkpoint includes hidden buffers, saved state, and incomplete parser input; visible ANSI alone is insufficient for subsequent tail interpretation. See the [codec contract](../../vendor/README.md). A correlated snapshot epoch ends with a head response. Unknown formats, missing pages, and unavailable reconstruction require explicit resync/error. The public golden fixture includes a real checkpoint.

The default replay limits are 10,000 events and 8 MiB of event payload per session. Evict oldest entries when either cap is reached. Small-event workloads reach the count cap; large-event workloads reach the byte cap. This resolves the design's ambiguous count/byte wording into bounded independent limits. Limits can be raised by configuration. Return the actual floor/head and explicit truncation.

Every 5 seconds while attached, and on reconnect, reconcile the host head cursor. This detects a lost final frame even when no later output arrives. Do not use the gateway transcript copy as the head authority.

Sequence allocation must not reuse exposed values after crash. Task 016 must persist a high-water reservation before exposure or an equivalent durable guarantee. Skipped reservations or unrecoverable replay storage produce explicit gap/truncation metadata. If the allocator cannot preserve uniqueness, refuse remote publication until recovery while continuing to drain local process output. Recording errors cannot kill the process.

## Limits and overload

These are initial implementation defaults, not measured capacity claims. A peer may negotiate smaller limits. Raising a hard wire limit requires an explicit compatibility change.

| Resource | Initial bound | Saturation behavior |
| --- | --- | --- |
| JSON message | 1 MiB UTF-8; nesting depth 64 | Reject before dispatch; use a bounded parser. |
| Handshake | 16 KiB; at most 64 capabilities | Reject negotiation before allocating session state. |
| PTY byte payload | 32 KiB per message | Split output before encoding; reject oversized input. |
| Snapshot | 8 MiB reconstructed; pages at most 256 KiB | Bound assembly and page count. Reject/resync incomplete snapshots. |
| SDK pending requests | 128 | Reject a new unsent call as `overloaded/not_sent`. |
| Per-connection outgoing lanes | Control: 128 messages/1 MiB; events: 512/2 MiB; bulk: 256/8 MiB | Never silently drop accepted mutation or approval state. Disconnect a slow consumer and require state reconciliation. |
| Active local sessions | 64 by default | Reject new starts before durable acceptance; preserve running sessions. |
| Heartbeat | Ping every 20 s; missing pong deadline 5 s | Close with `4408`. Presence TTL is at most 30 s. |

Count and byte caps both apply. Use weighted priority so bulk output cannot starve control, and control cannot prevent all output indefinitely. Coalesce safe output before sequencing. Every task owner must also bound worker concurrency, retry work, and local storage; these queue defaults do not establish fleet memory capacity.

## Machine challenge bytes

Sign the following exact bytes in order, without JSON encoding:

1. ASCII domain separator `qgenie-remote:machine-auth:v1` followed by a zero byte.
2. Big-endian `u16` length and UTF-8 bytes of the configured HTTPS origin. Normalize its hostname to lowercase, omit port 443, and exclude path, query, fragment, and user information. Reject a noncanonical origin.
3. Raw 16-byte machine ID, key ID, and challenge ID, in that order.
4. A cryptographically random 32-byte nonce.
5. Big-endian `u64` issued-at and expires-at Unix milliseconds.

Bind the key to its machine and owner in durable state. A challenge expires after at most 60 seconds. Consume it conditionally once before issuing a machine token. Verify origin, identity, expiry, and current access, not only the signature. Machine tokens last at most 15 minutes and cannot act as user/admin credentials. Key storage and cryptographic operations stay outside `proto`.

## Stable errors and outcomes

Use stable error codes including `unauthenticated`, `forbidden`, `access_stale`, `unsupported_version`, `unsupported_capability`, `invalid_frame`, `frame_too_large`, `overloaded`, `idempotency_conflict`, `stale_generation`, `already_resolved`, `history_truncated`, and `capacity_exhausted`. Public errors must not expose raw backend diagnostics or secrets.

- `not_sent` means the owning boundary knows the operation was not accepted or applied. It cannot describe a timeout after dispatch.
- `rejected` means a definitive owner response refused the operation with no newly accepted effect.
- `unknown` means the effect cannot be established. It requires operation-specific reconciliation.

A retryable flag concerns the stated operation and outcome; it is not permission to generate a new identity. Start lookup, duplicate approval delivery, and receipt retry use their original identities. Inputs are never resent automatically.

Keep close codes `4401`, `4403`, `4408`, `4409`, `4426`, and `4503` for authentication/revocation, prohibition, heartbeat expiry, replacement, upgrade requirement, and draining. A close code does not establish whether an earlier mutation applied.

## Verification owners

Task 004 must add golden JSON/binary vectors, full-u64 boundaries, malformed input, canonical fingerprints/challenges, unknown-field behavior, and deterministic public schemas. Tasks 005/006 enforce bounds before allocation. Tasks 016/023 verify replay, tail loss, and input acknowledgment behavior.
