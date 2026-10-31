# Implementation goal and completion

Updated: 2026-10-31 (UTC).

## Current goal — overall Linux completion

[Task 030](030-complete-linux-release-readiness.md) sets overall completion as the goal.
Its ordered steps cover remaining code clarity, regression coverage, release-build host
measurements, and real-environment qualification. Work continues until each applicable
gate has evidence. External qualification remains open; the completed development
milestones below do not close it.

Task 030's local repairs and checks pass. The [BeautifulRust review](../docs/validation/beautiful-rust-review-2026-10-26.md)
records fixed pagination, cancellation, launcher ownership, and recording-worker lifetime
defects. The [final composed proof](../docs/validation/linux-completion-2026-10-26.json)
passes all functional phases. The separate host budget remains unmet at 32.53 MB aggregate
RSS and 0.57 percent idle CPU. The [capacity report](../docs/validation/capacity.md) gives the
measurement limits. Real enterprise access, user services, and the developer pilot remain unrun.

[Task 031](031-add-ra-jit-development-builds.md) adds the requested optional ra-jit
development workflow. It is installed and tested on Linux x86-64. Warm CLI edits measured
0.33–0.43 seconds, with a slower 9.10-second first JIT edit. Normal Cargo and CI retain
Rust 1.99.1. See the [development guide](../docs/development/ra-jit.md).

[Task 032](032-review-herdr-core.md) reviews the separately cloned Herdr Core runtime.
The [source comparison](../docs/validation/herdr-core-review-2026-10-31.md) recommends
readable output, bounded waits/events, activity summaries, client observation recovery,
and small launch profiles. The review kept setup, sign-in, real deployment, and the pilot as
priorities. Upstream runtime tests were not executed. Task 032 changed documentation only.

[Task 033](033-implement-daily-remote-workflow.md) implements the accepted local workflow.
It adds saved setup, client-owned OAuth refresh, readable output, bounded events, typed
activity and waits, an attention filter, and automatic observation recovery. It also fixes
discarded native turn outcomes and connection error classification. The
[validation report](../docs/validation/daily-remote-workflow-2026-10-31.md) records workspace,
focused, managed Codex, native, keyboard, schema and lint results. Launch profiles and naming
remain after pilot feedback. Task 030 retains enterprise, service, pilot and resource gates.

[Task 034](034-integrate-work-and-bend.md) brings the implementation and Bend specifications
onto one `main` branch. It extends client laws for completion evidence, recovery eligibility,
and credential validity. The specification runner compares those decisions with the actual
SDK and records separate model-proof and implementation-comparison evidence.

[Task 035](035-fix-github-actions.md) tracks the hosted CI repairs. The Linux and
macOS matrix, database/native integration, deployment job, aggregate CI, and
specification workflow all pass on `00a27fe`. A fresh `main` run exposed a
routing-fixture recovery race, stale TUI startup metadata, and a deployment probe
that read durable inventory instead of contacting the native owner. Those repairs
pass local checks. Final hosted verification is in progress.
The changes fix private test permissions,
macOS portability, presence-cleanup and protocol-recovery races, and native Codex tool
cleanup across Linux process groups. The task records the plan, regressions, and hosted
run links. See the [CI guide](../docs/development/ci.md) for workflow coverage and required
results. External release qualification remains separate.

## Completed milestone — core journey repairs

[Task 029](029-stabilize-core-journey.md) records the user-authorized repair plan and its
implementation evidence. All six Linux repair steps passed their local acceptance checks.
The [composed result](../docs/validation/core-journey-2026-10-26.json) records the process,
Codex, recording, and crash proofs. The sequence is: fix lifecycle limits, complete directory
discovery, verify the composed journey, then qualify real enterprise access and user
services with a small pilot. Keep Linux, one owner, shells, and pinned Codex as the scope.
Do not expand adapters or deployment targets before this journey passes.

The completion statement below records the earlier Linux development milestone.

## Selected goal — complete

Implement and verify tasks 001–028 for the **Linux standalone development scope**. The user
waived the unavailable corporate CLI source and explicitly selected: “Finish Linux development
scope; document external release gates.” [ADR 0007](../docs/adr/0007-standalone-preview.md)
records both decisions. Task 001's external inspection is superseded, rather than falsely
reported as performed. Tasks 002–028 are complete within that selected development scope.

Original criteria requiring macOS, a real tenant, actual user-service managers, production
fault domains/operators or beta hardware remain unchecked. They are recorded with required
evidence in [external release gates](../docs/validation/external-release-gates.md). This is
not enterprise release approval. Native runtime IPC reconnect and external session adoption
are not advertised; uncertainty never silently launches replacement work.

## Repository and resulting behavior

Nine product crates separate protocol, transport, pure policy, adapters, sessiond, host,
client, TUI and gateway. `apps/qgenie-remote` composes a runnable standalone CLI/TUI; a separate
fixture issuer, xtask, schemas, CI, real-process runners and Compose deployment support it.
PostgreSQL owns durable gateway state and audit; Redis is disposable routing infrastructure.
A separate private native daemon owns PTY/managed-agent processes, starts, replay and capture.
Connector or UI loss preserves local sessions. Input is never automatically resent.

Public Linux workflows include enrollment, machine metadata, descriptor-scoped browsing,
persisted-key starts, observe/control/takeover, bounded structured or terminal replay,
lease-independent owner approval, confirmed termination, protected redacted history and
holds/retention. Pinned Codex 0.159.2 supports verified discovery, managed creation,
prompt/steer/interrupt, command approvals and guarded historical resume in a new directory.
Revocation keeps local sessions alive and reports completion conservatively after original
authority expires; every status still requires the primary.

## Task closure

