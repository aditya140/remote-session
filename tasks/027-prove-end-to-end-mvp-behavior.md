# 027 — Prove the terminal and Codex MVP through real failure scenarios

- **Status:** Complete for the selected Linux standalone development scope; external release qualification remains separately tracked
- **Milestone:** M0 smoke evidence / M1 acceptance
- **Workstream:** End-to-end verification
- **Depends on:** [019](019-implement-revocation-and-entra-freshness.md), [022](022-implement-legal-holds-and-retention.md), [025](025-integrate-cli-and-host-services.md), [026](026-build-development-compose-stack.md)
- **Index:** [Task plan](README.md)

**Scope:** The user selected Linux development completion in [ADR 0007](../docs/adr/0007-standalone-preview.md). See [current progress](PROGRESS.md) and [external release gates](../docs/validation/external-release-gates.md). Original criteria that require unrun environments or waived corporate inspection remain unchecked; this status does not claim full release qualification.

## Goal and repository context

Demonstrate integrated behavior across actual process boundaries; passing separate crate tests is insufficient for the promised recovery and security guarantees.

**Read first:** [PRD.md](../PRD.md) §§8–9; [architecture.md](../architecture.md) §11; [subcrate-standard.md](../subcrate-standard.md) §7.

## Owned scope

- `tests/e2e/, conformance/scenarios/ executable runners`
- `docs/validation/mvp-evidence.md and CI failure-artifact integration`

Owned paths define this task’s implementation boundary; current source and executed evidence are recorded below. Follow [the subcrate standard](../subcrate-standard.md), add only useful modules, and coordinate shared manifests/exports and migrations as described in the index.

## Implementation

1. Build reusable multi-process fixtures and first exercise headless enrollment, machine listing, terminal start/input/detach/reattach through the gateway and corporate proxy. Add the pinned Codex prompt/tool/approval/resume path.
2. Run concurrent starts with dropped acknowledgments, controller takeover with delayed stale input, competing owner approvals, and committed-decision delivery/application failures.
3. Kill gateway/connector processes, drop client connectivity, fail Redis, and validate replay including idle tail loss. Exercise service isolation, pause/resume, and local session survival.
4. Suppress revocation broadcasts, disable a user through the validated Entra integration, and measure denial budgets with valid existing credentials. Verify cross-user and same-user policy boundaries.
5. Fail recording storage, fill the spool, recover it, test RLS/admin audit and hold-retention races, and verify TUI gaps/warnings without session interruption.

## Acceptance criteria

- [x] The M0 terminal demo and M1 Codex flow run from documented commands with explicit environment and pinned version evidence.
- [x] No silent duplicate starts, stale-controller mutation, unauthorized approval/read, hidden recording loss, or false successful native application occurs.
- [ ] Gateway revocation meets 10 seconds and Entra disablement 15 minutes; local sessions survive both. Failure/stale-authority cases are included.
- [x] Results map to P0 requirements and the recording release gate from 002. Unrun native/tenant/macOS/proxy scenarios remain visible blockers, not passing mocks.

## Verification

- Run the real-process suite on the dev stack, including Linux and macOS service checks in suitable environments. Store sanitized logs, timing measurements, versions, and reproducible failure artifacts.

Run the relevant formatting, lint, dependency-boundary, and supported-platform checks once code exists. Record real results and any unavailable environment; do not report planned or mocked external validation as completed.

## Boundaries and handoff

Begin writing/running the M0 smoke subset as its components land; all listed prerequisites gate final M1 completion. This task does not replace focused tests owned by earlier tasks or claim GA load readiness.

**Deliver:** A reproducible acceptance report and regression suite, with any remaining release blockers explicitly enumerated.

## Current acceptance report

[MVP evidence](../docs/validation/mvp-evidence.md) maps executed fixtures to P0 requirements
and the protected recording release gate. The synchronous runner composes actual Linux
PTYs, pinned Codex 0.159.2, verified HTTPS/WSS, two gateways, PostgreSQL remote-apply and
Redis. Its optional JSON report distinguishes terminal-only runs, managed Codex, database
probes and unrun external gates; CI preserves this sanitized result on success/failure.

Actual SDK/TUI creation, structured observation, controlled prompts, owner-only approval
races and in-session observe-mode approval choices pass. Independent loss before delivery
and after native application causes durable retry without a second command effect.
Historical resume uses the saved thread and a new directory/process/session; lost/unknown
recovery never relaunches. The Linux owner-SIGKILL fixture observes native exit through a
pidfd. Native startup-error cleanup waits for actual reap.

The full workspace/all-features suite passes 178 tests with 21 explicitly ignored fixture
tests. The acceptance runner also requires all six managed start crash boundaries, owner SIGKILL
and caller-loss recovery when the pinned native binary is supplied. Scoped native session
changes and exact pushed heads pass through the actual two-gateway fixture. The
active-session development proofs now include local execution during Redis/primary outages,
SIGKILL of the PTY's actual hosting gateway, four rolling recreations, original-key recovery,
new controlled input and verified retained history. Real Entra disablement, service-manager/macOS,
extended network loss/suspend and production durability/operations remain external release gates.
The selected Linux development scope is complete.

The latest sanitized [Linux checks](../docs/validation/linux-development-checks-2026-10-18.json)
and [alert validation](../docs/validation/operational-alerts-2026-10-18.json) distinguish
workspace components, explicit native proofs and development operations from release gates.
