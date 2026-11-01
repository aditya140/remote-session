# Device, agent, and telemetry console design

Status: revision 3 design. The prototype uses synthetic metadata. Production
collection, APIs, browser session control, and anomaly rules are not implemented
by this design change.

## User requirement

An admin needs to answer: Which devices belong to this user? Which are servers?
Which are connected? Where are agents running? Which harness and model does each
session use? When did each host connect, and when did it last report? How many
sessions are active? What changed unusually across users or the fleet?

A user needs the same device and session context for their own resources. The user
also needs to stop a selected native session from the browser with confirmation.

## Screens and navigation

The main admin overview now contains a user inventory. Each user row shows enrolled
and connected device counts, enrolled and connected server counts, confirmed active
sessions, unknown sessions, and last-seen time. Select a user to open their devices
and sessions. Select a device to inspect its execution records.

An execution row shows session identity, device, harness name and version, model,
model source, native process state, and task state. A failed turn may belong to a
running process. A disconnected device may still have locally running work. Such
work has unknown current state; it does not disappear or count as confirmed active.

The Telemetry & signals view shows aggregate counts, harness/model distribution,
collection coverage, illustrative evaluation runs, and explainable review signals.
Each signal includes a subject, observation window, rule, evidence, and a benign
explanation to consider. Select Inspect evidence to review that context.

The user workspace lists the user's devices and their agent or terminal sessions.
It supports a device drill-down and confirmed Stop session. The work inbox also
retains the separate approval card. Admin metadata views do not gain native stop
rights over another user's sessions.

The Studio, Signal, and Atlas visual styles all support these views. This revision
changes the information architecture; it does not add another competing console.

## Terms and counts

| Term | Definition |
| --- | --- |
| Device | One enrolled QGenie host identity. This is not the identity provider's device inventory or a browser tab. |
| Server | A device whose reported class is server. Count it as a subset of devices. Keep unknown classification explicit. Do not infer class from hostname or OS. |
| Connected device | An enrolled host with current activated routing/presence evidence. The UI must report observation freshness. |
| Enrolled device count | All non-retired device identities in the selected owner scope, including offline devices. |
| Last connected | Most recent successful connector activation time. It is not last seen or a new JWT issue time. |
| Last seen | Most recent accepted authoritative presence observation for that device. |
| Active session | A distinct native session ID whose latest fresh observation is running or idle and has no confirmed end. Count processes, not turns, sockets, or viewers. |
| Running agent | An active agent session with an identified harness. Installed agent inventory alone is insufficient. |
| Harness | The execution program or integration, such as Codex CLI, with its adapter and version. A PTY shell is a harness without an AI model. |
| Model | A requested model and, when available, a separately reported effective model. Model routing can change per turn. |
| Unknown session | A last-known session whose live state is unavailable or stale. Keep it out of confirmed active counts. |
| Session start event | One durable start identity accepted during a time window. This differs from the current session count. Deduplicate original start keys. |
| Telemetry evaluation run | One bounded aggregation or rule evaluation with a source interval, status, coverage, rule version, and original run identity. |

Every aggregate must include its scope, source, time window, observed timestamp,
and unknown coverage. Snapshot totals must not be mixed with event rates. A model
share denominator excludes terminal sessions and shows an unknown-model bucket.
Token totals must declare session/turn scope and collection coverage. Never label
partial usage as fleet total or infer billing from tokens without provider pricing.

The prototype cohort has three users, six enrolled devices, four servers, four
connected devices, five confirmed active sessions, and three unknown sessions.
Three agent processes are confirmed active. Two report a sample model and usage;
one does not. The two known usage totals sum to 30,700 tokens. This is a partial
session-lifetime total, not a last-hour or fleet-wide total. Historical signal
event counts include closed sessions and retired devices, so they differ from
this snapshot. The prototype makes no claim about the actual deployment.

## What the repository supplies today

