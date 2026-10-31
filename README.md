# QGenie Remote

The standalone Rust CLI/TUI supports local shell sessions and authenticated remote PTY and managed Codex workflows. A separate session daemon owns native processes, replay and recording. Closing the TUI leaves sessions running. Reattach to observe output or request input control.

The supported Linux journey has development acceptance evidence. The host resource budget,
real enterprise access, user-service lifetime, and pilot qualification remain open. See the
[overall completion plan](tasks/030-complete-linux-release-readiness.md) for the ordered
remaining work. The earlier 28 tasks are complete within the user-selected development scope,
with external release gates documented separately. [Progress](tasks/PROGRESS.md) records the implementation and
executed evidence. [ADR 0007](docs/adr/0007-standalone-preview.md) authorizes the new TUI
without the existing corporate CLI source. See [external release gates](docs/validation/external-release-gates.md)
for unrun tenant, platform, production operations and beta-scale qualification.

The [BeautifulRust review](docs/validation/beautiful-rust-review-2026-10-26.md) ranks the
remaining Rust API and readability work. Its [pinned skills](.agents/skills/README.md)
are installed for this repository.

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

The [core-journey repair plan](tasks/029-stabilize-core-journey.md) records the current
completed implementation, ordered steps, acceptance checks, and external pilot gates.
The [overall completion plan](tasks/030-complete-linux-release-readiness.md) continues
with dispatch clarity, required regression coverage, host resource measurements, and
real-environment qualification.

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

The [Bend specification suite](specs/README.md) checks selected product laws and compares
authorization and selected SDK decisions with Rust. Follow the
[verification instructions](specs/VERIFY.md) to run it.
These model checks complement the Rust, native process, and deployment tests.

See the [transport API](crates/qgenie-remote-transport/README.md), [native Codex proof](docs/integration/codex-capabilities.md), [gateway setup](crates/qgenie-remote-gateway/README.md), and [development deployment](docs/runbooks/development-deployment.md). The CLI supports local and authenticated remote PTY workflows.

## Design references

- [Task plan](tasks/README.md) and [current progress](tasks/PROGRESS.md).
- [Product requirements](PRD.md) and [architecture](architecture.md).
- [Components](components.md) and [subcrate standard](subcrate-standard.md).
- [Implementation decisions](docs/adr/README.md) and [operation matrix](docs/contracts/operation-matrix.md).


### Remote terminals and owner history

```sh
qgenie-remote remote setup --gateway https://your-gateway.example \
  --oauth-config /private/oauth.json --ca-bundle /path/to/enterprise-ca.pem
qgenie-remote remote
```

Setup saves connection settings in the selected `--state-dir`. Each new invocation signs
in through browser PKCE, or a device code with `--device`. The running client refreshes its
user credentials in memory. It never saves access or refresh tokens. Automation can still
supply `--token-file /private/current-user-token`. The file must be a private regular file
owned by the current OS user. `--ca-bundle` is optional when system roots already trust the gateway. Use Enter to browse machines, sessions and recorded streams;
`v` opens a live terminal, `c` opens the allowed-directory picker for a new `/bin/sh`, and
`k` looks up persisted start identities without resending launches. In the picker, Enter
browses a path, `p` edits it, Backspace opens its parent and Ctrl-N submits one start in the
current directory. Start identities use the global `--state-dir`; their journal contains
scope/key/fingerprint metadata, without argv or credentials. `n` reads another history page,
Esc goes back, `r` retries a connection, and `q` exits. Observation reconnects automatically
after temporary loss, with at most six attempts per recovery. Denied access stops recovery.
A failed credential refresh requires a new sign-in. Token-file users supply a new token. Protected gateway history must
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
owner approval badge reports pending, partial or unavailable status. In the session list,
`!` filters the current page to pending approvals, failed or uncertain lifetime, and recording
warnings. The agent pane shows typed activity separately from process lifetime. In the creation picker,
F5 selects an advertised agent. When no valid directory is selected, the picker offers the host's allowed roots.
F1 returns to root selection; `p` accepts a manual path when an older peer cannot list roots.
Session lists open on live work. In the remote list, `b` switches live/history, `n` advances
one page, and Home returns to the first page. The local list uses `h`, `]`, and `[`.
Filtering applies to the current page. Completed history does not consume live capacity.
The agent pane renders structured messages/tools and accepts controlled prompt/steer input.
`a` opens owner-wide approvals; F7 does the same from observe mode without taking control.
In the session list, `u` resumes eligible confirmed-ended agent history in a new session and
selected directory; `k` remains read-only recovery of an original start identity. Lost or
outcome-unknown history is not resumed automatically. See the [MVP evidence report](docs/validation/mvp-evidence.md).