| Task | Selected-scope status | Executed result |
| --- | --- | --- |
| [001](001-inspect-existing-qgenie-integration.md) | Waived / superseded | Corporate CLI source inspection waived; standalone composition root selected in ADR 0007. |
| [002](002-resolve-runtime-and-protocol-contracts.md) | Complete (Linux development) | Written authority, retry, lifecycle, wire and protected-recording contracts. |
| [003](003-bootstrap-rust-workspace-and-ci.md) | Complete (Linux development) | Pinned Rust workspace, dependency/schema checks and Linux/macOS CI definitions. |
| [004](004-implement-protocol-and-conformance.md) | Complete (Linux development) | Typed public/private protocol, full-u64 generations, golden fixtures and generated schemas. |
| [005](005-implement-websocket-transport.md) | Complete (Linux development) | Verified TLS/CONNECT proxy, idle/heartbeat/queue/slow-reader/cancellation proofs. |
| [006](006-implement-secure-local-ipc.md) | Complete (Linux development) | Private framed Unix IPC, independent admin/data endpoints and actual foreign-UID denial. |
| [007](007-implement-policy-engine.md) | Complete (Linux development) | Pure owner/controller/approver/capability/path policy with stale-access denials. |
| [008](008-implement-adapter-contract-and-pty.md) | Complete (Linux development) | Actual PTY lifecycle, resize/snapshot, interrupt versus terminate, 512 KiB burst and observer loss. |
| [009](009-implement-verified-codex-adapter.md) | Complete (Linux development) | Actual pinned Codex 0.159.2 managed subset; unsupported runtime reconnect remains explicit. |
| [010](010-implement-gateway-foundation-and-audit.md) | Complete (Linux development) | PostgreSQL migrations, restricted roles, transactional audit, health/drain and real process proofs. |
| [011](011-implement-gateway-identity-and-enrollment.md) | Complete (Linux development) | Verified JWT/JWKS, machine challenge/enrollment/recovery and user/machine token separation. |
| [012](012-implement-routing-and-presence.md) | Complete (Linux development) | Two gateway OS processes plus actual four-container native routing, push and dependency recovery. |
| [013](013-implement-host-enrollment-and-credentials.md) | Complete (Linux development) | PKCE/device fixture, private credentials, idempotent enrollment, explicit recovery and unenrollment. |
| [014](014-implement-host-dispatch-and-filesystem-policy.md) | Complete (Linux development) | Independent descriptor policy; actual existing PTY controls outside start roots while browsing is denied. |
| [015](015-implement-session-runtime-and-start-journal.md) | Complete (Linux development) | Durable scoped starts, original-key recovery, six actual managed crash boundaries and owner SIGKILL. |
| [016](016-implement-session-replay-and-snapshots.md) | Complete (Linux development) | Bounded PTY/agent checkpoints, causal replay, scoped push heads and idle-tail repair. |
| [017](017-implement-fenced-control-leases.md) | Complete (Linux development) | Native/primary generation floors, explicit takeover, original deadlines and stale input denial. |
| [018](018-implement-durable-approvals.md) | Complete (Linux development) | One audited winner/outbox; SDK/TUI owner choice without control; delivery/ACK losses execute once. |
| [019](019-implement-revocation-and-entra-freshness.md) | Complete (Linux development) | Suppressed-hint denial within ten seconds, original native grants, conservative completed status and Graph freshness. |
| [020](020-implement-recording-spool.md) | Complete (Linux development) | Independent native capture, redaction/no-echo, durable gaps/receipts, cap/stall/restart and actual ENOSPC. |
| [021](021-implement-transcript-storage-and-access.md) | Complete (Linux development) | Protected synchronous upload/history, lost ACK, RLS/admin audit and PgBouncer isolation. |
| [022](022-implement-legal-holds-and-retention.md) | Complete (Linux development) | Hold/retention races and real jobs; held references/content survive primary-volume destruction. |
| [023](023-implement-rust-client-sdk.md) | Complete (Linux development) | Bounded REST/WSS SDK, persisted starts, uncertain-input fencing, observation and approval workflows. |
| [024](024-implement-tui-workflows.md) | Complete (Linux development) | Actual keyboard PTY/agent creation/control, observe-mode approvals, metadata, history and guarded resume. |
| [025](025-integrate-cli-and-host-services.md) | Complete (Linux development) | Standalone commands, independent processes, pause/resume, service generation and Linux unit/update checks. |
| [026](026-build-development-compose-stack.md) | Complete (Linux development) | Four healthy non-root gateways, actual hosting-node SIGKILL, rolling restarts, dependency loss and isolated restore. |
| [027](027-prove-end-to-end-mvp-behavior.md) | Complete (Linux development) | Composed actual PTY/Codex/SDK/TUI/native/storage proofs with sanitized machine-readable reports. |
| [028](028-measure-capacity-and-write-operations-runbooks.md) | Complete (Linux development) | Refreshed reduced baseline/storage estimate, bounded transport/replay/approval metrics, 13 evaluated alerts and runbooks. |

## Final executed verification

- [Linux checks](../docs/validation/linux-development-checks-2026-10-18.json): **178 passed,
  zero failures, 21 explicitly ignored environment proofs**; formatting, strict all-target/
  all-feature Clippy, default all-target check, dependency boundaries and all three schemas pass.
- [Synchronous acceptance](../docs/validation/synchronous-acceptance-2026-10-18.json): actual
  PTY/Codex public SDK/TUI pipeline, all six managed start boundaries, native-owner SIGKILL,
  caller loss and actual primary tmpfs destruction/promotion pass. Held redacted bytes,
  receipts and audits survive. Actual scrape counters/histogram describe the routed workflows.
- [Revocation proof](../docs/validation/revocation-propagation-2026-10-18.json): Redis hints
  suppressed across two gateway OS processes; old/new access denied within ten seconds;
  pending/completed identity stable; primary failure cannot return cached completion. Signed
  machine revocation preserves the same real local PTY.
- [Development rollout](../docs/validation/development-roll-2026-10-18.json): the same active
  PTY executes local input during Redis and primary outages, survives its actual hosting
  gateway's SIGKILL and four rolling recreations, reconciles its original key, accepts new
  remote input and retains redacted protected history. Isolated backup/restore also passes.
- [Operational alerts](../docs/validation/operational-alerts-2026-10-18.json): 13 actual rules,
  40 firing/recovery/missing-probe assertions, four private gateway metric sources, three
  scheduled aggregate probes and actual native spool ingestion pass. No external notification
  destination or production operator is claimed.
- [Capacity](../docs/validation/capacity.md): refreshed 8-socket/2-PTY/200-echo baseline,
  zero input retries/cleanup failures, point-in-time empty spool/replica lag and an explicit
  measured-input retention/hold/replica/backup estimate. This does not qualify full beta load.

Run commands and precise limits are in [MVP evidence](../docs/validation/mvp-evidence.md),
[deployment](../docs/runbooks/development-deployment.md) and
[operations](../docs/runbooks/operations.md). The validation records describe the implementation
before its Git commit. No production deployment, push or pull request was performed.

## Historical implementation log

The following entries preserve earlier checkpoints, counts and next actions. They are
historical, including statements that integration was disabled or the goal was incomplete;
the completion record above and linked final artifacts describe the current implementation.

### Verification completed

Workspace formatting, all-target check, strict all-feature Clippy, and baseline tests passed on Linux after the current transport, native-adapter, gateway, and identity work. The dependency checker and supported no-default-feature builds passed. Protocol tests include 30 public JSON vectors, binary vectors, full-u64 boundaries, malformed data, fencing, fingerprints, timestamps, and schema exclusion. Schema export/check is deterministic.

The real daemon-process suite covers duplicate starts, changed-key conflicts, takeover fencing, session survival across client exit, resize, termination confirmation, daemon restart/key retention, no-echo input, and administrative schema separation. The real TUI keyboard harness covers create/input/detach/observe/control/reattach/quit/terminal restoration. SIGTERM cleanup is included in the current harness. Re-run affected checks as new layers land.

### External evidence gates

The missing corporate CLI source is not a blocker. Real tenant configuration/directory access, Linux user services, macOS execution, deployment infrastructure, representative scale hardware, backup targets, and operating ownership still need evidence in their owning tasks. Component tests cannot substitute for those proofs.

### Next actions

