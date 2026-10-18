# ADR 0007: Build a standalone TUI

Status: selected by user direction on 2026-10-16.

## User direction

The user stated: "not needed create a sample tui or a new tui where needed". The user also confirmed the goal of finishing the tasks.

## Decision

The existing QGenie CLI source is no longer a prerequisite for local implementation. Build `apps/qgenie-remote` as a standalone composition root. Keep the reusable protocol, transport, policy, adapter, sessiond, client, and TUI boundaries. Add host and gateway crates when their actual behavior is implemented.

Use Rust 1.99.1, edition 2024, and resolver 3 for this standalone workspace. The toolchain is available and has built the protocol and local components. This is a new project's toolchain choice. It is not a claim about compatibility with the absent corporate CLI.

The first runnable flow uses a private same-user Unix socket and an independent local daemon. It advertises shell/PTY capability only. It provides durable start identities, local replay, control leases, and explicit termination. The standalone process launcher is not proof of systemd or launchd isolation.

Corporate OAuth, directory disablement, native Codex capabilities, WSS routing, PostgreSQL history, legal holds, and deployment scale retain their original implementation and evidence requirements. A local shell preview does not claim those capabilities. It is not an enterprise rollout or a substitute for the protected-recording release gate.

Task 001's original external source inspection is superseded for the standalone path. Its integration inventory remains historical evidence. Task 003 can proceed using this toolchain and the new composition root. When corporate integration is requested, implement it through verified facilities without changing the reusable crate direction.

## Development completion scope — 2026-10-18

The user explicitly selected: "Finish Linux development scope; document external release
gates" in response to the macOS, real Entra disablement and beta-hardware prerequisites.
Complete and verify the Linux standalone implementation and development fixtures. Keep
real tenant, macOS/platform services, production fault domains/operators and full beta-scale
qualification documented as unrun release gates. This decision does not label mocked or
unrun external validation as passed, and does not remove functional Linux requirements.
