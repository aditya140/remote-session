# ADR 0003: Separate session lifetime from network lifetime

Status: selected for implementation. Linux and macOS service proof remains in task 025.

## User task and problem

Users must pause hosting, close a client, or survive a connector update while their work continues. A detached child can still belong to a service cgroup that the service manager kills.

## Decision and reason

Run the connector and session owner as separately managed services under the enrolled OS user. The connector owns network access. Sessiond owns PTYs, native-agent handles, replay, recording, and SQLite state. No viewer or connector task owns session cleanup.

On Linux, use independent systemd user units and cgroups. The sessiond unit must have no stop-propagating `PartOf`, `BindsTo`, or parent cleanup relationship to the connector. Starting sessiond on demand must go through the existing service integration. `setsid` alone is insufficient. Task 025 must inspect real unit definitions and kill the connector cgroup while PTY and Codex sessions run.

On macOS, use independent launchd jobs with distinct labels. Do not tie sessiond's shutdown to a connector child handle. Task 025 must prove job restart and update behavior on macOS. Do not infer it from Linux tests.

| Action | Connector | Sessiond and sessions |
| --- | --- | --- |
| Client detach or exit | Keep hosting | Keep execution and recording. Remove the viewer; release its control lease. |
| Connector crash/restart/update | Reconnect with a new host generation | Keep execution and spool. Reject stale remote generations. |
| `down --keep-sessions` | Stop remote access | Keep execution and spool. Resume uploads after `up`. |
| User/machine/global revocation | Close remote access and back off | Keep execution. Do not auto-approve or auto-enroll. |
| Unenroll | Revoke access and remove the machine credential | Keep existing local sessions. Preserve local session/spool state for explicit recovery or cleanup. |
| `down` without keep-sessions | Stop access | Use a separate local administrative request for confirmed termination after a configured grace period. |
| Sessiond update | No implied termination | Defer while sessions are active. A forced update must state that sessions may end and require explicit local action. |
| Idle sessiond | Connector may remain online | Exit only with no active sessions, pending native work, or unflushed durable work. Reopen the same private state on demand. |
| Logout or reboot | Remote access ends | Report platform limits. Reconcile recovered native sessions; otherwise mark sessions `lost`. Do not promise survival through reboot. |

User services do not universally survive logout. Record Linux lingering and macOS login-session behavior as prerequisites after native inspection. Do not silently enable system-wide services or lingering. A `--no-service` mode must declare its foreground/service-lifetime limits. It cannot establish the service-isolation acceptance criterion.

The updater must preserve the executable revision used by an active sessiond until it can exit safely. The exact mechanism depends on the inspected CLI updater. No binary replacement API is assumed here.

## Private IPC

Use Unix-domain sockets on Linux and macOS. Create the runtime directory with mode `0700` and sockets with mode `0600`. Require directory/socket ownership by the expected UID. Reject symlinks and unsafe ancestors. Validate peer credentials with the native platform API. Files containing local credentials, SQLite state, spool content, or journal data must remain private to that user.

Use two endpoint types:

- The dispatch endpoint accepts only the public operation allowlist plus internal activation, access-renewal, recording, and reconciliation messages.
- The administrative endpoint accepts local lifecycle operations such as explicit owner shutdown. The connector's remote decoder must have no variant that maps to those commands.

The administrative endpoint remains available to the same OS user. Same-user IPC is a trust boundary against other users; it is not a sandbox against malicious processes running under that UID.

Version IPC independently as `ipc_major` and `ipc_minor`. The initial contract is `1.0`. Negotiate capabilities before dispatch. Prefix each JSON payload with a big-endian `u32` length, excluding the prefix. Reject lengths above 1 MiB before allocation. Use the public scalar encoding rules, but keep administrative schemas out of public schema export.

Each dispatch carries the authenticated remote actor, machine/session IDs, action, host generation, applicable controller generation, current policy revision, and access-renewal context. Treat peer UID as local transport authentication. Sessiond must also check the action, generations, policy facts, and deadline. Do not trust a remote routing hint as actor identity.

Cap each IPC connection at 128 pending requests and 4 MiB queued bytes. A request that cannot enter a queue returns `overloaded/not_sent`. Do not silently discard an accepted mutation or approval. A broken reply channel can produce `unknown`; it cannot cancel committed local work.

## Socket recovery and shutdown

Use a lifetime-held exclusive lock to establish the sessiond instance. Validate existing socket type, owner, and directory before connecting or attempting cleanup. A PID file alone does not prove that a socket is stale. Do not unlink a live endpoint or a path that changed ownership/type during recovery. Use directory-relative operations and identity checks to prevent path substitution.

Reject incompatible peers and interrupted frames explicitly. Cancel and join owned transport tasks on shutdown. Closing dispatch must not implicitly terminate adapters. There is no fallback remote TCP listener. Windows named pipes and ConPTY remain later work.

## Verification owners

Task 006 verifies native IPC permissions, foreign-user rejection, framing, and stale-socket races. Task 015 verifies local ownership. Task 025 verifies actual systemd and launchd lifetimes. Task 027 combines these cases with pause, revocation, and reconnect.