1. Complete remaining active-stream/gateway process and extended network-fault boundaries; native protocol reconnect remains an explicit unsupported capability. Preserve original authority, explicit uncertainty and independent native lifetime.
2. Complete the remaining active-stream/gateway and native approval crash cases; use the [MVP evidence report](../docs/validation/mvp-evidence.md) to track requirements rather than treating component completion as release acceptance.
3. Finish detailed queue/replay/approval metrics and native revocation propagation/completed reporting. Install/supervise the host probe and qualify notification routing with assigned operators. Obtain real user-manager/macOS service drills and actual Entra tenant/disablement evidence.
4. Qualify production synchronous/backup fault domains and the stated beta workload. Continue implementable work without requesting the waived corporate CLI source or marking unrun external proofs complete.

### Development service for current work

An ephemeral PostgreSQL 17 container named `qgr-goal-postgres` is running on loopback port 55432 with tmpfs data for implementation tests. Its synthetic DSN binding is outside the repository under `/workspace/.remote-session-tools/private/postgres-test-url` (0600). This is local test infrastructure, not production configuration. PostgreSQL integration tests create/drop generated test databases; the process smoke fixture uses `qgr_smoke`. Remove the goal container after database-dependent work and checks finish.

### Historical implementation notes

These entries record earlier implementation stages. Their then-open work is superseded by
the current matrix above and the latest evidence below.

Task 011 now has explicit CLI identity configuration, real verified HTTPS JWKS tests, stable Entra directory-object ownership across pairwise subjects, single-use signed challenges, key recovery/audit, and strict user/machine token separation. WSS admission is verified in 012. Additive schema 0004 reserves durable host generations and stores generation-checked inventory/session projections; foundation supports [1,15], identity requires 3, routing requires 4. Schema 0005 separates directory projections from manual flags, and 0006 journals admin revocation; their features require schemas 5 and 6 respectively.

Task 016 now uses a complete versioned terminal checkpoint from documented, checksum-pinned local vt100/vte patches. It preserves both grids, scrollback, saved state, modes, and pending UTF-8/escape/string input. Native parser OSC storage is capped at 4 KiB; original replay bytes remain intact. Every byte-boundary cutover, corrupt checkpoint, Unicode resize, durable checkpoint plus pruned-tail reconstruction, fixed paged capture, and idle final-frame reconciliation pass. Checkpoints commit in the same SQLite transaction that prunes their required replay suffix. Historical grids load only for capture and finished sessions release grids. Missing/corrupt checkpoints and unconfirmed tails after native-owner loss fail explicitly. Structured-agent and native remote integration remain.

Task 012 now has two actual gateway-process proofs through verified HTTPS JWKS/WSS and disposable PostgreSQL/Redis. Maximum-size JSON, bounded slow observers, Redis pub/sub loss and finite pause, same-node reads, recovery, host replacement, push presence/counts, revocation, full-u64 exhaustion, and SIGTERM cleanup pass. Host peers are protocol fixtures; native remote mutations remain explicitly not-sent until task 017 activation. The ephemeral `qgr-goal-redis` Redis 8 service runs on loopback 56379 with tmpfs data; its private URL binding is outside the repo. Remove it after Redis-dependent work finishes.

Task 013 now implements standalone scoped PKCE/device OAuth, private durable identities, optional Linux Secret Service/macOS Keychain storage (software Ed25519 remains exportable), and disclosed 0600 fallback. Real HTTPS/fake OAuth plus actual PostgreSQL gateway tests pass lost replies, hostname reuse, explicit interrupted key recovery, old-key denial, cancellation/expiry/slow-down/scope checks, machine-only authentication, unenrollment, and no user-token persistence. The actual missing-keyring proof passed with an explicitly unavailable Linux session bus, including refusal to silently switch an existing backend. Real tenant/native-keychain and CLI/service proofs remain separate. Routing publisher/subscriber callbacks now carry a connection epoch, source publish failures announce bounded resync obligations after repair, and same-node clients survive Redis loss.

Routing now retains a delivered inventory digest per client, serializes authoritative reads with process revisions, and orders initial pages/deltas in one FIFO lane. A skipped reconciliation during initial transmission is checked again against that client, and older captures cannot roll back newer ones. The adversarial component test and actual two-gateway metadata/routing proof pass. Snapshot reply correlation now survives every page until the final head; late duplicate reads do not disconnect a host. The actual two-process proof includes a paged terminal checkpoint. Native remote/SDK integration remains task 023/027.

Task 014 now has actual HTTPS/WSS/PG/Redis proofs for one outbound machine-only connector, in-place refresh without generation change, replacement/reconnect and cancellation. Descriptor-scoped filesystem tests cover traversal, ownership, symlink substitution and changed pagination cursors. Native IPC dispatch/atomic cwd transfer and remote policy checks remain. Task 015 reserves the supervisor before durable acceptance/native launch, keeps the runtime lock through native ownership, and rechecks leases after stalled SQLite input intent. All local crash/concurrency proofs passed with an ordinary 0022 test umask. CI includes foreign-UID IPC and the real connector test.

Native follow-through: SQLite schema 3 pins origin/machine/owner and per-session scopes without adopting local history; upgrade/restart and same-key cross-scope PTY-marker proofs pass. Sessiond independently resolves allowed cwd paths via no-follow root descriptors, and the pinned portable-pty patch performs validated fchdir before exec. Actual path/relative-executable substitution and permission-change proofs pass. Primary reaping is independent of inherited PTY EOF, with a bounded 500 ms drain and explicit gap. Input writes now respect the monotonic lease deadline on every write; expiry cannot be renewed after a timed-out write. Native remote activation/control/policy integration is still required before enabling network mutations.

Next integration: [native authority constraints](../docs/integration/native-authority.md) preserve the original gateway-bound access nonce on queued dispatch, immutable scope filtering, SQLite generation floors and a consistent effect/activation lock order. Do not turn on network mutations before those checks. Capture/SDK assembly concurrency caps are four each; local metadata list replies cap at 512 KiB pending pagination. All workspace tests (including real 65-second proxy), strict Clippy, schemas and dependency boundaries passed; affected native/scope/PTY/IPC and foreign-UID proofs also pass after the latest changes.

Native authority now uses schema 4 host floors/connection identity and IPC 1.1 negotiated `native-authority-v1`. Real SQLite/Unix-IPC proofs cover bounded challenges, duplicate ACK deadlines, full-u64 generations, restart, actor/scope filtering and stalled commit expiry without restoring old authority. Scoped list/lookup exist; gateway/connector composition and all remote effects/controller activation remain disabled. Original deadlines additionally use Linux CLOCK_BOOTTIME/macOS mach_continuous_time; deterministic suspended-time tests pass, while real suspend and macOS qualification remain external. Local control/input uses the same elapsed clock and cannot revive an expired lease. Service startup handles a bounded prior-owner lock handoff before opening durable state. Native input intent consumes its sequence even when later cancelled.

Task 019 now has a concrete verified HTTPS Graph provider and bounded per-node directory freshness. Evidence expires 840 seconds from provider read start using suspend-aware elapsed time; UTC projections cannot restore authority. Transactional idempotent admin global/user/machine revocation and independent connection monitors/socket I/O gates are implemented. Actual two-process WSS tests pass with Redis paused throughout revocation and primary SQL stalled while dispatch is blocked. Propagation remains pending until cross-node/native acknowledgments exist; real tenant, suspend/resume/macOS and the full native mutation chain remain unqualified. See the directory integration document.

