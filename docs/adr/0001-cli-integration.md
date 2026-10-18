# ADR 0001: Integrate through the existing QGenie CLI

Status: package direction proposed; toolchain and API selection blocked on [task 001](../../tasks/001-inspect-existing-qgenie-integration.md).

## User task and problem

Users must run remote commands through their distributed QGenie CLI. This repository has no copy of that CLI's platform facilities. Choosing replacement APIs or a Rust version without its source can prevent integration.

## Proposed action and reason

Keep reusable remote crates and the gateway in this repository. The existing CLI composes the client, TUI, host, and sessiond libraries. Pin its remote dependencies to one Git revision until a coordinated package release exists. Follow the dependency direction in [components](../../components.md).

Implement the CLI-side credential, signing, launch, service, and redaction glue beside its command composition code. Use verified existing facilities. Do not depend on the CLI executable crate from a remote library.

Do not choose a product toolchain, edition, or MSRV until the existing CLI is inspected. The installed development toolchain is not compatibility evidence. Do not create a substitute CLI or OAuth stack to remove this blocker.

## Evidence and remaining work

[The integration inventory](../integration/qgenie-cli.md) records what is available and what must be inspected. Task 001 remains blocked. Task 003 cannot be completed from this proposal alone. Task 002 can continue because its contracts do not depend on a Rust edition or unverified CLI symbols.
