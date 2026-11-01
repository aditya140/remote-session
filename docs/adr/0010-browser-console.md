# ADR 0010: Gateway-hosted browser console

Status: approved implementation scope. The user approved the admin and user
console design and requested implementation on 2026-11-01.

## Context

The original first-release client was the standalone TUI. Admins now need browser
inventory, audit, governance, and aggregate activity views. Owners need to see
which devices run their work and stop one selected session.

## Decision

Serve a dependency-free browser application from the existing gateway origin.
Keep the native owner as the only execution authority. Reuse existing protected
recording, revocation, legal hold, and correlated session termination operations.
Add bounded owner and admin metadata read APIs in the gateway.

Use OIDC authorization code with PKCE. Store opaque browser session identities in
cookies and encrypted access tokens in PostgreSQL. All gateways share the browser
key and session authority. Check origin and CSRF for cookie-authenticated actions.
Bind browser WSS lifetime to browser logout and expiry. Keep bearer CLI behavior.

Model configuration is an optional native observation. Requested configuration
is distinct from effective model reporting. Missing usage and unknown native
state remain explicit. Activity rules show real committed-event evidence without
automatic enforcement. No template or sample series enters the production UI.

## Boundaries

This companion does not provide a full browser terminal, arbitrary remote shell
execution, group policy delivery, or a scheduled anomaly engine. Unsupported
policy editing stays unavailable. The current owner confirmation and independent
approval rights remain authoritative. Metadata visibility grants no cross-owner
control or transcript access.

Native SQLite stays compatible through optional metadata. PostgreSQL schema 18
adds shared browser sessions, one-time login states, and audit lookup indexes.
Old gateways reject the newer schema under the existing migration contract.
Use a backup and coordinated deployment. Do not restore an old backup over new
acknowledged inputs or recording receipts.

## Evidence

[Task 040](../../tasks/040-implement-browser-console.md) records checks and the
local deployment proof. Linux Keycloak evidence does not qualify production
Entra registration, production infrastructure, or other native platforms.
