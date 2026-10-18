# Pinned native Codex integration

The standalone adapter pins `codex-cli 0.159.2`. Public source tag `rust-v0.159.2` resolves to commit `ff6aec96948b70d94983af2641a6b67c94faeff5`. The actual installed binary was exercised on Linux, rather than substituting a fake app server. All 314 generated native schemas have checked-in SHA-256 hashes under `conformance/codex/0.159.2/`.

## Managed transport and relay control

The adapter owns an independent foreground `codex app-server --listen unix://…` process and a private CODEX_HOME. It connects over a WebSocket on Unix IPC. Native 0.159.2 advertises a symlink to `/tmp/codex-daemon-<uid>/<sha256(canonical advertised path)>` (with canonical `/private/tmp` on macOS). The adapter accepts only this exact mapping, an owned 0700 protected directory, an owned 0600 physical socket, and a same-UID connected peer. It connects to the checked physical path directly. Cleanup checks original inodes. Arbitrary socket/HTTP listeners and existing-daemon adoption are not exposed by this API.

Source inspection found that `features.remote_control` is **removed** in this version. Passing `--disable remote_control` alone does not prove the real relay is disabled. The verified startup control is `CODEX_INTERNAL_APP_SERVER_REMOTE_CONTROL_DISABLED=1`, read before native worker threads start, selecting `RemoteControlStartupMode::DisabledEphemeral`. This is an internal, version-pinned vendor interface: a future version needs a new proof. The adapter requires `remoteControl/status/changed` with `disabled` before returning a handle. Any later status other than `disabled`, including missing/invalid status, shuts down its owned native process. Fixtures exercise that guard. The API does not expose native relay-enablement methods. The launch environment cannot override the reserved home/key/disable bindings.

Relevant source: `codex-rs/app-server-transport/src/transport/{unix_socket.rs,remote_control/mod.rs}` and `codex-rs/uds/src/daemon_directory.rs` at the pinned commit.

## Proven capabilities and limits

| Operation | Evidence and behavior |
| --- | --- |
| Initialize / thread start | Real native initialization and `thread/start`, with QGenie provider configuration injected by the host. |
| Prompt / streamed items | Real `turn/start`, tool start/output/completion, agent messages, turn completion, and native token counters through a local synthetic Responses endpoint. |
| Steer | Real `turn/steer` with the required active-turn ID precondition. |
| Interrupt | Real active-turn interrupt. A start acknowledgment can precede native activation; an immediate interrupt can be rejected with “no active turn.” The adapter does not retry it. |
| Command approval | Real native request, allow-once response, actual temporary-file action, native resolution, and streamed tool completion. Offered decisions govern availability; the fixture offers allow-once and cancel, but not allow-session. Product deny maps to native cancel (interrupting the turn). Unsupported options fail. |
| Duplicate response | The in-memory native request ledger suppresses repeated responses. This does not survive an adapter/process restart; durable public decision deduplication belongs to sessiond (task 018). |
| Cancellation | Real interrupt while approval is pending, native resolution, subsequent answer returns cancelled, and the temporary file remains absent. |
| Restart / resume | A new native process with the same private home resumes the persisted thread identity after orderly shutdown. Historical resume is a separate execution transition, not proof that an old live turn survived. |
| Application acknowledgment | A successful response write returns `outcome_unknown`. Native `serverRequest/resolved` can also mean cancellation. Neither proves exactly-once application. Tool completion was independently verified in the native test. |
| File-change approvals / other native requests | Not advertised. Unknown native server requests receive an explicit unsupported-method error. More approval families need their own real native proofs. |
| External/shared daemon adoption / TCP listeners | Not supported by the managed-launch API. No non-loopback agent listener is created. |

## Boundaries and repeatable proof

Provider endpoint, model, and scoped credential enter through a host-only `Provider`; credentials go into child environment only. Provider/environment Debug output is redacted. The gateway/client receive no credential or raw native RPC API. HTTPS is required for provider endpoints; a separate explicit option permits a literal loopback HTTP endpoint for local native tests. The corporate tenant/provider integration remains task 013/025 evidence.

