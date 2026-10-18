# QGenie Remote

The standalone Rust CLI/TUI supports local shell sessions and authenticated remote PTY and managed Codex workflows. A separate session daemon owns native processes, replay and recording. Closing the TUI leaves sessions running. Reattach to observe output or request input control.

Linux standalone development across all 28 tasks is complete within the user-selected scope,
with external release gates documented separately. [Progress](tasks/PROGRESS.md) records the implementation and
executed evidence. [ADR 0007](docs/adr/0007-standalone-preview.md) authorizes the new TUI
without the existing corporate CLI source. See [external release gates](docs/validation/external-release-gates.md)
for unrun tenant, platform, production operations and beta-scale qualification.

## Run the local preview

Install Rust through rustup. This workspace selects Rust 1.99.1. Run these commands from the repository root:

```sh
cargo run --locked --package qgenie-remote -- tui
```

The command starts the local session daemon if necessary. Press `n` to enter a working directory and start a shell. The daemon initially allows the directory from which it was started. Use `--root /absolute/path` before `up` or `tui` to select another root.

State defaults to `$XDG_DATA_HOME/qgenie-remote` or `$HOME/.local/share/qgenie-remote`. Use `--state-dir /absolute/private/path` or `QGR_STATE_DIR` for a separate instance. The state directory must have mode `0700`. Socket and database files must be private to the current OS user. Unix socket paths must not exceed 100 bytes.

## TUI keys

| View | Key | Action |
| --- | --- | --- |
| Machines | `n` | Start a shell in an allowed directory. |
| Machines | Up/Down or `j`/`k` | Select a session. |
| Machines | Enter, `a`, or `o` | Observe a session. |
| Machines | `x` | Request termination with confirmation. |
| Machines | `p` | Show approval availability. |
| Machines | `q` | Close the TUI. |
| Terminal | Ctrl-], then `c` | Request control. Confirm a competing takeover. |
| Terminal | Ctrl-], then `o` | Release control and observe. |
| Terminal | Ctrl-], then `d` | Detach while the shell continues. |
| Terminal | Ctrl-], then `i` | Interrupt the foreground turn. |
| Terminal | Ctrl-], then `k` | Request explicit termination. |
| Terminal | Ctrl-], then `q` | Close the TUI while the shell continues. |

Only the active controller sends input or resizes the PTY. Input is never resent automatically. Output and input acknowledgments use separate counters. Terminal mode is restored on normal exit, errors, unwinding, SIGINT, and SIGTERM. SIGKILL cannot run cleanup.

## Scriptable commands

Build the binary once:

```sh
cargo build --locked --package qgenie-remote
target/debug/qgenie-remote up
target/debug/qgenie-remote status --json
target/debug/qgenie-remote run -- /bin/sh
target/debug/qgenie-remote list --json
```

`run` prints the start key before dispatch. The SDK also saves the key and original parameters in private local state. Use `reconcile <idem_key>` to look up an uncertain start without sending a new launch. Use `attach <session_id>` to open the TUI for a session. `send <session_id> TEXT --enter` requests control, sends one input, then releases control. A competing controller requires explicit `--takeover`.

`terminate <session_id> --yes` explicitly kills that PTY session. `stop` refuses to stop the daemon while sessions remain. `stop --terminate-sessions` explicitly terminates them before shutdown. `doctor` reports local availability and unsupported features. `logs` reads bounded daemon diagnostics; it does not print PTY content.

## Scope and persistence

The preview supports Linux/macOS Unix IPC and advertises shell capability only. Linux behavior has been exercised in the managed workspace. macOS checks are configured in CI but have not run here. Windows is unsupported.

SQLite stores start identities, input acknowledgments, and bounded replay in the private state directory. The replay retains at most 10,000 events or 8 MiB per session. Old cursors return truncation. A running PTY has a screen snapshot for reattachment. A daemon crash can lose the native PTY; restart reports lost/uncertain state and preserves start deduplication. A client exit does not stop the daemon.

Local replay can contain terminal output and secrets printed by commands. Raw terminal input is not stored separately. Unenrolled local preview sessions are separate from protected remote recordings. Remote recording, redaction, audited history, retention and holds now have component and composed proofs; production deployment/platform/identity qualification remains incomplete.

WSS/proxy transport, host OAuth/enrollment, directory freshness, gateway audit, protected recording and original-authority native dispatch are implemented. Remote PTY workflows compose through the SDK, gateways and the independently owned daemon. The four-gateway development deployment and service installers are implemented; real user-manager/platform qualification remains separate. Managed Codex discovery/creation, structured control, owner approval delivery and historical resume now pass composed Linux fixtures; the full release fault/platform matrix remains open. Same-user IPC does not protect against another process running as that OS user.

