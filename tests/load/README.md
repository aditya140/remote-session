# Reduced development baseline

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
