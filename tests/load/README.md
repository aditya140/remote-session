# Reduced development baseline

## Host resource check

Build the supported Linux host in the release profile. Use the composed fixture to
measure the native owner and connector after the PTY workflow finishes:

```sh
cargo build --locked --release -p qgenie-remote --features managed-codex
QGR_CODEX_BINARY=/absolute/path/to/codex \
  python3 tests/e2e/recording_durability.py --require-managed-codex \
  --host-binary target/release/qgenie-remote --measure-host \
  --report target/validation/release-host.json
```

The measurement requires Linux and the composed fixture prerequisites in
[MVP evidence](../../docs/validation/mvp-evidence.md). Both measured processes use the
selected binary. The report includes its SHA-256 hash. The host has no active sessions
during the 60-second sample. Recording remains enabled. The fixture verifies the
connector route after measurement. The sample follows real PTY work; it is not an empty
process startup measurement. The managed-agent phase runs separately in the same proof.

`host_resources` reports process and aggregate RSS and CPU. It compares aggregate values
with 30,000,000 bytes and 0.5 percent of one core. A `target_not_met` phase is an open
performance finding even if the functional proof passes. One-second RSS samples can miss
short peaks. Child-process memory, socket count, fleet variation, and long-duration
behavior need separate evidence. Do not compare debug and release binaries as if they
were the same deployment configuration.

## Reduced gateway workload

Run the real four-gateway deployment first, then build the standalone CLI and SDK load example:

```sh
cargo build --locked -p qgenie-remote
cargo build --locked -p qgenie-remote-client --example remote_load
python3 deploy/compose/stack.py --client-bin target/debug/qgenie-remote \
  --load-bin target/debug/examples/remote_load baseline
python3 tests/load/storage_model.py deploy/compose/secrets/last-baseline.json
```

The default owns eight persistent WSS clients, two simultaneous real native terminals,
100 unique echo requests per terminal, and a 51,200-byte initial burst per terminal. One
original start key per terminal is persisted before sending; lookup must match. Input is
never resent. The reader fails on a gap/checkpoint instead of counting missing output as
success. All known terminals are explicitly terminated, the private native spool must seal
and reconcile to empty, and protected recording sizes come from the scoped stored receipts.
No benchmark token, input body, terminal content or machine identity enters the report.

`baseline` stores a private `secrets/last-baseline.json` and prints aggregate values. Its
private diagnostics remain in `secrets/last-smoke.log`. The two-second idle CPU sample is
short and tick-quantized. Process RSS after the workload is not a peak measurement. Docker
stats are a final snapshot, not a peak or averaged workload CPU trace. WAL includes authority
and audit writes as well as recording. The latency is SDK input submission through native
echo observed by a one-millisecond replay poll. It includes input ACK and replay request
costs; no local-PTY baseline was subtracted. It is not a claim about pure network overhead
or interactive TUI latency; supporting TUI peers now use pushed heads with bounded idle reconciliation.

Options before `baseline` select `--load-sockets`, `--load-terminals` and `--load-samples`.
The SDK example accepts an owned private JSON configuration and can also run against an
explicitly prepared synthetic deployment. Source fields are in `examples/remote_load.rs`.
Admission is paced; it can retain up to 200,000 clients and 2,000 native terminals, with at
most 32 concurrent echo workers and two million measured samples. These are tool bounds,
not demonstrated capacity. The current composed wrapper has a ten-minute total deadline.
Its current owner token must remain valid for the whole measurement. Startup/admission,
resource exhaustion, expiry, replay gaps and uncertain cleanup fail visibly.

Full beta evidence still needs distributed generators, 20,000 simultaneous structured
streams, 2,000 simultaneous terminal burst producers at 50 KB/s, 200,000 authenticated
sockets, representative corporate network/identity traffic, time-series peaks, approval
delivery latency and database write-lag distributions. Do not infer those from eight
sockets or change retention to fit an estimated disk budget.

`storage_model.py` uses measured normalization/compression and WAL factors, explicit duty
cycle, about 120-day monthly-rounded ordinary retention, additional held copies, replicas,
full backup copies and a stated row/index factor. It also prints an uncompressed-normalized
scenario. Indefinite audit/receipt tombstones and unlimited holds need a separate horizon;
the model names these exclusions instead of presenting a finite total as a capacity limit.
