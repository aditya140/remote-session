# Reduce CI duration

Status: In progress.

## Goal and baseline

Reduce feedback time without removing platform, feature, native-process,
recording, deployment, or specification coverage. Keep the required `CI` and
`laws` results reliable when expensive jobs intentionally do not run.

The last successful run on `99574ee` is the baseline:
[41229220164](https://github.com/aditya140/remote-session/actions/runs/41229220164).
Deployment takes 925 seconds, including 714 seconds of uncached image builds.
Database/native integration takes 737 seconds. Linux takes 676 seconds, macOS
343 seconds, and the separate specification workflow takes 165 seconds.

## Implementation sequence

1. Add a conservative documentation-only classifier. Unknown paths, unavailable
   base commits, and manual runs require the full suite. Handle deleted and renamed
   files. Keep workflow results present and reject unexpected skipped jobs.
2. Split workspace checks from feature checks on both platforms. Split database,
   enrollment, and native recording work into independent jobs. Preserve each
   existing test command or its equivalent coverage. Remove duplicate compilation
   and the duplicate adapter test invocation.
3. Persist Docker build layers in the GitHub cache. Separate dependency compilation
   from application sources with pinned cargo-chef. Include the real vendored
   dependencies. Keep the production gateway image separate from fixture identity.
4. Cache specification Rust dependencies. Keep toolchain, action and image pins,
   read-only permissions, timeouts, original test deadlines, and cleanup behavior.
5. Validate routing and gate decisions, lint the workflows, and check cached image
   builds. Run a full hosted build to populate caches, a second full run to measure
   reuse, and a documentation-only push to verify the fast path. Record measured
   outcomes and remaining limits.

## Local verification

All seven Python tooling tests pass. The CI tests cover real Git push and pull
request diffs, multiple commits, code-to-document renames, document deletion,
unavailable bases, manual/empty-diff full runs, and failed/cancelled/skipped gate
results. Actionlint passes. Docker Buildx checks the revised image definition
without warnings. Full image compilation and runtime validation follow in CI.

The specification harness creates a temporary Cargo project. Its target directory
must be set explicitly to `target/spec-conformance`; caching the root workspace's
default target alone does not preserve those compiler outputs. The workflow sets
that directory and caches it under its own key.

Development and test artifacts currently include full debug information. The next
full run will disable those symbols in CI to reduce compilation, linking, and
cache size. Keep debug assertions enabled and leave the production release
profile unchanged. Validate the changed CI profile with the full suite, then measure a run
that reuses its caches. Keep cold-cache and warm-cache evidence separate.

Use one shared application-build stage for the gateway and fixture binaries. Cook
and build the same package set so Cargo can reuse the dependency feature set.
Compile shared application crates once. Continue to copy only the selected binary
into each separate runtime image. Validate both images with the deployment suite.

The first hosted run passed all Rust, native, and specification suites. The
gateway image compiled and exported its cache. The fixture image failed because
the new build context omitted its two compile-time synthetic identity inputs.
Restore those exact files in the context and source layer, then rerun the full
suite. The aggregate gate correctly failed. Do not count that run as successful
deployment evidence.

The superseded warm-up run also exposed a cache-policy issue. With
`cache-on-failure: true`, cancelled compiles saved incomplete dependency caches.
GitHub cache entries are immutable, so later exact hits cannot fill those entries.
Save caches only after successful jobs. Use fresh version-three keys for the final
profile, then let the full run finish before measuring cache reuse.

The local Docker context export passes. It contains the fixture source and both
synthetic identity inputs. It excludes Git metadata, target outputs, documentation,
unrelated test fixtures, and Compose files. Parent-directory exceptions were
removed because Docker also applies those exceptions to their children.

## Hosted validation

The full suite passes on `ba89a66`: [Rust checks and deployment](https://github.com/aditya140/remote-session/actions/runs/41232902764)
and [specification laws](https://github.com/aditya140/remote-session/actions/runs/41232902753).
Docker restores the chef and dependency layers, then compiles both application
binaries in 370 seconds. The fixture image reuses that build. All startup,
monitoring, native smoke, baseline, restore, and fault checks pass.

The [measurement snapshot](../docs/validation/ci-runtime-2026-10-31.json) records
baseline and warm-up job and step times. The next full run measures reuse of the
completed caches. Its only changes are this evidence and the measurement snapshot.
The JSON report deliberately selects the full suite. A later Markdown-only push
will verify the documentation path and record the final comparison here.
