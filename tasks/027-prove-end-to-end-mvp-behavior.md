# 027 — Prove the terminal and Codex MVP through real failure scenarios

- **Status:** Not started
- **Milestone:** M0 smoke evidence / M1 acceptance
- **Workstream:** End-to-end verification
- **Depends on:** [019](019-implement-revocation-and-entra-freshness.md), [022](022-implement-legal-holds-and-retention.md), [025](025-integrate-cli-and-host-services.md), [026](026-build-development-compose-stack.md)
- **Index:** [Task plan](README.md)

## Goal and repository context

Demonstrate integrated behavior across actual process boundaries; passing separate crate tests is insufficient for the promised recovery and security guarantees.

**Read first:** [PRD.md](../PRD.md) §§8–9; [architecture.md](../architecture.md) §11; [subcrate-standard.md](../subcrate-standard.md) §7.

## Owned scope

- `tests/e2e/, conformance/scenarios/ executable runners`
- `docs/validation/mvp-evidence.md and CI failure-artifact integration`

Paths describe proposed implementation locations; they do not exist yet unless another task creates them. Follow [the subcrate standard](../subcrate-standard.md), add only useful modules, and coordinate shared manifests/exports and migrations as described in the index.

## Implementation

1. Build reusable multi-process fixtures and first exercise headless enrollment, machine listing, terminal start/input/detach/reattach through the gateway and corporate proxy. Add the pinned Codex prompt/tool/approval/resume path.
2. Run concurrent starts with dropped acknowledgments, controller takeover with delayed stale input, competing owner approvals, and committed-decision delivery/application failures.
3. Kill gateway/connector processes, drop client connectivity, fail Redis, and validate replay including idle tail loss. Exercise service isolation, pause/resume, and local session survival.
4. Suppress revocation broadcasts, disable a user through the validated Entra integration, and measure denial budgets with valid existing credentials. Verify cross-user and same-user policy boundaries.
5. Fail recording storage, fill the spool, recover it, test RLS/admin audit and hold-retention races, and verify TUI gaps/warnings without session interruption.

## Acceptance criteria

- [ ] The M0 terminal demo and M1 Codex flow run from documented commands with explicit environment and pinned version evidence.
- [ ] No silent duplicate starts, stale-controller mutation, unauthorized approval/read, hidden recording loss, or false successful native application occurs.
- [ ] Gateway revocation meets 10 seconds and Entra disablement 15 minutes; local sessions survive both. Failure/stale-authority cases are included.
- [ ] Results map to P0 requirements and the recording release gate from 002. Unrun native/tenant/macOS/proxy scenarios remain visible blockers, not passing mocks.

## Verification

- Run the real-process suite on the dev stack, including Linux and macOS service checks in suitable environments. Store sanitized logs, timing measurements, versions, and reproducible failure artifacts.

Run the relevant formatting, lint, dependency-boundary, and supported-platform checks once code exists. Record real results and any unavailable environment; do not report planned or mocked external validation as completed.

## Boundaries and handoff

Begin writing/running the M0 smoke subset as its components land; all listed prerequisites gate final M1 completion. This task does not replace focused tests owned by earlier tasks or claim GA load readiness.

**Deliver:** A reproducible acceptance report and regression suite, with any remaining release blockers explicitly enumerated.
