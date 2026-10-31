# 032 — Review Herdr Core for useful product ideas

Status: complete for source review. Requested on 2026-10-31.

## Goal

Identify Herdr Core functions that improve the selected QGenie Remote journey.
Keep Linux, one owner, shell sessions, and pinned Codex as the first delivery scope.
Separate existing behavior, proposed changes, and later features.

## Ordered steps

1. Find the exact upstream repository. Clone it outside this repository. Record its
   revision and license. Distinguish Herdr Core from the older Herdr application.
2. Read the commands, API, implementation, and relevant tests. Check the behavior
   behind each useful feature. State whether a finding has runtime evidence.
3. Compare each feature with QGenie Remote. Identify existing equivalents and gaps.
   Preserve authority, recording, process ownership, and uncertain-action contracts.
4. Rank useful changes by user value, implementation cost, and release scope.
   Document the implementation order and acceptance conditions before product edits.
5. Save a source-linked review. Verify its references and report remaining unknowns.

## Scope

This task clones and reviews upstream source. It does not install the upstream
runtime, launch agents, copy implementation code, or expand the product automatically.
Product recommendations require a separate implementation step.

## Source

- Repository: <https://github.com/herdrdev/herdr-core>.
- Local checkout: `/workspace/herdr-core`.
- Reviewed revision: `1d09234dac41a729d07212f70f8ef73c1e107f0f`.
- Declared package version: `0.1.15`.
- License: Apache-2.0. Retain applicable attribution and notices if code is reused later.

## Result and verification

All five review steps are complete. The
[comparison](../docs/validation/herdr-core-review-2026-10-31.md) records useful functions,
existing equivalents, constraints, source references, and a proposed implementation order.
The main recommendations are readable output, bounded event/wait commands, activity
summaries, automatic observation recovery, and small launch profiles.

Both OpenRPC documents parse and contain 32 unique methods. The reviewed feature methods
are present. Six local references and 19 pinned upstream file references were checked.
The reference checkout is unchanged. Documentation whitespace checks pass.

Herdr Core was not built or run. Zig 0.16 was not available on PATH, and its Ghostty
submodule was not initialized. Upstream tests were inspected, not executed. No QGenie
product code changed. Task 030's real deployment, pilot, and host budget gates remain open.
