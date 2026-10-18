# 003 — Bootstrap the Rust workspace and repository checks

- **Status:** Complete for the selected Linux standalone development scope; external release qualification remains separately tracked
- **Milestone:** M0 foundation
- **Workstream:** Build tooling
- **Depends on:** [001](001-inspect-existing-qgenie-integration.md)
- **Index:** [Task plan](README.md)

**Scope:** The user selected Linux development completion in [ADR 0007](../docs/adr/0007-standalone-preview.md). See [current progress](PROGRESS.md) and [external release gates](../docs/validation/external-release-gates.md). Original criteria that require unrun environments or waived corporate inspection remain unchecked; this status does not claim full release qualification.

## Goal and repository context

There is currently no Cargo manifest, lockfile, toolchain, or CI. Create a usable build foundation while preserving the proposed dependency direction.

**Read first:** [components.md](../components.md) §§2, 5; [subcrate-standard.md](../subcrate-standard.md) §§2–3, 6–7.

## Owned scope

- `Cargo.toml, Cargo.lock, rust-toolchain.toml, rustfmt.toml, .gitignore`
- `xtask/ and .github/workflows/ or the repository's selected CI configuration`
- `Root README.md build instructions`

Owned paths define this task’s implementation boundary; current source and executed evidence are recorded below. Follow [the subcrate standard](../subcrate-standard.md), add only useful modules, and coordinate shared manifests/exports and migrations as described in the index.

## Implementation

1. Use task 001's toolchain decision; centralize edition, MSRV, resolver, versions, lints, and profiles. Commit the lockfile.
2. Implement a useful xtask dependency-direction check against the nine-crate graph, with explicit failures for upward edges and gateway dependencies on host, adapters, keyring, or TUI code.
3. Add formatting, lint, applicable tests, and supported-feature checks that grow with implemented workspace members.
4. Document developer commands and how to add a real crate with a README and focused public API. Create product crates when behavior lands, not as empty future skeletons.

## Acceptance criteria

- [x] A fresh checkout can execute the documented build/check commands with the pinned toolchain.
- [x] The dependency checker accepts the planned graph and rejects a representative forbidden edge.
- [x] CI discovers new implemented members and fails on formatting, lint, or dependency-boundary violations.
- [x] No empty nine-crate scaffold, invented team ownership, or secrets are committed.

## Verification

- Run cargo fmt --all -- --check, cargo clippy --workspace --all-targets -- -D warnings, cargo test --workspace, and the implemented xtask dependency check.

Run the relevant formatting, lint, dependency-boundary, and supported-platform checks once code exists. Record real results and any unavailable environment; do not report planned or mocked external validation as completed.

## Boundaries and handoff

Protocol implementation belongs to 004; runtime/deployment checks arrive with their owners.

**Deliver:** Workspace conventions and working CI used by every implementation task. Coordinate root-manifest and lockfile changes when merging parallel work.

## Blocker (2026-10-16)

**Resolved by user direction:** [ADR 0007](../docs/adr/0007-standalone-preview.md) authorizes the standalone composition and Rust 1.99.1. The source blocker below is historical. The workspace, dependency check, schema tool, and CI now exist. Verification is in progress.

Task 001 has no access to the actual QGenie CLI source. Its toolchain, edition, MSRV, runtime conventions, and package compatibility remain unverified. No Cargo workspace or substitute CLI has been created. Resume this task after the inspection and integration decision in 001. [Progress](PROGRESS.md) preserves the full implementation goal.

## Completion evidence

ADR 0007 replaces the original CLI-source prerequisite for this standalone workspace. Rust 1.99.1, edition 2024, resolver 3, shared dependency versions, lints, profiles, and the lockfile are present. Seven product crates contain working behavior; the app composes them. Each crate has a README.

The workspace formatting, check, strict Clippy, and test commands passed on Linux. `cargo xtask boundaries` passed. Its test rejects gateway-to-adapter and gateway-to-keyring edges. CI discovers workspace members and runs the same commands, schema checks, supported feature checks, and the real TUI smoke test on Linux/macOS. Remote CI and macOS execution remain unobserved; their configuration is not presented as a completed platform proof.

`cargo xtask schema --check` passed. No-default-feature builds for proto, transport, and adapters passed. The real-process and keyboard harnesses are documented in the root README.
