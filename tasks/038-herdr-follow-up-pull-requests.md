# Implement the Herdr follow-up issues as separate pull requests

Status: In progress. Authorized on 2026-10-31.

## Goal

Deliver one reviewable pull request for each of GitHub issues 1 through 8.
Work in issue order. Use a stacked branch when a change depends on an earlier
pull request. Keep each diff limited to its issue. Do not merge the pull requests
as part of this task.

## Sequence

1. Preserve the native prompt turn ID through a correlated receipt. Add exact
   send-and-wait behavior, capability checks, and failure/recovery tests.
2. Add local launch profiles and durable, scoped session names and labels. Reuse
   existing launch validation and stable session identities.
3. Add a bounded work inbox across machines. Keep task outcome, process lifetime,
   approvals, freshness, and incomplete coverage distinct.
4. Add fresh-output and explicit any/all target waits. Reuse observation and
   exact task receipts. Do not introduce implicit task creation or cancellation.
5. Generate command documentation and check released interface compatibility.
   Retain current schema/conformance checks and measure added CI work.
6. Audit the requested reliability scenarios. Add focused tests for gaps and fix
   defects demonstrated by those tests. Preserve existing bounded queues and
   authority fences.
7. Complete the deferred agent-coordination research as a design PR. Record
   evidence, missing pilot evidence, lifecycle decisions, and implementation gates.
8. Complete the deferred native-owner replacement research as a design PR. Record
   the handoff state, failure cases, old/new binary proof, and a go/no-go decision.

## Verification and delivery

- Read the issue and affected contracts before each change.
- Record the issue-specific design and acceptance checks before implementation.
- Run focused behavior tests, formatting, and relevant compilation/lint checks.
- Run broader checks when an interface or dependency change requires them.
- Generate affected schemas and document unsupported or unknown outcomes.
- Commit, push, and create each pull request with its issue and test evidence.
- State the base PR for dependent changes. Keep research gates explicit.
- Check GitHub CI and repair failures before declaring delivery complete.

The local development stack and its native demo session belong to the previous
deployment proof. Preserve them. Use isolated test state and disposable services
for implementation checks.

## Issue 1 design

- Add a distinct prompt-with-receipt input and an optional turn ID in its reply.
  Negotiate support at the client, gateway, connector, and native owner.
  Reject unsupported receipt requests before the native mutation.
- Preserve the ID from the native `turn/start` response. Do not infer the ID
  from observed activity. Store it with the existing input acknowledgment in
  one SQLite statement. Duplicate input returns the same receipt.
- Keep the existing prompt API for older peers. The new CLI prompt command
  requests a receipt. Add `--wait` and a bounded timeout. Release control before
  the existing read-only exact-turn wait. Print the receipt before waiting.
- Keep uncertain input uncertain. Do not resubmit it or guess its turn ID.
- Test receipt persistence, duplicate input, old peer refusal, exact correlation,
  and fast completion. Generate the wire schema and run applicable checks.

The SQLite migration adds a nullable receipt column. The previous native owner
rejects the new schema version. Downgrade requires a pre-migration state backup.
The network protocol remains additive for clients that use the old prompt API.

### Issue 1 implementation and evidence

The receipt now passes through the native owner, SQLite input journal, connector,
gateway, SDK, and CLI. The CLI releases control before the exact-turn observer.
An interrupted wait emits `cancelled` and leaves native work running.

Local Linux checks completed:

- Formatting, the public schema check, and the all-target workspace check passed.
- Clippy passed for the default workspace and the managed Codex application.
- The workspace run passed 222 tests and skipped 19 environment-gated tests. The
  new conformance fixture initially omitted the protocol `body` wrapper. After
  correction, all 11 tests in that suite passed. No runtime test failed.
- The managed session owner suite passed 71 tests. Eight gated tests were skipped.
- The explicitly selected pinned Codex owner test passed. It checks the exact
  completed turn ID, duplicate receipts, and a single native application.
- The full CLI suite passed ten tests. It checks fast completion, concurrent
  activity, observation recovery, old peers, lost or missing receipts, and SIGTERM.
- The isolated composed proof passed all five phases. They cover the synchronous
  database pair, the public session pipeline, managed Codex, native start failure
  boundaries, and loss of the primary database volume. The public pipeline includes
  the new CLI send-and-wait flow through the real connector and gateways.

The composed report is local at `target/validation/issue-1-native.json`. Native
platform evidence is Linux only. Hosted CI remains a separate delivery check.
