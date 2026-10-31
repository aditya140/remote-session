# Bend branch integration

## Goal and sequence

Merge the specification suite into `main` with repeatable checks and explicit limits.
Keep the product runtime and its dependencies unchanged.

1. Compare `bend` with `main` and review the models against the product contracts.
2. Run the pinned kernel checks, regression mutations, and Rust policy comparison.
3. Add missing control-admission laws and irreversible access-loss trace laws.
4. Make each new guarantee reject a targeted regression. Check positive behavior too.
5. Run the suite in CI. Pin tool versions and verify the Lean archive checksum.
6. Record fresh evidence, check the final diff, and merge only after checks pass.

## Review findings

- The branch adds only `specs/`. It has no content conflict with `main`.
- Only authorization has an automated comparison with Rust. The other models define
  contracts; their proofs do not establish implementation correctness.
- Control admission checks lease expiry and three identity fields. The original laws
  do not independently protect those checks against removal.
- Access laws preserve the original deadline. They do not yet establish that arbitrary
  retry and refresh sequences preserve revocation and clock failure.
- The suite has no CI job. A checked report can become stale after later source changes.

These findings describe the branch before the changes below.

The first integration covered `bend` only. The user then selected integration of `work`
as well. [Task 034](../tasks/034-integrate-work-and-bend.md) records the combined merge
and its additional client laws and SDK comparisons.

## First Bend integration results

The original suite passed 90 kernel-checked laws, 14 regression mutations, and all
18,432 Rust authorization comparisons. The review then tested additional regressions
against the original laws. Six changes still passed: ignoring lease expiry, preventing
valid activation, restoring revocation after token refresh, restoring a failed clock
after a Redis hint, ignoring gateway capability support, and accepting a non-durable
recording receipt. These were gaps in the specifications, not observed Rust defects.

The integration adds eleven laws. Five cover control admission and successful activation.
Four preserve revoked and clock-invalid grants across single steps and arbitrary finite
traces. Two cover gateway capability support and recording receipt durability.
The mutation runner now checks twenty-three regressions with unchanged laws and proofs.
The identity comparison mutations can also invalidate the structure of another proof;
the new inequality laws state the required denials explicitly.

Local verification on Linux:

- All 101 laws passed the pinned Bend checker and Lean-built kernel.
- All 23 model regressions failed law checking.
- All 18,432 authorization inputs matched Rust, including error outcome and retryability.
- All 19 Rust policy and protocol tests passed, including protocol conformance.
- Rust formatting, targeted check, and strict Clippy checks passed.
- Actionlint 1.7.7 accepted the new workflow. `git diff --check` passed.

The current [verification report](verification.json) records exact source hashes and tool
versions for the latest combined integration. The counts above describe the first Bend review.
The [workflow](../.github/workflows/specs.yml) repeats the proof, mutation, and Rust
comparison checks. Hosted execution and required branch protection were not established
by these local checks. The GitHub integration denied access to commit status data.

The first Bend integration changed no product runtime code. That step did not rerun the
full workspace, database, native process, or deployment suites. Their existing
evidence and [external release gates](../docs/validation/external-release-gates.md) remain
separate. At this first review, only authorization had an automated Rust comparison.
The combined integration adds the selected SDK comparisons described in [models.md](models.md).
