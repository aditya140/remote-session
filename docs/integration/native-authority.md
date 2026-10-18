# Native remote authority integration

The native foundation persists ownership/start namespaces, retains directory capabilities through exec, and implements host-floor/access activation over private IPC. Gateway/connector activation and scoped read dispatch use original native nonces through PostgreSQL/Redis/WSS. Public PTY and verified managed Codex starts compose original-authority native launch and mandatory protected recording readiness. Controllers, input, resize, interrupt, confirmed owner termination and lease-independent owner approvals pass the Linux composed fixtures.

## Ownership and cwd

`ipc::RemoteScope` is `(canonical gateway origin, owner, machine)`. SQLite schema 3 stores immutable bindings and each session's original scope. Local/legacy sessions have no remote scope. Every remote list/lookup/action must filter or match the stored scope; never populate it from the connector's current enrollment. Trusted local operations can still manage the same OS user's sessions. Native bindings come from locally trusted enrollment composition; a public remote message cannot create/reassign one.

Scoped keys include origin/owner/machine/key. Their fingerprint excludes transport correlation and start key. Never reuse local unscoped lookup remotely. Historical native resume needs a separate public session/start identity and proven same-scope native correlation.

The connector enforces its filesystem policy. Sessiond independently resolves the requested path under pinned roots without following symlinks, then supplies that directory capability to `PreparedPty::with_directory`. Child `fchdir` and relative executable lookup use the capability. Do not trust a connector's earlier path lookup; no descriptor transfer is necessary merely to reuse it. Existing-session actions do not reapply new-session cwd roots.

## Activation and deadlines

Host generations come from PostgreSQL; native SQLite schema 4 preserves generation floors/connection identity per binding. Lower generations fail after restart/replacement. Native activation acknowledges only after the floor commits; a duplicate acknowledgment cannot extend the original deadline. The gateway confirms that its PostgreSQL proposal remains current before acknowledging activation. Native controllers implement that ordering, explicit takeover, a holder and an original 15-second lease, including public renewal/routing composition.

Native access renewal starts with a sessiond-issued random nonce and local monotonic issue time. A gateway grant binds nonce, scope, current host generation and remaining verified authority lifetime. That lifetime is at most ten seconds minus the authoritative read's age, further capped by directory evidence and credential expiry. Native expiry is `nonce_issued_monotonic + remaining`, never receipt time plus a fresh budget. Restart invalidates nonces. Bound outstanding nonces without extending their original deadlines.

Read-only recording IPC requires `recording-read-v1` as well as native authority. Fetch/status
filter the immutable original scope before selecting a gap or returning queue counts. Release
authority/session locks before worker I/O, then validate the same original nonce again before
returning any content. Expiry or host replacement during I/O denies the result. There are four
recording read slots on the server and one recording slot within the connector's four-request
native budget. Context-bound reclamation separately negotiates `recording-reclaim-v1`.
A native-issued challenge binds an independently authenticated HTTPS backend epoch to the
original scope/connection/generation/access nonce. A receipt cannot supply that epoch.
Reclaim validates exact immutable metadata and the original grant after obtaining the spool
write lock, immediately before logical deletion. Newer bound grants fence older exchanges;
restart forgets epoch pins. Legacy receipt-only messages remain unsupported.

The native side enforces the original budget through ordinary and suspend-aware clocks, bounded challenges/grants and scoped list/lookup. IPC 1.1 negotiates `native-authority-v1`; minor-zero local clients remain compatible. The gateway now caps grants by read-start primary evidence, directory freshness and credential expiry. Current native policy revision is immutable startup revision 1; dynamic host policy and approval preflight invalidation remain.

Bind each dispatched request to the nonce/grant current **at the gateway before enqueueing**. The connector must not assign its newest nonce to an old queued request; that would extend delayed input across expiry. New renewals cannot extend an older request. Gateway-to-host context must carry the original access identity/budget. Native dispatch checks it against its issued nonce, immutable scope, activated generations and current policy.

Serialize native activation against effects. Recheck authority before acceptance, after a blocking SQLite intent commit, and immediately before launch/input/resize/interrupt/approval/termination. A reserved launch supervisor carries/rechecks its original authority before spawn; caller cancellation still cannot orphan or cancel a committed child. Input/interrupt support a monotonic deadline before each nonblocking write; use the minimum of lease and access deadlines. A later write-result renewal cannot revive expiry.

Do not hold a non-sendable mutex guard while waiting for a spawned launch to reacquire it. Define one lock order for authority, starts, session and store. Uncertain writes keep their identity/outcome and are never replayed automatically. Policy revision changes invalidate old preflight facts. Local takeover must fence remote effects too; reconcile a native floor ahead of restored PostgreSQL rather than resetting it.

Input digests use a process-ephemeral key. After native restart a controller generation must advance beyond its retained floor before becoming live; do not reactivate an old generation whose deduplication key is lost. Same-generation acknowledgment reconciliation is safe only for an in-process active generation. Use the owner's input guard to recheck both access and lease clocks on every write. Atomic cwd/executable capability proofs currently cover PTYs; managed-agent launch needs a verified equivalent vendor contract before enabling remote agent starts.

## Integrated read/activation evidence

`NativeClient` owns at most four three-second same-user IPC operations and no native process. Trusted local admin registration adds an immutable binding without restarting the daemon or adopting local history. The data socket cannot represent that command. Sealed challenges/access values preserve the original nonce and an earlier conservative host clock; restricting a dispatch budget never resets either clock. Native and gateway capabilities are acknowledged before operation traffic.

