# qgenie-remote-transport

Bounded WSS and private same-user Unix IPC. Direct product dependency: proto. Consumers are client, host, sessiond, and gateway; authorization and operation reconciliation stay with their callers.

## WSS

Enable `client-ws` and/or `server-ws`. `ws::connect(ClientConfig, Limits, metrics)` makes one connection attempt. `ws::accept` accepts a caller-owned, authenticated stream, including a TLS stream. `Connection::adopt` also accepts an authenticated framework socket through the exported socket message types. Callers authenticate upgrades before handing an accepted server stream to this transport.

Client TLS uses native trust plus an optional PEM CA bundle, verified by rustls. `ClientConfig::new` snapshots HTTPS_PROXY/https_proxy and NO_PROXY/no_proxy. HTTP and HTTPS CONNECT proxies support redacted Basic credentials. NO_PROXY supports domain boundaries, ports, IPs, and CIDRs. URL/header diagnostics redact credentials and query strings. WSS is required; an explicit development option permits direct connections whose actual TCP peer is loopback.

Each connection owns its driver task. Drop cancels that task; `close().await` flushes a normal close with a write deadline. `cancellation().cancel()` independently aborts the driver/queues while its owner or socket writer is blocked. Dropping that cancellation handle has no effect. No background reconnection or mutation retries occur. `Backoff` supplies jittered delays only. Heartbeats default to 20 seconds with a five-second pong deadline; intervals cannot exceed 25 seconds.

`sender().try_send(priority, frame)` validates the envelope/binary frame and returns a `Transmission`. Awaiting `transmitted()` proves a network write, not authorization or native application. A full/closed queue returns `not_sent`; interrupted writes/unsent queued receipts after connection failure return `unknown`. Callers reconcile original operation identities. Priorities use an 8-control/4-event/1-bulk weighted schedule, with separate byte/count quotas. Defaults are 128/1 MiB control, 512/2 MiB event, 256/8 MiB bulk. JSON and WebSocket messages are capped at 1 MiB; PTY payloads at 32 KiB. No compression extension is enabled.

Incoming frames hold their byte quota until dropped. `Limits.shared_outgoing` adds per-service control/event/bulk byte budgets across connections; `shared_incoming` adds a service receive budget. Shared bulk saturation cannot borrow control capacity. A consumer exceeding its count or byte limit is disconnected with `SlowConsumer` and must resnapshot/replay. Writes have a five-second deadline. One slow connection cannot block another driver. Optional metrics callbacks receive bounded event metadata, run inline, and must not block; no global logging subscriber is installed.

## IPC

The independent `elapsed-clock` feature exposes a Linux/macOS clock that includes machine suspension without selecting IPC or WebSockets. Sampling failures/regression remain errors until process restart; no weaker clock fallback or later deadline revival occurs. Sampling is serialized to avoid mistaking concurrent return order for clock regression. This is a mechanical clock; authority decisions and grant state remain with callers.

Default `local-ipc` supports Linux/macOS Unix peers; disabling defaults excludes the IPC implementation and fs2. Windows pipes/ConPTY are reserved future boundaries.

`ipc::Listener`, `connect`, `client_handshake`/`server_handshake`, `read_dispatch`/`write_dispatch` provide independently versioned dispatch framing. Dispatch cannot encode administrative lifecycle operations. Administration uses a separate socket/schema. The daemon caps dispatch connections at 64 and admin connections at four; the SDK caps pending requests at 128. Every new dispatch connection negotiates IPC major 1 before sending an operation.

Directories require mode 0700 and files/sockets 0600. Directory ancestors, file ownership, no-follow opens, socket ownership, and actual peer UID are checked. Linux/macOS peer checks use Tokio's SO_PEERCRED/getpeereid support. Instance locks prevent cooperating duplicate owners. Existing sockets must fail a nonblocking liveness probe before stale cleanup; uncertain/live sockets are preserved. Listener drop removes only its original socket inode. Lengths are checked before allocation, with ten-second frame deadlines. A cancelled/partial framed operation requires dropping its connection.

## Verification

```sh
cargo test -p qgenie-remote-transport --all-features
cargo clippy -p qgenie-remote-transport --all-features --all-targets -- -D warnings
cargo check -p qgenie-remote-transport --no-default-features --features client-ws
cargo check -p qgenie-remote-transport --no-default-features --features server-ws
```

Linux verification includes real trusted/untrusted TLS, HTTP/HTTPS CONNECT, proxy rejection/bypass, a real 65-second connection through a 60-second-idle proxy, dropped pong, stalled writer, malformed/oversized frames, byte/count saturation, fair priorities, slow-observer isolation, cancellation, shutdown, IPC negotiation, stale/live sockets, and partial reads. macOS is configured in CI but has not run in this workspace. `bash tests/e2e/foreign_uid_ipc.sh` passed actual UID 0/1001 directory-permission and independent client/server peer-credential checks in an isolated container with no host mounts, network, or privileged flag.
