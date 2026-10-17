# remote-session

This repository contains the design for QGenie Remote.
QGenie Remote provides terminal and coding-agent sessions through a corporate gateway.
The first release uses a Rust terminal user interface (TUI).

The repository does not yet contain application code or a Cargo workspace.
The development container configuration is an initial setup. It has not been built or tested.

## Project documents

- [PRD.md](PRD.md) defines the product requirements.
- [architecture.md](architecture.md) describes the system design.
- [components.md](components.md) proposes the crate boundaries.
- [subcrate-standard.md](subcrate-standard.md) defines the proposed Rust conventions.
- [AGENTS.md](AGENTS.md) gives instructions for work in this repository.

## Development container

The configuration in [.devcontainer](.devcontainer/devcontainer.json) provides these tools and services:

| Component | Purpose |
| --- | --- |
| Rust, Cargo, rustfmt, and Clippy | These tools build, format, and check Rust code. |
| PostgreSQL 18.6 | This service stores local development data. |
| Redis 8.2.10 | This service provides local routing and presence data. |
| SQLite tools and libraries | These tools support local session storage development. |
| tmux | This tool supports terminal session development. |
| Git, ripgrep, jq, curl, and native build tools | These tools support source changes and diagnostics. |

The `dev` service uses the `vscode` account.
It mounts the repository at `/workspaces/remote-session`.
PostgreSQL and Redis use the private Compose network. They do not publish host ports.
The configuration starts these services but does not start a gateway or host connector.

PostgreSQL keeps its data in a named volume.
Redis does not keep data after its container stops.
Cargo stores build output inside the development container.
A container rebuild can remove Cargo caches and build output.

### Open the environment

The configuration requires a compatible container runtime and a Dev Containers client.

1. Start the container runtime.
2. Open this repository in VS Code with the Dev Containers extension.
3. Select **Dev Containers: Reopen in Container** from the command palette.
4. Use the container terminal for development commands.

### Service configuration

The `dev` service receives these environment variables:

| Variable | Development value |
| --- | --- |
| `QGR_DATABASE_URL` | `postgres://qgenie_dev:local_dev_only@postgres:5432/qgenie_remote` |
| `QGR_REDIS_URL` | `redis://redis:6379/0` |
| `QGR_LISTEN` | `0.0.0.0:8080` |
| `QGR_ADMIN_LISTEN` | `0.0.0.0:9090` |

These values prepare the environment for the proposed gateway configuration.
No application reads them yet.
The editor can forward ports 8080 and 9090 when an application uses them.

Use the supplied password only for local development.
The `qgenie_dev` account is a database administrator.
Tests for row-level security require separate application roles and policies.
The repository does not yet contain those roles or policies.

### Current limits

The container image supplies the initial Rust toolchain.
The project minimum supported Rust version and edition remain undecided.
The team must inspect the existing QGenie CLI before it selects those values.

The container does not include QGenie credentials or native coding-agent installations.
It does not provide systemd, launchd, or native credential-store integration.
Tests for host service behavior require the applicable native operating system.

The configuration follows the [Dev Container specification](https://containers.dev/implementors/json_reference/).
Its Rust base comes from the [Dev Container Rust image](https://github.com/devcontainers/images/tree/main/src/rust).
