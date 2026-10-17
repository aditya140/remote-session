# PRD: QGenie Remote

| | |
| --- | --- |
| Status | Draft v0.8 |
| Date | 2026-10-15 |
| Owner | TBD |
| Related | [Architecture](architecture.md), Market research |

## 1. Summary

QGenie Remote lets a developer see all of their machines (laptops, lab servers, build hosts) in one place and work on any of them from the corporate network or through a corporate VPN. From the QGenie TUI, they can start or resume coding-agent sessions (Codex, Claude Code, Pi) and terminal sessions on those machines. Only the gateway must be reachable; users do not need SSH, direct access to machine addresses, or open inbound ports on hosts. **The first release is TUI-only.** Desktop app integration follows after the MVP.

Each machine runs `qgenie remote`, which is part of the QGenie CLI that is already distributed. It signs in with QGenie OAuth, registers the machine, and keeps one outbound WSS connection to the QGenie Remote Gateway. Clients connect to the same gateway. The gateway authenticates both sides, authorizes every action, and routes traffic between them.

## 2. Problem

- Developers run agents on several machines, such as a laptop plus one or more Linux servers. Today each machine needs SSH, tmux, and remembering which agent runs where.
- Many hosts sit behind NAT, VPN or firewalls. Inbound access is unreliable or forbidden.
- Long agent runs die or become invisible when the laptop sleeps or the SSH connection drops.
- Agents ask for approvals (run this command? edit this file?). Answering them means being at that terminal.
- No central view exists of which machines are online or what agents are doing, and no audit trail exists for remote execution.

## 3. Goals and non-goals

### Goals

1. **One machine list:** every machine a user has enrolled, with live presence, labels, OS, and available agents.
2. **Remote interaction:** open a full terminal, or a structured agent session, on any of the user's machines from the TUI; desktop app support follows after the MVP.
3. **Durable sessions:** sessions keep running when clients disconnect, and reattach with missed output replayed.
4. **Remote approvals:** see and answer agent approval requests from any connected client signed in as the session owner, subject to host policy, with one winning decision per request. Approval rights do not depend on holding the session control lease.
5. **Enterprise identity:** QGenie OAuth (Entra ID) for users, a separate identity for each machine, on-demand revocation by gateway admins, revocation within 15 minutes of Entra disablement, and full audit. Revocation blocks remote access while host sessions keep running.
6. **Zero inbound networking:** both hosts and clients use only outbound HTTPS/WSS on port 443, through corporate proxies.
7. **Reuse:** build on the existing Rust QGenie CLI's OAuth, keyring, service management, update channel, and endpoint injection.

### Non-goals (v1)

- Sharing machines with other users or teams (designed for, shipped in v2).
- Browser/web client (designed for, shipped later).
- Public-internet access to the gateway without the corporate network or a corporate VPN.
- End-to-end encryption that hides content from the gateway. QGenie is the trusted operator; see the architecture doc, decision D5.
- Remote desktop, GUI forwarding, general file sync, or port forwarding.
- Provisioning new machines or VMs (this is not Coder).
- Sandboxing agents. Sessions run with the OS user's permissions; the product enforces policy at the API level only.
- Discovering sessions owned by other OS users or root. Sessions started outside QGenie Remote are supported only when they run as the same OS user as the host daemon (SE-12).
- Plain shells in arbitrary terminal windows that aren't in tmux or Herdr (another program's PTY can't be attached to).

## 4. Users

| Persona | Needs |
| --- | --- |
| Developer with several machines (primary) | Start an agent on a beefy lab server from a laptop; check progress from the desktop app; approve a command; reattach later. |
| On-the-go developer | Close the laptop lid. The remote agent keeps running. Open the desktop app elsewhere and keep going. |
| Security / IT admin | Know who executed what, where; revoke a machine or user instantly; enforce org policy (allowed agents, allowed roots, retention). |
| Platform admin | Read any transcript, with a stated reason, for investigations; place and release legal holds on specific individuals. |
| Platform operator | Run the gateway with high availability, observe it, roll out upgrades without breaking older CLIs. |

## 5. User experience

### 5.1 Enabling a host

