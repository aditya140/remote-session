# Launch profiles and session metadata

A launch profile saves repeatable choices on the client. It binds one HTTPS gateway
origin and machine ID to a directory and shell argv or an agent and opening task.
It does not grant permissions. Each run uses current authorization, allowed roots,
and advertised agent support. A profile called `read-only-reviewer` has no special
security rights. Keep credentials out of commands and task text.

Save profiles without signing in. Supply the gateway origin explicitly:

```sh
qgenie-remote remote --gateway https://gateway.example profile save dev-server MACHINE --cwd /projects/api -- /bin/bash -l
qgenie-remote remote --gateway https://gateway.example profile save review-api MACHINE --cwd /projects/api --agent codex --task 'Review the API changes.'
qgenie-remote remote profile list
qgenie-remote remote profile show review-api
```

The state directory holds at most 64 profiles in `launch-profiles`. Files use mode
0600. Profile names contain at most 64 ASCII letters, digits, hyphens, or underscores.
The reader rejects unknown fields, credentials fields, unsupported versions,
symlinks, and files larger than 64 KiB. Use `profile save --replace` to replace a
profile. Use `profile remove NAME` to remove one. Removal does not stop any session.

Run a profile with the normal saved connection settings or explicit authentication
options. The profile supplies its bound gateway origin. An explicit different origin
is rejected. Authentication settings remain separate from the launch profile.

```sh
qgenie-remote remote profile run review-api
```

The command prints the original start key before submission. It saves that identity
in the existing start journal. If the reply is lost, use `remote reconcile MACHINE KEY`.
Do not run the profile with a new key to recover uncertain work. An explicit `--key`
uses the same existing idempotency checks. Changing a profile does not change a prior
start identity. Profiles do not add a task scheduler or automatic retries.

## Name and find sessions

Session names and labels belong to the authenticated gateway owner and selected
machine. PostgreSQL stores them separately from native session state. A rename keeps
the session ID, start key, process lifetime, controller, and recordings unchanged.
A name is an alias. Multiple sessions can share a name, but lookup then returns
`ambiguous_name`. Use a stable session ID to select the intended session.

```sh
qgenie-remote remote name-session MACHINE SESSION review-api
qgenie-remote remote label-session MACHINE SESSION team api
qgenie-remote remote find-session MACHINE review-api
qgenie-remote remote sessions MACHINE --filter review-api
qgenie-remote remote remove-session-label MACHINE SESSION team
qgenie-remote remote name-session MACHINE SESSION --clear
```

Names contain 1–128 UTF-8 bytes without control characters or edge whitespace.
Each session has at most 16 labels. Keys contain 1–64 ASCII letters, digits, dots,
hyphens, or underscores. Values contain at most 128 bytes without control characters.
Name lookup is exact and case-sensitive. CLI and TUI text filters ignore case and
match names and labels. The TUI displays both. The stable ID remains available for
all actions. Neither a name nor a label is an authorization claim.

Metadata updates are audited. Concurrent edits to different labels preserve both
values. Native host updates cannot overwrite names or labels. If a metadata reply
is lost, read the session to reconcile it. Do not start another session. Connected
views receive a metadata update; refresh the view if that notification is lost.

## Compatibility and deployment

Metadata writes and name lookup require `session-metadata-v1`. Older peers return
`unsupported_capability` before a write. Existing records without metadata read as
unnamed sessions with no labels. Older clients can still read sessions and use IDs.
Profiles use the existing launch protocol and its capability checks.

Gateway binaries in this change require PostgreSQL schema 17. Apply migration 0017
with the migration credential before enabling the new gateways. Previous binaries
accept schema versions only through 16. Use the documented schema compatibility
procedure when deploying or rolling back. The native owner and its running sessions
remain separate from the gateway deployment.
