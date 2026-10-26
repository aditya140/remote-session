# Standalone remote host and TUI commands

The standalone `qgenie-remote` binary composes the SDK, TUI, outbound host and independently
owned session daemon. Build it with `cargo build --locked -p qgenie-remote`. No gateway or
connector owns a native child. Use the same compatible revision for both native roles.

## Host configuration and enrollment

Create an absolute, current-user-owned `0600` JSON configuration file. Paths are absolute;
roots must satisfy both descriptor-scoped host and native filesystem policy. For example:

```json
{
  "gateway": "https://remote.example.com",
  "os_user": "your-os-account",
  "name": "Development host",
  "hostname": "your-hostname",
  "roots": ["/home/your-os-account/work"],
  "allow_private_file": false,
  "labels": {"purpose": "development"}
}
```

Optional `ca_bundle`, `identity_directory` and `redaction_file` paths are also absolute.
`redaction_file` contains a private JSON array of literal strings for native recording
redaction. Default identity state is `STATE/identity`. Set `allow_private_file` to `true`
only when accepting the disclosed exportable `0600` machine-key fallback if the OS credential
store is unavailable. User access/refresh tokens are never saved in host enrollment state.

Linux builds using `--features managed-codex` also accept an optional `codex` object:
`{"binary":"/absolute/pinned/codex","endpoint":"https://provider.example/v1",
"model":"your-model","credential_file":"/private/provider-key.json"}`.
The private credential file contains a JSON string. Only sessiond reads this file and
injects the credential into the native child. A configured build checks Codex's pinned
version during startup discovery and again under the original launch grant. Builds without
this feature reject the configuration explicitly. The connector advertises the verified
native inventory; the gateway only starts an agent offered by the current host.

```sh
qgenie-remote --state-dir /private/state host --config /private/host.json \
  login --oauth-config /private/oauth.json --device --acknowledge-recording
# Or use browser PKCE by omitting --device.
# For caller-managed short-lived credentials:
qgenie-remote --state-dir /private/state host --config /private/host.json \
  enroll --token-file /private/current-user-token --acknowledge-recording
```

Login/enrollment prints the recording/retention/hold disclosure and requires its explicit
acknowledgment. Browser URLs go directly to `open`/`xdg-open`; device codes are displayed for
the interactive flow. Only the current dedicated Remote.Control access token reaches
enrollment. Refresh credentials are discarded. Repeating enrollment retains the original
installation/key and reconciles a lost response; missing keys require explicit recovery
through the host identity API, rather than silent replacement.

## Independent processes and user services

```sh
qgenie-remote --state-dir /private/state host --config /private/host.json up
qgenie-remote --state-dir /private/state host --config /private/host.json status
qgenie-remote --state-dir /private/state host --config /private/host.json doctor
qgenie-remote --state-dir /private/state host --config /private/host.json down --keep-sessions
```

`up` installs user definitions, enables/starts the native systemd user unit, and resumes the
connector separately. macOS uses two launchd jobs. The jobs have no shared lifetime dependency;
stopping/restarting the connector preserves native ownership and recording. Machine-only
authentication/refresh, WSS and upload run in the connector. `status` reports the enrollment,
actual local connector lock and native IPC. `doctor` additionally performs a verified TLS
machine authentication exchange. A running process/lock alone does not prove current remote
authority. No doctor mutation, controller acquisition or raw credential logging occurs.

`down --keep-sessions` requests connector cancellation and waits for its exclusive lock to
be released. The native owner keeps running. `down --terminate-sessions` explicitly terminates
sessions belonging to this machine binding after the connector stops. Unenrolled local
preview sessions are outside that termination selection. Outstanding native authority retains
its original maximum ten-second deadline; cancellation cannot renew it. `host up` reconnects
and uploads retained recording content. A lost input reply is never retried.

When no user manager is available, use explicit process mode:

