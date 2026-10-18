# Directory freshness and manual revocation

The gateway reads Microsoft Graph `GET /v1.0/users/{oid}?$select=id,accountEnabled` using a dedicated application identity. Tenant/object IDs must be canonical UUIDs, the returned ID must match, and `accountEnabled` must be present as a boolean. A 404 denies access. Redirects, malformed/duplicate JSON, missing properties, wrong IDs, 401/403/429/5xx, oversized responses and TLS failures provide no new evidence. A 401 evicts the application-token cache; the failed operation is not retried automatically.

## Configuration

Configure `--directory-config-file /etc/qgenie/directory.json` alongside `--identity-config-file` and `--machine-key-file` when serving the gateway. Synthetic example:

```json
{
  "tenant": "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
  "client_id": "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb",
  "credential_file": "/run/secrets/graph_application_secret"
}
```

The client uses the tenant's `https://login.microsoftonline.com/{tenant}/oauth2/v2.0/token` endpoint, client credentials, and `https://graph.microsoft.com/.default`. The tenant must match the verified user configuration. Explicit HTTPS `token_url`, `graph_base` and `ca_bundle` overrides support sovereign clouds and synthetic fixtures. TLS uses native trust plus the configured CA bundle; redirects are disabled. The credential is separate from user tokens and the machine signing key, redacted in diagnostics, and never reaches Redis, hosts, sessiond or argv.

Configuration/CA/credential files must be regular no-follow files owned by root/the service user, without foreign write permissions. Credentials permit service-readable 0440/0600/0640 and deny other-user access. Reads are bounded and FIFOs cannot block opening. Mount operator-managed secrets and rotate by replacing/restarting the composition. Libraries configure directory verification after identity and before starting routing/cloning the gateway.

Microsoft's [Get user documentation](https://learn.microsoft.com/en-us/graph/api/user-get?view=graph-rest-1.0) lists `User.Read.All` as the least application permission for this endpoint. Its `accountEnabled` property note describes combined permissions for reading **and writing**. This implementation performs only GET. An authorized tenant administrator must qualify the actual property response with approved read permissions; the code does not grant permissions or infer that write permission should be granted. A successful response without the property fails closed. Real tenant permissions and disablement evidence remain release gates.

## Freshness and enforcement

Each gateway owns at most 4096 cache entries. First contact and restart require a provider read; UTC projections and JWT expiry cannot restore fresh authority. Provider work is capped at 16 concurrent operations, with no waiting queue; initial database lookups have another 16-operation cap. Token-lock wait, OAuth, Graph and response body share a five-second deadline. Bodies are capped at 32 KiB. Errors preserve the last good proof's original deadline.

Evidence expires 840 seconds after the provider read **starts**, using Linux `CLOCK_BOOTTIME` / macOS `mach_continuous_time`. A response taking over five elapsed seconds, including suspension, is rejected. The owned worker checks active entries every second, refreshes after 300 seconds and spaces failed attempts by 30 seconds. It owns at most 16 tasks and cancels/joins on shutdown. Entries unused for 1800 seconds can be evicted. Saturation fails explicitly.

User admission, enrollment/recovery, machine-token issuance, machine authentication and connected-principal monitoring share this service. Machine tokens inherit their owner's directory state. Primary flags and credential expiry are checked after provider waits. PostgreSQL `directory_enabled`/`directory_verified_at` are observability projections, separate from manual flags; successful directory updates cannot clear manual disablement or resurrect keys.

Independent connection monitors check primary authority every two seconds within a three-second total budget. They operate independently of Redis, paging and dispatch handlers. Denial aborts the actual driver and queues. The gateway socket also checks elapsed authority before every read/write poll. Its deadline is at most ten seconds from the primary read's start, capped by directory evidence and the original credential deadline. Queued frames cannot acquire a receipt-time budget after suspension. Denied gates cannot be renewed by an old pending allow. Refresh with the same credential expiry cannot extend its original elapsed deadline.

These bounds leave margin inside the 900-second requirement. Actual tenant disablement and real suspend/resume qualification remain mandatory release gates. Public PTY/managed control and recording now compose under the original native access grant in the Linux development implementation.
The native read/activation bridge now carries that original identity through source-node queues and confirms PostgreSQL proposals after native acknowledgment; controller effects remain disabled. Clock sampling is serialized, and a kernel-clock failure/regression makes the elapsed clock unavailable until process restart, preventing an old deadline from reviving. JWKS and Graph application-token cache deadlines use the same suspend-aware clock; JWKS freshness starts before its verified HTTPS read and failed refresh attempts are rate limited.

## Admin revocation

`POST /v1/admin/revocations` requires a currently enabled platform-admin principal, stable typed request ID and explicit confirmation. Targets are global, user or machine. Flags, machine-key revocation, a full-u64 access revision, idempotency result and audit event commit together. Same actor/request/target returns the original result; a changed target conflicts. Missing targets and revision exhaustion roll back. An enabled administrator can use the admin plane during global denial.

`GET /v1/admin/revocations/{request}` reads that actor's durable result. The accepting node fences matching local peers; other nodes independently check primary state. No termination, approval decision or automatic re-enrollment is issued. Domain writers and revocation take global authority locks before user/machine/key locks.

Responses initially report `propagation: "pending"`. A gateway that has observed the exact
committed primary row for the full ten-second suspend-aware maximum authority lifetime may
report `completed`. This conservatively proves that pre-revocation authority cannot admit
new remote I/O; it does not wait for native execution to finish or terminate local sessions.
Every response still reads the primary. Another gateway's first observation, restart or
cache eviction waits a new horizon; SQL or clock failure cannot produce cached completion.
Redis hints and wall time supply no completion evidence. See
[ADR 0009](../adr/0009-revocation-completion.md) and the
[external release gates](../validation/external-release-gates.md).

## Verification and release gates

Verified HTTPS fixtures cover provider credentials/headers/token reuse, enabled/disabled/404 reads, malformed/missing/wrong properties, TLS/status/size errors, deadlines, saturation and file permissions. Disposable PostgreSQL tests exercise restart freshness, separate upstream/manual state, signed user/machine admission and expiry during HTTPS verification, plus concurrent/lost-ACK audited admin revocation. Two actual gateway processes prove missed-hint revocation and stalled primary authority while a dispatch handler is also blocked. Deterministic elapsed-time tests exercise queued I/O expiry across simulated suspension.

Run the gateway README's `--include-ignored` contract with disposable PostgreSQL/Redis bindings. Synthetic providers establish component behavior; they do not establish the Entra SLO. Real tenant, suspend/resume and macOS execution, native activation, propagation completion and enterprise recording retain their own release gates.