```text
$ qgenie remote up
Signing in to QGenie Remote… (browser opened; or code ABCD-EFGH at https://microsoft.com/devicelogin)
✓ Signed in as aparkhi@qualcomm.com
Machine name [hu-aparkhi-lv]:
Allowed working roots [~/, /local/mnt/workspace/aparkhi]:
✓ Enrolled machine hu-aparkhi-lv (m_01J9…)
✓ Installed user service (systemd --user: qgenie-remote.service)
✓ Connected to qgenie-remote.qualcomm.com
Detected agents: codex 0.160.1, claude 2.1.282, pi 1.1.0, shell
Found 3 existing sessions: codex (2), tmux (1)
Note: remote sessions on this machine are recorded. Retention is 90 days plus monthly purge rounding, about 120 days at most; le…
```

Running `qgenie remote up` again is idempotent. It reports the current state and does not enroll the machine a second time.

### 5.1a Laptops: enabled from the desktop app

This workflow is deferred until after the TUI-only MVP. In the first release, users enroll laptops by running `qgenie remote up` themselves.

On first sign-in, the Electron desktop app asks: **"Make this laptop available as a remote host?"** (default on, with a short explanation). If the user confirms, the app runs `qgenie remote up` for them. From then on:

- The laptop is a host by default. The user service starts at login and stays connected even when the desktop app is closed.
- A tray icon shows hosting state (● hosting, ⏸ paused, ⚠ error), with Pause hosting / Resume (`qgenie remote down --keep-sessions` / `up`). Pause stops remote access while local sessions keep running.
- A notification appears when a remote client attaches to a session on this laptop.
- A sleeping laptop is shown as sleeping, not offline/error. Agents continue after wake.

A laptop is never a host unless `qgenie remote up` has run, either through the desktop app's prompt or by hand.

### 5.2 TUI client

Running `qgenie remote`, or opening the Machines tab in `qgenie dashboard`, shows:

```text
┌ QGenie Remote — aparkhi@qualcomm.com ──────────────────────────────────────────────────┐
│ Machines (4)                         │ hu-aparkhi-lv  ● online  linux x86_64           │
│ ● hu-aparkhi-lv       3 sessions      │ labels: lab, gpu    agents: codex claude       │
│ ● aparkhi-mbp         0 sessions      │                             pi shell           │
│ ○ sd-build-07         last 3h ago     │ Sessions                                       │
│ ● win-perf-02         1 ⚠ approval    │ ▸ codex  remote-gateway    running  12m        │
│                                      │   claude modem-fw         idle     2h         │
│                                      │   shell  ~                detached            │
│                                      │ ↗ codex  dragon-genie (external) 40m          │
│                                      │                                                │
│ [/] filter  [enter] open              │ [n] new [a] attach [o] observe                │
│ [r] rename  [x] revoke                │ [k] kill [p] approvals                        │
└──────────────────────────────────────┴────────────────────────────────────────────────┘
```

- **New session:** choose an agent (from the detected ones) and a folder (directory browser limited to the allowed roots), optionally enter a first prompt, and confirm.
- **Attach** opens an agent session in a structured transcript view, with tool calls, diffs, and an approval prompt shown inline, plus an input box. For a terminal session, attach opens a full-screen terminal pane.
- **Observe** gives a view without session input control. Only one client can send input or resize/interrupt a session at a time; taking over control requires confirmation. Any connected client signed in as the owner may answer approvals, subject to host policy, without taking over the control lease.
- **Detach** (`Ctrl-] d`) leaves the session running.
- **Approvals** from any machine show up as a badge, and `p` opens a list across all machines.
- **External sessions** (↗) are ones the user started directly, such as the Codex TUI, `claude --bg`, Pi, or a tmux session. They can be attached to like any other session. Sessions outside the allowed roots support full remote control when the adapter supports it; observe-only fallbacks reflect adapter limits (see SE-12).
- Session views show a small recorded indicator.

### 5.3 Desktop app

Desktop app integration is not part of the first release. This section describes the later client.

The desktop app (the existing Electron app) shows the same machine list and session model, with richer rendering: a diff viewer, xterm.js terminals, and native notifications for approvals and turn completion. It uses the same gateway protocol as the TUI through `@qgenie/remote-sdk`, a TypeScript SDK generated from the Rust protocol types (architecture §4.5). The connection and tokens live in Electron's main process; the renderer never sees tokens.

