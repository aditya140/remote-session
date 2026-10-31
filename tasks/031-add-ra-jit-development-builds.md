# 031 — Add optional ra-jit development builds

Status: complete for the optional Linux development workflow. Requested on 2026-10-26.

## Goal

Reduce repeated development build time with ra-jit. Measure the result on this repository.
Keep Rust 1.99.1 as the compiler for normal Cargo commands, CI, and release qualification.

## Ordered steps

1. Verify the upstream project, available binaries, compiler requirement, and limitations.
2. Add a small repository-local installer and launcher. Pin the upstream artifact hashes.
   Give the tool its required compiler, separate build output, and a private daemon directory.
3. Install the tool. Verify its version and compiler. Exercise real workspace targets.
4. Compare repeated function edits against Cargo. Check output and daemon status so a Cargo
   fallback cannot be reported as a JIT speedup. Stop the test daemon after measurement.
5. Document setup, build, test, status, stop, fallback behavior, and measured limitations.
   Verify that normal Cargo still uses the repository's pinned compiler.

## Source decisions before implementation

The X post is inaccessible through this environment's network policy. The upstream project
is [indexable-inc/blaze](https://github.com/indexable-inc/blaze). It was named ra-jit until
2026-10-24. Current source uses nightly Rust, but the published binary is an older nightly
artifact from revision `8b15e4b406f010f98eb456ec867e16e0a9ebb0d5`. That binary requires
Rust 1.99.0. Read the installation contract at that revision rather than applying newer
Blaze instructions to an older binary. Pin each archive's SHA-256 from GitHub release
metadata. Refuse a changed archive instead of silently updating the tool.

This release has no borrow checker. Keep ordinary `cargo check`, Clippy, tests, and release
builds authoritative. Do not enable a global Cargo wrapper. Do not add a live-reload
dependency to the host, gateway, or session owner. The first build and unsupported changes
can use Cargo. The goal is faster repeated edits, not a promised cold-build speedup.

## Implementation and evidence

All five steps are complete. [The launcher](../tools/ra-jit) installs the pinned binaries
and compiler. It scopes compiler selection, output, and daemon state to this workflow.
Development/test debug information defaults to zero. The repository toolchain remains
Rust 1.99.1. CI checks the launcher with Python tests and continues to use ordinary Cargo
for product qualification. The development container now explicitly installs Python.

The actual CLI compiled through the launcher. A private source copy then exercised three
CLI function edits through Cargo and three through ra-jit. Every build produced the exact
expected changed diagnostic and version. All three JIT edits were served by the daemon.
The original source remained unchanged. The [sanitized measurement](../docs/validation/ra-jit-builds-2026-10-26.json)
retains every sample, compiler version, flags, initial load times, and limitations.

Cargo edit times were 1.34, 1.41, and 1.62 seconds. JIT edit times were 9.10, 0.43, and
0.33 seconds. The first JIT edit and daemon load are substantial costs. Do not summarize
these results as a cold-build improvement. Both paths used Rust 1.99.0 and the same
development profile. This result applies to repeated small CLI edits on one Linux host.

Validation:

- Three installer/launcher tests pass. They cover checksum rejection, regular archive
  members, idempotent installation, isolated compiler/output selection, program argument
  forwarding, and refusal of release/shared-output flags.
- The policy test command succeeds through the launcher. Its Cargo/JIT fallbacks remain
  visible. A successful test command does not imply that every function was JIT compiled.
- `cargo fmt --all -- --check` and ordinary `cargo check --workspace --all-targets --locked`
  pass. `rustc --version` still reports Rust 1.99.1 outside the launcher.
- Both owned compiler daemons were stopped. The published Linux x86-64 binary requires
  glibc 2.34, which is compatible with the development container's Debian Bookworm base.
- The development container image and GitHub workflow were not rebuilt or executed here.
  Linux ARM64 and Apple silicon archives are pinned but remain unexecuted.

Use [the development guide](../docs/development/ra-jit.md) for commands, measured behavior,
and the required normal-Cargo validation. No product crate dependency or live-reload loop
was added. The overall product completion gates in task 030 remain open.
