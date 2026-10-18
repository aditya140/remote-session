# Gateway identity and enrollment

The standalone gateway accepts dedicated remote user access tokens and separate short-lived machine credentials. There is no inference-token or unauthenticated development bypass in the binary. Real tenant/directory qualification remains an external proof; the development checks use a signed synthetic identity provider.

## Configure user identity

`serve --identity-config-file /etc/qgenie/remote-identity.json --machine-key-file /run/secrets/machine_signing_key --origin https://gateway.example.com` enables identity. Without those files, domain authentication is denied. The public configuration is JSON:

```json
{
  "issuer": "https://login.microsoftonline.com/TENANT_ID/v2.0",
  "tenant": "TENANT_ID",
  "audience": "api://qgenie-remote",
  "remote_scope": "Remote.Control",
  "admin_role": "QGenie.Remote.PlatformAdmin",
  "jwks_url": "https://login.microsoftonline.com/TENANT_ID/discovery/v2.0/keys",
  "ca_bundle": null
}
```

Replace tenant/audience/role values with the approved registration. JWKS fetching requires verified HTTPS, native/custom CA trust, bounded responses (256 KiB/32 keys), five-second deadlines, and no redirects. Proxy/NO_PROXY behavior comes from the HTTP client. Refresh normally occurs after five minutes, with a one-minute cooldown for unknown keys; cached keys stop authorizing after ten minutes without a successful refresh. This key-cache lifetime is not the Entra account-disablement bound (task 019).

User validation fixes RS256, issuer, tenant, audience, scope, expiry/not-before/issued-at, and the optional configured platform-admin role. A valid ID or inference token does not substitute for the remote scope. Required `oid` is the canonical Entra directory object; JWT `sub` may differ across applications. User identity is stable under issuer/tenant/oid. A second pairwise subject for the same object returns the same internal owner and cannot bypass its disabled flag. The original subject is retained for legacy binding.

Schema 0003 adds directory identity without rewriting 0002. Legacy preview rows with no object ID can be bound only by a verified token matching their old subject. New unknown subjects are denied while such rows remain unmapped; do not silently create a replacement for a revoked legacy owner. A real preview-data migration needs verified mapping/backfill or a clean disposable development database. Identity activation requires schema 3; foundation-only readiness supports schemas 1–3.

## Machine identity flow

| API | Authentication and result |
| --- | --- |
| `POST /v1/enrollments/challenge` | Dedicated user token plus installation/key/public-key/OS-user and metadata. Returns a one-minute signed-challenge target. |
| `POST /v1/enrollments` | Same user token plus challenge ID and Ed25519 signature. Commits enrollment and audit atomically. |
| `GET /v1/machines` | Owner-only metadata, currently bounded to 1024 machines. Presence/inventory/session push projection is task 012. |
| `PATCH /v1/machines/{machine}` | Owner-only name/labels, bounded fields, audited commit. |
| `POST /v1/machines/{machine}/challenges` | Public authentication bootstrap with machine/key IDs; bounded outstanding challenges and no domain mutation. |
| `POST /v1/machine-tokens` | Signature over the exact stored canonical challenge, active key/owner/access checks, and single-use consumption. Returns a machine token lasting at most 900 seconds. |

DTOs live in proto's `rest` module and the generated `conformance/schema/rest-v1.json` catalog. JSON duplicate fields and oversized bodies fail before domain decoding. Challenges use ADR 0004's exact domain-separated bytes, including the configured HTTPS origin, machine/key/challenge identities, random 32-byte nonce, and times. The signing host must also check the expected origin. Signature, expiry, purpose, owner, key, and consumed-state checks happen in the transaction; a request cannot consume another purpose's nonce.

Persist a random installation/key before requesting enrollment. An interrupted response reuses that identity and requests another challenge; it does not allocate a replacement installation. Concurrent initial challenges retain the same provisional machine ID. Each nonce is single-use, while enrollment itself is idempotent under authenticated owner/installation/OS-user/public-key identity. Equal hostnames do not merge machines. Hostname changes update metadata; repeated enrollment preserves the owner's renamed display name/labels.

Key replacement requires `recovery: {"machine": "m_…", "confirmed": true}`, current user authentication, and proof of the new key. The transaction inserts the new key, revokes the old one, updates the machine, and audits recovery. Old machine tokens fail immediately against current key state. Revoked machines/keys cannot silently re-enroll or recover themselves; explicit access recovery is separate from silently resetting revocation. The same public key cannot be reused under a different installation. A copied complete private identity cannot be reliably distinguished from its original; durable host generations in task 017 fence older connections.

The service's 32-byte machine-token HMAC key is base64 in a private/service-readable file, shared across gateway replicas and separate from the routing key. It never travels to hosts. Machine tokens use their own issuer, audience, HS256 key, and `qgr-machine+jwt` type; user JWT validation only accepts the configured RS256 identity issuer. Hosts renew using their own Ed25519 key, not user refresh tokens. Credential Debug/error formatting is redacted. Enrollment stores public identity only.

## Access and verification

Every identity operation checks PostgreSQL ownership/current disabled state. A recovery/rename/enroll commit includes its audit atomically. Live cross-node revocation, directory reconciliation, and the 15-minute disablement proof are task 019; they are not inferred from token expiry. WSS admission uses the same gateway `authenticate_client` / `authenticate_host` interfaces; task 012 supplies actual upgraded connections and admission tests.

Real Linux tests passed for:

- Trusted/untrusted HTTPS JWKS, stale-cache refusal, wrong issuer/audience/scope/tenant/expiry/not-before, and forged token families.
- Stable directory-object ownership across pairwise subjects and preservation of user disablement.
- PostgreSQL concurrent/repeated/lost-response enrollment, same hostname with distinct keys, rename/labels, foreign ownership, wrong/replayed/expired signatures, key recovery, revoked-key denial, and cross-use of user/machine tokens.
- Schema migration/checksum/role/outage checks, authenticated REST reads, and gateway process readiness/SIGTERM drain.

The real TUI process/keyboard regression also passes after IPC and parser changes. No real tenant or directory service was contacted. OAuth user interaction, credential storage on hosts, WSS routing, bounded revocation propagation, and the full remote TUI remain their owning tasks.