| Data | Current evidence | Required addition |
| --- | --- | --- |
| Device identity and owner | `wire::Machine` has ID, owner, name, hostname, OS, architecture, and CLI version. | Cross-owner admin read DTOs and indexed filters. |
| Presence and last seen | `wire::Machine` has online and last_seen; presence events carry last_seen. | Explicit age and source coverage in UI projections. |
| Installed harnesses | `AgentInventory` includes version and supported capabilities. | Do not treat this list as active processes. |
| Session process state | `wire::Session` has identity, launch kind, state, created_at, ended_at, and capabilities. | Fresh observation timestamps and metadata read views. |
| Native agent identity | `ManagedSessionInfo` has agent, native instance, and native thread references. | A bounded public execution metadata DTO; avoid exposing native private paths. |
| Model configuration | The Codex adapter has provider/model configuration. | Distinguish requested configuration from effective runtime model. No public normalized per-session model field exists today. |
| Connection history | Host connection and activation audit events exist. | A durable, deduplicated activation history read model and explicit disconnect/expiry observations. Do not assume complete connection intervals already exist. |
| Server classification | Machine labels are available. | An explicit reported device class with provenance. Do not trust it as an authorization fact. |
| Usage and unusual activity | Existing operational counters and audit events supply some evidence. | Normalized usage, bounded rollups, historical baselines, and explainable rule evaluation. |
| Stop a native session | The owner WSS termination operation exists. | Browser authentication, capability checks, and a targeted confirmation/result UI. |

Source files: [wire models](../../crates/qgenie-remote-proto/src/wire.rs),
[native session metadata](../../crates/qgenie-remote-proto/src/model.rs),
[Codex adapter](../../crates/qgenie-remote-adapters/src/codex/mod.rs),
[audit actions](../../crates/qgenie-remote-gateway/src/audit.rs), and
[operation authority](../contracts/operation-matrix.md).

## Proposed API additions and extensions

These are extensions to the [console API proposal](admin-user-console.md).
They are not new runtime routes in this change. Keep existing owner machine and
recording routes. Do not add duplicate device aliases.

| Proposed endpoint | Contract |
| --- | --- |
| `GET /v1/admin/users` | Add connected/enrolled device and server counts, confirmed active/unknown session counts, and last-seen metadata per user. Include snapshot scope and coverage. |
| `GET /v1/admin/users/{user}` | Minimal user identity, access state, source timestamps, and aggregate metadata. No transcript text. |
| `GET /v1/admin/users/{user}/devices` | Stable device page with type, classification source, connection state, last activation, last seen, and execution counts. |
| `GET /v1/admin/users/{user}/sessions` | Stable page of the user's sessions with device, harness, process/task state, and optional model/usage observations. |
| `GET /v1/admin/machines/{machine}/sessions` | Same bounded metadata DTO, filtered to a device. Enforce admin authority before revealing resource existence. |
| `GET /v1/admin/machines/{machine}/connections` | Deduplicated activation history with generation, connection/observed-end times, and incomplete-interval markers. No tokens or raw transport secrets. |
| `GET /v1/admin/telemetry/summary` | Snapshot aggregates with known/unknown counts, distinct identities, freshness, and filter scope. |
| `GET /v1/admin/telemetry/series` | Bounded time buckets by time, harness, reported model, or device class. Return coverage and a defined denominator. |
| `GET /v1/admin/telemetry/runs` and `/{run}` | Collection/evaluation status, interval, duration, source coverage, errors, and rule versions. Do not return private log bodies. |
| `GET /v1/admin/activity-signals` and `/{signal}` | Rule, severity, confidence basis, subject, evidence references, time window, baseline sample size, and review status. |
| `GET /v1/me/sessions` | Add device filters and the owner-safe execution DTO. Return only current authorized owner resources. |
| Existing `GET /v1/machines` and `/{machine}` | Add optional class and connection metadata compatibly. Preserve legacy behavior and owner scope. |
| Owner browser WSS terminate | Reuse the current correlated owner termination operation. Do not add a generic retrying REST stop facade. |

Admin lists are privileged metadata reads. Normalized telemetry does not bypass
reason-gated recording reads. User responses exclude other users, legal holds,
admin read activity, and private investigation signals.

Use cursor pagination with a default 25 and maximum 100 for new inventory pages.
Bind cursors to the actor, scope, sort, and filters. Limit series to 24 hours and
120 buckets initially. Use bounded precomputed read projections or rollups; do
not fan out to every native owner on each page request. Keep per-user/session
identifiers out of Prometheus label sets. Use authorized database reporting for
high-cardinality user drill-downs. Platform metrics remain on the private scrape
and OTel paths.

## Proposed execution metadata

