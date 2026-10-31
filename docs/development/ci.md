# Continuous integration

Both workflows run for pushes, pull requests, and manual dispatches. A newer run
cancels an older run for the same branch or pull request. Each job has a timeout.
Actions use exact commit pins. Dependabot checks action updates each week.
Workflow tokens have read-only repository access.

## Required results and changed paths

Require `CI` and `laws` in branch protection. This document does not configure
repository rules. Both results remain present for documentation-only changes.

The change classifier permits a fast path only when every changed path is a
Markdown file under `docs/` or `tasks/`, or one of `README.md`, `AGENTS.md`,
`PRD.md`, `architecture.md`, `components.md`, and `subcrate-standard.md`.
Specification documents, generated reports, scripts, unknown paths, and code
changes require the full suite. Manual runs, empty diffs, new branches, and
unavailable comparison commits also require the full suite. Rename detection is
disabled when collecting paths, so moving a code file into documentation still
requires the full suite.

For documentation-only changes, CI tests the routing rules and reports why the
runtime jobs were skipped. The aggregate gate accepts these skips only when the
classifier succeeded and selected that path. For other changes, every selected
job must succeed. A failed, cancelled, missing, or unexpectedly skipped job fails
the gate. The specification workflow applies the same classification.

## Full coverage

| Job | Checks |
| --- | --- |
| `check` / workspace | Ubuntu 24.04 and macOS 15. Format, Python tools, workspace Clippy and tests, schemas, crate boundaries, and CLI/TUI/history. Linux also checks foreign-user IPC and full recording storage. |
| `check` / features | Both platforms. Supported minimal and expanded features, transport tests, adapter lifecycle and contracts, owner tests, and managed-Codex Clippy. |
| `postgres` / gateway | PostgreSQL/Redis integration, directly and through transaction pooling. |
| `postgres` / enrollment | Enrollment, native connector recovery and revocation, roots, credentials, and host tests. |
| `recording` | Synchronous recording, primary-volume loss, managed-start recovery, and the pinned native Codex adapter. |
| `development-deployment` | Development image build, startup, monitoring, native smoke, capacity baseline, restore, and fault recovery. |
| `laws` | Specification proofs, mutation checks, and Rust conformance. |

The platform suites and integration groups run independently. A matrix failure
does not cancel another platform or suite. Workspace tests use `--no-fail-fast`.
The adapter lifecycle step keeps its four-minute timeout. The actual 65-second
idle-proxy heartbeat proof and all native/recording assertions remain unchanged.
Clippy covers workspace type checking. The repeated all-feature adapter unit run
was removed; the bounded adapter step runs the full adapter suite once.

## Caches

Development and test builds omit debug symbols to reduce compilation, linking,
and cache size. Debug assertions and the production release profile remain as
configured in `Cargo.toml`. Rust jobs retain dependency caches after failure.
Platform suites and integration groups use separate keys to avoid overwriting different feature artifacts.
Specification comparisons use an explicit target directory for their temporary
Cargo project, so the cache retains its dependency artifacts.

The deployment uses Buildx with GitHub cache exports for both images. Pinned
cargo-chef `0.1.78` separates dependency compilation from application sources.
The dependency layer includes real vendored crates, registry data, and compiler
artifacts. It does not rely on mutable cache mounts surviving a runner. The build
context includes only build inputs. One shared build stage compiles both binaries
with the same package set as the dependency layer. Each runtime image receives
only its own binary. Runtime images retain the pinned base images, UID/GID 10001,
CA trust, and separate gateway/fixture-identity contents.

The first run populates new caches. Later runs can reuse them. Cargo.lock,
toolchain, vendored source, or dependency recipe changes require rebuilding the
relevant layers. Cache eviction and runner queue time can still affect duration.
See [task 036](../../tasks/036-optimize-ci-runtime.md) for measurements.

## Diagnose a failure

1. Open the failed run in the repository Actions tab.
2. Check the change-classification summary, then the first failed step in each job.
3. Run that command locally with the pinned toolchain and required services.
4. For private-directory fixtures, use `umask 0022` to match hosted Linux defaults.
   Create private state with explicit `0700` permissions. Keep product checks intact.
5. Push the correction and check both workflows for that exact commit. Use manual
   dispatch when a documentation-only change must receive full runtime verification.

The recording job uploads its sanitized acceptance report when available. The
specification job uploads its successful verification report after a full run.
Test credentials are synthetic. The workflows do not require production secrets.
Hosted macOS tests do not qualify native user services or credential-store access.
See [task 035](../../tasks/035-fix-github-actions.md) for the prior CI repairs.
