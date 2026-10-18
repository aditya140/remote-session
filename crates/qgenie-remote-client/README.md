# qgenie-remote-client

UI-independent Rust SDK for private local sessions and verified remote sessions. Product dependencies are proto and transport; no widgets, native adapters, OAuth implementation or credential store are embedded. The caller owns Tokio and supplies current credentials.

## Connect and read

`GatewayClient::new(GatewayConfig)` accepts a canonical HTTPS origin and optional CA bundle. REST methods receive an injected `AccessToken`. `connect` opens verified WSS and negotiates capabilities before operations are sent. Redirects and automatic HTTP retries are disabled.

`machines`, `history_streams`, `history`, `admin_history_streams` and `admin_history` use separate typed owner/admin paths. Admin reasons/cases are request bodies. Sixteen HTTP requests and four blocking history decoders are admitted. Decoders verify the stored SHA-256, one zstd frame, bounded normalized records, fragments, sequence and capture ranges. Returned bytes remain untrusted presentation data.

`LiveClient` owns a WSS read driver. Its cloneable `LiveHandle` provides machine/session inventory, scoped browse, original-key lookup, attach/capture/replay, approvals and explicit reauthentication. Call `next` to drain unsolicited events. Dropping the connection owner stops its driver; pending mutations remain uncertain.

## Own an output observer

`GatewayClient::observe_session(token, machine, session)` opens a dedicated connection. Alternatively, `LiveClient::into_observer(machine, session)` transfers an existing connection. The returned `RemoteObserver` installs a verified checkpoint, deduplicates continuation output and reports gaps, truncated history and unconfirmed tails. It reacts immediately to pushed native head/session changes and performs read-only continuation checks, including a 250 ms idle check to discover a lost final notification. Supporting peers negotiate `session-changes-v1`; `session_changes_available()` exposes that support. Native content still crosses bounded checkpoint/replay reads.

Consume `next()` for `ObservedOutput` values. At most eight updates and 16 MiB of retained update bytes are admitted; an update holds its byte permit until dropped. Slow consumers close with an actionable `Overloaded` error. The final error is retained outside the queue, returned once by `next()` and remains available through `failure()`.

`handle()` allows explicit owner/controller operations and caller-supplied token refresh on the same connection. The observer owns its target attachment; use `close()` rather than manually detaching that target. Closing/dropping an observer cancels its reads and leaves native execution running. Opening another observer installs a new checkpoint. Reconnect, refreshed credentials and controller reacquisition are explicit; neither input nor approval answers are replayed.

## Start and control

`LiveHandle::start(state_dir, owner, machine, launch)` persists an immutable scoped key and launch fingerprint before dispatch. PTY and advertised agent starts use the same journal. Journal files contain no argv, prompts or credentials. Changed parameters fail with `NotSent`. Retain the returned identity and use `lookup_start` after uncertainty; a reconnect never creates another start automatically. Generic `request` refuses mutations.

`take_control` returns a `RemoteController` tied to the exact connection, machine, session and native-confirmed host/controller generations. It supports explicit renewal, takeover, release, input, resize and interrupt. Agent prompt/steer require advertised support. Input is poisoned before awaiting delivery; cancellation, loss or stale authority requires explicit reacquisition. Renewal and token refresh cannot clear an uncertain input or resend it. Owner termination requires explicit confirmation and no control lease.

Historical agent resume uses a new start identity with a native resume ID. The managed host permits only confirmed ended history with matching private native-home provenance. It creates a new native process/session and uses the newly selected scoped cwd. Lost or outcome-unknown history cannot be silently adopted or relaunched.

## Owner actions and approvals

`patch_machine` and explicitly confirmed `revoke_machine` work without an online host or controller. Lost/malformed/wrong-target success replies remain `Unknown`; reconcile inventory before another mutation. Revocation removes remote access while native execution/capture remain on-device.

Approval index/status reads and one-shot resolution require negotiated approval capabilities, current owner access and an offered decision, without a controller lease. A committed winner is separate from native application. Lost answer replies remain `Unknown`; reread durable status rather than resend. Unsupported/unoffered choices fail before sending a frame.

## Resource bounds and recovery

The WSS driver admits 128 requests, 64 queued commands and 8 MiB of pending request bytes. Ten-second response budgets are not refreshed by queued work. Its event queue is bounded at 64 events/8 MiB; a slow raw event consumer receives `ResyncRequired`. Correlation IDs cannot be reused (65,536 retained IDs per connection). Late/duplicate responses cannot complete another request.

Four concurrent attachment assemblies share a 16 MiB response budget. Each fixed checkpoint has ordered matching pages, a 25-second deadline and an 8 MiB/32-page bound. Malformed or incomplete assembly fails before bytes reach a renderer. Read-only recovery never repeats a native mutation. Typed errors preserve `NotSent`, `Unknown` and `Rejected` outcomes.

Local `Client::local(state_dir)` provides start, lookup, attach, replay and control over private same-user IPC, with 128 calls and 16 journal workers admitted.

## Verification

```sh
cargo test -p qgenie-remote-client
cargo clippy -p qgenie-remote-client --all-targets -- -D warnings
```

Twenty-four SDK tests pass on Linux, including real TLS fixtures for reversed/duplicate replies, PTY/agent lost-start replies, poisoned input, refresh during uncertainty, approval loss, malformed checkpoints, observer final-tail delivery, cancellation and slow consumers. The owned synchronous PostgreSQL/Redis/two-gateway/native fixture additionally exercises actual starts, controls, structured output, approvals, historical resume and protected history. macOS and the full release fault matrix remain task 027 evidence gates.
