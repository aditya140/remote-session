# QGenie CLI integration inventory

Status: blocked on source access. Updated: 2026-10-16.

This document belongs to [task 001](../../tasks/001-inspect-existing-qgenie-integration.md). The inspected remote-session baseline is `e90cb3b`. No QGenie CLI revision has been inspected.

## Available evidence

The tracked files and workspace contain no existing QGenie executable source or CLI Cargo manifest. The only repository remote identifies `aditya140/remote-session`. The design documents refer to an existing CLI, but do not identify its repository location.

The development environment reports Rust `1.99.1` and Cargo `1.99.1`. These versions establish tool availability only. They do not establish the CLI's edition, MSRV, runtime, or dependency compatibility.

## Inspection required

The names in this table describe integration responsibilities. They are not existing API names or verified Rust traits.

| Responsibility | Evidence required from actual source | Consumer and follow-up |
| --- | --- | --- |
| Toolchain and packages | Toolchain pin, workspace edition/MSRV/resolver, Tokio conventions, dependency versions, release linkage | 003 must use the verified compatible toolchain. |
| Scoped credentials | Actual PKCE and device-grant APIs; remote audience/scope; polling, cancellation, cache, and refresh behavior | 011/013/025 inject scoped credentials. Do not copy OAuth code. |
| Signing and key storage | Key generation/signing APIs; Linux/macOS backends; exportability; fallback permissions; recovery and rotation | 013 reports actual protection and supplies signing without exporting keys into protocol types. |
| Launch preparation | Actual Codex installation/version selection, managed configuration, endpoint injection, environment preparation, and credential redaction | 009/025 launch through the CLI's facilities. Provider credentials stay on the host. |
| Services and updates | Actual systemd/launchd installation and update APIs; unit/job definitions; executable replacement; logout behavior | 025 supplies independent connector/sessiond services. |
| Redaction | Actual streaming redactor, token-pattern sources, handling of split tokens, and maximum retained context | 020 reuses redaction before recording leaves the host. 021 applies writer-side redaction. |

After source access is available, replace each missing entry with the exact revision, file path, symbol, supported platform, and a relevant verification command. Record missing facilities as follow-up work in the task that owns the integration.

## Required interface properties

- Credential access must accept an explicit audience and scope. A user inference token is insufficient.
- A signing interface must return a signature and a key reference. Debug output must not expose key material.
- Launch preparation must return host-local configuration. It must not put provider secrets into gateway messages or recordings.
- Service integration must manage the connector and session owner separately. Stopping the connector must not stop sessiond's cgroup or launchd job.
- Redaction must support bounded streaming state. A split token must not bypass the verified redaction contract.

Concrete signatures remain undecided until their consumers and the existing APIs are inspected.

## Package integration

[ADR 0001](../adr/0001-cli-integration.md) records the proposed package direction and the blocked toolchain decision. No dependency may point from a remote library to the CLI executable crate. The CLI must consume shared remote crates from one compatible revision.

## Unblocking input

Provide a readable repository URL or workspace path for the actual QGenie CLI. Identify the intended revision if it differs from the default branch. Tenant configuration is a later proof requirement; source inspection does not require copying credentials into this repository.
