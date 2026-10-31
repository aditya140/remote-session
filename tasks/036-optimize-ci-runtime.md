# Reduce CI duration

Status: Full suite verified. Documentation-only validation is in progress.

## Goal and implementation sequence

Reduce feedback time without removing platform, feature, native-process,
recording, deployment, or specification coverage. Keep the required `CI` and
`laws` results reliable when expensive jobs intentionally do not run.

The implementation followed this recorded plan:

1. Add a conservative documentation-only classifier. Unknown paths, unavailable
   base commits, and manual runs require the full suite. Handle deleted and renamed
   files. Keep required results present and reject unexpected skipped jobs.
2. Split workspace checks from feature checks on both platforms. Split database,
   enrollment, and native recording work into independent jobs. Preserve existing
   coverage. Remove duplicate type checking and the duplicate adapter unit run.
3. Persist Docker build layers in the GitHub cache. Separate dependencies from
   application sources with pinned cargo-chef. Include real vendored dependencies.
   Build both binaries once, then copy each into its separate runtime image.
4. Cache the temporary specification harness at an explicit target directory.
   Omit CI debug symbols. Keep debug assertions, the release profile, toolchain
   and action pins, permissions, test deadlines, and cleanup behavior unchanged.
5. Check routing and gate decisions. Lint workflows and check actual build inputs.
   Verify a full rebuild, a repeat full run, and a documentation-only push.
   Record measured results and limits.

## Verification

All seven Python tooling tests pass. They cover real push and pull request diffs,
multiple commits, code-to-document renames, document deletion, missing bases,
manual and empty-diff full runs, and failed/cancelled/skipped gate results.
Actionlint and Docker Buildx checks pass. A real Docker context export contains
both required synthetic identity inputs. It excludes Git metadata, target outputs,
documentation, unrelated fixtures, and Compose files.

All platform, feature, database, enrollment, recording, native Codex, deployment,
and specification checks pass on `874572e`:

- [Full Rust checks and deployment](https://github.com/aditya140/remote-session/actions/runs/41234044082).
- [Specification proofs, mutations, and Rust conformance](https://github.com/aditya140/remote-session/actions/runs/41234044072).

The preceding [full rebuild](https://github.com/aditya140/remote-session/actions/runs/41232902764)
also passes. It restores Docker dependency layers and recompiles both application
binaries in 370 seconds. Both images pass startup, monitoring, native smoke,
capacity baseline, restore, and fault checks. The separate
[specification run](https://github.com/aditya140/remote-session/actions/runs/41232902753)
passes with the explicit `target/spec-conformance` cache.

## Measured results

The baseline is the successful `99574ee` run. The repeat run uses the final
configuration with unchanged image inputs and populated caches. Every Rust cache
restores successfully. These are individual hosted observations, not guarantees.

| Measurement | Before | Repeat run |
| --- | ---: | ---: |
| Full Rust workflow, including queue and final gate | 15m34s | 5m51s |
| Deployment job | 15m25s | 3m33s |
| Both image build steps combined | 11m54s | 12s |
| Specification workflow | 2m47s | 2m02s |

The full repeat run reduces elapsed time by 62 percent. The Linux workspace job
now determines the total duration at 5m33s. Its feature suite runs separately and
passes in 3m22s. The actual 65-second idle-proxy heartbeat proof remains intact.

The final-profile warm-up takes 13m45s. It creates new Rust caches and rebuilds
application images. Its two image steps take 7m04s. This is separate from the
fully cached comparison. Source changes still require application compilation.
Dependency, toolchain, or vendor changes can also rebuild dependency layers.
Cache eviction and runner queues affect duration. Parallel jobs use more runners
at once; these measurements do not establish billed-minute savings.

The [snapshot](../docs/validation/ci-runtime-2026-10-31.json) records baseline and
warm-up job and step times. Saved Rust caches shrink from 753 to 538 MB for the
Linux workspace, 468 to 401 MB for the macOS workspace, 536 to 381 MB for
enrollment, and 644 to 460 MB for recording. These sizes use decimal MB.

## Corrections found during verification

The initial image context omitted two compile-time fixture identity files.
The fixture image and aggregate gate failed. The corrected context includes those
exact files. Parent-directory exceptions were also removed because Docker applies
those exceptions to their children. The context export verifies the final paths.
The initial failed run is not successful deployment evidence.

Cancelled warm-up jobs exposed incomplete immutable Cargo caches created by
`cache-on-failure: true`. Only successful jobs now save Rust caches. Fresh
version-three keys avoid the partial entries. The temporary Rust specification
project also needs an explicit target directory; the root workspace cache alone
cannot retain its compiler outputs.

## Documentation-only verification

Pending. The next push changes only this task, the progress record, and the CI
guide. It must skip runtime jobs, keep `CI` and `laws` present, and pass the gate.