The adapter has 32 queued commands/256 KiB, 64 pending requests, 32 pending approvals, 128 recent resolutions, 64 event slots, and 1 MiB native WebSocket message limits. Requests have a 15-second acknowledgment deadline and writes five seconds. Queue rejection means not sent; lost writes/replies mean unknown. No mutation is automatically repeated. Dropping the native owner cancels its driver and kills its child. Sessiond must own this handle independently of observers and continuously drain its event stream.

```sh
python3 tests/e2e/codex_schema_check.py /path/to/pinned/codex
QGR_CODEX_BINARY=/path/to/pinned/codex cargo test -p qgenie-remote-adapters --all-features --test codex_native -- --ignored
cargo test -p qgenie-remote-adapters --all-features
```

Both native checks passed using `/opt/codex/bin/codex` on Linux. The synthetic model endpoint uses synthetic credentials and controlled temporary files; it validates real native protocol/tool execution without making a live provider call. Sessiond ownership and public SDK/TUI flows now pass the composed proofs below. macOS native execution, packaging/distribution and corporate model policy/telemetry remain separate evidence gates.


## Linux descriptor cwd qualification

The actual pinned native test `descriptor_cwd::actual_pinned_codex_retains_descriptor_cwd_through_launch_and_thread_path_replacement` passes on Linux. An owned open directory is retained as `/proc/<native-supervisor-pid>/fd/<fd>`. The fixture replaces the original path before app-server launch and replaces its renamed path again after `thread/start`. A real approved relative `exec_command` creates its marker in the retained inode and no marker in the outside replacement. Pinned source `core/src/utils/path_utils.rs` reexports `utils/path-utils`; `normalize_for_native_workdir_with_flag` returns the unchanged path on Unix. The actual command proof, rather than only that source reading, establishes the tested behavior.

The native supervisor must retain this descriptor until its owned process has been reaped; closing/reusing a referenced descriptor while a native child can still act is unsafe. A structured runtime must own that lifetime independently of request cancellation, enforce original authority at actual native writes and retain native processes across connector loss. macOS has no qualified equivalent yet. This proof does not enable remote managed-agent starts by itself.

The adapter now separates its process/reap owner from its native IPC driver. IPC failure,
including denied socket authorization, leaves the exact process and endpoint owned/alive.
`closed` observes driver closure; explicit `shutdown`/owner drop requests process cleanup.
Old request handles remain closed and are never rebound to a replacement connection.
Linux `launch_at` transfers the directory file into the reap worker before awaiting IPC
startup, so cancellation/IPC failure cannot close or reuse its cwd descriptor while the
owned child remains alive. Actual pinned native tests pass this lifetime and path-substitution
behavior; socket identity remains held until native reap.

`NativeEffectAuthorization` is injected into a cloned request handle and retained per work
item. A private Unix stream wrapper calls it around each nonblocking OS write, including
retries after actual socket backpressure. A real blocked Unix pair test expires a grant after
a partial write and confirms the suffix never arrives. The actual pinned Codex proof denies
a native prompt at the socket, closes that uncertain IPC, keeps its process/endpoint alive,
rejects old handles, then explicitly reaps it and resumes the saved thread in a new process.
Explicit `reconnect` now verifies both retained socket/alias inode identities and the still-owned
process before opening a fresh driver. The actual pinned proof resumes the saved thread in
that same process; old handles remain closed and the denied prompt is never resent. Replaced
endpoints are rejected. The owner must still mark its event timeline as having a gap.
Sessiond binds original access/controller/policy fences and composes the owned structured
runtime, journal, capture and public approval delivery described below.

`CodexProcessControl` is a separate cloneable handle with a four-request queue. It checks
the supplied original effect authorization immediately around the owned child's OS kill;
a denied request retains the owner and cannot trigger its local cleanup path. Losing a
request's acknowledgment stays uncertain. Explicit local owner shutdown/drop still reaps
its child independently of cancellation. Actual pinned tests reject an expired termination,
then admit a current one and observe exit; dropping a process-control clone has no effect.
An additional actual approved background-tool test waits for the descendant's own ready
marker, terminates the owner and verifies its delayed file effect never occurs. This is
Linux evidence for that controlled native tool path, not an arbitrary external daemon claim.

