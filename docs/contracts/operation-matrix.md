# Operation authorization matrix

Status: selected contract baseline for tasks 004, 007, and their consumers. No implementation is implied.

Use [ADR 0002](../adr/0002-state-authority.md) for authority and freshness. Use [ADR 0003](../adr/0003-session-lifetime-and-ipc.md) for local administration. Use [ADR 0005](../adr/0005-recording-and-retention.md) for historical content access.

## Required facts

An owner is the authenticated QGenie user bound to the machine/session. A controller is a specific authenticated client connection with the activated session control generation. An observer-owner retains ownership while lacking input control. A platform-admin role grants the specified administrative operations; it does not automatically grant live session control.

Every remote operation requires a valid credential of the correct kind, current access, a permitted operation, and verified resource provenance. Missing or stale facts deny access. A gateway allowance and a host allowance must both pass. Host policy can narrow a permitted action. Adapter capability can further narrow it.

Allowed roots apply to browsing, new-session directories, and future file transfer. They do not restrict supported control of an existing same-OS-user session. External-session adoption is later work, but this rule must remain part of the policy contract.

## User and host operations

| Operation | Actor/access | Controller lease | Host checks and root scope | Commit or acknowledgment |
| --- | --- | --- | --- | --- |
| Enroll machine | Current user with dedicated remote scope and signing proof | None | Persisted installation identity; verified credential protection | PostgreSQL machine/key and audit commit. Same identity returns the same machine. |
| List machines/sessions | Current owner | None | Only the authenticated ownership chain | Snapshot/projection response; it does not prove a process is running. |
| Rename/label machine | Current owner | None | Bounded validated metadata | PostgreSQL change and audit as applicable. |
| Browse directory | Current owner | None | Canonical, race-safe path within allowed roots | Host result. No generic file-read or shell endpoint. |
| Start session | Current owner | None | Allowed agent/terminal; same OS user; validated cwd within roots; launch capability | Sessiond durable key/intent and later native launch result. An accepted start does not automatically grant control. |
| Lookup uncertain start | Current owner of target machine | None | Original scoped key and fingerprint | Sessiond authoritative status, including uncertainty. |
| Observe/attach replay | Current owner | None for observation | Session identity and output capability; existing cwd does not deny observation | Host floor/head, snapshot boundary, and stream subscription. |
| Request writable control | Current owner | Activated lease required before mutation | Current host generation; supported control; explicit takeover if another holder exists | PostgreSQL proposal plus sessiond activation and gateway confirmation. |
| Input or steer | Current owner and activated controller connection | Required | Current host/controller generations, unexpired delegation/lease, agent/terminal policy, input capability | Input ID and adapter result. No automatic resend. |
| Resize | Current owner and activated controller connection | Required | Same checks; terminal resize capability; valid dimensions | Host result under current generation. |
| Interrupt turn | Current owner and activated controller connection | Required | Same checks; interrupt capability | Adapter outcome. A turn interrupt is not session termination. |
| List pending approvals | Current owner | None; attachment unnecessary | Request belongs to the owner's session | Gateway pending snapshot plus push updates. |
| Answer approval | Current owner | None; attachment unnecessary | Current access, host generation, pending native request, host-policy preflight, approval capability; no root check on existing cwd | One gateway decision plus outbox; application reported separately. |
| Detach/release own control | Current owner of attachment | Own lease only when releasing it | Remove only the caller's subscription/control claim | Detach response. Keep session execution. |
| Terminate session | Current owner; explicit confirmation | None | Current host generation/access, termination capability and host policy. External sessions need explicit external-session confirmation. No cwd root check. | Sessiond/adapter termination result plus audit. A disconnect is not this action. |
| Resume historical session | Current owner | None before new control activation | Treat as a new correlated start; require allowed cwd and supported resume | New start-key result; preserve historical session identity separately. |
| Revoke own machine | Current owner | None | No process termination | PostgreSQL revocation and audit; propagation status. |
| Inventory/output update | Authenticated machine connector | None | Machine/session provenance and current host generation | Projection update. Sessiond remains process/output authority. |
| Recording ingest | Authenticated current machine connector | None | Verified session owner binding and immutable batch validation | Durable content/gap receipt, or explicit expiry/error. |
| Read history | Current owner, or authorized platform admin | None | Owner RLS, or admin role/reason/audit; extra case context for held content | Bounded audited page. Not a live replay operation. |

An outside-root existing session can pass input, resize, interrupt, approval, and termination checks. Unsupported adapter actions still fail. Another QGenie user, a revoked actor, a foreign OS-user session, or expired access must fail even when the path is within allowed roots.

## Administrative and local actions

| Operation | Required authority | Remote boundary |
| --- | --- | --- |
| Global/user/machine admin revocation | Current platform-admin role; idempotency key; atomic audit | Gateway admin API only. No termination side effect. |
| Audit-log read | Current platform-admin role | Authenticated gateway API. Owner APIs cannot expose admin reads/holds. |
| Place/change/release legal hold | Current platform-admin role; case reference; atomic audit | Gateway admin API. One admin is sufficient. |
| Read another user's transcript | Current platform-admin role; reason; case reference when held; committed read audit | Protected history API and dedicated RLS role. No live-control grant. |
| Configure local roots/agent restrictions | Enrolled local OS user through verified CLI integration | Local administration; not a remotely callable arbitrary configuration endpoint. |
| Pause/unpause connector | Enrolled local OS user/service integration | Local command. Keep sessiond independent. |
| Stop/update sessiond | Explicit local lifecycle authority and active-session checks | Separate administrative IPC schema and endpoint. No dispatch conversion exists. |
| Retire partitions | Dedicated lifecycle role; database lock and preservation proof | Jobs process. Redis scheduling alone is insufficient. |

## Denial order and stable reasons

Validate framing and typed fields before domain dispatch. Authenticate next. Check current access and ownership before revealing whether another user's resource exists. Then evaluate action-specific lease, capability, policy, and path facts. Return a stable public reason without backend or secret details.

Policy tests must include owner/controller, observer-owner approval, other user, revoked principal, expired facts, foreign OS user, unsupported capability, outside-root existing control, out-of-root new start, explicit takeover, and separate termination. Decisions consume facts only; filesystem and directory I/O belong to their owning components.