### 5.4 CLI commands

| Command | Purpose |
| --- | --- |
| `qgenie remote` | Open the TUI client (machines and sessions). |
| `qgenie remote up [--name] [--label] [--root] [--no-service] [--device-code] [--non-interactive --json]` | Sign in if needed, enroll this machine, and install and start the host service. Idempotent. `--non-interactive --json` is the mode the desktop app uses. |
| `qgenie remote down [--keep-sessions]` | Stop the host connector. Pause hosting uses `--keep-sessions` so local sessions keep running while remote access stops. Without the flag, sessions are terminated after a grace period. |
| `qgenie remote status [--json]` | Host service state, connection, authentication, active sessions, version compatibility. |
| `qgenie remote logs [-f]` | Host service logs. |
| `qgenie remote doctor` | Diagnose OAuth, proxy, DNS, WSS, service manager, and agent detection. |
| `qgenie remote machines [--json]` | List machines (scriptable). |
| `qgenie remote attach <machine> [<session>]` | Attach directly without the TUI. |
| `qgenie remote run <machine> --agent codex --cwd DIR "prompt"` | Start a session non-interactively; print the session ID. |
| `qgenie remote revoke <machine>` | Revoke a machine's identity on the server side and block remote access. Running sessions continue locally. Works from any authorized client. |
| `qgenie remote unenroll` | Revoke the local machine and delete its key. |
| `qgenie remote config` | Allowed roots, agents, approval policy, labels. |

The existing hidden `qgenie daemon` (legacy telemetry) is not reused, to avoid confusion.

## 6. Functional requirements

Priority: **P0 = MVP, P1 = GA, P2 = later.**

### Identity and enrollment

| ID | Requirement | Pri |
| --- | --- | --- |
| ID-1 | Users authenticate through QGenie OAuth (Entra ID), using authorization code + PKCE in the browser. | P0 |
| ID-2 | Headless hosts (SSH session, no display) enroll with the Entra device authorization grant, chosen automatically or with `--device-code`. A machine that has a browser uses PKCE. | P0 |
| ID-3 | Remote access uses a dedicated audience/scope (e.g. `api://qgenie-remote/Remote.Control`). Existing QGenie inference tokens grant no remote access. | P0 |
| ID-4 | Each machine generates a non-exportable Ed25519 key pair stored in the OS keyring (protected file fallback). It registers only the public key, bound to user, OS user, and hostname. | P0 |
| ID-5 | Hosts authenticate with challenge-response signatures and receive short-lived machine tokens (≤15 min). User refresh tokens are not used for host connections. | P0 |
| ID-6 | A QGenie revocation committed at the gateway refuses new connections and removes affected live connections within 10 s. The accepting gateway denies access immediately after commit. Disabling a user in Entra revokes their user and enrolled-machine access within 15 minutes of disablement, including detection and gateway enforcement. Revocation blocks remote access; running sessions continue on the host. Token expiry alone is not proof of Entra disablement. | P0 |
| ID-7 | Users can list, rename, label, and revoke their machines from any client. | P0 |
| ID-8 | Re-enrolling the same machine and OS user keeps the machine record (no duplicates). | P0 |
| ID-9 | Sharing machines with specific users or groups at roles viewer, operator, or owner. | P2 |

### Presence and discovery

| ID | Requirement | Pri |
| --- | --- | --- |
| PR-1 | Machine list shows online/offline status, last seen time, OS/arch, CLI version, labels, detected agents and versions, and session count. | P0 |
| PR-2 | Presence is updated within 5 s of a host connecting, and within 30 s of an unclean disconnect. | P0 |
| PR-3 | Clients receive presence and session-list changes as push events (no polling). | P0 |
| PR-4 | Directory browsing on a host is restricted to the configured allowed roots. | P0 |

### Sessions

