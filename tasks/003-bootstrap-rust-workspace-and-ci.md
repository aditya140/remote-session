# 003 — Bootstrap the Rust workspace and repository checks

- **Status:** Not started
- **Milestone:** M0 foundation
- **Workstream:** Build tooling
- **Depends on:** [001](001-inspect-existing-qgenie-integration.md)
- **Index:** [Task plan](README.md)

## Goal and repository context

There is currently no Cargo manifest, lockfile, toolchain, or CI. Create a usable build foundation while preserving the proposed dependency direction.

**Read first:** [components.md](../components.md) §§2, 5; [subcrate-standard.md](../subcrate-standard.md) §§2–3, 6–7.

## Owned scope

- `Cargo.toml, Cargo.lock, rust-toolchain.toml, rustfmt.toml, .gitignore`
- `xtask/ and .github/workflows/ or the repository's selected CI configuration`
- `Root README.md build instructions`

Paths describe proposed implementation locations; they do not exist yet unless another task creates them. Follow [the subcrate standard](../subcrate-standard.md), add only useful modules, and coordinate shared manifests/exports and migrations as described in the index.

## Implementation

1. Use task 001's toolchain decision; centralize edition, MSRV, resolver, versions, lints, and profiles. Commit the lockfile.
2. Implement a useful xtask dependency-direction check against the nine-crate graph, with explicit failures for upward edges and gateway dependencies on host, adapters, keyring, or TUI code.
3. Add formatting, lint, applicable tests, and supported-feature checks that grow with implemented workspace members.
4. Document developer commands and how to add a real crate with a README and focused public API. Create product crates when behavior lands, not as empty future skeletons.

## Acceptance criteria

- [ ] A fresh checkout can execute the documented build/check commands with the pinned toolchain.
- [ ] The dependency checker accepts the planned graph and rejects a representative forbidden edge.
- [ ] CI discovers new implemented members and fails on formatting, lint, or dependency-boundary violations.
- [ ] No empty nine-crate scaffold, invented team ownership, or secrets are committed.

## Verification

- Run cargo fmt --all -- --check, cargo clippy --workspace --all-targets -- -D warnings, cargo test --workspace, and the implemented xtask dependency check.

Run the relevant formatting, lint, dependency-boundary, and supported-platform checks once code exists. Record real results and any unavailable environment; do not report planned or mocked external validation as completed.

## Boundaries and handoff

Protocol implementation belongs to 004; runtime/deployment checks arrive with their owners.

**Deliver:** Workspace conventions and working CI used by every implementation task. Coordinate root-manifest and lockfile changes when merging parallel work.
