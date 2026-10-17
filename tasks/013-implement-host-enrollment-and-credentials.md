# 013 — Implement host enrollment and machine credential storage

- **Status:** Not started
- **Milestone:** M0 enrollment
- **Workstream:** Host identity
- **Depends on:** [001](001-inspect-existing-qgenie-integration.md), [005](005-implement-websocket-transport.md), [011](011-implement-gateway-identity-and-enrollment.md)
- **Index:** [Task plan](README.md)

## Goal and repository context

Provide idempotent host enrollment using the existing CLI's authentication and credential facilities.

**Read first:** [PRD.md](../PRD.md) §5.1, ID-1–5, ID-8; [architecture.md](../architecture.md) §§6.1–6.2; [components.md](../components.md) §6.

## Owned scope

- `crates/qgenie-remote-host/src/{config,enrollment,error}.rs`
- `Host identity/{key,challenge,token}.rs and local identity persistence`

Paths describe proposed implementation locations; they do not exist yet unless another task creates them. Follow [the subcrate standard](../subcrate-standard.md), add only useful modules, and coordinate shared manifests/exports and migrations as described in the index.

## Implementation

1. Consume the existing scoped OAuth provider: browser PKCE by default where supported, device grant for headless/SSH or explicit --device-code, respecting polling intervals, cancellation, expiry, and tenant errors.
2. Generate/store the machine signing identity through the verified integration, disclose actual protection/fallback, and persist enrollment state safely across interrupted requests.
3. Sign gateway challenges, refresh short-lived machine tokens, and retain machine identity across repeated up calls and hostname changes.
4. Expose enrollment/revocation/unenrollment operations for CLI composition. Include mandatory recording/retention disclosure and the adoption disclosure when that later capability is enabled.

## Acceptance criteria

- [ ] Repeated enrollment after lost replies reuses identity without duplicate machine records.
- [ ] A headless device-code flow and browser PKCE flow use the dedicated remote scope and clear cancellation/error states.
- [ ] Long-running host authentication needs only machine credentials; user refresh tokens are not passed to the connector.
- [ ] Fallback files are private, secret material is redacted, and a revoked identity never silently re-enrolls.

## Verification

- Run enrollment tests against a fake OAuth provider and real gateway fixture, including lost acknowledgments, missing keyring, file permissions, key loss, and revoked identity. Record a real tenant/device-code proof when available.

Run the relevant formatting, lint, dependency-boundary, and supported-platform checks once code exists. Record real results and any unavailable environment; do not report planned or mocked external validation as completed.

## Boundaries and handoff

No service installer, remote command CLI, network dispatcher, or duplicated QGenie OAuth implementation.

**Deliver:** Host enrollment and credential APIs plus safe persisted state, consumed by 014 and 025.