Native read integration now passes: trusted local binding registration preserves live/local sessions; bounded NativeClient IPC and sealed original-time grants compose WSS host activation/renewal with PostgreSQL confirmation and source-bound Redis dispatch. Schema 7 stores the last acknowledged nonce; directory-enabled routing requires native capability. The combined test runs a real sessiond process and two gateway compositions, with synthetic HTTPS Graph/JWKS plus real PG/Redis. It verifies scope isolation, nonce renewal, replacement and revocation without killing a local PTY. Two-OS-process gateway and foreign-UID proofs remain distinct. Public conformance now includes 17 JSON vectors. Controllers, remote effects/attach, managed runtime, approvals, recording and the remaining deployment/client tasks are still open.

Latest authority checks also pass after making elapsed-clock failures/regression permanent for the process and moving JWKS/application-token cache deadlines to that clock. A failed SQLite controller activation now fences the old holder and skips the uncertain candidate in memory; its real PTY/SQLite-lock test passes. The real local TUI keyboard/SIGTERM/terminal-restoration proof passes after native integration and these changes.

Controller foundations now pass independently: private `native-controller-v1` IPC commits the SQLite generation floor before ACK, rejects implicit takeover, preserves original deadlines on duplicate ACK, and cannot revive released/restarted generations. Schema 8 journals idempotent PostgreSQL proposals, explicit takeover, full-u64 reconciliation, native confirmation, renewal-stage CAS, and release with atomic audit. Real application-role PostgreSQL and real native PTY/SQLite/Unix-IPC proofs cover these components. They are not yet composed into public controller routing or remote input.

Task 020 component proof now exists: bounded redacted normalized JSONL/zstd batching preserves binary fragments/capture times, a private replay-independent spool reserves sequences durably, and exact content/gap receipts validate authenticated recovery epochs before reclaim. Durable gap-before-eviction, immutable offered gap retry, process-exit recovery, corruption and actual 4 MiB tmpfs ENOSPC pass. A separately live PTY remains controllable during ENOSPC; full capture-worker integration is not yet proved. Shared normalized records and typed gap receipts have schema/conformance coverage. Real-user recording remains gated on 020–022 protections.

The recording worker now owns its blocking thread, 64-event capture queue and independent four-slot RPC queue. Capture admission never waits for SQLite/compression/network. Queue overflow becomes durable ordered loss; actual SQLite stalls/recovery, no-echo native PTY, detached-viewer survival, final drain and unknown-native-EOF tails pass. UTC month cutover/regression tests prevent newer capture extending earlier content retention. Native scoped PTY hooks and read-only spool IPC now have later integration proofs; authenticated upload/reclamation and protected persistent release remain open.

Task 021 has bounded codec/lifetime preparation and isolated role/RLS foundations. Actual application identities verify owner isolation, missing transaction context, size-one pool reset, direct partition/content denial, role exclusivity and held case membership. Subsequent authenticated ingest and audited history components pass actual PostgreSQL/PgBouncer proofs, including stale session context across distinct clients reusing one transaction-pool backend. Public APIs, native uploader and production durability qualification remain open. The prior full workspace/all-features suite passed, including the 65-second idle proxy; affected strict Clippy, recording/no-echo/ENOSPC and schema checks passed as components landed.

Gateway writer redaction now seals bounded protected recordings, preserves uploaded metadata/digest for receipts, clears unprotected upload bytes and separately hashes stored redacted content. Tests cover complete/cross-field/chunk-edge literals, binary data, real preparation bounds and saturation masking. The [recording integration document](../docs/integration/recording.md) records composition and production evidence still needed. Protected real-user release remains gated.

Schema 10 and recording commit components now pass real application-role PostgreSQL proofs: signed machine identity/current primary scope, ordered primary locks, one lifecycle lock, protected bytes plus immutable receipt in one transaction, duplicate write barriers, exact retry/conflict behavior, fragment identity, typed gap receipts, full-u64 values and recovery quarantine/new-epoch receipts. PostgreSQL 17+ bounds active SQL; the owned commit supervisor separately bounds synchronous COMMIT cancellation and response handling. Synthetic single-server mode is test-only; production paths require directory evidence, a qualified synchronous boundary and the forthcoming lifecycle schema. There are still no public upload/history APIs and real-user recording remains gated. Normalized agent metadata now uses fixed channel labels; arbitrary vendor/user text belongs only in redacted data.

Schema 11 audited history and schema 12 lifecycle now pass the 21-test gateway suite through actual PgBouncer 1.24.1. Distinct clients reuse one backend without stale-context access. Holds fence final retirement, overlapping coverage survives release, late held uploads do not recreate retired months, and transactional DROP rolls back on audit failure. Signed-role hold HTTP and actual jobs-process tests reject unqualified synthetic state. Production success still needs synchronous/restore qualification, native/uploader composition and deployed scheduling. The lifecycle runbook documents retained receipt/fragment metadata and backup limits.

Native spool fetch/status/receipt reclamation now also filter immutable original origin/owner/machine before selecting or offering a gap. Eighteen recording tests pass, including foreign binding attempts and worker RPC scope isolation. Later native PTY/read-only IPC integration adds original-authority fencing; authenticated uploader/reclamation remain open, and the global local-worker API is not a remote authorization boundary.

After these changes, the full workspace/all-features test suite, strict Clippy, all schema checks, dependency boundaries and real isolated ENOSPC proof passed. The native worker now commits an explicit recovery-loss tail when capture closes without observed native EOF, with reopen reconciliation if that commit fails; the eighteen recording tests pass after this change as well.

Native runtime capture now composes before scoped PTY launch, records output/resize without input, persists unknown-EOF tails immediately and exposes cached cap/storage warnings across restart. Seven runtime/Unix IPC tests pass, including independent live input/output under a recording SQLite lock and access expiry/replacement during read I/O. Read-only IPC/host bridge requires `recording-read-v1`; receipt placeholders remain unsupported pending independently authenticated backend epoch binding and native ACK fencing. Unscoped preview is never adopted. Strict workspace Clippy passes after these changes; affected real-process/native tests are being completed.

The actual synchronous-pair/primary-tmpfs-loss harness now passes production ingest, owner/admin history, signed hold HTTP and the real lifecycle CLI. It exposed PostgreSQL disabling ordinary timers during synchronous COMMIT. An owned, pool-bounded commit supervisor now cancels at the original transaction budget, retires the frontend and refuses receipts/pages from canceled local commits. Paused replication denies both original and duplicate receipts; resumed retry reconciles the same receipt. Acknowledged redacted content, held coverage and audit rows survive actual primary data-volume destruction/promotion. Caller-disconnect cleanup, blocked admin audit and interrupted jobs are also verified. New epoch/quarantine is required, and the promoted single server cannot acknowledge new content or admin audit pages without a synchronous partner. Direct/PgBouncer cancellation, verified TLS, rejection of untrusted roots and late-token replacement tests pass; backup restores and full native/uploader recovery remain separate.


