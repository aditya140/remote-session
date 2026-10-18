# 001 — Inspect the existing QGenie CLI and define integration seams

- **Status:** Superseded for the standalone path by explicit user direction; corporate inspection was waived
- **Milestone:** M0 foundation
- **Workstream:** Integration
- **Depends on:** None
- **Index:** [Task plan](README.md)

**Scope:** The user selected Linux development completion in [ADR 0007](../docs/adr/0007-standalone-preview.md). See [current progress](PROGRESS.md) and [external release gates](../docs/validation/external-release-gates.md). Original criteria that require unrun environments or waived corporate inspection remain unchecked; this status does not claim full release qualification.

## Goal and repository context

**Scope update:** The user authorized a new/sample TUI without the existing CLI source. [ADR 0007](../docs/adr/0007-standalone-preview.md) replaces this prerequisite with a standalone composition root. Do not request that source again to continue local implementation. The original inspection criteria below remain unperformed; they apply only to a future corporate integration.

This checkout contains only design documents. The existing QGenie CLI, its OAuth code, service installers, agent launchers, and redaction utilities are referenced but absent. Establish what can actually be reused before choosing implementation APIs or a Rust toolchain.

**Read first:** [PRD.md](../PRD.md) §5.4 and ID-1–5 / SE-5; [components.md](../components.md) §§5, 8; [subcrate-standard.md](../subcrate-standard.md) §§2–3.

## Owned scope

- `docs/integration/qgenie-cli.md`
- `Integration ADR under docs/adr/; no product implementation in this task`

Owned paths define this task’s implementation boundary; current source and executed evidence are recorded below. Follow [the subcrate standard](../subcrate-standard.md), add only useful modules, and coordinate shared manifests/exports and migrations as described in the index.

## Implementation

1. Inspect the actual CLI repository at a recorded revision. Identify toolchain/edition/MSRV, runtime conventions, remote-scope PKCE and device-grant support, credential storage, user-service installation, update behavior, endpoint injection, and redaction entry points.
2. Record concrete symbols and paths plus gaps; describe small credential, signing/key storage, launch-preparation, service, and redaction integration interfaces. Provider secrets stay on the host.
3. Choose how this repository's remote crates are consumed by the CLI, keeping shared crates at one compatible revision and avoiding a dependency on the CLI executable crate.
4. Document Linux/macOS support and actual Ed25519 protection. Distinguish non-exportable signing from exportable keyring storage and the protected-file fallback.

## Acceptance criteria

- [ ] The integration document cites an inspected CLI revision and actual APIs; guesses are explicitly marked.
- [ ] Toolchain and package-integration choices are evidence-based and sufficient for task 003.
- [ ] Each missing facility has a concrete follow-up within the relevant task, without duplicating existing QGenie facilities.
- [ ] If source or tenant access is unavailable, the missing access and affected tasks are recorded; this task remains blocked instead of claiming a completed inspection.

## Verification

- Review the recorded APIs against the inspected source and run its existing lightweight auth/service/launch examples where available. Never copy credentials into evidence.

Run the relevant formatting, lint, dependency-boundary, and supported-platform checks once code exists. Record real results and any unavailable environment; do not report planned or mocked external validation as completed.

## Boundaries and handoff

No new OAuth stack, service installer, or CLI executable. Access to the existing CLI repository is an external prerequisite; this checkout cannot supply it.

**Deliver:** A versioned integration map and toolchain decision consumed by 003, 009, 011, 013, 020, and 025.

## Evidence and blocker (2026-10-16)

The tracked file inventory and workspace were inspected. No existing QGenie CLI source, toolchain manifest, or source location is available. The requested input is a readable repository URL or workspace path, with its intended revision.

[The integration inventory](../docs/integration/qgenie-cli.md) records each required facility and its dependent task. [ADR 0001](../docs/adr/0001-cli-integration.md) records package direction and the blocked toolchain/API decisions. No actual CLI symbols or native key-protection claims have been invented.

`rustc --version` and `cargo --version` report `1.99.1` in the prepared development environment. These are tooling observations, not the product MSRV. No CLI authentication, service, launch, or redaction example could run without that source.
