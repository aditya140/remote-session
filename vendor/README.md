# Pinned dependency patches

The exact crates.io sources of vt100 0.16.2 and vte 0.15.0 are retained with their licenses. Each `UPSTREAM.json` records the original crate archive SHA-256. Cargo patches these sources locally; gateway-only builds do not depend on the terminal parser.

The `checkpoint` feature adds bounded serialization and validation of complete terminal state. It includes both grids, saved cursor/attributes, scroll regions, origin/input modes, scrollback and the VTE parser's pending UTF-8/escape/string state. Cells serialize only their live text and flags. VTE OSC storage is capped at 4096 bytes with the std feature. Two native rendering edge cases are fixed: wide glyphs in one-column terminals render a replacement, and shrinking a row clears a cut wide glyph.

`QGVT1` followed by Postcard serialization of `(vte::Parser, vt100::Screen)` is the `vt100_state_v1` snapshot format. Field/variant order, integer encoding, the live-cell representation and the parser versions form this format's contract. Change the format version when that contract changes. Serialized state is at most 8 MiB; restoration checks dimensions, indexes, glyph/continuation pairs, attributes, parser buffers and complete consumption before accepting it. Oversized state returns an explicit error. No pointer or process memory is serialized.

Native adapter tests exercise every cut in a terminal stream, restored alternate-screen exit and saved state, corrupted/truncated checkpoints, bounded unterminated OSC, Unicode resizing and the checked-in golden checkpoint. `cargo run -q -p qgenie-remote-adapters --example terminal_checkpoint` emits that golden checkpoint. Replay and IPC tests cover persistence and page assembly separately. Upstream source remains otherwise unchanged; these patches must be reviewed when upgrading either parser.

## Atomic PTY cwd

portable-pty 0.9.0 is also pinned with its upstream archive checksum and license. The Unix command builder can hold an owned directory capability above stdio. The child's pre-exec hook checks directory ownership/permissions and calls `fchdir` before closing descriptors. With this capability, relative executable/PATH resolution occurs after `fchdir`; the logical cwd pathname is never reopened or used for fallback. The capability can be cloned within the process but serialization is refused. Existing builders without a directory capability retain upstream behavior.

Actual PTY tests rename the validated cwd, substitute a symlink to another directory containing a different relative executable, and verify only the held directory's program executes. Another test changes permissions before exec and verifies refusal without a native marker. These patches support Linux/macOS Unix code; macOS execution remains an external proof gate.

## PostgreSQL cancellation

sqlx-postgres 0.9.0 is retained with both upstream licenses and its original archive SHA-256
in `UPSTREAM.json`. PostgreSQL disables ordinary statement/transaction timeouts during a
synchronous COMMIT wait. SQLx 0.9 retains the connection cancellation key internally but
does not expose a cancellation API. The local patch adds an opaque, redacted `PgCancelToken`
and uses the driver's existing connection/TLS policy to send a PostgreSQL CancelRequest
without a new login or an additional SQL backend. This also addresses PgBouncer's virtual
frontend cancellation key in transaction mode. No TLS policy is weakened.

Each cancellation token belongs to one checked-out frontend and must not survive its reuse.
The gateway retains the connection while handling cancellation, marks it for retirement and
never returns it to its pool afterward. A canceled synchronous wait may report local COMMIT
success; the gateway always treats a timed-out/canceled commit as uncertainty and returns no
reclaimable receipt or audited transcript page. Owned, pool-bounded commit tasks continue
cleanup after a caller disconnects and are joined during explicit shutdown. A redundant
upstream JSON re-export warning is suppressed without changing behavior.

The synchronous pair harness exercises paused replication, stable retry barriers, actual
production hold/audit/job paths and loss of the primary's disposable tmpfs volume. Its full
gateway proof and database-only probe pass on the disposable local pair. Direct and
PgBouncer 1.24.1 cancellation, verified TLS (including rejection of an untrusted CA),
and delayed cancellation keys against replacement frontends pass. Hard close drops the
socket immediately instead of waiting for a TLS shutdown handshake; the size-one pool
replacement test catches a stalled retirement. These are local protocol/fault proofs;
independent physical fault domains and backup restore qualification remain outstanding.
Retire this patch when upstream provides
an equivalent supported cancellation API and rerun these proofs on upgrade.