Latest composed recording evidence: all 21 PgBouncer gateway tests pass, including verified
TLS cancellation and retirement of the canceled size-one pool frontend. The protected HTTP
composition uses separate ingest/owner/admin credentials and matching database identities.
Machine-only uploads and independent epoch exchange, owner/admin isolation, signed admin
audit, duplicate receipts, a valid upload above the ordinary 64 KiB body limit and oversize
rejection pass against the actual synchronous pair. The real HTTPS + actual sessiond-process
spool proof uploads recovery gaps, retains a chunk after a lost response, reconciles one
logical receipt and reclaims only the original enrollment. Native tests fence SQLite-wait
expiry, host replacement and old epoch receipts. The connector owns a separate single-job,
bounded-backoff uploader; native capture remains independent of its lifetime.

Restored host generation reconciliation also passes the actual two-gateway/native connector
proof: reducing PostgreSQL's generation while native state survives reconnects at
`max(primary,native)+1`, without lowering the native floor or adopting local sessions.
The PostgreSQL component exercises full-u64 jump/exhaustion separately. Full remote effects,
exact native end metadata, managed runtime, approvals, SDK/TUI and deployment remain open.


Native SQLite schema 5 and gateway schema 13 now preserve the original native-observed
process end in local/public session projections. Unknown native recovery does not invent an
end; legacy/uncertain final projections keep their first upper bound. Repeated inventory
cannot extend that bound or rewrite a known end. Native restart/remote projection and
inherited-descriptor drain assertions pass; the gateway rejects post-end content and changed
end projections in the real database contract. Latest affected PG/PgBouncer and synchronous
checks are running after this additive migration. Full workspace/all-features and strict
Clippy passed immediately before this metadata change; affected checks will record its result.


Protected stream discovery now provides paginated owner/admin indexes with recovery-bound
cursors. Held-only streams require the applicable current case and a synchronous admin audit;
wrong cases deny access and paused replication cannot release index metadata. Actual signed
HTTP and SDK HTTPS tests pass normal/gap stream pagination, cross-owner/role denial, held-only
privacy, stored-hash rejection and native lost-ACK reclamation. The expanded synchronous
primary-volume-loss drill passes with four preserved held references.

The SDK now has a bounded owned WSS read driver and cloneable request handle. Six client tests
pass reversed/duplicate responses, correlation reuse rejection, lost replies without retry,
malformed snapshot ordering and slow subscriber resync; the actual native/two-gateway test
also passes SDK machine events, sessions, browse and explicit credential refresh. The CLI's
remote owner/history TUI is implemented with bounded asynchronous work and shared terminal
cleanup. Its real keyboard proof passes in the owned PG/standby/Redis/HTTPS fixture, including redacted history, explicit gaps, quit and SIGTERM terminal restoration.
Full remote mutations, managed runtime, approvals, services and deployment remain open.


Current full verification is being completed after the SDK/history/TUI integration. Latest
PgBouncer run passes all 21 tests and schema/boundary/strict all-feature Clippy checks pass.
The native/two-gateway SDK proof passes after generation-restore reconciliation. The owned
synchronous harness now owns a separate Redis fixture too and passes actual remote TUI
keyboard navigation, protected history/gap rendering, quit and SIGTERM mode restoration before
its primary-volume-loss proof. `screen_probe.py` reconstructs Crossterm cursor/erase output
for the keyboard oracle; raw diff bytes are not treated as complete screen text. Public PTY start is now composed and verified; controller/effect routing remains next.


Public PTY start now passes the actual SDK/WSS/gateway/connector/native process path on
qualified synchronous recording storage. Schema 14 preflight checks current owner/machine/key,
directory state, lifecycle/recovery readiness and a fresh remote-apply barrier before any
native launch. Unready storage launches no child; same-key retry creates one native session
and one atomic start audit. The SDK persists the original scoped key/fingerprint before
transmission, stores no argv/prompt in its journal, rejects changed parameters with NotSent,
and preserves uncertainty without resending when a TLS reply is lost. Seven SDK tests pass.
Native effects preserve original access through SQLite and launch, fence pending old effects
on host replacement, and wait for admitted OS calls before replacement ACK without retaining
a session-store lock. Public input/controller/approval and managed launch remain unfinished.

All primary authority transactions now share the recording commit supervisor. Actual paused
replication proves that a disconnected authority caller cannot leave a SyncRep commit holding
a size-one pool indefinitely. The original commit deadline cancels and retires its frontend;
its result remains uncertain. All 21 gateway PostgreSQL/process tests pass; strict workspace
Clippy passes. The owned synchronous harness passes public start, native automatic upload,
remote history/TUI and primary-volume-loss proofs (7.31s pipeline and 27.47s database proof).
These same-host synthetic fixtures do not qualify production fault domains, backups or Entra.


Public control now passes two independent gateway compositions, Redis routing, the real
native daemon, qualified recording storage, HTTPS history and the real remote TUI history
browser. An explicit takeover fences the old client; renewal preserves generation; release
and reacquisition advance the durable floor. Native input keeps its original host nonce and
lease through SQLite intents and each nonblocking write. Local IPC cannot borrow a remote
lease even with the same client ID. Resize and owner termination use the same native effect
barrier; confirmed termination requires a committed request audit and no controller lease.
The native capability is `native-effects-v1`, host control RPCs require `host-controller-v1`,
and the SDK negotiates `session-control-v1`. Host-only RPCs never reach owner subscriptions.

Keystrokes and their correlated ACKs use fresh socket gates, signed current-route hints and
native scope/controller checks, without a PostgreSQL query per keystroke. Hints retain the
original native nonce; cache loss cannot mint access or reset a floor. Routing requests use
32 owned workers and retain their original byte permits; responses/events keep their order
on a separate bounded worker. Metadata maintenance cannot block request admission. The actual
two-gateway proof locks the primary session table and observes native output within one
second before unlocking it. Controller confirmation persists the earlier native deadline,
so retrying a stage with a newer access nonce cannot extend its first acknowledged bound.

Eight SDK tests pass, including lost input replies and caller cancellation: both poison the
controller and never resend bytes. Fifty-two native tests plus replay pass (three environment
proofs remain ignored). Twenty network vectors and native control boundary/correlation
conformance pass. The latest owned pair passes the expanded public pipeline (10.58s) and
synchronous commit/primary-volume-loss proof (27.74s). Full workspace, strict Clippy, schemas,
boundaries and PgBouncer regressions are running after these changes. Full remote replay,
structured agents, durable approvals, services/deployment and external qualification remain.


Remote replay and TUI follow-through now pass the owned synchronous fixture. Native
`native-replay-v1`, host `host-replay-v1` and public `session-replay-v1` negotiate fixed
QGVT1 checkpoints, explicit capture and bounded continuation reads. A 128×512 checkpoint
crosses multiple IPC/WSS pages, preserves its captured epoch/head and is installed only
when complete. Native reads retain original authority across SQLite waits. Local/remote
replay cannot extend remote controller leases. SDK output handles reordering, body-identical
duplicates, lost final frames, gaps, checkpoint/live cutover and explicit reconnect cursors.
The public contract now has 24 JSON conformance vectors. Strict Clippy and schema checks pass.

