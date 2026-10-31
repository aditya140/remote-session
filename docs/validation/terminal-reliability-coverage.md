# Terminal reliability coverage

This audit addresses issue #6. It distinguishes existing coverage from newly
added regressions. The scenarios came from the inspected Herdr source. They were
candidate tests, not evidence that each defect existed in this project.

| Scenario | Existing evidence | Added evidence |
| --- | --- | --- |
| Slow reader and unrelated progress | The transport WSS test disconnects a slow observer while another connection makes progress. The PTY adapter tests bounded output and resize. | Two real native PTYs through HTTPS, the gateway, connector, and native owner. One SDK observer stops reading until its eight-entry queue reports overload. The other PTY accepts input and resize. |
| Replacement and late close | `routing_contract` checks host replacement. `presence_refresh_cleanup` proves that late old refresh and cleanup cannot erase the newer generation. | A new real output observer receives native completion after the stalled old observer closes. |
| Final output and recording | Native capture tests cover durable output, explicit spool loss, and retained gaps. The composed fixture checks authenticated history and primary-volume loss. | Under paced output pressure, native replay places the final marker before `Exited`. The authenticated, paginated history path must contain the final marker. A gap fails this normal no-loss fixture. |
| Viewer resize and control | The terminal handler gates resize on current ownership. Existing tests exercise controller takeover and stale input refusal. | Two real TUI observer connections change their window sizes and request checkpoints while an SDK controller holds its lease. Native geometry stays unchanged. The controller then changes it successfully. |
| Disconnect and native lifetime | Existing native and deployment tests preserve sessions across caller loss, connector replacement, pause, and revocation. | The healthy PTY remains running after the stalled observer disconnects. |
| Actual new lifetime after loss | Native process crash tests preserve original start keys and explicit lost or unknown results. The pinned Codex owner-SIGKILL test does not restart an uncertain key. | The PTY crash-boundary test explicitly starts a new process with a new key. It checks a distinct session ID and retains the old lost or unknown record. |
| Recording owner lock | The spool reopen test occasionally failed during parallel process creation. | A deterministic duplicate-descriptor test reproduces the retained lock. The lock guard now unlocks explicitly after SQLite closes. |

The remote terminal has no theme-change action. We did not invent a theme test or
claim that a nonexistent action was exercised. Rendering and window events do not
supply a control lease. A future theme action must preserve that boundary.

## Reproduced defect and fix

The spool previously relied on closing its file to release the exclusive lock.
A descriptor duplicated during concurrent process creation can keep the same
open file description alive until the child closes or executes. This caused an
immediate reopen to fail after the original owner closed.

The regression duplicates the descriptor, verifies that the live owner excludes a
second owner, closes the first owner, and opens a successor while the duplicate
still exists. It then closes the old duplicate and verifies that the successor
still excludes other owners. The test failed before the fix with an already-owned
error. It passed afterward.

A small private lock guard now performs explicit unlock. Field drop order closes
SQLite before the guard releases the lock. Normal ownership and exclusive-open
checks remain in place. This does not implement native-owner handoff or recovery
from an arbitrary process crash.

## Bounds, ownership, and validation

The pressure proof uses 80 paced 4 KiB writes and the existing bounded observer.
It has a 35-second outer deadline and bounded history pagination. The recording
fixture owns the native daemon, child processes, gateways, and database. The
viewer subprocess has a 30-second timeout and kill-on-drop cleanup. The valid
controller renews its lease while the viewer probe runs.

The two-session block passed in 6.51 seconds on this Linux workspace. The viewer
probe adds two real TUI connections; its standalone overhead was not measured.
The full deployment result and workspace test counts are recorded in
[the implementation log](../../tasks/038-herdr-follow-up-pull-requests.md).
No production, native macOS, Windows, or native-owner upgrade qualification is
claimed by these Linux results.

Relevant files:

- [Composed pressure proof](../../apps/qgenie-remote/tests/support/terminal_pressure.rs).
- [Real viewer probe](../../tests/e2e/remote_viewer_resize.py).
- [Gateway routing tests](../../crates/qgenie-remote-gateway/src/routing/tests.rs).
- [Transport WSS tests](../../crates/qgenie-remote-transport/tests/wss.rs).
- [Spool regression](../../crates/qgenie-remote-sessiond/src/recording/spool/tests.rs).
- [Native crash boundaries](../../crates/qgenie-remote-sessiond/src/runtime/tests.rs).
- [Earlier Herdr comparison](herdr-core-review-2026-10-31.md).
