# Standalone MVP acceptance evidence

Updated 2026-10-26. Development across tasks 001–028 is complete for the user-selected
Linux standalone scope. This report describes executed proofs and names the external
release gates. It is not enterprise release approval. The user waived corporate CLI
inspection and selected this scope in [ADR 0007](../adr/0007-standalone-preview.md).

[Task 030](../../tasks/030-complete-linux-release-readiness.md) continues toward overall
completion. Its [latest composed result](linux-completion-2026-10-26.json) passes all
functional phases. The host resource budget remains unmet. Real enterprise access,
Linux user services, and the developer pilot remain unrun.

## Reproduce the composed proof

Use Linux, Docker on `/var/run/docker.sock`, the pinned Rust toolchain, PostgreSQL 17 and
Redis 8 images, and an actual Codex **0.159.2** executable. No enterprise credentials are
needed for this disposable fixture; the model response endpoint and identity issuer are
controlled fixtures, while native execution, TLS, database storage and routing are real.

```sh
export QGR_CODEX_BINARY=/absolute/path/to/codex
python3 tests/e2e/recording_durability.py --require-managed-codex --report target/validation/synchronous-report.json
```

The runner creates an isolated PostgreSQL synchronous pair and Redis, uses verified HTTPS/WSS
through two gateways, and removes only its generated resources. The final database proof
destroys the owned primary's tmpfs data volume and promotes its independent standby.
Completion markers from both Rust proofs are mandatory. The [executed Linux result](linux-completion-2026-10-26.json)
records the latest composed pass and release-host measurement. Remove `--require-managed-codex`
and omit `QGR_CODEX_BINARY` to run only the terminal/recording subset. That subset explicitly
leaves managed Codex unrun in the JSON report.
`--probe-only` qualifies only a database probe. CI preserves the sanitized report on success
and failure; it includes no DSNs, tokens, provider keys, private paths or exception messages.

When `QGR_CODEX_BINARY` is set, the runner also requires all three managed process proofs:
all six start boundaries, owner SIGKILL and caller-loss/recovery. Their sanitized phase is
`managed_start_boundaries`. The same proofs and the adapter suite can run separately:

```sh
QGR_CODEX_BINARY=/absolute/path/to/codex cargo test -p qgenie-remote-sessiond --all-features --lib actual_managed -- --ignored
QGR_CODEX_BINARY=/absolute/path/to/codex cargo test -p qgenie-remote-adapters --all-features --test codex_native -- --ignored
```

Use [remote commands](../integration/remote-commands.md) for interactive operation and
[development deployment](../runbooks/development-deployment.md) for the four-gateway stack.
The [latest development rollout](development-roll-2026-10-18.json) records four healthy
gateways recreated from the rebuilt image while a real native PTY survived. The same PTY
also executed local input during Redis and primary outages and survived its actual hosting
gateway's SIGKILL, with original-key recovery and retained redacted history. Rebuild and
verify the selected image before qualifying another changed revision. The committed capacity artifact is a reduced PTY baseline, not managed
agent or beta-scale evidence.

## Requirements and executed coverage

