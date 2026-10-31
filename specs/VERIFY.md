# Verify the Bend specifications

The exact versions are in [toolchain.json](toolchain.json). The checker requires a
clean Bend source checkout at that commit. It runs from an empty temporary directory
so Bun does not load the product's `.env` or `bunfig.toml`.

## Prepare the tools

Use Python 3.11 or later for the runner.

Install Bun 1.4.4. One option is `npm exec --package=bun@1.4.4 -- bun --version`.
Use the resulting Bun executable with `--bun`, or install that version on `PATH`.
Install Lean 4.34.0 and put its `lean` and `leanc` executables on `PATH`.
Use the [official Lean release](https://github.com/leanprover/lean4/releases/tag/v4.34.0).

The Linux archive `lean-4.34.0-linux.tar.zst` used for the recorded check has SHA-256:

```text
caaa98356098c85dc0fcbbd28e1ec66f39eb6551829972b752ff20e1286b646b
```

Clone the Bend source into a directory outside this repository:

```sh
git clone https://github.com/bendlang/bend.git /tmp/qgr-bend
git -C /tmp/qgr-bend checkout 9e8d7b7958ebb3bb72464dfb6b6b5f06949fbcad
```

The checker does not install or update tools. Bend builds its kernel from the pinned
source on the first kernel check and caches that executable. The wrapper ignores an
inherited `BENDTT` override. Keep the local tool and kernel cache trusted.

## Run the proof and mutation checks

Run this command from the product repository root:

```sh
python3 specs/check.py --bend-source /tmp/qgr-bend --mutations \
  --report /tmp/qgr-spec-results.json
```

The default mode requires both Bend's checker and its independent kernel to accept
every proof. Use `--mode standard` only for a standard-checker result when Lean is
unavailable. The report records that weaker mode explicitly.

The mutation checks make thirty-two temporary copies. Each copy introduces one regression
in a model and retains its original laws and proofs. A check passes only when a named
law rejects the change. Syntax errors and unavailable tools do not count as rejection.
The command does not edit the repository's models.

The control regressions cover activation, lease expiry, each identity comparison,
and successful confirmed activation. Additional regressions try to restore revoked
or clock-invalid grants, bypass gateway capability support, and reclaim a chunk
without a durable receipt. The laws must reject each change.
The client regressions also cover fatal connection failures, invalid credential facts,
failed or legacy turn outcomes, wrong turn targets, and ignored recent evidence.

## Compare authorization and SDK decisions with Rust

Use the repository's Rust toolchain and fetch its locked dependencies first:

```sh
cargo fetch --locked
python3 specs/check.py --bend-source /tmp/qgr-bend --mutations --compare-rust \
  --report /tmp/qgr-spec-results.json
```

In the managed development environment, source
`/workspace/.remote-session-tools/env.sh` before the Cargo command.

The runner creates an isolated temporary Cargo harness with path dependencies on the
current policy, protocol, and client crates. It starts with the product lockfile and runs
Cargo offline. The harness invokes the real policy function. The comparison also checks
the independent permission predicate, rejection outcomes, and retryability.
The SDK harness compares connection eligibility, credential validity, and bounded
terminal-evidence traces. See [model boundaries](models.md) for the exact 4,274 SDK cases
and excluded behavior. This is separate from the 18,432 authorization cases.
The report preserves `rust_comparison` for authorization and adds `client_comparison`.

Build products remain outside the repository by default. Set `CARGO_TARGET_DIR` to an
external cache directory to reuse compiled harness dependencies. The harness disables
debug information and incremental compilation to limit temporary disk use.

## Continuous verification

The [specification workflow](../.github/workflows/specs.yml) runs on relevant pushes,
pull requests, and manual dispatch. It reads the tool versions from `toolchain.json`.
It checks the Lean archive against `lean_linux_sha256` before extraction. It fetches
locked Cargo dependencies, then runs the kernel, mutations, and both Rust comparisons.
A successful job uploads a fresh `spec-verification` artifact for that commit.

The workflow does not use the committed report as a test result. Repository branch
protection is separate configuration; this workflow file does not make its job a
required merge check. Kernel checks remain separate from native and deployment tests.

## Interpret the result

The report records the toolchain, mode, source hashes, law counts, mutation outcomes,
and whether Rust comparison ran. A nonzero exit is a failed verification. The runner
writes a success report only after every requested check passes. Do not reuse an old
report as evidence for a failed or changed run; compare its source hashes.

Read [model boundaries](models.md) with the report. Kernel acceptance does not prove
the Bend parser or translation correct. It does not prove backend compilation or Rust
behavior. The executable models are a specification layer with explicit assumptions.
The [external release gates](../docs/validation/external-release-gates.md) still apply.