| ID | Requirement | Pri |
| --- | --- | --- |
| SE-1 | Start a terminal session (PTY) with a chosen shell/command, cwd, and size. | P0 |
| SE-2 | Start a Codex session through the Codex app-server protocol: start/resume a thread, prompt, steer, interrupt, plus streamed items and approvals. The daemon reaches Codex only over local IPC/loopback (Unix socket or `127.0.0.1`). | P0 |
| SE-2a | Codex's built-in remote-control relay is disabled in QGenie-managed config. The host detects and reports (and by org policy, disables) any agent relay or non-loopback listener. QGenie Remote is the only remote path. Claude Code Remote Control is disabled or ignored (it needs a claude.ai login that QGenie-managed Claude doesn't use). | P0 |
| SE-3 | Start a Claude Code session through `claude -p --input-format stream-json --output-format stream-json` with permission prompts answered by the host. | P1 |
| SE-4 | Start a Pi session through `pi --mode rpc` plus a QGenie Pi extension that sends tool-call confirmations as approvals. | P1 |
| SE-5 | Agents are launched through QGenie endpoint injection (same models, limits, and telemetry as `qgenie codex`, etc.). | P0 |
| SE-6 | Sessions are owned by the host and keep running when all clients disconnect. | P0 |
| SE-7 | Reattach replays missed events and output from a cursor. For terminals: a screen snapshot plus the missing bytes. | P0 |
| SE-8 | Many observers can watch a session; one client holds the control lease (input/resize). Takeover is explicit. | P0 |
| SE-9 | Each approval request is delivered to attached owner clients and shown in the owner's approvals list. Any connected client signed in as the session owner may answer, subject to host policy; a control lease or writable attachment is not required. An outside-root cwd does not block owner approval answers when the adapter supports them. Revoked access cannot resolve approvals. Each request has one winning decision (the first valid answer wins), delivered with acknowledgment and deduplication. Unanswered approvals persist until answered, cancelled, or timed out by policy. | P0 |
| SE-10 | Starting a session is idempotent (client-supplied idempotency key). After a disconnect, an "outcome unknown" result is reported and the start is never blindly retried. | P0 |
| SE-11 | Detach, interrupt turn, terminate session, and resume historical session are separate actions. Revoking access does not terminate sessions; termination requires a separate explicit action. | P0 |
| SE-12 | Sessions started outside QGenie Remote are discovered and adopted automatically after host enrollment when they run as the same OS user as the host daemon: Codex sessions on the user's app-server daemon (full control), `claude --bg` (full control), interactive Claude in another terminal (observe-only, "resume here" when it exits), Pi via the QGenie extension (full control), tmux and Herdr sessions (full terminal control). Recording/backfill starts automatically on adoption; no separate enable step is required. Other users' and root's sessions are never shown. | P1 |
| SE-12a | Adopted sessions appear within 10 s of starting, are marked external, are recorded automatically from adoption with a backfill of earlier history, and require confirmation before a remote client terminates them. Host enrollment discloses automatic discovery, recording, and history backfill; users may disable adoption per agent. | P1 |
| SE-12b | Existing same-OS-user sessions outside the allowed roots allow full remote control: input, approvals, interrupt, resize, and termination, when supported by the adapter/session kind. An outside-root cwd is not a denial reason for these actions. Identity, access revocation, control leases, agent/terminal policy, and termination confirmation still apply. Allowed roots restrict browsing, new-session cwd selection, and file transfer. | P1 |
| SE-13 | Non-PTY command execution with separate stdout/stderr and exit code (`exec`), for scripts and automation. | P1 |
| SE-14 | Upload/download of a single file within the allowed roots (≤50 MB). | P2 |

### Clients

| ID | Requirement | Pri |
| --- | --- | --- |
| CL-1 | TUI as described in §5.2, inside the `qgenie` binary (ratatui). | P0 |
| CL-2 | After the TUI-only MVP, Electron desktop app support through `@qgenie/remote-sdk` (generated TS types + Node WS client), with the same conformance tests as the Rust client. | P1 |
| CL-3 | Notifications for approvals, turn completion, and session exit (desktop, TUI badge). | P1 |
| CL-5 | After the TUI-only MVP, the desktop app offers to enable hosting on first sign-in, runs `qgenie remote up --non-interactive`, supervises the host service (tray status, pause/resume), and keeps it updated through QGenie's update channel. | P1 |
| CL-6 | Laptop presence distinguishes sleeping from offline. An optional setting keeps the laptop awake while an agent turn is running (default off on battery). | P1 |
| CL-4 | Web client. | P2 |

