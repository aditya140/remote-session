# Fix GitHub Actions

Status: In progress.

## Goal and sequence

Make the existing Rust, platform, deployment, and specification checks pass on hosted
runners. Preserve behavioral assertions and report each platform independently.

1. Read the failing hosted jobs and identify the first failing command in each job.
2. Replace deprecated macOS clock bindings without changing suspension or failure semantics.
3. Make private test directories independent of the runner umask. Reproduce the recording
   acceptance failure under the hosted runner permissions and rerun the composed proof.
4. Update action runtimes, pin runner images, set timeouts and permissions, and stop one
   platform failure from cancelling another platform. Add a stable aggregate CI result.
5. Validate locally, push the fixes, and inspect the new hosted runs. Resolve additional
   failures until the checks pass. Record the run links and remaining qualification limits.

## Initial evidence

Run [41214716384](https://github.com/aditya140/remote-session/actions/runs/41214716384)
failed on `c14917a`. The macOS Clippy job rejected deprecated `libc` Mach time bindings.
The PostgreSQL job failed in the recording fixture while opening a private identity.
The Linux matrix job was cancelled by fail-fast. The deployment job passed.
The separate [specification run](https://github.com/aditya140/remote-session/actions/runs/41214716393)
also passed. Actions job reads work through the connector even though commit-status
reads and the CLI API route deny access.
