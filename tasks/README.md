# QGenie Remote development tasks

The original plan contains **28 development tasks** from a design-only baseline.
The repository now contains their standalone Rust implementation, fixtures and deployment.
The numbered order records dependencies; current results are in [progress](PROGRESS.md).
Later tasks record repairs, build tooling and the
[implemented daily remote workflow](033-implement-daily-remote-workflow.md).

**Selected scope:** The user authorized standalone CLI/TUI composition and then selected
“Finish Linux development scope; document external release gates” through
[ADR 0007](../docs/adr/0007-standalone-preview.md). Task 001's corporate-source inspection
is superseded for that path. Read [current progress](PROGRESS.md) before interpreting
baseline-only observations below. Original criteria requiring unrun environments remain
unchecked and are tracked in the [external release gates](../docs/validation/external-release-gates.md).

## Original planning baseline

Reviewed baseline: commit `c6c46a8`. The repository contains [README](../README.md), [PRD v0.8](../PRD.md), [architecture v0.8](../architecture.md), [the component proposal](../components.md), and [the subcrate standard](../subcrate-standard.md). There are no application sources, Cargo manifests, tests, deployment files, or repository `AGENTS.md` at this baseline. Paths inside the tasks are proposed implementation locations, not claims about existing code.

QGenie Remote is a Rust system for working on enrolled machines through a corporate gateway using outbound HTTPS/WSS. The client is a TUI in the existing QGenie CLI. A host connector handles network access and policy; a separate sessiond process owns PTYs/agents, replay, and recording. The gateway owns access, routing, durable metadata, approval decisions, audit, and transcript history. PostgreSQL is durable authority; Redis is disposable routing/presence infrastructure.

The task boundaries follow the nine-crate proposal: `proto`, `transport`, `policy`, `adapters`, `sessiond`, `host`, `client`, `tui`, and `gateway`. They also isolate CLI integration, deployment, and verification. All nine now have implemented behavior; the observations above describe the original baseline.

## Scope and release gates

The initial target is the **M0 terminal slice and M1 TUI/Codex MVP** on Linux and macOS. Tasks 020–022 cover mandatory recording and its access/retention protections. The PRD places those requirements in M2 while describing recording as always enabled; [002](002-resolve-runtime-and-protocol-contracts.md) must reconcile that release gate, following the proposal in components §7. This plan does not silently claim the milestone change is approved or permit unprotected persistent recordings. Task 028 starts measurement during alpha and explicitly retains the later beta-scale qualification gate.

Electron, the TypeScript SDK, Windows hosts, Claude/Pi/exec, external-session adoption/backfill, richer org policy, the production three-server topology, sharing, web access, file transfer, and Kubernetes need later tasks. Preserve their declared boundaries/capabilities without making them dependencies of the first terminal/Codex slice.

These requirements apply across tasks:

- Connector/client loss, pause, and access revocation preserve running local sessions. Explicit termination is a separate action.
- Owners can answer approvals without a controller lease or writable attachment, subject to current access and host policy.
- Allowed roots restrict browsing, new-session cwd, and future file transfer. Existing same-user sessions retain adapter-supported control outside those roots.
- Sessiond owns durable start reconciliation and live replay. An uncertain start is reconciled with its original key; input is never automatically resent.
- Recording is separate from replay, cannot be disabled, and may lose content only with explicit gaps under the documented failure/cap behavior.
- Gateway revocation must enforce access denial within 10 seconds; Entra disablement includes detection and enforcement within 15 minutes. Redis hints and token expiry alone cannot satisfy either guarantee.
- Native capabilities, non-exportable key storage, service isolation, and production scale require evidence, not assumptions from illustrative design examples.

## Task index

All tasks initially started as **Not started**. [Progress and blockers](PROGRESS.md) records the active goal and current status. Dependencies are direct prerequisites; their own prerequisites are transitive. The individual task documents define implementation scope, acceptance criteria, tests, exclusions, and handoff.