### Policy, audit, and admin

| ID | Requirement | Pri |
| --- | --- | --- |
| PO-1 | Host-side policy: allowed roots for browsing/new-session cwd/file transfer, allowed agents, whether terminal sessions are allowed, default approval mode. Existing same-user sessions are not restricted to observation because their cwd is outside the roots (SE-12b). Policy is enforced on the host even if the gateway is compromised. | P0 |
| PO-2 | Org-side policy: enable/disable the feature per user or group, minimum CLI version, allowed agents, retention period. | P1 |
| PO-3 | Audit events for enroll/revoke, connect/disconnect, session start/attach/takeover/terminate, approvals (who, what, decision), and policy denials. | P0 |
| PO-4 | Recording is enabled for every session and can't be turned off by users or orgs: agent transcripts (prompts, messages, tool calls, diffs, approvals) and terminal output (with resizes and timing; no-echo input such as passwords is not captured). Stored in Postgres with a 90-day retention cutoff (configurable); monthly partition deletion may retain content for about 120 days, with best-effort secret redaction. Legal holds can keep it longer. Sessions continue if recording fails or the host's recording spool fills. Data that cannot be retained is represented by explicit gap markers; the TUI shows a recording warning and operators are alerted. Recording is always enabled; storage failures can produce explicit gaps. | P1 |
| PO-6 | A nightly job drops monthly transcript partitions whose entire time range is older than the retention cutoff, except data under legal hold. The default 90-day cutoff allows retention rounding to about 120 days. Upload retries and history backfill do not reset content age. | P1 |
| PO-7 | Transcript access is limited to the owning user and platform admins. Admins must give a reason, and every admin read is recorded in the admin audit log. Users are not notified of admin reads. No other role (managers, other users, sharees, support) can read transcripts. Enforced in the API and by Postgres row-level security. | P1 |
| PO-8 | Legal hold for select individuals: only platform admins can place, change, or release a hold on a user (optional time window, case reference). One admin is sufficient. Holds are invisible to the user. Held transcripts, session metadata, and audit events are exempt from purging until the hold is released. Every hold action is audited. | P1 |
| PO-5 | Platform admins have an on-demand revoke/kill-switch API at `qgenie-remote.qualcomm.com` to disable remote access globally, for a user, or for a machine. It is available in the TUI-only MVP and does not depend on the desktop app or an admin UI. Deny access immediately at the accepting gateway after commit; apply the change across all gateways within 10 s, audit it, and report its propagation status. Running host sessions continue; revocation sends no termination command. | P0 |

## 7. Non-functional requirements

