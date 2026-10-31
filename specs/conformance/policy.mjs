import fs from "node:fs";
import { pathToFileURL } from "node:url";

const policy = (await import(pathToFileURL(process.argv[2]).href)).default;
const fields = ["owner", "access_fresh", "revoked", "same_os_user", "controller",
  "within_roots", "capability", "host_allows", "external", "termination_confirmed",
  "external_termination_confirmed"];
const rows = fs.readFileSync(process.argv[3], "utf8").trim().split("\n");
if (rows.length !== 18432) throw new Error(`Expected 18432 rows, got ${rows.length}`);
const seen = new Set();
for (const row of rows) {
  const [action, maskText, expected, outcome, retryable] = row.split(",");
  const mask = Number(maskText);
  const key = `${action}:${mask}`;
  if (seen.has(key)) throw new Error(`Duplicate input ${key}`);
  seen.add(key);
  const facts = Object.fromEntries(fields.map((name, i) => [name, Boolean(mask & (1 << i))]));
  const actual = policy.authorize({ $: action }, { $: "Facts", ...facts }).$;
  if (actual !== expected) throw new Error(`${key}: Bend=${actual}, Rust=${expected}`);
  if (retryable !== "false" || outcome !== (actual === "Allow" ? "success" : "Rejected")) {
    throw new Error(`${key}: unexpected public outcome or retryability`);
  }
  // Independent positive/negative permission definition from authorization.md.
  const eligible = facts.owner && facts.access_fresh && !facts.revoked
    && facts.same_os_user && facts.capability && facts.host_allows;
  const actionAllowed = ["Start", "Browse"].includes(action) ? facts.within_roots
    : ["Input", "Resize", "Interrupt"].includes(action) ? facts.controller
    : action === "Terminate" ? facts.termination_confirmed
      && (!facts.external || facts.external_termination_confirmed) : true;
  if ((actual === "Allow") !== (eligible && actionAllowed)) {
    throw new Error(`${key}: violates AUTH-001 permission definition`);
  }
}
console.log(JSON.stringify({ combinations: rows.length, mismatches: 0,
  compared: ["allowance", "error_code", "outcome", "retryable"],
  permission_definition: "AUTH-001 exhaustive comparison passed" }));