| Task | Deliverable | Depends on | Workstream |
| --- | --- | --- | --- |
| [001](001-inspect-existing-qgenie-integration.md) | Inspect the existing QGenie CLI and define integration seams | None | Integration |
| [002](002-resolve-runtime-and-protocol-contracts.md) | Resolve state authority, lifecycle, and protocol decisions | None | Contracts |
| [003](003-bootstrap-rust-workspace-and-ci.md) | Bootstrap the Rust workspace and repository checks | [001](001-inspect-existing-qgenie-integration.md) | Build tooling |
| [004](004-implement-protocol-and-conformance.md) | Implement public protocol types and conformance fixtures | [002](002-resolve-runtime-and-protocol-contracts.md), [003](003-bootstrap-rust-workspace-and-ci.md) | Protocol |
| [005](005-implement-websocket-transport.md) | Implement bounded WSS transport with proxy support | [004](004-implement-protocol-and-conformance.md) | Transport / WSS |
| [006](006-implement-secure-local-ipc.md) | Implement secure same-user local IPC | [004](004-implement-protocol-and-conformance.md) | Transport / IPC |
| [007](007-implement-policy-engine.md) | Implement deterministic action-level policy | [004](004-implement-protocol-and-conformance.md) | Policy |
| [008](008-implement-adapter-contract-and-pty.md) | Implement the adapter interface and PTY driver | [004](004-implement-protocol-and-conformance.md) | Adapters / PTY |
| [009](009-implement-verified-codex-adapter.md) | Validate and implement the pinned Codex adapter | [001](001-inspect-existing-qgenie-integration.md), [008](008-implement-adapter-contract-and-pty.md) | Adapters / Codex |
| [010](010-implement-gateway-foundation-and-audit.md) | Build the gateway application, durable store, and audit API | [004](004-implement-protocol-and-conformance.md), [007](007-implement-policy-engine.md) | Gateway foundation |
| [011](011-implement-gateway-identity-and-enrollment.md) | Implement gateway user and machine authentication | [001](001-inspect-existing-qgenie-integration.md), [005](005-implement-websocket-transport.md), [010](010-implement-gateway-foundation-and-audit.md) | Gateway identity |
| [012](012-implement-routing-and-presence.md) | Implement gateway routing, inventory, and push presence | [005](005-implement-websocket-transport.md), [010](010-implement-gateway-foundation-and-audit.md), [011](011-implement-gateway-identity-and-enrollment.md) | Gateway routing |
| [013](013-implement-host-enrollment-and-credentials.md) | Implement host enrollment and machine credential storage | [001](001-inspect-existing-qgenie-integration.md), [005](005-implement-websocket-transport.md), [011](011-implement-gateway-identity-and-enrollment.md) | Host identity |
| [014](014-implement-host-dispatch-and-filesystem-policy.md) | Implement the host connector and local filesystem enforcement | [006](006-implement-secure-local-ipc.md), [007](007-implement-policy-engine.md), [013](013-implement-host-enrollment-and-credentials.md) | Host connector |
| [015](015-implement-session-runtime-and-start-journal.md) | Implement sessiond and durable session-start reconciliation | [006](006-implement-secure-local-ipc.md), [007](007-implement-policy-engine.md), [008](008-implement-adapter-contract-and-pty.md) | Session runtime |
| [016](016-implement-session-replay-and-snapshots.md) | Implement bounded replay and atomic snapshot/live cutover | [015](015-implement-session-runtime-and-start-journal.md) | Session replay |
| [017](017-implement-fenced-control-leases.md) | Implement durable host and controller fencing | [012](012-implement-routing-and-presence.md), [015](015-implement-session-runtime-and-start-journal.md) | Control authority |
| [018](018-implement-durable-approvals.md) | Implement one-winning-decision approvals with durable delivery | [009](009-implement-verified-codex-adapter.md), [012](012-implement-routing-and-presence.md), [015](015-implement-session-runtime-and-start-journal.md) | Approval state |
| [019](019-implement-revocation-and-entra-freshness.md) | Implement bounded revocation and Entra disablement enforcement | [011](011-implement-gateway-identity-and-enrollment.md), [012](012-implement-routing-and-presence.md) | Access authority |
| [020](020-implement-recording-spool.md) | Implement the durable recording spool and explicit loss gaps | [001](001-inspect-existing-qgenie-integration.md), [015](015-implement-session-runtime-and-start-journal.md) | Host recording |
| [021](021-implement-transcript-storage-and-access.md) | Implement recording upload, durable ingest, and protected reads | [010](010-implement-gateway-foundation-and-audit.md), [013](013-implement-host-enrollment-and-credentials.md), [020](020-implement-recording-spool.md) | Gateway recordings |
| [022](022-implement-legal-holds-and-retention.md) | Implement legal holds and database-coordinated retention | [021](021-implement-transcript-storage-and-access.md) | Recording lifecycle |
| [023](023-implement-rust-client-sdk.md) | Implement the Rust client SDK and reconnect state | [005](005-implement-websocket-transport.md) | Rust client |
| [024](024-implement-tui-workflows.md) | Implement the TUI machine, session, terminal, and approval flows | [023](023-implement-rust-client-sdk.md) | TUI |
| [025](025-integrate-cli-and-host-services.md) | Integrate CLI commands and independent host services | [009](009-implement-verified-codex-adapter.md), [014](014-implement-host-dispatch-and-filesystem-policy.md), [016](016-implement-session-replay-and-snapshots.md), [017](017-implement-fenced-control-leases.md), [018](018-implement-durable-approvals.md), [020](020-implement-recording-spool.md), [021](021-implement-transcript-storage-and-access.md), [024](024-implement-tui-workflows.md) | CLI and service composition |
| [026](026-build-development-compose-stack.md) | Build a reproducible four-gateway development deployment | [012](012-implement-routing-and-presence.md) | Deployment |
| [027](027-prove-end-to-end-mvp-behavior.md) | Prove the terminal and Codex MVP through real failure scenarios | [019](019-implement-revocation-and-entra-freshness.md), [022](022-implement-legal-holds-and-retention.md), [025](025-integrate-cli-and-host-services.md), [026](026-build-development-compose-stack.md) | End-to-end verification |
| [028](028-measure-capacity-and-write-operations-runbooks.md) | Measure resource use and document operational recovery | [019](019-implement-revocation-and-entra-freshness.md), [022](022-implement-legal-holds-and-retention.md), [025](025-integrate-cli-and-host-services.md), [026](026-build-development-compose-stack.md) | Observability and operations |