```sh
qgenie-remote --state-dir /private/state host --config /private/host.json up --no-service
# Or independently foreground each role:
qgenie-remote --state-dir /private/state sessiond --host-config /private/host.json
qgenie-remote --state-dir /private/state host --config /private/host.json connect
```

Process mode has no logout/reboot supervision. Each child gets its own process group; stopping
the CLI or connector does not supervise or terminate sessiond. Repeated `up` uses the native
owner lock and connector lock, without another enrollment or a second active connector.
Logs are private `STATE/sessiond.log` and `STATE/connector.log`; service mode uses journald or
private launchd job logs. These diagnostics contain connection state, never PTY transcripts.
Apply host log rotation and disk monitoring; managed rotation remains an operations task.

`host service install --directory /absolute/output` generates definitions without touching a
service manager. The default destinations are `$XDG_CONFIG_HOME/systemd/user` (or
`$HOME/.config/systemd/user`) and `$HOME/Library/LaunchAgents`. Executables and native
configuration snapshots are pinned under private `STATE/versions`. Existing native definitions
are retained when sessions are active or IPC cannot establish idleness; connector definitions
can update independently. A service manager stop of the **native** unit is destructive to its
owned processes. Use the guarded upgrade path:

```sh
qgenie-remote --state-dir /private/state host --config /private/host.json \
  service upgrade-native --yes
```

The daemon's starts mutex checks idleness and seals shutdown atomically. A racing accepted
launch refuses the request before a service stop. Version updates do not automatically restart
an active native owner. Linux user services normally stop on final logout unless the operator
has enabled lingering; that is an explicit OS policy decision. macOS user jobs normally end at
logout. Reboot/native-owner loss produces lost/unknown native lifetime on restart, preserves
start identities and recording gaps, and does not automatically relaunch sessions.

`host ... unenroll --token-file PATH --yes` revokes this binding and removes its signing key
after the committed response. Native sessions remain local. Current access/freshness gates
prevent the connector from using a revoked enrollment; pending approvals never auto-approve.

## Remote TUI

`qgenie-remote --state-dir PATH remote --gateway ORIGIN --token-file PATH [--ca-bundle PATH]`
injects a current private user credential into the SDK. Enter opens machine sessions and
recorded history; `v` observes a live PTY; `c` browses an allowed directory and Ctrl-N starts
the selected runtime; F5 in that picker cycles through the terminal and advertised agents.
`k` looks up original persisted start keys after restart. Uncertain starts
cannot be resubmitted from the journal recovery view. The start identity journal contains scoped
identity/fingerprint metadata without argv, prompts or credentials.

F2 acquires control, explicit takeover asks for confirmation, F3 releases, F4 interrupts,
F6 confirms termination, F7 opens owner approvals, F8 requests a checkpoint and F10/Ctrl-] detaches. Observe mode drops
input. Input cancellation/lost replies discard unsent keys and require explicit reacquisition.
Native state and recording warnings stay visible independently of input/network reads. Use
`r` in the parent view to reconnect explicitly. Expired user credentials are replaced by
reopening with a fresh injected token. Structured agents have a bounded prompt editor;
Enter sends a prompt and F5 selects steer when advertised. In the session list, `u` resumes
eligible confirmed-ended agent history: choose a new allowed directory and press Ctrl-N.
This creates a new session/process with the saved thread, and labels earlier history partial.
Active, lost and outcome-unknown sessions cannot be resumed. Journal recovery with `k`
performs read-only lookup of the original session.

The machine list displays host/platform/CLI/native-agent versions and last seen time. Its
owner-wide approval badge uses bounded current status reads; incomplete scans and failed
reads remain explicit.

