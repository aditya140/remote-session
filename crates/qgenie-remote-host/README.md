# qgenie-remote-host

Outbound identity, policy, native IPC and recording bridge. Product dependencies are proto, transport, and policy. It has no adapter/sessiond execution dependency and does not own native children.

## Identity and enrollment

`Identity::open` binds a private state directory to a canonical HTTPS gateway origin, effective UID, OS-user identity, random installation ID, key ID, and Ed25519 public key. It persists the signing identity before any enrollment request. A nonblocking file lock serializes enrollment operations. Files are owned/0600 with no-follow opens under a private 0700 directory; atomic metadata replacement and fsync preserve the journal across interrupted replies. Repeated enrollment reuses installation/key IDs and reconciles a lost acknowledgement with a new challenge. Hostname is mutable metadata. Owner/machine bindings cannot silently change.

The default `os-keyring` feature uses Linux Secret Service or macOS Keychain, through four bounded blocking workers and a three-second caller deadline. The backend stores an **exportable software Ed25519 seed**; this implementation does not provide hardware non-exportable signing. `Protection::OsCredentialStoreExportable` reports that fact. If a new identity cannot use the OS store and `allow_private_file` is enabled, `Protection::PrivateFile` reports an **unencrypted 0600 seed file**. Existing identities never switch backend on a keyring failure. A locked/unavailable store is distinct from a missing/corrupt key. Key loss, changed keys, unsafe permissions, or revoked tombstones cannot cause automatic key generation or re-enrollment.

`GatewayApi::enroll` takes only an interactive access token, metadata, and explicit `RecordingConsent`. Composition must display `RECORDING_DISCLOSURE` and actual `Protection` before acknowledging. The disclosure states mandatory recording, default 90-day retention with monthly rounding to about 120 days, possible longer legal holds, audited admin access, best-effort redaction, and recording warnings/gaps. Public starts require the gateway's protected recording/retention readiness preflight.

`Identity::machine_credentials` returns only machine signer/binding state and releases the enrollment lock when the identity is dropped. `GatewayApi::authenticate_machine` accepts this separate type and no user credential. It validates challenge origin, machine/key, nonce encoding, age, and signature domain, then returns a redacted token with a conservative monotonic deadline of at most fifteen minutes. User access/refresh tokens never enter the identity journal or machine connector type.

`Identity::recover_key(..., confirmed=true)` explicitly stages a replacement while retaining installation/machine/owner. A durable recovery journal reuses the same pending replacement after interruption. `enroll` completes the gateway's owner-authenticated recovery, rejects changed bindings, and then commits/removes the recovery journal and retires the old credential. An unavailable pending replacement key fails closed and needs operator recovery; no second key is generated silently. `GatewayApi::unenroll` records revocation intent before its idempotent owner-authorized revoke, commits a tombstone after acknowledgement, and removes the key. Existing sessions remain outside this crate's ownership.

## Standalone scoped OAuth

The user authorized the standalone provider in ADR 0007. Corporate integration can supply its own `AccessToken` to enrollment without using this OAuth module. `OAuth::browser` creates a 127.0.0.1 loopback callback and returns an authorization URL for the composition's system-browser launcher. It uses S256 PKCE, random state/verifier, exact Host/path/state validation, duplicate-field rejection, bounded callback attempts/headers, and cancellation/expiry. `OAuth::device` exposes only the verification URI and user code for display; its device code is redacted. Polling respects provider intervals, adds five seconds on slow-down, handles 429, and stops on cancellation, expiry, denial, or uncertain exchange. The app's interactive/headless mode selection and browser launcher are task 025.

Set the optional `device_pkce` configuration field to `true` when the provider
requires the S256 extension for device authorization. The client keeps the verifier
in memory and binds every poll to the original challenge. The default is `false`.
Existing device providers keep their original request shape. The local Keycloak
profile enables this option.

Configure a dedicated native OAuth client with a registered loopback redirect, allowed device grant, and the remote resource scope:

