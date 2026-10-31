# qgenie-remote-proto

Runtime-independent wire contracts.

## Consumers and API

Host, sessiond, transport, policy, client, TUI, and gateway. No product dependency.

`Request`, `Response`, typed IDs, `Counter`, `PtyFrame`, `Hello`, and `MachineChallenge`.

## Configuration and platforms

The `schema` feature exports public JSON Schema. Administrative IPC types stay outside the public schema. The crate has no runtime, database, key storage, or native-agent dependency.

## State, cancellation, and limits

JSON is capped at 1 MiB and 64 nesting levels. PTY payloads are capped at 32 KiB. Counters serialize as canonical decimal strings. This crate owns no persistent state or retry loop.

`AgentEvent::TurnCompleted` includes an optional typed `outcome`: `completed`, `failed`,
or `interrupted`. The event means that a turn ended. Successful completion requires
`outcome: completed`. Old events omit the field and remain readable with an unknown outcome.
Native lifetime is separate. Additive event fields do not change public protocol v1.
The conformance fixtures retain both old and new event shapes.

## Verification

```sh
cargo test --package qgenie-remote-proto
cargo clippy --package qgenie-remote-proto --all-targets -- -D warnings
```

Use the root README for the real-process and TUI smoke checks. Linux tests have run in the managed workspace. macOS execution is pending CI.

Normalized recording v1 JSONL uses `RecordingRecord`/`RecordedContent`, a separate `cargo xtask recording-schema` schema, bounded base64 binary output/fragments, capture and elapsed time. There is no raw-input variant. `RecordingEpochId` distinguishes stream lifetimes. Batches contain one UTC capture month and hash exact compressed bytes. Content receipts match original sequence/capture bounds and epoch. Gap receipts have a distinct `GapId` target and hash canonical sorted-key JSON of the entire immutable loss range. Authenticated backend recovery epoch is a separate authority, never learned solely from a receipt. Public fixtures include normalized compressed content and a typed gap receipt.
