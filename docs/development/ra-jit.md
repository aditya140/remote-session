# Faster development builds with ra-jit

Use the repository launcher for an optional JIT development loop:

```sh
./tools/ra-jit setup
./tools/ra-jit build -p qgenie-remote --locked
./tools/ra-jit run -p qgenie-remote --locked -- --help
./tools/ra-jit test -p qgenie-remote-policy --locked
./tools/ra-jit status
./tools/ra-jit stop
```

Run the same build command after each edit. The daemon loads the workspace after the
first Cargo build. Wait for `status` to report `ready`. A build served by the JIT prints
`[ra-jit]`. A Cargo fallback is explicit and appears in the status counters. The first
build, manifest changes, and unsupported changes can use Cargo.

## Installation and isolation

Install Python 3.11 or newer, rustup, and the normal native build dependencies. The
development container supplies Python. Setup downloads the pinned archive, verifies its
SHA-256, and installs its required Rust 1.99.0 toolchain with `rust-src`. It writes binaries
under `target/tools/ra-jit`. It does not change the repository toolchain or global Cargo
configuration. Normal Cargo commands continue to use Rust 1.99.1.

The launcher writes experimental artifacts to `target/ra-jit-build`. It creates a private
daemon directory under the temporary directory, scoped by the user and repository path.
`stop` affects this repository's daemon. The daemon exits after ten idle minutes by default.
Stop it before removing its installed binaries. Cargo and release artifacts remain separate.

Development and test debug information defaults to zero in this launcher. Set
`CARGO_PROFILE_DEV_DEBUG=1` or `CARGO_PROFILE_TEST_DEBUG=1` for that command to keep line
information. Changing the profile setting requires another initial build and daemon load.
Build release artifacts with the normal command:

```sh
cargo build --release --locked -p qgenie-remote --features managed-codex
```

The launcher accepts development build, run, test, check, status, and stop commands. It
rejects release/custom profiles and overrides of its target directory or manifest path.
Cargo arguments and program arguments after `--` are forwarded directly without a shell.

## Compiler limits

The pinned ra-jit release has no borrow checker. Use the normal Rust 1.99.1 checks as the
authority for changes:

```sh
cargo check --workspace --all-targets --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

The JIT is a development tool. Its output does not qualify process ownership, authority,
recording durability, or release performance. No live-reload dependency is added to product
crates. The compiler daemon uses substantial memory; the observed CLI worker held about
4 GiB after loading this workspace. Its memory is separate from the deployed host budget.

## Measured result — 2026-10-26

The [measurement](../validation/ra-jit-builds-2026-10-26.json) uses a private copy of this
working tree. Each edit changes one diagnostic string in the real CLI's async `main`.
Each build runs `qgenie-remote --version` and verifies the exact new diagnostic and version.
The original application source stays unchanged.

Both paths use Rust 1.99.0, incremental compilation, no debug information, and
`build -p qgenie-remote --locked` on the same Linux host. Their edit times are:

| Edit | Cargo | ra-jit | JIT result |
| --- | ---: | ---: | --- |
| First | 1.34 s | 9.10 s | Correct changed output; initial edit work is expensive |
| Second | 1.41 s | 0.43 s | Correct changed output; served by the daemon |
| Third | 1.62 s | 0.33 s | Correct changed output; served by the daemon |

All three JIT edits were served by the daemon. The medians are 1.41 s and 0.43 s, but
the first JIT edit is slower and the daemon also needs about 34 seconds to load. This is
evidence for repeated small edits. It is not a cold-build speedup or a promise about every
crate, edit, platform, or feature set. The initial build times are not comparable: the
second path reused dependencies already built by Cargo.

To reproduce this method, copy the current source to a disposable checkout. Use Rust
1.99.0 and a separate target directory for both paths. Set `CARGO_PROFILE_DEV_DEBUG=0`
and `CARGO_INCREMENTAL=1`. Build the CLI, then change a distinct `eprintln!` diagnostic
at the start of async `main` three times. Time each build and verify `--version` after
each edit. Repeat with ra-jit after its status reports `ready`. Keep the first edit and
all fallbacks in the result. Stop the daemon and remove the disposable checkout.

## Upstream version

The project is now [Blaze](https://github.com/indexable-inc/blaze). It was renamed from
ra-jit on 2026-10-24. Current source uses a newer nightly compiler, but the available
binary release still uses Rust 1.99.0. This integration follows the
[published release contract](https://github.com/indexable-inc/blaze/blob/8b15e4b406f010f98eb456ec867e16e0a9ebb0d5/docs/cargo.md).
[tools/ra-jit.lock.json](../../tools/ra-jit.lock.json) records the release revision and
archive hashes. The `nightly` download URL can change. Setup rejects changed bytes until
the pin is deliberately reviewed and updated.

Linux x86-64 installation, compilation, and edits were exercised here. Linux ARM64 and
Apple silicon archives are pinned but were not executed. The X post supplied with the
request was blocked by the environment network policy; upstream documentation supplied
the installation and compatibility evidence.
