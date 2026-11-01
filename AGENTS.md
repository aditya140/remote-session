# Project instructions

These instructions apply to all files in this repository.

## Project status

This repository contains the standalone Rust workspace, application code, migrations, tests, and development configurations for QGenie Remote.
The user selected Linux development completion. See [task progress](tasks/PROGRESS.md) for executed evidence and [external release gates](docs/validation/external-release-gates.md) for unrun qualification.
Treat commands and interfaces in the design documents as proposals until implementation and tests establish their behavior.

The original first-release client uses a Rust terminal user interface (TUI).
The user approved a gateway-hosted admin and owner browser companion in
[ADR 0010](docs/adr/0010-browser-console.md). Full browser terminal interaction
remains outside that companion scope.
The gateway uses PostgreSQL for durable data and Redis for routing and presence.
The host connector and session owner are separate processes.
The session owner keeps agent and terminal processes alive when clients disconnect.

## Project references

Read the relevant documents before you change product behavior:

- Use [PRD.md](PRD.md) for product requirements and release scope.
- Use [architecture.md](architecture.md) for process, protocol, storage, and security design.
- Use [components.md](components.md) for crate responsibilities and dependency direction.
- Use [subcrate-standard.md](subcrate-standard.md) for Rust structure, public interfaces, and verification requirements.

If these documents disagree, identify the disagreement before you change the affected contract.
Keep facts, observations, proposed changes, and unknown information separate.
Do not describe a proposed capability as an implemented capability.

## Development environment

Use [.devcontainer/devcontainer.json](.devcontainer/devcontainer.json) as the development container entry point.
See [README.md](README.md) for environment details.
The configuration provides Rust tools, PostgreSQL, Redis, SQLite tools, and tmux.
It does not start an application.
Use [the local application guide](docs/development/local-stack.md) and
`python3 tools/dev.py up` for the complete Linux stack with real local Keycloak
sign-in. The default deployment profile remains the synthetic CI fixture.

The container image supplies a development toolchain.
The standalone workspace selects Rust 1.99.1 and edition 2024.
The user waived the corporate CLI source prerequisite in [ADR 0007](docs/adr/0007-standalone-preview.md).
Review that decision before you change `rust-toolchain.toml` or the workspace edition.

Keep real credentials, tokens, machine keys, and recordings out of Git.
Use the database credentials in the development configuration only for local development.
The development database account is a database administrator.
Do not use that account to establish row-level security (RLS) behavior.
Use separate application roles for RLS tests when the database schema exists.

## Rust implementation

- Follow the dependency graph in `components.md`.
- Use `qgenie-remote-<responsibility>` for product crate names.
- Start a new concern as a module unless a crate boundary has a concrete purpose.
- Do not create empty crates or speculative frameworks.
- Keep `proto` independent of runtimes, databases, identity providers, and native agent clients.
- Keep `policy` decisions free of input/output operations.
- Keep host execution dependencies out of the gateway.
- Keep vendor protocol structures inside their adapters.
- Reuse QGenie authentication, credential storage, service management, redaction, and agent launch facilities through explicit interfaces.
- Give each mutable state machine one documented authority.
- Bound queues, replay logs, recording buffers, and concurrent work.
- Give background tasks explicit owners and shutdown paths.
- Use typed errors for failures that callers must distinguish.
- Keep PostgreSQL and SQLite migrations with the crates that own their schemas.
- Commit `Cargo.lock` when the application workspace exists.

## Product contracts

- Keep sessions alive after client disconnection, host connector pause, or access revocation.
- Treat session termination as a separate action.
- Keep approval rights separate from the session control lease.
- Permit only one winning decision for each approval request.
- Preserve the distinction between `not_sent`, `unknown`, and `rejected` outcomes.
- Reconcile an uncertain session start with its original idempotency key.
- Do not retry an uncertain mutation as a new operation.
- Apply allowed roots to browsing, new-session directories, and file transfer.
- Preserve supported control of existing same-user sessions outside those roots.
- Keep durable authorization decisions in PostgreSQL. Redis does not replace that authority during a failure.
- Keep agent connections local to the host. Do not add another remote access path.
- Preserve mandatory recording and explicit loss reports when recording storage fails.
- Keep desktop integration, the TypeScript SDK, and later adapters outside the first-release dependencies.

## Verification

Select checks that establish the behavior of the change.
Follow any explicit user limit on tool execution or verification.
If you do not run a check, state that limit in the result.
Do not report a build, test, service, or deployment as successful without evidence.

When a Cargo workspace exists, use these baseline commands as applicable:

```sh
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Check supported feature combinations separately when the change affects them.
Do not assume that `--all-features` represents a supported configuration.
Test protocol contracts, authorization decisions, recovery paths, and process lifetimes at the layer that owns them.
Use native platform tests for service management, credential storage, and process survival.
A Linux development container does not establish macOS or Windows behavior.

Before you finish, summarize the changes, checks, failures, and remaining unknowns.
Preserve existing user changes and local backup files.

## Document writing

Apply ASD-STE100 Simplified Technical English when you write or revise any document, including Markdown files.

- Use short sentences. Give each sentence one topic.
- Use the active voice. State who does the action.
- Write procedures as direct instructions. Put a necessary condition before its action.
- Use complete sentences. Do not remove articles or subjects to reduce sentence length.
- Do not use contractions in documents.
- Use one term for one meaning. Define necessary technical nouns and verbs.
- Use the official standard to check word meanings and writing rules.
- Do not claim verified compliance without this check.
- Separate facts, observations, proposed changes, and unknown information.
- For a design decision, state the user task, the problem, the proposed action, and the reason.
- Preserve exact code, commands, names, and source quotations when their accuracy requires it.

Reference: [ASD-STE100, Issue 9](https://www.asd-ste100.org/assets/files/ASD-STE100_ISSUE9.pdf).