## How to execute the work

**Sequential:** work through 001–028 in numerical order. Do not mark external proofs complete just because mocked tests pass.

**Parallel:** the groups below are a conservative schedule computed from the dependency graph. Tasks in a group have no dependency on each other. A task may start earlier than its group once its own prerequisites are complete; nobody needs to wait for an unrelated workstream.

| Group | Tasks that can proceed together after their prerequisites |
| --- | --- |
| 1 | [001](001-inspect-existing-qgenie-integration.md) Integration; [002](002-resolve-runtime-and-protocol-contracts.md) Contracts |
| 2 | [003](003-bootstrap-rust-workspace-and-ci.md) Build tooling |
| 3 | [004](004-implement-protocol-and-conformance.md) Protocol |
| 4 | [005](005-implement-websocket-transport.md) Transport / WSS; [006](006-implement-secure-local-ipc.md) Transport / IPC; [007](007-implement-policy-engine.md) Policy; [008](008-implement-adapter-contract-and-pty.md) Adapters / PTY |
| 5 | [009](009-implement-verified-codex-adapter.md) Adapters / Codex; [010](010-implement-gateway-foundation-and-audit.md) Gateway foundation; [015](015-implement-session-runtime-and-start-journal.md) Session runtime; [023](023-implement-rust-client-sdk.md) Rust client |
| 6 | [011](011-implement-gateway-identity-and-enrollment.md) Gateway identity; [016](016-implement-session-replay-and-snapshots.md) Session replay; [020](020-implement-recording-spool.md) Host recording; [024](024-implement-tui-workflows.md) TUI |
| 7 | [012](012-implement-routing-and-presence.md) Gateway routing; [013](013-implement-host-enrollment-and-credentials.md) Host identity |
| 8 | [014](014-implement-host-dispatch-and-filesystem-policy.md) Host connector; [017](017-implement-fenced-control-leases.md) Control authority; [018](018-implement-durable-approvals.md) Approval state; [019](019-implement-revocation-and-entra-freshness.md) Access authority; [021](021-implement-transcript-storage-and-access.md) Gateway recordings; [026](026-build-development-compose-stack.md) Deployment |
| 9 | [022](022-implement-legal-holds-and-retention.md) Recording lifecycle; [025](025-integrate-cli-and-host-services.md) CLI and service composition |
| 10 | [027](027-prove-end-to-end-mvp-behavior.md) End-to-end verification; [028](028-measure-capacity-and-write-operations-runbooks.md) Observability and operations |

