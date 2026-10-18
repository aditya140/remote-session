# qgenie-remote-adapters

Native execution and terminal state, consumed by sessiond. The only direct product dependency is proto. The gateway and client do not link this crate.

`PreparedPty::new(StartSpec, LaunchEnvironment)` validates launch parameters and reports capabilities. `launch()` returns a `PtyProcess` with an independent bounded-chunk reader, serialized command handle, and native-independent exit facts. `spawn(&StartSpec)` supplies the default environment. `LaunchEnvironment` injects host-only values with a 64 KiB total cap and redacted Debug output.

Default `pty` uses portable-pty on Linux/macOS. Input writes have a 100 ms retry budget; read chunks are at most 32 KiB, with no unbounded adapter output queue. Sessiond owns up to 64 draining threads and bounded incoming dispatch. Resize and interrupt are distinct from explicit termination. Native input supports an owner-supplied monotonic deadline checked before each nonblocking write. Prepared directory capabilities remain pinned through the child’s validated fchdir/exec, including relative executable lookup. Termination signals owned foreground/root process groups; reaping and signaling share a lock. Viewer loss does not own cleanup. `Screen` owns ANSI/UTF-8/alternate-screen state and up to 64 scrollback lines (maximum 786432 cells including alternate screen, fixed-size vt100 cells). The pinned parser patches cap OSC storage at 4 KiB and expose a validated `vt100_state_v1` checkpoint preserving both grids, saved cursor/modes, scrollback, and incomplete parser input. Checkpoints cap at 8 MiB and fail explicitly when unavailable. Original output remains unchanged for replay/recording. See [parser patches](../../vendor/README.md). Public cursors, replay, recordings, and persistence stay in sessiond.

Optional `codex` provides `Codex::launch`, thread start/resume, prompt/steer/interrupt, bounded events, guarded command approval responses, and shutdown. It pins native 0.159.2 and uses owned Unix IPC. See the [native capability matrix](../../docs/integration/codex-capabilities.md) for proven operations, the actual relay-disable interface, credential injection, limits, and uncertainty behavior. Windows and existing-agent adoption are deferred.

```sh
cargo test -p qgenie-remote-adapters --all-features
cargo clippy -p qgenie-remote-adapters --all-features --all-targets -- -D warnings
cargo check -p qgenie-remote-adapters --no-default-features
cargo check -p qgenie-remote-adapters --no-default-features --features codex
```

Linux PTY/screen and actual pinned Codex proofs have run. macOS execution remains pending; CI checks feature isolation and fixture tests. Native Codex tests require an explicitly selected pinned binary and are not silently replaced by fake fixtures.
