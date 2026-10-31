# Continuous integration

Both workflows run for pushes, pull requests, and manual dispatches. A newer run
cancels an older run for the same branch or pull request. Each job has a timeout.
Actions use exact commit pins. Dependabot checks action updates each week.
Workflow tokens have read-only repository access.
Rust jobs retain dependency caches after a failed test, so the next correction can
reuse compiled dependencies.

## Required results

The `Rust checks` workflow has three groups:

| Job | Checks |
| --- | --- |
| `check` | Ubuntu 24.04 and macOS 15. Format, Python tools, workspace builds, Clippy, tests, supported features, schemas, crate boundaries, and native CLI fixtures. Linux also checks foreign-user IPC and full recording storage. |
| `postgres` | PostgreSQL and Redis integration, native enrollment, synchronous recording, primary-volume loss, pinned Codex, and transaction pooling. |
| `development-deployment` | Development image build, startup, monitoring, smoke tests, capacity baseline, restore, and fault recovery. |

The `CI` job requires all three groups to succeed. A failed, skipped, or cancelled
dependency fails this result. A matrix failure does not cancel the other platform.
Workspace tests use `--no-fail-fast`, so a failed crate does not hide later crate
results. The adapter lifecycle step has a separate four-minute timeout.
The separate `Specification laws` workflow checks proofs, mutations, and Rust
conformance. Both workflows run without path filters, so documentation changes
also receive final results.

For branch protection, require `CI` and `laws`. This document does not configure
repository rules. The workflow keeps each detailed job visible for diagnosis.
Hosted macOS tests do not qualify launchd or native credential-store integration.
See [task 035](../../tasks/035-fix-github-actions.md) for the repairs and hosted evidence.

## Diagnose a failure

1. Open the failed run in the repository Actions tab.
2. Open the first failed step in each job. The `CI` job only reports job outcomes.
3. Run that command locally with the pinned toolchain and required services.
4. For private-directory fixtures, use `umask 0022` to match hosted Linux defaults.
   Create private state with explicit `0700` permissions. Do not weaken product checks.
5. Push the correction and check both workflows for that exact commit.

The PostgreSQL job uploads a sanitized recording acceptance report, including on
failure when the runner produced it. The specification job uploads its successful
verification report. Test credentials are synthetic and apply only to disposable
services. The workflows do not require production credentials.