| Area | Target |
| --- | --- |
| Scale (GA) | 30,000 users, ~90,000 enrolled machines (2–3 per user). Peak: ~65k host sockets, ~25k client sockets, ~20k streaming sessions; designed for 2x (200k sockets). Sizing in architecture §9.7. |
| Latency | Keystroke echo overhead added by the gateway: p50 < 30 ms, p99 < 150 ms within a region. Approval delivery < 1 s. |
| Availability | Gateway 99.9%. Losing a gateway container or a whole server causes reconnects, not session loss. Postgres failover is manual in v1 (target < 15 min). |
| Durability | Host replay buffer of at least 10k events or 8 MB per session (configurable). Session metadata survives gateway restarts. Recordings (agent and terminal) use a 90-day cutoff, rounded by monthly purging to about 120 days, or longer under legal hold. A Redis failover causes no lost or duplicated output (sequence-number resync). |
| Host footprint | Idle host daemon < 30 MB RSS, < 0.5 % CPU, one TCP connection. |
| Platforms | Hosts: Linux x86_64/arm64 (P0), macOS (P0), Windows (P1, ConPTY). First-release client: TUI on the supported platforms. Electron desktop app (macOS, Windows, Linux) follows after the MVP. |
| Networking | Outbound TCP 443 only. Hosts and clients must reach `qgenie-remote.qualcomm.com` through the corporate network or a corporate VPN. Public-internet gateway access is not required. Hosts require no inbound ports or direct client reachability. Honors `HTTPS_PROXY`/`NO_PROXY` and corporate CA bundles. Connections survive proxy idle timeouts (heartbeat ≤ 25 s). |
| Compatibility | Protocol versioned with capability negotiation. The gateway supports CLI versions N-2. Older hosts degrade gracefully. |
| Infrastructure | Latest stable Redis (primary + replica + Sentinel) and latest stable PostgreSQL (primary + standby, point-in-time backups), both as containers in the deployment. |
| Hardware | Servers with 128 cores, 1 TB RAM, 4–5 TB storage; more can be added. Dev: one server (single-server mode). Production: three servers; Postgres servers likely need ≥ 8 TB for 90 days of mandatory recording (architecture §9.7). |
| Deployment | v1: one on-prem deployment using Docker Compose, across three servers in production (single-server mode for dev, now). All components are Docker containers. Rolling upgrades with no session loss. Kubernetes, managed databases and more sites are later; the images must not depend on Compose-specific behavior. |
| Security | TLS 1.2+ (1.3 preferred). No inbound ports. Host daemon never runs as root by default. Every message authorized. Secrets never logged. Threat model reviewed before GA. |
| Observability | OTel metrics/logs/traces to `aiplatforms-otel.qualcomm.com`. Per-connection and per-session IDs in all logs. |

## 8. Success metrics

- Adoption: weekly active users with ≥1 enrolled host. Target: 30% of active QGenie CLI users 3 months after GA.
- Reliability: at least 99.5% of sessions reattach successfully after a client disconnect; < 0.1% duplicate session starts.
- Time to first remote session after `qgenie remote up`: < 2 min median.
- Approval latency (request → shown on client): p95 < 1 s.
- Zero P0 security incidents. Gateway-admin revocation meets the 10 s SLO in 100% of drills, including missed Redis broadcasts. Entra disablement blocks user and enrolled-machine access within 15 minutes, including detection. Running sessions survive both forms of revocation.

## 9. Release plan

| Phase | Scope | Exit criteria |
| --- | --- | --- |
| M0: Spike (2–3 wk) | On the single dev server: protocol crate, gateway (4 containers) + Redis + Postgres, a host connector, a TUI listing machines, a terminal session end-to-end, device-code enrollment over SSH. | Demo: laptop TUI → lab-server shell through the gateway, behind the corporate proxy. |
| M1: MVP / internal alpha (6–8 wk) | TUI-only client: ID-1..8, PR-1..4, SE-1, SE-2, SE-2a, SE-5..11, CL-1, PO-1, PO-3, PO-5, including the platform-admin revocation API. Gateway with HA (2+ nodes). | Dogfood with about 20 users. Measure recording volume per user to confirm storage sizing. Disconnect/reattach, gateway restart, gateway-admin revocation with a missed broadcast, Entra disablement within 15 minutes, and duplicate-start tests pass. Revocation blocks access while sessions continue. Desktop integration is not an exit criterion. |
| M2: Beta | SE-3, SE-4, SE-12, SE-12a, SE-12b, SE-13, CL-2 (Electron), CL-3, CL-5, CL-6, PO-2, PO-4, PO-6, PO-7, PO-8, Windows hosts. | Desktop app integration, about 300 users, SLOs measured. Production three-server deployment; load test at 200k sockets passes. |
| M3: GA | Hardening, threat-model fixes, runbooks, admin UI and remaining admin APIs; the revocation API already ships in M1. | Security sign-off, SLOs met for 4 weeks. |
| Later | Sharing (ID-9), web client, file transfer, Kubernetes, multiple sites/regions. | — |

## 10. Risks