Start 002 immediately from the design documents. Start 001 by obtaining and inspecting the real QGenie CLI source; that repository is not in this checkout. If it is unavailable, record the integration/toolchain blocker while contract work continues. After 004, WSS transport, local IPC, pure policy, and PTY adapter development have distinct boundaries and shared fixtures.

Prioritize the terminal path first: enrollment → gateway routing → host policy → sessiond/PTY → replay → SDK/TUI → CLI service composition. Develop Codex proof in its own adapter workstream. Task 027's M0 smoke harness should be written and run as those pieces land; its complete dependency list gates final M1 acceptance, not creation of an early smoke test.

Tasks 023 and 024 can use protocol-backed fake servers/SDK events. Tasks 020 and 021 use a spool/receipt contract. Gateway domain tasks use the foundation from 010. This allows independent verification before final wiring. Contracts must come from 002/004; do not invent incompatible private substitutes.

## Ownership and merge coordination

In task scope lists, Gateway, Host, and Sessiond abbreviate `crates/qgenie-remote-gateway/`, `crates/qgenie-remote-host/`, and `crates/qgenie-remote-sessiond/`. Source modules live under each crate's `src/`; migrations stay with the owning crate. Existing CLI paths must be verified in 001 rather than assumed to be present here.

- Each task owns its listed modules and tests. No task is permission to implement a different task's subsystem or create empty future crates.
- Each new crate owns its initial manifest, README, and public exports. Parallel contributors coordinate root `Cargo.toml`/`Cargo.lock`, shared `lib.rs`/module exports, and CI edits as small integration changes.
- 005/006 share transport composition only; their WSS and IPC modules are separate. 008 establishes the adapter interface before 009 adds Codex.
- 010 establishes gateway composition/base migrations. Identity, routing, leases, approvals, revocation, transcripts, and retention append domain changes. Coordinate unique migration identifiers; never rewrite an already merged migration.
- 015 establishes sessiond's registry/start journal and extension hooks. 016, 017, 018, and 020 own replay, fences, approval application, and recording respectively.
- Changes to public protocol or shared operation semantics land as a focused prerequisite update with fixtures and affected task notes, rather than conflicting implementations.
- Use separate branches/worktrees or bounded changes for concurrent work. Use separate fixtures/databases for destructive fault/retention/load tests, especially 027 and 028.

## External inputs and design decisions to track

| Input or unresolved decision | Owning task | Completion evidence |
| --- | --- | --- |
| Actual QGenie source, toolchain, OAuth, key storage, launch, service, and redaction facilities | 001; integration in 025 | Inspected revision and working integration; no invented APIs |
| Protocol precision/limits, IPC, state authority, fencing, lifecycle, and recording release mapping | 002; encoded in 004 | Decision records, operation matrix, and compatible fixtures |
| Actual distributed Codex version, native commands, relay control, and capabilities | 009 | Pinned native contract tests |
| Tenant configuration and Entra disablement detection | 011/013/019 | Real remote-scope/device-code and disablement proof |
| Independent Linux/macOS service lifetime | 025 | Actual systemd/launchd restart/pause drills |
| CA/proxy/dev infrastructure and service-readable secrets | 026 | Reproducible four-gateway stack and connectivity evidence |
| Data-volume allocation, backup target, operating owners, and beta-scale hardware | 028 | Measurements, recovery evidence, named assignments or explicit blockers |

Illustrative SQL, paths, and vendor commands in the design documents are inputs to validate. The component proposal explicitly refines older examples: hostname is not unique identity; Redis is not lease authority; setsid alone is not service isolation; a committed approval is not proof of native application; retention cannot rely on a Redis leader lock alone.

## Completion standard

A task is complete when its acceptance criteria and relevant behavioral checks pass, its public contract and limits are documented, and its handoff is usable by dependent tasks. Update its status and record verification results and any remaining blockers in that file. Tests against fakes can establish a component contract; they cannot establish real tenant, vendor, platform-service, proxy, or scale guarantees.

No application tests can be run at the reviewed baseline because application code does not exist. For this task-plan change, verify document completeness, link targets, unique task IDs, an acyclic dependency graph, and consistency between the index and task files.
