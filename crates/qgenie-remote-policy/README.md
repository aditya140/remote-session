# qgenie-remote-policy

Pure action-level policy decisions.

## Consumers and API

Gateway, host, and sessiond. Direct product dependency: proto.

`authorize(Action, Facts)` returns a stable `RemoteError` on denial.

## Configuration and platforms

No configuration or environment is read by this library. Callers supply trusted identity, access, path, capability, and control facts. All platforms are supported.

## State, cancellation, and limits

Owner approvals do not require control. Existing-session control does not require an in-root cwd. New starts and browsing do. Termination requires explicit confirmation. The crate owns no mutable state and performs no I/O.

## Verification

```sh
cargo test --package qgenie-remote-policy
cargo clippy --package qgenie-remote-policy --all-targets -- -D warnings
```

Use the root README for the real-process and TUI smoke checks. Linux tests have run in the managed workspace. macOS execution is pending CI.
