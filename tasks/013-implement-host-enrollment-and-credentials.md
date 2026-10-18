# 013 — Implement host enrollment and machine credential storage

- **Status:** Complete for the selected Linux standalone development scope; external release qualification remains separately tracked
- **Milestone:** M0 enrollment
- **Workstream:** Host identity
- **Depends on:** [001](001-inspect-existing-qgenie-integration.md), [005](005-implement-websocket-transport.md), [011](011-implement-gateway-identity-and-enrollment.md)
- **Index:** [Task plan](README.md)

**Scope:** The user selected Linux development completion in [ADR 0007](../docs/adr/0007-standalone-preview.md). See [current progress](PROGRESS.md) and [external release gates](../docs/validation/external-release-gates.md). Original criteria that require unrun environments or waived corporate inspection remain unchecked; this status does not claim full release qualification.

## Goal and repository context

Provide idempotent host enrollment using the standalone authentication and credential interfaces authorized by ADR 0007. The unavailable original CLI is not a prerequisite.

**Read first:** [PRD.md](../PRD.md) §5.1, ID-1–5, ID-8; [architecture.md](../architecture.md) §§6.1–6.2; [components.md](../components.md) §6.

## Owned scope

- `crates/qgenie-remote-host/src/{config,enrollment,error}.rs`
- `Host identity/{key,challenge,token}.rs and local identity persistence`

Owned paths define this task’s implementation boundary; current source and executed evidence are recorded below. Follow [the subcrate standard](../subcrate-standard.md), add only useful modules, and coordinate shared manifests/exports and migrations as described in the index.

## Implementation

1. Implement the authorized standalone scoped OAuth provider: browser PKCE by default where supported, device grant for headless/SSH or explicit --device-code, respecting polling intervals, cancellation, expiry, and tenant errors.
2. Generate/store the machine signing identity through the verified integration, disclose actual protection/fallback, and persist enrollment state safely across interrupted requests.
3. Sign gateway challenges, refresh short-lived machine tokens, and retain machine identity across repeated up calls and hostname changes.
4. Expose enrollment/revocation/unenrollment operations for CLI composition. Include mandatory recording/retention disclosure and the adoption disclosure when that later capability is enabled.

## Acceptance criteria

- [x] Repeated enrollment after lost replies reuses identity without duplicate machine records.
- [x] A headless device-code flow and browser PKCE flow use the dedicated remote scope and clear cancellation/error states.
- [x] Long-running host authentication needs only machine credentials; user refresh tokens are not passed to the connector.
- [x] Fallback files are private, secret material is redacted, and a revoked identity never silently re-enrolls.

## Verification

- Run enrollment tests against a fake OAuth provider and real gateway fixture, including lost acknowledgments, missing keyring, file permissions, key loss, and revoked identity. Record a real tenant/device-code proof when available.

Run the relevant formatting, lint, dependency-boundary, and supported-platform checks once code exists. Record real results and any unavailable environment; do not report planned or mocked external validation as completed.

## Boundaries and handoff

No service installer, remote command CLI, or network dispatcher. The standalone OAuth provider is authorized by ADR 0007; retain a provider interface for later corporate CLI integration.

**Deliver:** Host enrollment and credential APIs plus safe persisted state, consumed by 014 and 025.

## Implemented evidence

The host crate now has verified HTTPS PKCE and device grants for the dedicated remote scope, bounded polling/cancellation/expiry, state and S256 validation, durable origin/UID/installation/key binding, redacted secrets, and explicit recording disclosure/consent APIs. A real PostgreSQL gateway fixture proves lost enrollment acknowledgements, repeat/hostname reuse, machine-only authentication, private-file/key-loss handling, interrupted journaled key recovery preserving owner/machine, old-key rejection, and explicit unenrollment tombstones. Software Ed25519 keys are exportable even when stored in Linux Secret Service/macOS Keychain; the fallback is an unencrypted 0600 file in a private directory. No non-exportability claim is made. Native keychain/real tenant and CLI/browser-launch proofs remain external/025 integration gates. See the host crate README for configuration and verification.