The real remote keyboard proof now creates one native terminal through the directory picker,
persists its original scoped start key before sending, observes and controls it, releases
control, drops read-only keystrokes, detaches, cancels then confirms termination, and reopens
the same ended session from its journal by lookup only. SIGTERM and normal quit restore
terminal modes. The SDK's 14 tests include scope-filtered journal recovery after a lost reply.
The live view periodically reconciles native state/recording warnings independently of input;
output currently uses bounded read polling, so native pushed subscriptions and broader process
faults remain. Managed agents, durable approvals, services, deployment and external evidence
remain open; this proof does not complete all 28 tasks.


Standalone CLI host composition now passes the owned synchronous fixture with actual connector
processes. SIGKILL/restart advances native authority without killing sessiond or its PTY;
local output captured while the connector is stopped uploads after resume. Confirmed pause
and SIGTERM preserve native ownership. Generated Linux user units pass systemd-analyze,
idempotent installation and active-owner update deferral checks. Executables/configuration
are privately pinned; connector and owner have separate jobs/cgroups. `host up --no-service`
and guarded idle owner upgrade are implemented; their additional integration checks remain.
PKCE/device/injected-token enrollment, status/doctor and unenrollment compose the existing
host APIs. The full workspace/all-features suite and strict Clippy passed before schema 15.
The latest composed pipeline passed in 64.67s and primary-volume-loss proof in 27.53s.
An actual systemd user manager is offline here, and launchd/macOS is unavailable; those
lifetime/upgrade/logout proofs, managed Codex and operations qualification remain open.
See docs/integration/remote-commands.md for configuration, commands and limits.


Schema 15 approval components now pass the 21-test real PostgreSQL/PgBouncer suite (55.87s): sixteen owner clients produce one immutable winner, audit and delivery obligation without a controller; audit failure rolls back all three; host-policy change, native cancellation, timeout and revocation deny inappropriate choices. Claims survive pool-backend loss, expire/retry safely and reject stale acknowledgments. Native OutcomeUnknown is kept distinct from a committed gateway decision. Owner-wide reads require no online host/control lease. The midnight run exposed a shared retention-fixture month collision; the hold fixture now chooses a distinct earlier month, and the full suite passes.

A separate private FULL/WAL SQLite native approval journal passes three concurrent/restart/source-isolation tests. It poisons an answer before vendor I/O, returns uncertainty for a lost/offered attempt, cancels never-offered requests on owner restart and never adopts an old native source or silently sends a second answer. Public routing/outbox delivery and original-authority native I/O still need composition; approval capabilities remain disabled. Approval request/winner bounds and native menus are now validated by the shared public protocol.

Actual pinned Codex 0.159.2 also passes a Linux retained-descriptor cwd test through app-server launch, thread creation, two path substitutions and a real approved relative tool command (0.29s). It writes only inside the original inode. This resolves the tested Linux vendor cwd equivalence; the supervisor must still retain the descriptor through native process reap. Managed runtime, native write fencing/ownership, public approvals and structured UI remain the next integration work; macOS cwd equivalence remains unqualified.


Native approval journal now also passes an actual Codex 0.159.2 write/acknowledgment-loss and cancellation drill (0.35s). One controlled tool effect occurs; offered redelivery stays OutcomeUnknown across journal restart without another answer. Native cancellation before delivery remains Cancelled and executes no command. A cloneable adapter request handle is independent of the owning event receiver; both earlier actual native capability/cwd tests still pass (0.58s). Public authority-bound application delivery remains disabled pending composition.

Task 026 now has pinned musl/distroless images with CA trust and runtime UID/GID 10001. The gateway image is 19,330,783 bytes and the explicitly separate synthetic identity image 5,621,503 bytes. The default build target contains no fixture identity binary. Compose configuration and idempotent private input initialization validate. Four explicit node IDs, private bridges/secret volumes, actual synchronous standby qualification, bounded lifecycle scheduling, health probes/private operational metrics, OTel, log rotation and guarded logical restore scripts are implemented. Actual startup/rolling/native proof is in progress; do not mark it complete from image/config success alone. Proxy resolution was fixed by mapping the already configured proxy's host address into the build, with verified TLS and build-secret CA. See ADR 0008 and the development deployment runbook.

Task 026's actual proofs now pass. Four gateways run healthy with verified fixture identity,
TLS/password Redis and a real synchronous standby. The actual CLI/native start/input,
cross-node dispatch, four sequential restarts, surviving PTY, original-key reconciliation,
termination and verified redacted history pass repeatedly. Private publication/blocked admin
paths, invalid configuration failure, Redis degraded readiness/resubscription and primary-loss
liveness/unreadiness/recovery pass. Isolated logical backup/restore passes schema/quarantine;
completed dump publication now uses file/directory fsync and excludes failed partial files.
The final gateway image is 19,351,263 bytes. CI startup/smoke/baseline/restore/fault jobs are
wired; YAML validation caught and fixed an existing unquoted trailing-colon run command.
Hosted CI itself has not been observed. This completes only the development deployment scope.

Task 028 now has an actual persistent SDK load tool and reduced connected baseline: eight
sockets, two simultaneous PTYs, 200 unique echoes, p50/p95/p99 18.09/38.42/48.43 ms including
input ACK/replay observation. Both persisted start keys reconcile; no input retries or known
cleanup failures occur, and the sealed native spool drains to zero pending work. Measured
protected normalized/compressed bytes are 173,077/3,074; workload WAL is 255,592 bytes and final
standby lag zero bytes. Short idle host resource samples, final container snapshots, precise
limitations and explicit unrun beta targets are in docs/validation/capacity.md. The storage
model includes roughly 120-day rounded retention, additional held copies, WAL, replicas and
backups, and separates measured inputs from future assumptions and indefinite metadata.

Private gateway metrics expose degraded routing and authority readiness. The actual development
operator telemetry probe collects aggregate replica/gap/outbox/disk/completed-backup evidence;
optional native spool metrics also pass an actual live-owner probe with sealed/empty native
spool, zero warnings and independent read-only SQLite collection.
Alert definitions parse, but no evaluator/schedule/notification destination is installed.
Operations runbooks name remaining authority-age/queue/approval metrics, external fault domains,
tenant/platform evidence and unassigned production operators. Task 028 remains in progress.
Full workspace/all-feature tests, strict Clippy, dependency boundaries and all three schema
checks pass after schema 15 and deployment/load additions (148 passing tests; 15 explicit
environment proofs ignored). All 21 actual PgBouncer tests pass in 56.64s. The latest owned
synchronous native/CLI/TUI recording pipeline passes in 35.33s and primary-volume-loss/held
content proof in 27.30s. The pipeline now also proves repeated `host up --no-service` is
idempotent, reuses the live native owner/session and resumes capture/upload. Exact scoped
background cleanup uses the connector lock; no stored PID signaling is introduced. Real
systemd/launchd ownership remains unqualified. Build-only Docker caches were reclaimed and
normal ignored target-directory storage restored; source/persistent data were preserved.

