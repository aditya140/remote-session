# AUTH: Action authorization definition

Status: Defined, modeled, and checked for the scope in [models.md](models.md).
See [authorization/LAWS.bend](authorization/LAWS.bend) for the named theorems.

## User task and scope

An owner must be able to observe and operate an eligible session. Approval rights
must remain independent of input control. Allowed roots must constrain new work
without preventing supported control of an existing session.

Sources: PRD PR-4, SE-8, SE-9, SE-11, SE-12a, SE-12b, and PO-1; the
[operation matrix](../docs/contracts/operation-matrix.md); and
[ADR 0002](../docs/adr/0002-state-authority.md).

Define `decide(action, facts) -> Allow | Deny(code)` for the nine actions represented
by [qgenie-remote-policy](../crates/qgenie-remote-policy/src/lib.rs).
This is a pure decision contract. It owns no mutable state and performs no I/O.
An allowance passes this policy stage. It does not itself activate control, launch
a process, choose an approval winner, or establish native execution.

## Inputs and their meaning

All facts below are Boolean. Trusted callers must establish them for the authenticated
actor, target, and requested action. Unverified or missing evidence cannot establish
a permissive fact. Validate credentials and resource provenance before this decision.

| Fact | Meaning |
| --- | --- |
| `owner` | The authenticated remote actor owns the target resource. |
| `access_fresh` | Current authoritative evidence permits this access at the decision boundary. |
| `revoked` | An applicable global, user, or machine revocation blocks access. |
| `same_os_user` | The session or launch belongs to the enrolled host OS user. |
| `controller` | This connection has the activated, current, unexpired control authority required for a controlled mutation. |
| `within_roots` | The relevant new-session or browsing path satisfies the host's allowed-root checks. |
| `capability` | The adapter or operation provider supports the requested action. |
| `host_allows` | Current host policy permits the requested action. |
| `external` | The target is an externally created session. |
| `termination_confirmed` | The caller explicitly confirmed termination of this target. |
| `external_termination_confirmed` | The confirmation explicitly covers termination of an external session. |

The ACCESS and CONTROL models must justify freshness and controller facts.
Host path validation must justify `within_roots`, including filesystem races.
The policy model does not prove those mechanisms or cryptographic identity checks.

## Permission definition

Define the common eligibility predicate:

```text
eligible(f) = f.owner AND f.access_fresh AND NOT f.revoked
              AND f.same_os_user AND f.capability AND f.host_allows
```

Every action requires common eligibility. It also requires the action condition below.

| Action | Additional condition | Meaning of the allowance |
| --- | --- | --- |
| `Observe` | None. | The actor can pass the observation policy check. |
| `Start` | `within_roots`. | The actor can pass the new-session policy check. |
| `Browse` | `within_roots`. | The actor can pass the directory-browsing policy check. |
| `Control` | None. | The actor can request control. Separate activation and takeover rules still apply. |
| `Input` | `controller`. | The actor can pass the input policy check. |
| `Resize` | `controller`. | The actor can pass the resize policy check. |
| `Interrupt` | `controller`. | The actor can pass the turn-interruption policy check. |
| `Approve` | None. | The actor can pass approval eligibility. Pending-request and winner checks still apply. |
| `Terminate` | `termination_confirmed AND (NOT external OR external_termination_confirmed)`. | The actor can pass explicit termination policy without a control lease. |

`AUTH-001`: For every action and every fact assignment, return `Allow` if and only if
common eligibility and the action condition both hold. This defines positive behavior
as well as denial. A policy that always denies violates this law.

## Required laws

| Law ID | Required property |
| --- | --- |
| `AUTH-002` | A non-owner, a revoked actor, or a foreign OS-user target is denied for every action. |
| `AUTH-003` | Stale access, denied host policy, or missing capability cannot produce an allowance. |
| `AUTH-004` | For `Approve`, changing only `controller` or `within_roots` leaves the decision unchanged. An eligible owner can approve with both false. |
| `AUTH-005` | `Start` and `Browse` require `within_roots`. Control ownership cannot bypass that requirement. |
| `AUTH-006` | For existing-session actions, changing only `within_roots` leaves the decision unchanged. Other eligibility conditions still apply. |
| `AUTH-007` | `Input`, `Resize`, and `Interrupt` require active control. Ownership alone is insufficient. |
| `AUTH-008` | `Control` permits a request without an existing lease. It does not grant takeover or native mutation authority. |
| `AUTH-009` | Termination requires explicit confirmation. An external target additionally requires external-session confirmation. A lease cannot replace either fact. |
| `AUTH-010` | For `Terminate`, changing only `controller` leaves the decision unchanged. |
| `AUTH-011` | Changing confirmation or external-session facts leaves non-termination decisions unchanged. |
| `AUTH-012` | A denial follows the stable reason order below. Repeated evaluation of identical inputs produces the same result. |

Existing-session actions in `AUTH-006` are `Observe`, `Control`, `Input`, `Resize`,
`Interrupt`, `Approve`, and `Terminate`.

## Denial results and precedence

The operation matrix requires stable public reasons. The current pure policy function
uses this detailed order. Preserve it in the first model-to-Rust comparison. This is
the policy-stage order, not the complete network authentication and validation pipeline.

| First failing condition | Code |
| --- | --- |
| The actor is not the owner, is revoked, or is not the same OS user. | `Forbidden` |
| Access evidence is stale. | `AccessStale` |
| Host policy denies the action. | `Forbidden` |
| The requested capability is unavailable. | `UnsupportedCapability` |
| A start or browse path is outside allowed roots. | `Forbidden` |
| Input, resize, or interrupt lacks activated control. | `StaleGeneration` |
| Termination lacks the required confirmation. | `Forbidden` |

Every policy denial has public outcome `rejected` and `retryable = false`, as defined
by [RemoteError](../crates/qgenie-remote-proto/src/model.rs). Error-message wording is
outside this initial model. The decision produces no native side effect.

## Successful and failing examples

Assume all common eligibility facts hold unless a row states otherwise.

| Case | Required result |
| --- | --- |
| Approve with no control and an outside-root working directory. | Allow at the policy stage. |
| Request control with no current lease and an outside-root working directory. | Allow the request at the policy stage. |
| Send input outside the roots with active control. | Allow at the policy stage. |
| Start outside the roots while holding control. | Deny with `Forbidden`. |
| Send input without active control. | Deny with `StaleGeneration`. |
| Terminate an external session with only ordinary termination confirmation. | Deny with `Forbidden`. |
| Terminate an external session with both confirmations and no control lease. | Allow at the policy stage. |
| Approve with revoked and stale access. | Deny with `Forbidden`; revocation takes precedence. |

## Evidence plan and exclusions

The input domain contains `9 * 2^11 = 18,432` combinations. Compare all combinations
against the Rust policy function. Compare allowance, denial code, outcome, and retryability.
Separately prove the named laws over the model. Include mutations that require control
for approvals, apply roots to existing-session control, or omit termination confirmation.
The unchanged laws must reject those mutations.

This definition does not cover enrollment, history administration, file transfer,
approval winner selection, lease activation, or termination execution. Define those
contracts at their owning boundaries. There is no progress or crash-recovery obligation
inside this pure function; the composed operation owns those obligations.

No conflict was identified between these policy-stage permissions and the cited
operation matrix. The detailed denial order is an observed Rust compatibility baseline.
The model-to-Rust input mapping is in [conformance/policy.mjs](conformance/policy.mjs).
The exhaustive comparison covers `AUTH-001`; the Bend laws cover the explicitly named
subproperties. Do not count every written law as a separate kernel-checked theorem.