```json
{
  "client_id": "YOUR_NATIVE_CLIENT_ID",
  "remote_scope": "api://YOUR_REMOTE_RESOURCE/Remote.Control",
  "authorization_endpoint": "https://login.microsoftonline.com/YOUR_TENANT/oauth2/v2.0/authorize",
  "token_endpoint": "https://login.microsoftonline.com/YOUR_TENANT/oauth2/v2.0/token",
  "device_endpoint": "https://login.microsoftonline.com/YOUR_TENANT/oauth2/v2.0/devicecode",
  "ca_bundle": null
}
```

Endpoint origins must agree. HTTPS uses verified native trust plus an optional PEM CA bundle and standard HTTPS_PROXY/NO_PROXY behavior. Redirects are disabled. Dedicated remote scope is mandatory; inference scopes are rejected. Request/response/callback sizes and deadlines are bounded. Token Debug output redacts both access and refresh credentials. Refresh credentials are returned only to interactive client composition. `OAuth::refresh`
performs one bounded exchange without retries. Composition serializes exchanges and replaces
rotated credentials. The standalone remote client keeps them in memory; enrollment discards
them. They never enter machine state.

## Native connector

The outbound connector authenticates with machine credentials and binds native requests to
the original gateway origin, owner, machine, connection and suspend-aware access challenge.
Host replacement and controller takeover preserve durable generation floors. Descriptor-scoped
browsing and native launch validate roots independently. Uncertain mutations are never resent.
Public PTY/managed start, output/control, owner approvals and protected recording uploads
compose through the standalone app; provider credentials remain with sessiond.

`NativeClient::watch_changes` requires `native-changes-v1` and owns a separate one-slot
subscription permit. Notifications coalesce without carrying output. Grant renewal replaces
the read-only subscription; cached metadata is retained until transport queue admission.
`host-changes-v1` sends session metadata and exact output heads through bounded gateway
lanes. Supporting clients negotiate `session-changes-v1`; older peers retain bounded
inventory reconciliation. Replay/checkpoint reads remain the source of rendered content.
The uploader has an independent bounded job and requires authenticated recovery-epoch
evidence before native receipt reclamation. Connector loss leaves execution/capture running.

## Verified scope

```sh
cargo check -p qgenie-remote-host --no-default-features
cargo clippy -p qgenie-remote-host --all-targets --all-features -- -D warnings
cargo test -p qgenie-remote --test host_enrollment
QGR_PG_TEST_URL_FILE=/private/disposable-postgres-dsn \
  cargo test -p qgenie-remote --test host_enrollment -- --include-ignored
DBUS_SESSION_BUS_ADDRESS=unix:path=/tmp/qgr-no-session-bus \
QGR_EXPECT_MISSING_KEYRING=yes \
  cargo test -p qgenie-remote-host --lib -- --include-ignored
```

Linux proofs use real verified HTTPS, a clearly synthetic OAuth/JWKS issuer, and the actual PostgreSQL-backed gateway. They cover PKCE/state attacks, device pending/slow-down/cancellation/expiry/denial, scope/TLS rejection, lost enrollment acknowledgements, repeat/hostname changes, private-file/key-loss handling, interrupted explicit key recovery, old-key denial, machine-only authentication, unenrollment, and absence of persisted user tokens. An actual unavailable Linux session bus proves disclosed fallback and refusal to silently switch an existing backend. Actual two-gateway/native SDK/TUI proofs cover remote sessions, original-access fencing, pushed heads, durable approval losses and lost-upload-ACK reconciliation. A real tenant, working native keychain, macOS and actual platform-service execution remain separate evidence gates. See [MVP evidence](../../docs/validation/mvp-evidence.md).

## Session inventory and directory discovery

New native connectors require `host-session-pages-v1` from the gateway and
`native-pages-v1` from sessiond. Live inventory and approval discovery read active
sessions. Historical metadata advances one bounded page per pass. `session.updates`
contains partial metadata; it does not replace the complete session list. The gateway
projects these updates and sends them only to clients with `session-pages-v1` support.
The connector periodically repeats history traversal to repair missed updates.

`fs.roots` returns the configured roots after the same owner, generation, access, and
browse-policy checks as `fs.list`. It does not permit browsing or launching outside
those roots. The host and session owner still validate the directory before launch.