`a` in the parent view opens owner-wide approvals without acquiring a controller or
attaching to a session. Use 1–9 to select an offered native decision and `y` to confirm;
`n` cancels. A committed choice remains separate from native application, cancellation,
or OutcomeUnknown. This view sends each choice once. Use `r` for current durable status,
`p` for the first page and `n` for the next page. Reopening/reconnecting reads status and
never automatically resends an answer. F7 from an observing live pane uses this same inbox
without taking control; returning installs a fresh output checkpoint. Approval support requires negotiated
`approval-status-v1`/`durable-approvals-v1` and native `host-approvals-v1` support.

In the machine list, F2 renames, F3 adds/edits a label (Ctrl-D confirms removal) and F6 asks
for confirmation before revoking remote access. Revocation preserves native execution and
local capture. These one-shot changes work for offline machines too; an uncertain result
requires inventory reconciliation rather than an automatic retry. `/` filters machine IDs/names,
hostnames and labels, or session IDs/cwd/state/agent/command labels. Enter applies, Esc cancels
the edit and Ctrl-U clears; hidden rows cannot receive actions.

Scriptable owner metadata/access operations use the same current user token and SDK:

```sh
qgenie-remote remote --gateway ORIGIN --token-file PRIVATE_TOKEN rename MACHINE 'New name'
qgenie-remote remote --gateway ORIGIN --token-file PRIVATE_TOKEN label MACHINE team research
qgenie-remote remote --gateway ORIGIN --token-file PRIVATE_TOKEN remove-label MACHINE team
qgenie-remote remote --gateway ORIGIN --token-file PRIVATE_TOKEN revoke MACHINE --yes
```

`remote send MACHINE SESSION --stdin [--enter]` accepts up to 32 KiB including optional Enter,
reads it before taking control and sends once. It supports piped input without placing the
text in process arguments. The original literal-text positional form remains available.
For agent text, use `send MACHINE SESSION --stdin --prompt` or `--steer`; text must be
UTF-8 without NUL. Create with `run MACHINE --cwd PATH --agent codex`; add `--resume-id THREAD`
for eligible confirmed-ended history in a new session. Both creation and
text use the same durable identities and no automatic resend as their SDK counterparts.

## Verification boundary

The owned synchronous fixture exercises actual CLI connector processes, native PTY ownership,
verified HTTPS/WSS, two gateway compositions, redacted history and keyboard workflows. It also
checks generated Linux service syntax and deferred native definition updates. These process
and static-unit checks do not prove an actual user-manager cgroup lifetime. Run Linux
`systemctl --user` and macOS launchd kill/pause/upgrade/logout drills on qualified hosts,
including the configured pinned Codex runtime. This workspace's user
manager reports `offline`; macOS execution is unavailable. The development deployment is
verified separately. Managed runtime, approval loss/reconciliation and historical resume pass
the composed Linux fixture. Remaining enterprise/platform release evidence is tracked in
`tasks/PROGRESS.md`.

## Inventory pages and upgrade order

`list --json`, `remote sessions MACHINE`, and `remote starts MACHINE` retain their JSON
array output. They read bounded pages while writing the array. Use `--page` for one atomic
page with a `next` cursor. Pass that cursor with `--page --after CURSOR` to continue.
`remote sessions MACHINE --active` selects live work. Local `list --active` returns one
live page. `remote roots MACHINE` returns authorized configured roots.

Pages contain at most 256 records and 512 KiB of metadata; the CLI requests 64 records.
Session cursors use stable ID ordering. Concurrent inserts before the cursor require a
new traversal. A streaming command can fail after partial output; check its exit status.
Use individual session reads for status and original-key lookup for uncertain starts.

Deploy compatible gateway code and PostgreSQL migration 0016 before new connectors.
Upgrade sessiond to SQLite schema 7 through the existing guarded native upgrade procedure.
Do not restart an active native owner merely to install this change. New connectors require
paged native metadata. Older clients retain bounded complete-list reads and manual paths.
Upgrade all client processes that share a state directory before its first SQLite journal
migration. Keep the private legacy files as recovery backups. The migration does not delete
start identities, replay, recordings, retention records, or legal holds.
