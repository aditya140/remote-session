import fs from "node:fs";
import { pathToFileURL } from "node:url";

const model = (await import(pathToFileURL(process.argv[2]).href)).default;
const rows = fs.readFileSync(process.argv[3], "utf8").trim().split("\n").map(JSON.parse);
const schema = JSON.parse(fs.readFileSync(process.argv[4], "utf8"));
const codes = new Set(schema.$defs.ErrorCode.enum);
const outcomes = new Set(schema.$defs.OperationOutcome.enum);
const statuses = ["NativeCompleted", "NativeFailed", "NativeInterrupted", "LegacyUnknown"];
const results = { Completed: "completed", Failed: "failed", Interrupted: "interrupted",
  HistoryUnavailable: "history_unavailable" };
const counts = { recovery: 0, credential: 0, turn: 0 };
const seen = new Set();

function requireInput(condition, row) {
  if (!condition) throw new Error(`Invalid comparison input: ${JSON.stringify(row)}`);
}
for (const row of rows) {
  let expected, key;
  if (row.kind === "recovery") {
    requireInput(codes.has(row.code) && outcomes.has(row.outcome)
      && typeof row.retryable === "boolean", row);
    key = `recovery:${row.code}:${row.outcome}:${row.retryable}`;
    const failure = row.code === "access_stale" ? "AccessStale"
      : row.code === "overloaded" ? "Overloaded" : "Fatal";
    expected = model.can_reconnect({ $: failure });
  } else if (row.kind === "credential") {
    requireInput(Number.isInteger(row.mask) && row.mask >= 0 && row.mask < 8, row);
    key = `credential:${row.mask}`;
    expected = model.credential_valid(Boolean(row.mask & 1), Boolean(row.mask & 2), Boolean(row.mask & 4));
    requireInput(row.error === (expected ? null : "unauthenticated"), row);
  } else if (row.kind === "turn") {
    requireInput(Number.isInteger(row.length) && row.length >= 0 && row.length <= 3
      && Number.isInteger(row.index) && row.index >= 0 && row.index < 8 ** row.length
      && [0, 1].includes(row.target) && [0, 1, 2, 3].includes(row.scenario), row);
    key = `turn:${row.length}:${row.index}:${row.target}:${row.scenario}`;
    let list = { $: "Nil" }, symbols = row.index, hasEvidence = false;
    for (let i = 0; i < row.length; i++) {
      const symbol = symbols % 8, turn = Math.floor(symbol / 4);
      symbols = Math.floor(symbols / 8);
      hasEvidence ||= turn === row.target;
      // Cons chronological input to give the model the newest evidence first.
      list = { $: "Con", head: { $: "Completion", turn,
        status: { $: statuses[symbol % 4] } }, tail: list };
    }
    requireInput(row.scenario === 0 || hasEvidence, row);
    const evidence = model.latest_completion(row.target, list);
    expected = evidence.$ === "None" ? null : results[evidence.value.$];
    requireInput(expected !== undefined, row);
  } else {
    throw new Error(`Unknown comparison kind: ${row.kind}`);
  }
  if (seen.has(key)) throw new Error(`Duplicate comparison: ${key}`);
  seen.add(key);
  counts[row.kind]++;
  if (row.result !== expected) {
    throw new Error(`${key}: Bend=${expected}, Rust=${row.result}`);
  }
}
if (counts.recovery !== codes.size * outcomes.size * 2 || counts.credential !== 8 || counts.turn !== 4170) {
  throw new Error(`Incomplete comparison: ${JSON.stringify(counts)}`);
}
console.log(JSON.stringify({ combinations: rows.length, mismatches: 0, ...counts,
  scope: "connection eligibility, credential admission, and terminal evidence traces of length 0..3" }));