| Risk | Mitigation |
| --- | --- |
| Remote code execution as a service broadens the attack surface. | Separate scope, a key per machine, authorization on every action, host-side policy enforced locally, kill switch, threat model, pen test before GA. |
| Agent protocols change (the Codex app-server is experimental). | Version-pinned adapters, generated schemas, capability flags, CI against the versions QGenie installs. |
| Device-code phishing (an attacker gets a user to enter the attacker's code). | Show machine name and hostname on the consent/confirmation page; notify the user on every new enrollment; let them revoke in one click. Conditional Access stays enforced. |
| Redis pub/sub drops messages under load or failover. | Sequence numbers + resync from the host; end-to-end acks on control messages; Redis failover in fault tests. |
| Stored recordings contain secrets or sensitive code (terminal output especially). | Best-effort redaction on the host and the writer; encryption at rest, 90-day cutoff with monthly purge rounding to about 120 days; owner/admin-only access with RLS; audited admin reads; legal/privacy review before beta. |
| Transcript tables grow large in Postgres. | Compressed batches, monthly partitions dropped on expiry, capacity sized for about 120 days at the default cutoff, optional separate transcript database. |
| Users forget their laptop is a host. | Explicit opt-in prompt, tray indicator, attach notifications, one-click pause, and org policy to force it off. |
| A Codex update re-enables or renames its remote-control setting. | Adapter contract tests per pinned Codex version; continuous detection and reporting on the host. |
| Corporate proxies break WebSockets. | `qgenie remote doctor`. Fall back to long-poll HTTP/2 transport in P2 if needed. |
| Laptops sleeping make presence flap. | Heartbeat expiry with grace period; clear "sleeping/offline" state. |
| Sessions killed by host daemon restarts and updates. | Session host separated from the connector (architecture decision D7). Prefer agent-native background daemons. |
| Adopting external sessions exposes something the user didn't intend to share. | Same-OS-user and owning-QGenie-user only; enrollment discloses automatic recording and full control of supported sessions, including outside-root sessions; adoption can be turned off per agent; termination requires confirmation. |

## 11. Decisions made

| Topic | Decision |
| --- | --- |
| Gateway language | Rust (shared protocol crate with the CLI) |
| First-release client | TUI only; users enroll hosts through `qgenie remote up` |
| Desktop app | After MVP: Electron, through the generated TS SDK |
| Revocation | Gateway admins can revoke on demand; accepting node denies immediately after commit, all gateways within 10 s. Entra disablement blocks user and enrolled-machine access within 15 minutes. Running sessions continue locally |
| Gateway reachability | Corporate network or corporate VPN; public-internet access is not required |
| Headless login | Entra device code flow (allowed by policy); PKCE where a browser is available |
| Routing bus | Redis, latest stable (sharded pub/sub + TTL keys) |
| Database / recordings | PostgreSQL, latest stable. Recording always enabled, including terminal output. Default cutoff 90 days; monthly purge rounding allows about 120 days, except legal holds. Sessions continue during recording failures; missing data is marked and alerted |
| Approval rights | Any connected client signed in as the session owner, subject to host policy; no control lease required |
| Outside-root sessions | Full remote control when supported: input, approvals, interrupt, resize, termination. Allowed roots govern browsing/new-session cwd/file transfer, not control of existing same-user sessions |
| Pause hosting | Stop remote access with `qgenie remote down --keep-sessions`; local sessions continue |
| External-session discovery | Automatic after host enrollment, including recording and history backfill; no separate enable step |
| Transcript access | Owning user and platform admins only (admin reads audited) |
| Legal hold | Per individual; placed and released only by platform admins (no second approver); invisible to the user |
| Admin transcript reads | Audited, but the user is not notified |
| Codex remote control | Allowed only over local loopback/IPC; Codex's relay disabled; QGenie Remote is the only remote path |
| Laptops | Hosts after explicit `qgenie remote up`; desktop enrollment and supervision follow after MVP |
| Deployment | v1: a single on-prem Docker Compose deployment (Redis/Postgres as containers); more sites and Kubernetes later |
| TLS certificates | Internal CA with automatic renewal (integration later) |
| Scale | 30k users, ~90k machines |
| Hardware | 128 cores / 1 TB RAM / 4–5 TB per server; one server for dev now |
| External sessions | Adopted when they run as the same OS user |
| Claude Remote Control | Disabled or ignored |

## 12. Open questions

1. Which team operates the deployment (on-call, Postgres failover, backups, storage alerts)?
2. Production storage for Postgres: ≥ 8 TB data volumes on the primary and standby servers, or revisit after measuring recording volume in dev?