An optional typed metadata envelope should contain harness ID, harness version,
requested model, reported effective model, model provider where known, source,
observed timestamp, and observation scope. Scope the actual model and usage to a
turn when the harness can switch models. A session-level summary is an observation,
not a guarantee that every turn used that model.

Collect this data through supported native adapter metadata. Never extract it
from arbitrary terminal text or expose full launch arguments, environment values,
credentials, prompts, private local history paths, or hidden reasoning. Validate
lengths and allowed fields. Preserve unknown values for old or unsupported peers.
Negotiate any new host reporting capability and add old-peer behavior tests.

Example proposed fields:

```json
{
  "session": "s_example",
  "machine": "m_example",
  "harness": {"id": "codex", "version": "0.159.2"},
  "model": {
    "requested": "example-model",
    "effective": null,
    "source": "launch_configuration",
    "scope": "session_configuration"
  },
  "process_state": "running",
  "task_state": "failed",
  "observed_at": "2026-10-31T13:42:08Z",
  "freshness": "fresh"
}
```

The example IDs and model name are placeholders. Requested configuration does not
establish the effective model. Report usage as cumulative native counters with
source and cursor identity. Deduplicate reconnects, retries, and replay. Do not
sum successive cumulative samples. Declare unavailable or reset counters explicitly.

## Unusual activity and evidence

Start with transparent rules, not an opaque user risk score. Candidate rules are
a burst of device enrollments, session starts above the user's recent baseline,
repeated authorization denials, a newly observed harness, and stale collection.
Treat stale telemetry as a coverage warning rather than a security accusation.

A signal must retain the rule/version, original event references, subject, actual
window, baseline sample count, threshold, unknown coverage, and explanation.
Missing or sparse history produces insufficient baseline. Scheduled jobs, new
projects, and legitimate parallel work can explain unusual behavior. Group signals
by stable subject and window to avoid a notification per poll. Human review may
link to the existing audit and revocation workflows. Do not automatically stop
sessions, approve tools, revoke users, or claim abuse from these rules.

Authenticated metadata collection must be allowlisted and bounded. Establish its
retention policy before persisting it; transcript legal-hold rules do not silently
define retention for a new usage table. Sensitive investigation details stay admin-
only. No transcript or model response content enters a generic telemetry event.

## Stop flow and failure semantics

Stop session means native process termination. It is distinct from interrupting a
turn, closing observation, pausing a connector, or revoking remote access.

1. The owner selects one exact session on one device. Display its name, native
   session ID, device, and the effect on in-progress work.
2. Require explicit confirmation. In the prototype, type STOP. Cancel performs no
   mutation. A real browser command uses current authorization, host generation,
   adapter terminate capability, and the existing committed request audit.
3. Owner termination does not require a controller lease under the current
   operation matrix. Interrupting a turn does require the current control lease.
   External session termination needs its additional existing confirmation.
4. Keep other sessions and device access unchanged. Do not replace termination
   with device revocation. Platform-admin status supplies no new cross-owner
   session control right.
5. Display pending until a native result or fresh state confirms termination.
   After a lost reply, show Stop unknown. Reconcile the original command/session;
   never silently send a fresh operation. A conflict or denial is not success.
6. Offline or stale devices cannot offer a confirmed live stop. Disable the action
   and explain that the process may still run locally. Require fresh authorization
   again after reconnect. A real replacement process has a new lifetime ID and
   cannot inherit an old stop intent.

The prototype offers both confirmed and lost-reply response scenarios. These
change only local display state. They do not contact a native owner or API.

## Implementation order

1. Deliver user/device/session metadata projections, freshness, and count definitions.
2. Add native execution/model metadata and optional usage reporting with provenance.
3. Add connection history and bounded aggregate read models. Verify query cost and
   account for incomplete old history without inventing past connections.
4. Connect owner browser stop through the existing authorized WSS protocol. Test
   exact target, cancel, unsupported adapter, external confirmation, lost reply,
   stale generation, offline host, and unrelated-session survival with real PTYs.
5. Add explainable activity rules after event quality, history coverage, and baseline
   requirements pass. Test benign bursts, sparse data, and duplicate event delivery.

The browser identity and protected recording gates from the main proposal still
apply. This design does not establish production telemetry, stop behavior, or an
anomaly detector. The prototype checks verify its navigation and local simulation.
