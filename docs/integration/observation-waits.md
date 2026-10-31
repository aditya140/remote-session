# Fresh output and multiple task waits

These commands observe existing sessions. They do not request control, submit
input, create tasks, answer approvals, or terminate sessions. Cancellation and
timeout leave native work running.

## Fresh output

Use an explicit event cursor from a prior observation:

```sh
qgenie-remote remote wait-output MACHINE_ID SESSION_ID --after 42 --literal 'Listening on' --timeout 30
```

Only printable PTY text emitted after cursor 42 can satisfy this wait. The match
can span output chunks and UTF-8 boundaries. The existing bounded terminal parser
removes escape sequence payloads. The result identifies the machine, session,
selected boundary, and last observed sequence. A literal match is evidence that
text was emitted. It does not prove application readiness or grant permission.

The matcher parses retained events from sequence zero to establish escape parser
state. It discards text at or before the selected boundary. This prevents an OSC
sequence that starts before the boundary from exposing its payload as text after
that boundary. The matcher never searches a terminal checkpoint or old screen.
If the required prefix is unavailable, the result is `history_unavailable`.

The literal limit is 512 UTF-8 bytes. The total decoded output limit is 8 MiB,
including the parser prefix. The result is `resource_limit` when this bound is
reached. Escape parser memory and literal matching memory do not grow with the
output size. Matching uses emitted text order. It does not reconstruct cursor
movement or prove the current visible terminal contents.

The outcomes are `matched`, `ended_before_match`, `history_unavailable`,
`resource_limit`, `timeout`, and `cancelled`. A known output gap stops the wait.
A reconnect retains the parser, match prefix, and exact output cursor. It cannot
repeat a completed match or restore native control.

## Multiple targets

Select each exact target explicitly. Repeat `--target` for up to 16 targets:

```sh
qgenie-remote remote wait-many --mode all --target MACHINE_ID/SESSION_ID/turn:TURN_ID --target OTHER_MACHINE_ID/OTHER_SESSION_ID/exit --timeout 300
```

Use the turn ID returned by `remote send --prompt`. The word `exit` selects native
process exit. The prefix `turn:` selects an exact turn, including a turn named
`exit`. Duplicate targets and invalid identities are rejected before observation.

`all` waits until every target has a resolved outcome or the command stops.
`any` stops after one target has a resolved outcome. A resolved outcome can be a
failure, interruption, missing history, or pending approval. It does not mean
success. Each result retains its exact target and approval coverage. A null
outcome after `any_resolved` means that this target was not resolved before the
wait stopped.

The command emits one JSON result. It retains resolved target outcomes when a
later target times out or the user cancels. Unresolved targets report `timeout`
or `cancelled` in those cases. An observation error has its own code and never
becomes a claim that the native task failed.

At most two target reads run at once. Each round visits every unresolved target.
A slow target cannot reserve a worker for the entire wait. Reads use the existing
request deadlines, owner authorization, typed checkpoints, and approval summary.
Connection recovery repeats observation only. The timeout range is 1 to 3600
seconds for both commands.