Sessiond's optional `managed-codex` feature now binds its original host `Fence` to this
effect interface. It preserves WouldBlock while checking the suspend-aware deadline and
replacement barrier around each OS effect. Managed registry/capture and public operations
now use this binding.

Sessiond now composes the adapter behind its optional Linux `managed-codex` feature.
A locally injected `CodexProfile` supplies the binary and provider; a public launch cannot
supply those credentials or configuration. Managed and PTY starts share one immutable scoped
key namespace and journal lock. The owner is reserved before acceptance, retains cwd through
reap, commits the native thread before the initial prompt, and drains independently of the
caller, connector, replay SQLite and recording encoder. It retains the process on native IPC
loss, records uncertainty, and continues to accept explicit owner termination. Runtime IPC
reconnect is not yet enabled.

Historical resume is an explicit new start after a confirmed native exit. It uses a new
session/process identity, the saved thread and its canonical private native home, and a
new descriptor-scoped cwd. The host rejects active, lost, unconfirmed or ambiguous history.
Resumed checkpoints label prior history as partial; they do not invent missing conversation
events. Actual managed and two-gateway SDK/TUI proofs resume the saved thread and verify a
relative approved tool writes in the newly selected directory.

Each prompt/steer request has a durable keyed input identity. Its original host fence and
controller generation/deadlines cover each actual native write; renewal or takeover cannot
refresh an older queued request. Approval delivery instead carries current original owner
access with no controller requirement. Native requests bind the exact process instance and
thread. A reserved/offered answer is never resent; a native write remains Unknown unless
there is stronger native evidence. The composed actual test exercises approval execution,
lost application reply, repeated decision, controlled prompt deduplication and caller loss.

`agent_state_v1` checkpoints contain versioned JSON with an exact sequence, at most 256
structured events and 512 KiB encoded data. Truncation and an unconfirmed native tail are
explicit. They are captured at one fixed cursor and persist with replay pruning; they do
not claim a complete transcript. Agent text is separately recorded through redacted Events
streams, including explicit recovery loss on incomplete EOF. The actual test uses a two-event
replay cap and restores structured content after native-owner restart.

Run this additional composed proof with a pinned local binary:

```sh
QGR_CODEX_BINARY=/path/to/pinned/codex cargo test -p qgenie-remote-sessiond --all-features --lib actual_managed_owner -- --ignored
```

Public approval delivery now passes the owned synchronous fixture through two gateways,
the SDK and actual TUI keys. `approval-status-v1`/`durable-approvals-v1` expose durable
choice versus native application. Public agent launch now also passes this fixture. Native
startup verifies the actual pinned binary before advertising its inventory; each launch
rechecks the binary under its own original grant. `host-agent-v1` and `agent-events-v1`
compose discovery, launch, structured snapshots and controlled text. Actual TUI keys select
advertised Codex 0.159.2, create, prompt, refresh and terminate through the SDK. Build the
optional standalone native integration with `--features managed-codex`. Runtime IPC reconnect
remains disabled; its adapter proof does not enable it implicitly.

Startup errors explicitly kill and reap both failed version probes and owned app-server
startup before returning. The owning service waits for its native readers to finish after
confirmed shutdown, including startup still in progress. A Linux broken-process fixture
checks ESRCH immediately on return, distinguishing actual reap from kill-on-drop alone.

On Linux, managed children also bind process death to their dedicated owner thread using
`PR_SET_PDEATHSIG`. The actual SIGKILL fixture observes native exit through a pidfd, reopens
the journal with lost/unknown state and verifies original-key lookup never spawns again.
Native readers continue while a deferred original-authority write waits; a real Unix socket
proof verifies event drain, one eventual permitted write and zero writes after grant expiry.
