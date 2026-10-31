# qgenie-remote-tui

Interactive machine/session and PTY interface.

## Consumers and API

Standalone CLI. Direct product dependencies: proto and client.

Async `run(client, cwd, initial_session)` uses the caller runtime. Remote callers use
`run_remote`, `run_remote_with_state`, or `run_remote_authenticated`. The last accepts
caller-owned credential updates. `render_remote_snapshot` returns bounded safe plain text
for CLI reads.

## Configuration and platforms

Uses ratatui/crossterm and a VT100 screen on Linux/macOS terminals. It makes no direct daemon, database, or native-agent call.

## State, cancellation, and limits

Views separate observation from control. Ctrl-] d detaches. Takeover and termination require confirmation. Raw mode and the alternate screen have a cleanup guard. Owned signal handling also restores the terminal on SIGINT/SIGTERM. Local replay/recording limits remain visible. Remote views support structured agents, durable approvals and protected recorded history.
Temporary connection loss starts bounded observation recovery. The pane retains its output
cursor and discards control, confirmations and queued input. F2 requests control again.
The session list shows pending approvals. `!` filters the current page to pending approvals,
failed or uncertain lifetime, and recording warnings. Partial approval scans remain explicit.

## Verification

```sh
cargo test --package qgenie-remote-tui
cargo clippy --package qgenie-remote-tui --all-targets -- -D warnings
```

Use the root README for the real-process and TUI smoke checks. Linux tests have run in the managed workspace. macOS execution is pending CI.
