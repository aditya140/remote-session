# qgenie-remote-tui

Interactive machine/session and PTY interface.

## Consumers and API

Standalone CLI. Direct product dependencies: proto and client.

Async `run(client, cwd, initial_session)` uses the caller runtime.

## Configuration and platforms

Uses ratatui/crossterm and a VT100 screen on Linux/macOS terminals. It makes no direct daemon, database, or native-agent call.

## State, cancellation, and limits

Views separate observation from control. Ctrl-] d detaches. Takeover and termination require confirmation. Raw mode and the alternate screen have a cleanup guard. Owned signal handling also restores the terminal on SIGINT/SIGTERM. Local replay/recording limits remain visible. Structured agents and gateway flows are not implemented.

## Verification

```sh
cargo test --package qgenie-remote-tui
cargo clippy --package qgenie-remote-tui --all-targets -- -D warnings
```

Use the root README for the real-process and TUI smoke checks. Linux tests have run in the managed workspace. macOS execution is pending CI.
