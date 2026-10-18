# Standalone QGenie Remote composition

This binary composes the local SDK, session owner, and TUI. It is the user-authorized replacement for the absent corporate CLI during local development.

Run `cargo run --package qgenie-remote -- tui` from the workspace. Use `--help` for scriptable commands. The root README documents roots, private state, lifetime, and recovery limits.

The daemon runs as an independent process with two Tokio workers. It owns PTY reader threads and state. The TUI can exit without terminating sessions. This process launcher does not establish systemd or launchd isolation.

`cargo test --package qgenie-remote` runs separate daemon-process tests. `python3 tests/e2e/local_tui_smoke.py target/debug/qgenie-remote` exercises the real keyboard interface and terminal restoration after building the binary.
