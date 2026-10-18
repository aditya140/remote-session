# QGenie Remote subcrate standard

Status: workspace convention. Applies to the implemented and planned crates described in [components](components.md). Manifests, pinned tooling, schema export, and dependency checks now exist; [task progress](tasks/PROGRESS.md) records verification.

## 1. When to make a crate

A subcrate owns a named capability and a small public contract. Extract it when at least one concrete boundary justifies the cost:

- Two or more independent consumers need the capability, such as host/client/gateway transport.
- It isolates substantial dependencies or a platform/vendor integration, such as native agent adapters from gateway builds.
- It has an independently testable behavioral contract used by other components, such as deterministic policy decisions.
- It is a deployment/composition target, such as the gateway binary.

Then require all of the following: a describable owner/responsibility, an acyclic dependency direction, a useful public API, and no need to expose the parent's internals to operate. "This file is large," "one struct," and "we might reuse it someday" are insufficient by themselves.

Start a concern as a module if it shares lifecycle, state, dependencies, and consumers with its parent. Size is a signal to review cohesion, not an automatic extraction threshold. In particular, start individual agent adapters, TUI views, repositories, and replay/spool implementation as modules in their designated crates.

## 2. Naming, manifest, and layout

Use package names `qgenie-remote-<responsibility>`, imported as `qgenie_remote_<responsibility>`. Use `snake_case` Rust modules/files, specific type names, and explicit feature names. Avoid `common`, `utils`, and generic `Manager` APIs without a defined resource/lifecycle.

```text
crates/qgenie-remote-example/
├── Cargo.toml
├── README.md
├── src/
│   ├── lib.rs             # purpose, supported contract, focused re-exports
│   ├── config.rs          # only if this crate has configuration
│   ├── error.rs
│   ├── example.rs         # cohesive implementation module
│   └── subsystem/        # add when a concern requires multiple files
│       ├── mod.rs
│       └── operation.rs
├── tests/                # cross-module/public API behavior, when warranted
├── examples/             # only useful executable usage examples
└── migrations/           # only the crate owning that durable schema
```

This is a convention, not a requirement to create empty files/directories. Use `foo.rs` until it earns a `foo/` subtree. Do not mix `foo.rs` and `foo/mod.rs` for the same module. Name files after the behavior they implement, not their technical category (`helpers.rs`, `types2.rs`). Split a file when it mixes responsibilities or obscures a state machine; keep tightly related transitions together. Do not impose arbitrary line-count gates.

The root workspace owns the chosen edition, MSRV, dependency versions, lints, profiles, and resolver compatible with the chosen toolchain. Inspect the existing CLI before choosing toolchain values. Members inherit package metadata, shared dependencies, and workspace lints. Commit the workspace lockfile because it builds deployed applications. Path dependencies inside the workspace must be accompanied by compatible versions when internal package publication is introduced.

Minimum crate README:

1. Purpose and what state, if any, the crate owns.
2. Consumers and allowed direct product dependencies.
3. Public entry points with one useful example when applicable.
4. Configuration, optional features, and supported platforms.
5. Cancellation, resource limits, persistence/retry behavior, and relevant verification commands.

An actual owning team must be recorded when staffing is assigned; do not invent one in an initial manifest.

## 3. Dependency and public API rules

- Follow the [component dependency graph](components.md). No upward dependency, circular dependency, or product dependency on the CLI executable crate.
- `proto` is a lightweight wire-contract library: no `tokio` runtime, OAuth/keyring, `sqlx`, Redis, `ratatui`, or native agent client. Schema generation dependencies may be optional. Define signed challenge bytes here; store keys and perform identity operations outside it.
- `policy` evaluates caller-supplied facts without I/O. Filesystem enforcement belongs on the host; server authorization freshness belongs in the gateway.
- `transport` moves validated, bounded frames. Client session replay/idempotency belongs in `client`; process ownership and durable logs belong in `sessiond`; access decisions belong in `policy`/gateway/host.
- Public APIs expose domain types, not PostgreSQL rows, Redis commands, ratatui state, or vendor JSON-RPC structures. Native extras may be carried in the agreed protocol field without making a vendor protocol the shared model.
- Keep implementation modules private or `pub(crate)`. Re-export the few intended entry points in `lib.rs`; do not make every module public.
- Use explicit integration interfaces for credentials, launch preparation, services, and redaction where existing QGenie facilities must be injected. Do not introduce a trait for every concrete implementation. Existing auth code supplies scoped credentials; it is not copied into the SDK.
- Start with concrete implementations. Add a repository trait when a real alternate backend or meaningful test seam needs it; avoid generic plugin frameworks and a giant dependency container.
- Libraries receive typed config and dependencies; entry points read environment/CLI options and configure tracing. Libraries do not install global log subscribers, call `process::exit`, or start an invisible runtime.

## 4. Errors, async work, and bounded resources

Use a typed `Error` with distinguishable cases for failures callers must handle. `thiserror` is suitable for crate errors; contextual application errors are appropriate at command/binary boundaries. Do not leak secrets or backend diagnostics in public error bodies.

Wire errors use stable codes and the protocol's outcome distinctions: `not_sent`, `unknown`, and `rejected`. A network timeout is not proof that a start or approval had no effect. Retry behavior belongs with the operation's idempotency/acknowledgment contract, not a blanket transport retry loop.