With schema 7 and directory verification, routing requires `host-native-authority-v1`. The connector relays the native challenge, applies a bounded gateway grant, acknowledges the durable native floor, and waits for gateway confirmation that its PostgreSQL proposal remains current. It renews access using new nonces and retains at most four live identities. Host reconnect reserves a higher generation. Native-owner restart loses grants and forces a new activation. Missing acknowledgments cannot reset a durable floor.

PostgreSQL stores the last acknowledged native access identity for source-node routing. Before any local/Redis routing queue, the source chooses that nonce and caps its original budget by the requesting client's current evidence/credential deadline. The destination validates and preserves it. A newer renewal cannot rebind an older queued request. Existing native generations, connection, policy revision and nonce must match; a narrower original budget is allowed. Sessiond independently rechecks original issue time before and after scoped reads.

Actual tests use verified HTTPS/JWKS and synthetic Graph, PostgreSQL/Redis, two gateway compositions in one process, the outbound connector, and a real standalone sessiond process. They verify activation, renewal without generation changes, cross-node scoped lookup/filesystem reads, replacement with an unacknowledged proposal, native fencing, revocation and local child survival. Separate tests use two actual gateway processes for missed-Redis/SQL-outage denial. These are distinct proofs; the integrated native test does not yet use two gateway OS processes.

## Qualification boundaries

The Linux composed proofs cover controller activation, delayed stale input, uncertain actions,
existing PTY control outside allowed start roots, protected recording, immutable original-key
reconciliation, managed Codex and durable approval delivery. Revocation completion follows
[ADR 0009](../adr/0009-revocation-completion.md). Real tenant directory evidence, actual
suspend/platform services and production fault domains remain
[external release gates](../validation/external-release-gates.md).

## Controller components

`native-controller-v1` is a separate required IPC capability for query/activate/renew/release. The SQLite session generation floor commits before acknowledgment. Duplicates preserve the original deadline; native restart forgets active holders and requires a higher generation. A newer host may bind the same live holder without extending its deadline. Storage uncertainty fences old control. PostgreSQL schema 8 journals immutable acquisition requests and native-confirmed proposals with original elapsed budgets, renewal activation IDs/CAS and one audit per grant. A proposal is metadata, never a standalone mutation permit. The caller binds the authenticated client and current directory/JWT/native evidence. Separate PostgreSQL/Unix-IPC and public composed effect proofs pass.


## Restored primary generation reconciliation

`native-floor-v1` reads the immutable scope's durable native generation floor. Before a
native-backed WSS connection, the connector sends this value in the `native_floor` upgrade
query and requires `host-generation-floor-v1`. The authenticated gateway allocates
`max(primary_floor, native_floor) + 1` atomically and audits the new connection. It never
lowers a native floor or reuses a restored connection identity. Full-u64 exhaustion denies
activation. The value is machine-scoped by verified machine identity; a host cannot advance
another machine through this endpoint.


## Public PTY start

Private `native-start-v1` is negotiated independently. A scoped start carries its original
access nonce through the native start queue, SQLite acceptance and its owned launch supervisor.
A per-binding native effect barrier invalidates old grants before replacement waits for any
already admitted OS call; the session store is acquired only after this barrier wait. The
barrier is released before result journaling. Expiry during a real SQLite exclusive lock
retains the accepted failed key across native restart and launches no child.

Gateway schema 14 uses the isolated recording ingest identity to check owner/machine/key,
current directory projection, lifecycle readiness and recovery quarantine under the primary
lock order. A fresh write with `remote_apply` must commit before native start is dispatched.
Managed-agent starts require the separately verified pinned directory capability; unsupported vendor configurations are refused.
The gateway stores argv[0] as display metadata; the complete launch/fingerprint stays in the
private native start journal. Start projection and one start audit commit together.

The actual synchronous harness launches through the persisted SDK key, verified WSS gateway,
activated connector and native IPC. Same-key retry produces one child/audit; changed parameters
fail locally. The connector uploads real native output and original observed end metadata.
The expanded proof also exercises public remote leases/effects and managed agent workflows;
see [MVP evidence](../validation/mvp-evidence.md) for its precise fixture boundaries.


## Public native control and PTY effects

The gateway uses separately negotiated host-only control queries/activation/release and
never forwards these responses to owner subscribers. Original client, scope, nonce, host
connection and generation survive both control round trips. Primary confirmation persists
the earlier native deadline under the original CAS stage. A controller object is metadata;
only native state plus fresh original access authorizes each OS effect.

The SDK `RemoteController` supports explicit takeover, renewal, release, input, resize and
interrupt. Dropping it or reconnecting does not send input or reacquire control. Input is
poisoned before awaiting transmission; cancellation, unknown delivery and stale generation
require explicit release/reacquisition. No raw input is journaled or recorded. Confirmed
owner termination works without a lease and is audited before native delivery. Native
capability negotiation prevents an older daemon from executing unsupported RPCs.

Per-keystroke PostgreSQL reads are absent. Original access in signed route hints remains
bounded by native issue time and current source/destination socket gates. Independent
primary/directory watchers deny stale sockets; Redis cannot create a new nonce, alter a
native floor or revive expiry. Bounded request workers keep original queue byte permits;
ordered response/event work and metadata maintenance are separate. The two-gateway fixture
holds a primary session-table lock and observes native input/output before releasing it.
