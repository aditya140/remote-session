# Product specification definitions

Status: Executable specification models with checked safety laws. See
[model boundaries](models.md) and [verification instructions](VERIFY.md).

## Purpose

Define the behavior that QGenie Remote must preserve as its implementation changes.
A specification states permitted behavior, forbidden behavior, and behavior under failure.
It identifies the assumptions under which each statement applies.

The user must be able to work on a remote machine without losing session control,
duplicating uncertain operations, or confusing access loss with session termination.
These requirements cross process and storage boundaries. A function signature alone
does not describe them. Define small models of product decisions and state transitions.
Connect those models through explicit authority and acknowledgment contracts.

Start with the [contract catalog](catalog.md). The first detailed definition is
[action authorization](authorization.md). Fourteen model directories now formalize
selected contracts. They add no product runtime dependency or new product capability.

## Relationship to existing documents

| Source | Responsibility |
| --- | --- |
| [PRD](../PRD.md) | Defines user tasks, requirements, and release priorities. |
| [Architecture](../architecture.md) and [ADRs](../docs/adr/README.md) | Define selected authority, process, storage, and recovery decisions. |
| [Operation matrix](../docs/contracts/operation-matrix.md) | Defines operation permissions and acknowledgment boundaries. |
| [Failure walkthroughs](../docs/contracts/failure-walkthroughs.md) | Supply failure sequences for model review and conformance tests. |
| This folder | Gives each selected contract precise inputs, states, rules, assumptions, and proof obligations. |
| [MVP evidence](../docs/validation/mvp-evidence.md) | Records implementation evidence. It does not replace a requirement. |
| [External release gates](../docs/validation/external-release-gates.md) | Records qualification that remains outside the Linux development scope. |

Link each law to its requirement and selected design decision. Do not copy the entire
PRD into a model. If sources disagree, record the disagreement before defining the
affected law. An implementation observation does not silently become a product requirement.

The initial implementation boundary is the standalone Rust TUI, PTY sessions, and
the supported managed Codex subset. Keep later clients, adapters, external adoption,
and platforms separate from claims about that boundary. Existing-session policy can
be defined for external sessions without claiming that adoption is implemented.

## Kinds of specifications

| Kind | Question it answers | QGenie Remote example |
| --- | --- | --- |
| Decision contract | For these facts, which result must this action produce? | An eligible owner can answer an approval without a control lease. |
| State transition contract | Which state and effects can follow this event? | Detach removes an attachment without issuing session termination. |
| Safety invariant | What must hold after every permitted finite sequence of events? | A request has at most one committed approval winner. |
| Progress contract | Under which assumptions must pending work eventually advance? | A committed decision is delivered when access remains valid and delivery keeps making progress. |
| Recovery contract | What survives failure, and what can recovery safely do? | A start with a lost reply is reconciled through its original scoped key. |
| Observation contract | What can a response or UI state claim from the available evidence? | A committed approval winner does not prove native application. |
| Resource contract | What happens at a declared capacity limit? | A full input queue rejects work before acceptance; a full recording spool exposes loss. |
| Boundary contract | Which identities, data, and operations can cross a boundary? | A public remote request cannot invoke local sessiond shutdown. |

A domain can need several kinds. A safety invariant alone is insufficient: a product
that denies every request can satisfy many negative rules. Define successful cases
and progress obligations as well as denials.

For progress claims, define fairness: which continuously available work must eventually
run, and which retried messages must eventually arrive. Permanent message loss can be
permitted by a safety model while preventing progress. State that distinction explicitly.

Performance, usability, deployment configuration, and platform behavior still need
measurements or native tests. A model can define a deadline and its clock assumptions.
It cannot establish that a production deployment meets that deadline.

## Required contents of a domain specification

1. State the user task, scope, requirement IDs, and specification status.
2. Define actors, identities, inputs, outputs, and observable effects.
3. Name the authority for each state. Separate durable state from disposable projections.
4. Define initial states, permitted events, transition guards, and commit boundaries.
5. State safety laws and successful cases with stable law IDs.
6. State progress assumptions, including delivery, scheduling, access, and storage availability.
7. Define failures, recovery, uncertainty, capacity limits, and acknowledgment meanings.
8. Identify exclusions, dependencies, and unresolved choices.
9. Define the Rust-to-model mapping and the evidence needed to check it.

For a pure decision, state that it owns no mutable state. Do not invent transitions.
For a stateful model, define an allowed initial state and a transition function.
An invariant must hold initially and remain true after every permitted transition.
A useful trace is a sequence of modeled events, including failures and recovery.
Trace examples illustrate laws; they do not prove laws for every sequence.

Use notation such as `decide(action, facts)` or `step(state, event)` in definitions.
This notation is explanatory. It is not executable Bend syntax.

## Shared modeling rules

- Separate permission, durable acceptance, dispatch, native execution, and acknowledgment.
- Separate actual native state from the system's evidence about that state.
- Preserve the public outcomes `not_sent`, `unknown`, and `rejected`.
- Treat a missing response as missing evidence. It does not prove that no effect occurred.
- Model relevant message loss, duplication, delay, reordering, crashes, and stale generations.
- State which durable records survive each modeled failure. A database restore is not an ordinary restart.
- Treat Redis presence and routing as projections. They cannot replace durable authorization.
- Express time through an explicit clock and deadline model. State suspension and clock-failure behavior.
- Define bounds and overflow behavior. Unbounded mathematical integers do not prove safe machine-counter behavior.
- Assume neither eventual recovery from a permanent outage nor successful native execution without adapter evidence.

Give each composed model an explicit input contract. For example, authorization consumes
a `controller` fact; the control model must justify that fact from activated generations
and deadlines. A policy proof does not prove the fact collector correct.

## Bend representation and evidence

Each modeled domain contains `Model.bend`, `LAWS.bend`, and `PROOF.bend`.
[toolchain.json](toolchain.json) pins Bend, Bun, and Lean.
[check.py](check.py) checks all declared proof entry points and optional regression
mutations. It can also compare authorization with Rust. CI integration is not enabled.
Use [models.md](models.md) to identify each theorem's scope and remaining obligations.

Review requirement and law changes explicitly. Do not weaken a law solely to make a
failing implementation or proof pass. Exclude unchecked assumptions, unsafe definitions,
and foreign code from claims that require a closed proof.

Record these evidence states separately:

| Evidence state | Required record |
| --- | --- |
| Defined | The written contract identifies its assumptions and open choices. |
| Modeled | Executable types and transitions represent that contract. |
| Checked | Named laws pass the recorded Bend checker version. |
| Kernel checked | The same laws pass `--verdict`; record the kernel and toolchain. |
| Implementation compared | Rust decisions or traces match the model under a stated mapping and coverage. |
| Environment qualified | Native and deployment tests establish the assumptions for the named environment. |

A checked Bend model does not prove the Rust program correct. Exhaustive comparison
can cover a finite pure input domain. Stateful comparisons cover the exercised traces
unless a separate refinement proof establishes more. A refinement proof would show
that each relevant implementation behavior corresponds to permitted model behavior.

Advance a domain's evidence state only with reproducible artifacts, explicit coverage,
and recorded results. Checked laws can cover only part of a catalog definition.
