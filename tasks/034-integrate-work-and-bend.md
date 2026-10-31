# Integrate the implementation and Bend laws

Status: Complete (integration and local validation).

## Goal

Publish one `main` branch that contains the completed `work` implementation and the
reviewed `bend` specification suite. Preserve both branch histories.

## Sequence

1. Merge `work` into the current `main` on an integration branch. Check the combined diff.
2. Extend the client model for exact-turn completion evidence, connection recovery
   eligibility, and caller-owned credential validity. Keep their assumptions explicit.
3. Compare the model with the actual SDK functions. Cover bounded event sequences,
   every public error code, and every credential-validity fact combination.
4. Add mutations that break each new contract. Require the unchanged laws to reject them.
5. Run the combined workspace checks and supported managed Codex checks. Refresh the
   proof report against the combined implementation. Check schemas and CI configuration.
6. Record results and remaining deployment limits. Fast-forward `main` to the checked
   integration and push it. Verify that both source branch heads are ancestors.

## Scope

The completion model selects the newest matching terminal evidence. The implementation
comparison uses complete retained event histories for running sessions. Extra comparisons
preserve known terminal evidence across later process loss and history uncertainty.
Other wait results, keyboard behavior, backoff timing, and credential refresh exchanges
retain their existing Rust and integration tests. These are not full refinement proofs.

No new runtime feature or dependency is planned.

## Implementation

The integration starts from `main` at `8ab046c` and merges `work` at `84e5d62`.
It retains `bend` at `e16944c` and both implementation commits from `work`.
The merge has no conflicts. Runtime sources, product manifests, and protocol fixtures
match `work` exactly. The README retains the workflow instructions and the Bend links.

Fourteen new client laws raise the suite to 115 laws. Nine new mutations raise the
regression checks to 32. The isolated harness invokes the actual SDK functions and
compares 4,274 inputs with the client model. These comprise 96 recovery decisions,
eight credential decisions, and 4,170 targeted waits. The comparison checks its error
code and operation-outcome coverage against the generated public schema.

The runner records both Rust comparisons and hashes the policy, protocol, and SDK
sources. Its temporary build disables debug information and incremental compilation.
The CI job runs both comparisons when product sources, schemas, or specifications change.

## Executed verification

Date: 2026-10-31. Platform: Linux x86-64. Rust: 1.99.1.
Counts overlap; do not add them as a unique test count.

| Check | Result |
| --- | --- |
| Pinned Bend checker and Lean-built kernel | 115 laws passed. |
| Model mutations | All 32 regressions rejected by law checking. |
| Rust authorization comparison | All 18,432 inputs matched, including public result fields. |
| Rust SDK comparison | All 4,274 inputs matched. |
| Comparison checker probes | Wrong results, missing rows, and duplicate rows were rejected. |
| Workspace tests | 213 passed; 18 ignored by their environment prerequisites. |
| Managed Codex sessiond library | 70 passed; 7 ignored. |
| Managed Codex application and workflow tests | 9 passed. Includes the actual keyboard recovery and terminal restoration test. |
| Codex adapter library | 16 passed. |
| Workspace and managed-feature Clippy | Passed with warnings denied. |
| Isolated conformance harness Clippy | Passed with warnings denied. |
| Workspace check and Rust formatting | Passed. |
| Dependency boundaries and all three schema checks | Passed. |
| Workspace Rust documentation | Passed with warnings denied. |
| Specification workflow | Actionlint 1.7.7 passed. |

The [proof report](../specs/verification.json) contains current hashes and tool versions.
Local command logs use the `target/validation/work-bend-` prefix.

## Remaining limits

The SDK comparison covers the stated finite inputs and traces. It is not a proof of
all SDK behavior or runtime composition. Other stateful models still lack automatic
Rust comparisons. See [model boundaries](../specs/models.md).

Hosted CI status remains unverified because GitHub status access was denied during
this integration task. The full database deployment, pinned native Codex process proof,
and real enterprise qualification were not rerun. Existing evidence and the
[external release gates](../docs/validation/external-release-gates.md) remain separate.
