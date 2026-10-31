# Run and improve the local development deployment

Status: Complete. The full deployment and developer commands pass locally.

## Goal

Provide a reproducible local application deployment with real user sign-in.
Run the CLI, host connector, independent session owner, gateways, PostgreSQL,
Redis, recording services, and identity provider together. Record the exact
setup and executed end-to-end checks. Keep external release qualification separate.

## Findings before implementation

The devcontainer starts database dependencies but does not start the application.
The four-gateway deployment has a synthetic issuer that does not authenticate a
person. The gateway requires Entra-style tenant/object claims and current account
enabled evidence. Keycloak therefore needs explicit development claim mappings
and a small directory compatibility service. Do not weaken gateway validation.

The workspace has approximately 5 GB free. Most storage is regenerable Rust build
output. Preserve usable native binaries and private state before reclaiming build
cache space. Use only the local Docker engine and resources owned by this task.

## Implementation sequence

1. Inspect requirements, deployment contracts, runtime tools, resources, and
   existing authentication flows. Prepare enough build space without deleting
   source, recordings, credentials, or unrelated Docker resources.
2. Build and run the full existing deployment. Exercise its native session,
   recording, monitoring, restart, and dependency-recovery checks locally.
3. Add an optional pinned Keycloak development setup. Use real PKCE/device OAuth,
   generated local credentials, verified local HTTPS, and stable claim mappings.
   Translate actual Keycloak enabled-state responses into the existing directory
   contract. Keep the synthetic CI profile and production gateway behavior intact.
4. Add a small development entry point that generates connection files and gives
   reproducible commands for startup, host enrollment, client use, status, and
   shutdown. Preserve persistent database and identity state across restarts.
5. Test real sign-in and enrollment, owner isolation, browsing, remote terminal
   input, disconnect/reconnect survival, history/redaction, and revocation.
   Exercise the real TUI. Check provider failure and recovery where supported.
   Test native Codex with the existing isolated fixture and state its limits.
6. Document setup commands, component addresses, credentials and CA handling,
   persistence, cleanup, executed checks, observed failures, and remaining limits.
   Commit the working setup and sanitized evidence. Leave an inspectable local
   development deployment available.

## Evidence

The final clean Keycloak seed passes all 12 real-user checks in 148.738 seconds.
The synthetic four-gateway deployment, monitoring evaluator, and backup restore
also pass locally. The native Codex acceptance runner passes all five phases.
The workspace suite passes 220 tests, with 23 ignored environment proofs. Strict
Clippy, formatting, and all 10 Python tool tests pass.
The final gateway image passes the 12-check real-user proof again in 140.567
seconds. All 26 gateway checks pass, including the PostgreSQL/Redis proof and a
handshake regression that fails against the old gateway. The developer commands
pass browser login, host diagnostics, actual TUI use, and a full shutdown/restart.
The same native demo session and original start key survive that restart.

Implementation adds the optional Keycloak profile, a real directory adapter,
private configuration, local build cache mounts, and `tools/dev.py`. It fixes
public source-file permissions, loopback redirect registration, missing token
claims, refresh permission, encoded directory queries, and optional device PKCE.
It also fixes a client-handshake ordering race and waits for the public gateway
route before it reports startup success. The local stack and demo remain running.

The [execution report](../docs/validation/local-development-2026-10-31.md) records
commands, successful checks, failed attempts, and limits. The
[development guide](../docs/development/local-stack.md) gives daily setup and
shutdown procedures. External enterprise, service-manager, model-service, and
production qualification remain separate.