Async APIs run on a caller-owned `tokio` runtime. Tasks have explicit ownership, shutdown/cancellation paths, and joinable handles. No unbounded queues for network, session events, or recordings. Define count/byte limits and the response to saturation: reject a request, disconnect/resync a slow observer, coalesce safe output, or record a gap under the recording policy. Do not silently drop approvals or queued mutations.

Keep blocking PTY/file/compression/SQLite work off async executor threads when it can stall them. Bound concurrent blocking work as well. Avoid holding locks across unrelated network/database waits. Where correctness needs serialization, identify the state machine or per-session actor that supplies it.

Document cancellation at irreversible boundaries. Dropping a future after a start commit cannot implicitly cancel a child process; return/reconcile the committed operation. Session runtime lifetime cannot depend on a viewer task being alive.

## 5. Durable state and security contracts

Each mutable state machine has one documented authority, as mapped in [components](components.md). Cached/projected state does not silently become authority during an outage.

For persistent operations specify:

- Stable identity/idempotency scope and conflict behavior.
- Transaction/commit boundary and crash recovery from each externally visible stage.
- What an acknowledgment proves, and what remains unknown.
- Ordering/fencing, cursor retention, and any snapshot/live transition.
- Authority freshness, resource caps, degradation, and operator/user-visible loss.

Postgres and SQLite migrations live with their owning crates. Use backward-compatible expansion/migration/contraction for rolling releases; define a supported schema range instead of requiring an exact version during transition. A destructive cleanup job needs database-authoritative coordination, not only an ephemeral Redis leader lock.

Enforce identity and authorization at trust boundaries: gateway per operation, host per allowed action/path, session owner for active mutation generations and secure same-user IPC. Keep local administrative commands separate from remotely callable operations. Never trust a routing hint as identity.

Use secret wrappers/redacted formatting where credentials enter interfaces. Do not serialize provider credentials into public protocol messages or log raw PTY input/approval content as diagnostic metadata. Reuse QGenie's redaction; recordings still receive strict access control because redaction is best effort.

Owner approval rights and controller rights are separate. Outside-root existing sessions retain adapter-supported control. These are product contracts and must not change as an incidental result of moving code between crates.

## 6. Features, compatibility, and distribution

Features should select additive capability or expensive optional dependencies, such as transport `client-ws` / `server-ws` / `local-ipc`, adapter `codex` / `pty` / later vendor integrations, and protocol `schema`. Do not use a compile feature to enable/disable authorization or mandatory recording. Do not rely on Cargo features as a security boundary: Cargo unifies features across consumers.

Use target-specific dependencies for OS-specific code; a gateway build must not pull PTY/ConPTY, keyring, or TUI libraries through an accidental dependency. Build supported feature combinations explicitly. Unsupported combinations should fail clearly, not quietly lose required behavior.

Initially release remote crates together at compatible package versions/revisions. Keep package version, wire protocol version, local IPC version, durable schema version, and native-agent compatibility version distinct. Generate public schemas from `proto`; check deterministic output in CI. Future TypeScript types and SDK conformance consume that contract. Validate numeric precision, binary encoding, errors, and negotiation across languages; generated types alone do not prove compatibility.

Maintain a pinned native-agent capability matrix. Advertise only tested capabilities; unavailable native control remains observe-only or explicitly deferred. Adapter upgrades need native contract verification without exporting vendor churn into other crates.

## 7. Verification and review standard

Verify meaningful behavior at the layer that owns it:

| Layer | Required evidence as functionality is added |
| --- | --- |
| Protocol | Golden JSON/binary fixtures; invalid/truncated frame handling; version/capability negotiation; future JS-compatible values |
| Policy | Owner/controller/approver distinctions; revoked access; new-session roots versus outside-root existing control; adapter capability limits |
| Transport/client | Proxy/CA connectivity, heartbeat and shutdown, bounded queues, lost final-frame detection, reconnect and outcome-unknown handling |
| Sessiond/adapters | Durable same-key start recovery, controller fencing, approval application uncertainty, snapshot/replay cutover, actual pinned agent protocols |
| Recording/gateway | Lost commit ack dedupe, spool cap gaps, cross-user and held-table RLS, transaction-pooled context isolation, audited reads, hold/retention races |
| End-to-end | Real service restart, gateway/Redis failure, revocation with missed broadcasts, Entra verification failure, pause/resume, process survival |

Unit tests belong beside logic; crate integration tests exercise public contracts; workspace end-to-end tests exercise real processes/dependencies. Shared conformance fixtures are consumed by implementations, not duplicated. Avoid tests that merely repeat an obvious reversible file edit or mirror implementation structure.

Once Rust code exists, CI should run formatting, workspace linting, applicable unit/contract tests, dependency-direction checks, and the relevant supported target/feature builds. Full native-agent, deployment-fault, and scale drills belong in environments that can run them. Require checks appropriate to the changed behavior; avoid running an entire fleet load test for a UI wording change.

A new crate review must answer: why a module is insufficient, who consumes it, what API/state it owns, what dependencies it introduces, how it fails, how it is verified, and how it integrates with existing QGenie. For crate extraction, move existing behavior first without silently changing wire/security semantics.
