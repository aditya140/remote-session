FROM mcr.microsoft.com/devcontainers/rust:1-bookworm@sha256:681df5d201782e6ebf4855e4ff3c1c92e222ba0486fec47acc924aae0799b106

USER root

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        build-essential \
        ca-certificates \
        cmake \
        curl \
        git \
        jq \
        libsqlite3-dev \
        libssl-dev \
        pkg-config \
        postgresql-client \
        python3 \
        redis-tools \
        ripgrep \
        sqlite3 \
        tmux \
        zstd \
    && rm -rf /var/lib/apt/lists/*

USER vscode

RUN rustup component add rustfmt clippy rust-src

WORKDIR /workspaces/remote-session