`remote read MACHINE SESSION` prints a bounded, safe text checkpoint. `remote events` emits
bounded JSON lines. `remote activity` reports typed activity and durable approval status.
`remote wait MACHINE SESSION --turn TURN_ID --timeout 300` waits for that exact turn;
`--exit` waits for native exit. Timeout leaves work running. These commands never take control.
See [remote commands](docs/integration/remote-commands.md) for bounds and result meanings.

Scriptable `remote rename`, `label`, `remove-label` and `revoke --yes` use the same owner SDK.
`remote send MACHINE SESSION --stdin [--enter]` sends piped input once, with a 32 KiB bound
including optional Enter, after reading it and acquiring control. Unknown mutation outcomes
require explicit inventory/key reconciliation. See [remote commands](docs/integration/remote-commands.md).

## Faster development builds

An optional pinned ra-jit launcher supports repeated development edits:

```sh
./tools/ra-jit setup
./tools/ra-jit build -p qgenie-remote --locked
./tools/ra-jit status
./tools/ra-jit stop
```

It uses separate build output and Rust 1.99.0. Normal Cargo commands and CI use Rust
1.99.1. The measured warmed edits took 0.33–0.43 seconds; the first JIT edit took 9.10 seconds.
Keep the normal Cargo checks: this pinned JIT has no borrow checker. See the
[setup, measurements, and limits](docs/development/ra-jit.md).

## Editor development container

The existing configuration in [.devcontainer](.devcontainer/devcontainer.json) provides these tools and services.
This editor container has not been built or tested here.
It is separate from the exercised [four-gateway development stack](docs/runbooks/development-deployment.md).

| Component | Purpose |
| --- | --- |
| Rust, Cargo, rustfmt, and Clippy | These tools build, format, and check Rust code. |
| PostgreSQL 18.6 | This service stores local development data. |
| Redis 8.2.10 | This service provides local routing and presence data. |
| SQLite tools and libraries | These tools support local session storage development. |
| tmux | This tool supports terminal session development. |
| Git, ripgrep, jq, curl, and native build tools | These tools support source changes and diagnostics. |

The `dev` service uses the `vscode` account.
It mounts the repository at `/workspaces/remote-session`.
PostgreSQL and Redis use the private Compose network. They do not publish host ports.
The configuration starts these services but does not start a gateway or host connector.

PostgreSQL keeps its data in a named volume.
Redis does not keep data after its container stops.
Cargo stores build output inside the development container.
A container rebuild can remove Cargo caches and build output.

### Open the environment

The configuration requires a compatible container runtime and a Dev Containers client.

1. Start the container runtime.
2. Open this repository in VS Code with the Dev Containers extension.
3. Select **Dev Containers: Reopen in Container** from the command palette.
4. Use the container terminal for development commands.

### Service configuration

The `dev` service receives these environment variables:

| Variable | Development value |
| --- | --- |
| `QGR_DATABASE_URL` | `postgres://qgenie_dev:local_dev_only@postgres:5432/qgenie_remote` |
| `QGR_REDIS_URL` | `redis://redis:6379/0` |
| `QGR_LISTEN` | `0.0.0.0:8080` |
| `QGR_ADMIN_LISTEN` | `0.0.0.0:9090` |

These variables are legacy development defaults.
The implemented gateway uses explicit flags and private secret files.
Use the [gateway setup](crates/qgenie-remote-gateway/README.md) to configure it.
The editor can forward ports 8080 and 9090 when an application uses them.

Use the supplied password only for local development.
The `qgenie_dev` account is a database administrator.
Tests for row-level security require separate application roles and policies.
Gateway migrations and the disposable database runners define separate test roles and policies.

### Current limits

The container image supplies a development toolchain.
The standalone workspace selects Rust 1.99.1 and edition 2024 in `rust-toolchain.toml` and `Cargo.toml`.
[ADR 0007](docs/adr/0007-standalone-preview.md) records the user decision to proceed without corporate CLI source.

The container does not include QGenie credentials or native coding-agent installations.
It does not provide systemd, launchd, or native credential-store integration.
Tests for host service behavior require the applicable native operating system.

The configuration follows the [Dev Container specification](https://containers.dev/implementors/json_reference/).
Its Rust base comes from the [Dev Container Rust image](https://github.com/devcontainers/images/tree/main/src/rust).