| PRD requirement | Executed evidence | Qualification still required |
| --- | --- | --- |
| ID-1–3, ID-5, ID-8 | Verified TLS OAuth/JWKS fixtures, PKCE/device flows, dedicated user/machine audiences, machine challenge/refresh, lost enrollment reply and explicit key recovery. | Real corporate Entra tenant and deployment configuration. |
| ID-4 | Actual unavailable-keyring refusal, explicit private-file fallback, private identity permissions and no persisted user refresh token. | Native OS credential-store execution on supported hosts; software Ed25519 is exportable, not a hardware non-exportability claim. |
| ID-6, PO-5 | Primary-authoritative revocation, suppressed-hint gateway checks, signed admin/native denial within ten seconds and conservative elapsed pending/completed reporting; actual local PTYs stay alive. | Real Entra disablement budget, real suspend and deployed enforcement measurements. |
| ID-7 | Actual CLI/TUI rename, label/remove, filter and confirmed revoke; mutation loss stays uncertain. | Supported-platform release execution. |
| PR-1–2, PR-4 | Host inventory/version, presence, descriptor-scoped directory browsing, replacement and current-generation routing. Verified Codex discovery uses the actual pinned executable. | Representative deployed timing and supported-platform qualification. |
| PR-3 | Pushed presence/count/session metadata over bounded WSS, scoped native subscriptions and exact output heads; the actual resize proof receives a head without a replay request. | Complete extended network/process/platform qualification; content uses bounded continuation reads and idle reconciliation. |
| SE-1, SE-6, SE-10 | Actual public PTY start with persisted identity, same-key lookup, changed-key conflict, concurrent starts, caller/connector loss and independent native capture. | Remaining platform/service and extended network-fault cases. |
| SE-2, SE-2a, SE-5 | Actual pinned local Unix Codex IPC; relay guard; private injected provider; public agent creation, prompt/steer/interrupt, tool approval and new-cwd historical resume. | Corporate provider integration and supported-platform runtime qualification. |
| SE-7 | Complete PTY and typed agent checkpoints, bounded pages, replay pruning/restart, causal deduplication, idle final-tail recovery and explicit uncertainty. SDK observer cancellation/slow-consumer TLS proofs. | Complete active-stream process/network-fault matrix and supported-platform qualification. |
| SE-8 | Two-gateway take/takeover/renew/release/input/resize/interrupt; original-generation fencing at actual native effects and real observing keyboard flow. | Full gateway failover/crash and suspend/resume matrix. |
| SE-9 | Non-controller SDK winner race and TUI choice; atomic audit/outbox; independent pre-delivery and post-native-application ACK drops; native journal deduplication; offline durable status. | Remaining full-process crash boundaries and supported-platform execution. |
| SE-11 | Distinct detach/interrupt/confirmed termination/read-only original-key recovery/new historical start. Confirmed native exit/provenance required; lost/unknown history cannot resume. Actual owner SIGKILL and six managed start crash boundaries report lost/unknown without relaunch. | Remaining active-stream/gateway crash boundaries; runtime IPC reconnect remains unsupported. |
| CL-1 | Standalone ratatui SDK workflows, advertised agent picker, structured pane, owner pending-approval badge/inbox, machine/version inventory, history, explicit reconnection and terminal restoration on quit/SIGTERM. | Corporate CLI integration is waived; macOS and full release fault qualification remain. |
| PO-1, PO-3 | Independent host/native descriptor policy and original authority; actual existing PTY capture/control/interrupt/input/termination outside allowed start roots while browsing stays denied; atomic audited starts, approvals and protected admin reads; no controller lease for owner approvals. | Full deployed policy/revocation and operational audit qualification. |
| Recording release gate, PO-4/6/7/8 | Real no-echo/redaction, bounded spool/ENOSPC, exact lost-ACK receipts, owner/admin/RLS isolation, audited reads, hold-retention races, actual jobs and synchronous primary-volume-loss preservation. | Deployed scheduling, alert delivery, independent fault-domain backups/restore and production durability boundary. |

## Regression entry points

- `cargo test --workspace --all-features` covers the default component suite. Ignored tests
  require explicit external fixture runners; an ignored result is not a passing proof.
- `tests/e2e/recording_durability.py` composes
  `apps/qgenie-remote/tests/support/recording.rs` and its `support/managed.rs` fixture with the gateway
  synchronous durability test. Keyboard scripts cover PTY, machine metadata, approvals and
  advertised agent creation/recovery/resume.
- `tests/e2e/pgbouncer_recordings.py` exercises actual transaction pooling, winner/outbox
  atomicity, expiry during primary lock waits, transcript isolation, holds and retention.
- `tests/e2e/foreign_uid_ipc.sh` and `recording_disk_full.sh` prove foreign-UID IPC denial
  and isolated real ENOSPC behavior.
- The development deployment runner exercises rolling gateway restarts, Redis/Postgres
  outages and isolated backup/restore. See the separate [capacity report](capacity.md).

## Development checks and external qualification

The [Linux check artifact](linux-development-checks-2026-10-18.json) records 178 passing
tests and 21 explicitly ignored environment proofs. Separate composed runners establish
native, synchronous durability and deployment coverage; ignored tests are not counted as
passed. The [revocation result](revocation-propagation-2026-10-18.json) records bounded denial
and conservative completed reporting. The synchronous report also records real traffic and
five native approval delivery attempts (aggregate 0.03573 seconds, including fault-injected
attempts). That histogram measures submission through received/persisted result or error,
not commit-to-application or a release latency distribution.

The [development alert artifact](operational-alerts-2026-10-18.json) records 13 evaluated
rules, actual private/native metrics and a stopped-probe alarm. Real tenant disablement,
user-manager/macOS execution, real suspend and extended fault/soak qualification, production
fault domains, off-server backups, notification delivery, probe supervision, operator
assignments and full beta load remain unrun or unconfigured. The detailed
[external release gates](external-release-gates.md) retain required evidence and task owners.
Runtime native IPC reconnect remains explicitly unsupported; uncertainty is visible and
does not relaunch native work. This declared capability limit does not claim reconnection
support or leave a hidden retry path.