## Development checks

```sh
cargo fmt --all -- --check
cargo check --workspace --all-targets --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo xtask boundaries
cargo xtask schema --check
cargo build --locked --package qgenie-remote
python3 tests/e2e/local_tui_smoke.py target/debug/qgenie-remote
```

Run `cargo xtask schema` after a public contract change. Commit the generated schema and lockfile. The dependency checker enforces the [component graph](components.md). Create a crate only with implemented behavior, a focused public API, and a README that explains persistence, cancellation, limits, and verification.

The managed workspace provides its tools through `/workspace/.remote-session-tools/env.sh`. Source that file before development commands in this environment.

See the [transport API](crates/qgenie-remote-transport/README.md), [native Codex proof](docs/integration/codex-capabilities.md), [gateway setup](crates/qgenie-remote-gateway/README.md), and [development deployment](docs/runbooks/development-deployment.md). The CLI supports local and authenticated remote PTY workflows.

## Design references

- [Task plan](tasks/README.md) and [current progress](tasks/PROGRESS.md).
- [Product requirements](PRD.md) and [architecture](architecture.md).
- [Components](components.md) and [subcrate standard](subcrate-standard.md).
- [Implementation decisions](docs/adr/README.md) and [operation matrix](docs/contracts/operation-matrix.md).


### Remote terminals and owner history

```sh
qgenie-remote remote --gateway https://your-gateway.example \
  --token-file /private/path/current-user-token --ca-bundle /path/to/enterprise-ca.pem
```

The caller supplies a current dedicated remote user token in a private regular file owned by
that OS user; this command stores no credential. `--ca-bundle` is optional when system roots
already trust the gateway. Use Enter to browse machines, sessions and recorded streams;
`v` opens a live terminal, `c` opens the allowed-directory picker for a new `/bin/sh`, and
`k` looks up persisted start identities without resending launches. In the picker, Enter
browses a path, `p` edits it, Backspace opens its parent and Ctrl-N submits one start in the
current directory. Start identities use the global `--state-dir`; their journal contains
scope/key/fingerprint metadata, without argv or credentials. `n` reads another history page,
Esc goes back, `r` reconnects, and `q` exits. Expired tokens
are replaced by reopening with a fresh injected credential. Protected gateway history must
be configured and qualified. The view displays original gaps and redacted text with bounded
memory and independent keyboard/signal handling. Local PTY workflows remain available.

The actual remote keyboard, redaction, gap and SIGTERM terminal-restoration proof runs inside
`tests/e2e/recording_durability.py` against owned synchronous PostgreSQL/Redis/HTTPS fixtures.
The live view begins in observe mode. F2 acquires control (takeover asks for confirmation),
F3 releases, F4 interrupts, F6 asks for termination confirmation, F7 opens owner approvals,
and F8 captures a fresh
checkpoint and F10/Ctrl-] detaches. Input is bounded, sequentially acknowledged and never
resent after uncertainty or reconnect. Native state and recording warnings remain visible.
Closing the view leaves the native session alive. In the machine list, F2 renames, F3 edits
labels (Ctrl-D confirms removal), and F6 confirms remote-access revocation. Revocation keeps
native sessions and local capture alive. `/` filters machines by ID/name/hostname/labels and
sessions by ID/cwd/state/agent/command. Enter applies, Esc cancels the edit and Ctrl-U clears;
hidden rows cannot receive actions. The machine list shows host/platform/CLI/native-agent versions and last seen time; its
owner approval badge reports pending, partial or unavailable status. In the creation picker,
F5 selects an advertised agent.
The agent pane renders structured messages/tools and accepts controlled prompt/steer input.
`a` opens owner-wide approvals; F7 does the same from observe mode without taking control.
In the session list, `u` resumes eligible confirmed-ended agent history in a new session and
selected directory; `k` remains read-only recovery of an original start identity. Lost or
outcome-unknown history is not resumed automatically. See the [MVP evidence report](docs/validation/mvp-evidence.md).

Scriptable `remote rename`, `label`, `remove-label` and `revoke --yes` use the same owner SDK.
`remote send MACHINE SESSION --stdin [--enter]` sends piped input once, with a 32 KiB bound
including optional Enter, after reading it and acquiring control. Unknown mutation outcomes
require explicit inventory/key reconciliation. See [remote commands](docs/integration/remote-commands.md).
