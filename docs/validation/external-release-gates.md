# Release qualification outside the Linux development scope

The user selected: **“Finish Linux development scope; document external release gates.”**
This scope uses the standalone CLI/TUI authorized in
[ADR 0007](../adr/0007-standalone-preview.md). Completion of development does not assert
enterprise release qualification. Original acceptance criteria spanning these environments
remain unchecked in the task files.

| Gate | Tasks | Required evidence | Current state |
| --- | --- | --- | --- |
| Real Entra configuration and disablement | 011, 013, 019, 027 | Authorized tenant, application permissions, dedicated audience/scope, device/browser flows, still-valid user and machine credentials denied within 15 minutes of actual disablement | Not run; HTTPS signed fixture and elapsed freshness/failure checks passed |
| macOS execution and credentials | 006, 008, 009, 013, 025, 027 | Native PTY/Codex, peer credentials, keychain, launchd lifetime and connector replacement on a Mac | Hosted macOS 15 PTY, IPC, CLI, TUI, recording, and history checks pass; native Codex, keychain, and launchd qualification remain open |
| Linux user service manager | 025, 027 | Real user systemd, logout/login, cgroup separation, upgrades and connector stop with active terminal and agent recording | Unit syntax and separate-process lifecycle checks passed; user-manager/logout drill not run |
| Real suspend and extended network faults | 019, 025, 027 | Suspend/resume, corporate proxy deployment, prolonged disconnect, network partitions and process fault combinations on supported hosts | Deterministic elapsed-clock and verified CONNECT fixtures passed; deployment and extended soak matrix not run |
| Production durability | 021, 022, 026, 027 | Separate fault domains, synchronous replica loss/failover, recovery epoch/receipt reconciliation and off-server restore | Same-host synchronous PostgreSQL, actual primary-volume destruction, promotion and isolated restore passed; production topology unqualified |
| Production operations | 022, 028 | Named on-call and backup owners, backup target, scheduled lifecycle jobs, supervised probe, notification destination and delivery drill, measured thresholds | Development jobs/evaluator/probe/runbooks exist; production assignments, supervision, notification delivery and off-server target unconfigured |
| Full beta capacity | 028 | 200,000 sockets, 20,000 structured streams, 2,000 terminals producing 50 KB/s bursts, representative traffic, latency/write-lag distributions and long-run resources | Reduced 8-socket/2-terminal/200-echo baseline only; storage extrapolation is an estimate |
| Corporate CLI integration, if later requested | 001, 003, 009, 025 | Inspect actual corporate revision, integrate its authentication/configuration APIs and run compatibility checks | User waived this dependency in favor of the standalone application |

The supported managed-agent subset pins Codex 0.159.2. Runtime IPC reconnection to a lost
native peer is explicitly unsupported: uncertain/lost execution is surfaced, never silently
relaunched. Historical resume creates a new session/process in the selected directory from
the saved thread. External session adoption, other agents, Windows and the production
three-server deployment remain outside this implementation's declared scope. These limits
are recorded in the [capability matrix](../integration/codex-capabilities.md).

The [MVP evidence](mvp-evidence.md), [Linux checks](linux-development-checks-2026-10-18.json),
[synchronous acceptance](synchronous-acceptance-2026-10-18.json) and
[capacity report](capacity.md) distinguish executed development proofs from these gates.
An ignored test is not counted as a passed proof.