Managed adapter follow-through: the Codex process/reap worker is now independent of native
IPC failure and keeps its socket identity and Linux cwd descriptor through actual process
reap. Linux `launch_at` transfers descriptor ownership before any native IPC startup wait.
Actual pinned Codex tests pass descriptor substitution, denied native-write/IPC-failure
survival, closed old handles, explicit reap and saved-thread resume. An injected per-work
authorization wraps every actual nonblocking Unix write; a real socket-backpressure test
expires it after partial delivery and observes no suffix. Eleven adapter tests and strict
adapter Clippy pass. This remains a component: sessiond original-fence binding, reconnect,
structured runtime/capture, public durable approval delivery and UI composition remain open.

Explicit Codex IPC reconnect also passes against the actual pinned binary (both native tests
0.58s): it checks original socket/alias inode identities, keeps the exact process, resumes the
saved thread and leaves old handles closed without resending the denied prompt. Endpoint
replacement is rejected. The actual native approval lost-ACK/cancellation proof still passes
(0.34s) after process/write ownership changes. Reconnect loses event continuity and the
managed runtime must record that gap; no public agent capability has been enabled.

Native process control now uses a separate bounded queue and original effect authorization
at the actual owned-child kill. Denied termination retains the owner; dropping control
clones does not kill it. Current authorization admits a kill and actual exit is observed.
An actual approved background-tool descendant writes its own ready marker before owner
termination; its delayed file effect does not occur (Linux controlled proof, 1.42s).
Sessiond's optional managed-codex feature implements the original host Fence as native effect
authorization, preserving OS backpressure and replacement/deadline checks. Session/controller/
policy binding, managed-session registry/replay/recording and public approvals remain open.

Managed native launch can now carry the original effect grant through both version-probe
spawn and the actual owned app-server spawn. An actual pinned-version wrapper permits the
probe, expires the grant and proves native startup has no filesystem effect. Four actual
Codex proofs pass together (1.58s), including owned cwd, denied IPC/reconnect, current/expired
termination and a descendant's own ready marker. Strict workspace/all-feature Clippy passes.

SDK owner machine patch/revoke APIs now use bounded verified HTTPS, explicit confirmation,
no implicit reqwest retry and current owner scope. Lost, malformed-success and wrong-target
replies remain Unknown; invalid input and absent confirmation are NotSent. All fifteen SDK
tests pass, including actual TLS lost-reply/count checks. CLI rename/label/remove-label/revoke
commands return JSON through the same APIs. `remote send --stdin` is implemented with a
32 KiB total bound and reads before acquiring control; its connected smoke check is pending.

The real TUI machine dialog passes rename, label add/remove, cancel and confirmed revoke.
A scoped native PTY survives revocation, accepts local input and retains unuploaded capture.
Machine/session filters are cached outside rendering, accept bounded text, preserve scope and
do not permit hidden rows as action targets. Keyboard checks include matching labels, `q`
inside filters, empty results, clearing and continued live-terminal workflows. Latest expanded
native/CLI/TUI pipeline passes 44.27s; primary-loss final verification is running. Managed
agents, durable approval delivery/UI, subscriptions and external qualification remain open.

The expanded pipeline's independent primary-volume-loss/held-content proof also passes
(27.27s). Machine/session filtering and metadata/access dialogs are actual keyboard proofs,
not widget-only checks. The further four-gateway stdin-send smoke is being verified against
the latest CLI build. No production identity/platform or full-scale claims were added.

Managed Codex sessiond composition now passes an actual pinned 0.159.2 proof (2.09s):
eight concurrent starts share one scoped native owner and initial prompt; PTY/agent key reuse
conflicts; native thread identity commits before prompt delivery; structured output reaches
bounded replay and independent redacted Events recordings. A controlled second prompt uses
durable input identity, duplicate delivery does not resend it, changed bytes conflict and a
captured grant cannot write after controller takeover. Caller loss retains the owner lock;
explicit termination preserves the first observed native end and thread identity on restart.

The same composed proof obtains a real native command approval, durably journals its exact
source/body, applies an owner winner without any controller, drops its application reply and
reconciles Unknown without a second command effect. Native resolution/end cancel pending
requests; offered answers remain uncertain. Source scope and process instance cannot be
adopted across restart. Separate negotiated native-agent-v1/native-approvals-v1 capabilities
are required on capable Linux managed builds, including historical reads without a launch profile; public gateway agent/approval capability
advertisement remains disabled until gateway delivery and structured clients are complete.
Sixty native unit tests and strict workspace/all-feature Clippy pass. The broader workspace
suite is being rerun after native IPC integration; additional managed crash/reconnect,
structured checkpoints/UI and public delivery qualification remain open.

Structured checkpoint composition now also passes against pinned Codex with a two-event
replay cap: `agent_state_v1` retains a bounded 256-event/512 KiB timeline, labels truncation
and unconfirmed tails, fixes capture sequence across pages, and restores after pruning and
owner restart. Public schema was updated intentionally. The SDK validates structured bytes
before releasing a checkpoint and exposes prompt/steer through the same poisoned-on-uncertainty
input cursor; six actual TLS loss/cancellation scenarios cover PTY/prompt/steer without resend.
The TUI has structured message/tool rendering, scrolling, prompt/steer editing and terminal
escape isolation; its full remote agent keyboard proof remains pending. Local PTY keyboard
regression passes. Native IPC negotiates optional typed formats while preserving old PTY
owners and rejects unrequested formats. Strict workspace Clippy passes; the composed
native/CLI/TUI recording and synchronous-primary-loss regression is running.

Public approval composition now passes the expanded synchronous fixture (53.07s). Real pinned
Codex requests reach independent connector polling and owner-wide gateway state. Two owner
SDK clients obtain one winner; actual TUI keys cancel then confirm a separate native request.
The primary outbox delivers through the gateway holding the current host, and native journal
application produces one file effect and one approval audit per request with zero controller
leases. Status reads work after the connector stops. Signed cross-node notifications contain
identities and re-read current primary state. Native snapshot loss cancels only pending
unchosen requests. The bootstrap is explicitly trusted private IPC; public agent launch and
installed-agent inventory remain unfinished. Complete crash/message/ACK drop drills remain.

The SDK's 19 tests cover approval pagination, wrong identity, missing capability, no-lease
choice and lost reply with no resend. The real 21-test PostgreSQL/PgBouncer suite passes
(55.83s), including primary row-lock expiry that rolls back winner/outbox, missing-native
reconciliation and current-node delivery eligibility. Public conformance now has 30 vectors.
The full workspace/all-features suite passes 161 tests with 18 explicit environment proofs
ignored. Strict Clippy, default build, boundaries and schema checks are being finalized.

Failed native version/IPC startup now waits for actual child reap before returning; confirmed
service shutdown waits for native readers instead of abandoning cleanup after two seconds.
The controlled Linux broken-start fixture verifies both child PIDs are absent immediately on
return. Optional standalone managed-codex host configuration supplies an absolute pinned
binary, HTTPS endpoint, model and private JSON-string credential file. Only sessiond opens
the provider credential; public launches and connector configuration cannot supply it.

### Latest composed managed and client evidence — 2026-10-18

Public startup discovery verifies actual Codex 0.159.2 before advertising it; each launch
rechecks the pinned binary under its original grant. Public agent start uses the same scoped
durable identity and qualified recording preflight as PTY. SDK/CLI/TUI creation, structured
prompt/control/checkpoint and original-key recovery pass through the real native/two-gateway
fixture. Confirmed-ended historical resume starts a new session/process with the saved
thread/canonical private home and fresh descriptor-scoped cwd; lost/unconfirmed history is
blocked. Prior conversation truncation is explicit in the agent checkpoint.

The latest combined synchronous fixture passes its public workflow in 79.31s and independent
primary-volume-loss/hold/audit proof in 28.30s. Its native proxy independently drops delivery
before native I/O and the application ACK after native I/O; the primary outbox retries at
least three times and the command writes one marker. Actual F7 keys answer from an observing
agent pane with zero controller leases and return to a fresh checkpoint. Machine/session
filters now accept exact IDs, avoiding ambiguous action targets across machines.

The SDK-owned output observer installs a checkpoint, deduplicates output, performs bounded
read-only idle continuation and retains slow-consumer errors outside its eight-update/16 MiB
queue. Dropping it cancels reads without resending input or killing native execution. Explicit
token refresh cannot clear uncertain input. These behaviors pass 23 SDK tests and the actual
managed fixture. This is bounded polling, not a native push subscription claim.

Managed owner SIGKILL and caller-loss/deduplication/resume proofs pass together in 2.26s.
A pidfd confirms the actual native process exits after its Linux owner thread dies; reopened
original-key lookup reports lost/unknown and creates no new child. Broken native startup waits
for actual reap. Real Unix socket tests keep native event reads progressing while an
original-authority write is deferred, and expiry permits no later bytes.

The full workspace/all-features suite passes **166 tests, zero failures, 20 explicit ignored
fixture tests**. Default all-target check, strict all-feature Clippy, dependency boundaries
and all three schema checks pass. Additional managed/native/PG proofs run through their
explicit fixture commands; ignored tests are never counted as successful external evidence.
The [MVP report](../docs/validation/mvp-evidence.md) maps requirements and remaining gates.
The synchronous runner now emits sanitized phase/status JSON, including failure/unrun states;
CI preserves that artifact.

The expanded keyboard fixture verifies the owner badge changes from one pending approval
to zero after an observe-mode decision, and displays actual CLI/Codex inventory versions.
The badge uses one bounded read job, at most 32 pages/ten seconds and explicit partial or
unavailable labels; it grants no authority. The latest sanitized composed artifact records
a **122,240 ms** complete pass and explicitly leaves enterprise/platform/scale gates unrun.

Latest development rollout also passes after fixing an old-image restart bug. The command
now applies additive migrations, recreates gateways sequentially and verifies actual
readiness and the selected running image before continuing. All four healthy non-root
gateways now use the rebuilt **19,752,671-byte** image
`sha256:52791c58b44ab6b7abeca32b35c71bdc8e0c7b74b3451c0be63631f942ec6034`.
The actual CLI/native PTY smoke preserves native ownership and original-key lookup across
all four recreations, then verifies new controlled input, cross-node dispatch, confirmed
termination and redacted protected history. The [rollout artifact](../docs/validation/development-roll-2026-10-18.json)
records the actual images. The development identity image also rebuilt successfully.
These checks qualify this development PTY deployment, not a deployed managed-agent/provider
or production service/identity/backup boundary.

### Native subscriptions and remaining start boundaries — 2026-10-18

Three new Linux PTY process checks qualify interrupt versus termination, lossless bounded
512 KiB bursts with resize/input, and continued native execution after the last output
reader drops. Three scoped Unix notification tests qualify foreign-owner denial, idle
expiry, cancellation, 10,000-change coalescing and blocked writes retaining their original
access budget. The native change map has 64 scope slots and four stream permits; each
actual write checks the suspend-aware original fence and replacement barrier.

`native-changes-v1` and `host-changes-v1` compose native revisions with independent connector
reads. Metadata pushes owner-wide session state; exact floor/head/input counters reach
attached viewers. Cache updates follow queue admission and periodic full state repairs
missed metadata. Head work uses the gateway's bounded metadata lane. The real two-gateway
PTY proof receives a resize head without making any replay request. `session-changes-v1`
lets supporting TUI peers stop session-list polling; output continuations retain bounded
idle reconciliation. Dialogs drain notifications and return to fresh state/checkpoints.

All six managed start boundaries now pass against actual Codex 0.159.2: before journal,
accepted, launching, native launch, committed identity/state and before caller ACK. An owned
helper barrier permits parent/pidfd validation before each injected post-launch exit. Known
keys reopen as lost/unknown with no second spawn; pre-journal lookup remains explicitly
unknown and sends no launch. These hooks exist only in test builds. The combined three
managed proofs (including owner SIGKILL and caller-loss/resume) pass in 2.27s. Native IPC
reconnect is still unsupported: IPC failure reports uncertainty and retains explicit owner
termination rather than automatically restarting or replaying an uncertain operation.


### Latest verified Linux checkpoint — 2026-10-18

The full workspace/all-features suite passes **174 tests, zero failures and 21 ignored
environment proofs**. Strict workspace/all-target/all-feature Clippy, default all-target
check, formatting, all three schemas and dependency boundaries pass. The sanitized
[Linux check artifact](../docs/validation/linux-development-checks-2026-10-18.json) links
the separately executed proofs. The acceptance runner now requires all three actual managed
process tests when Codex is supplied, including the six-point start crash matrix. Its
latest composed report passes all five phases in **108,986 ms**. The public workflow and
primary-loss phases take 69.10s and 27.39s respectively; fixture startup/qualification and
managed crash proofs are additional.

The pinned private Prometheus evaluator passes **13 alert rules / 40 fixture assertions**
for sustained failure, recovery, healthy state and missing probes. Actual collection
reaches all four gateways, includes original-elapsed directory freshness and ingests the
real private native spool's lag/pending/warning measurements. A three-iteration scheduled
probe and an actual stopped-probe firing state pass. No notification destination or host
probe service supervisor is installed. Monitoring data is private and ephemeral; it is
separate from protected product recordings.

The latest gateway image is **19,769,055 bytes**,
`sha256:f0c896346a5081c714d364083fbf5859cc82d8966d13fef28b47851bf44645b8`. All four healthy non-root gateways use it.
Actual CLI/native PTY ownership and the original key survive all four recreations; new
input, cross-node dispatch, termination and protected redacted history pass. The fixture
image also rebuilt successfully after a disk-full retry; unused build cache was cleared,
with database/secret volumes and running native owners preserved.

The refreshed eight-socket/two-PTY/200-echo baseline has p50/p95/p99
**46.74/56.79/60.84 ms**, zero input retries or cleanup failures and zero final spool/replica
lag. These are SDK input-to-replay observations, with no direct PTY baseline subtracted.
The capacity/storage artifacts use this measured sample and label future scale/storage
projections as estimates. They do not qualify the full beta workload.

The standing goal remains all 28 tasks. Remaining development follows the next-action
list above; actual Entra/macOS/service-manager evidence, representative beta hardware,
production fault domains, backup targets and operating assignments require suitable
environments and owners. No unrun requirement has been marked complete.
