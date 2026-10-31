# syntax=docker/dockerfile:1.7
FROM rust:1.99.1-alpine@sha256:557a94c82b485a14b569120e00867267108c734efe33f10cc31ca0cd5dd1eca9 AS chef
ENV RUSTUP_TOOLCHAIN=1.99.1
WORKDIR /source
# The CA is build-only. Keep TLS verification enabled in every networked stage.
RUN --mount=type=secret,id=ca_bundle \
    if [ -f /run/secrets/ca_bundle ]; then export CARGO_HTTP_CAINFO=/run/secrets/ca_bundle; fi; \
    cargo install cargo-chef --version 0.1.78 --locked

FROM chef AS planner
COPY . .
RUN --mount=type=secret,id=ca_bundle \
    if [ -f /run/secrets/ca_bundle ]; then export CARGO_HTTP_CAINFO=/run/secrets/ca_bundle; fi; \
    cargo chef prepare --recipe-path recipe.json

FROM chef AS dependencies
COPY --from=planner /source/recipe.json recipe.json
# These patched crates are outside the workspace. Compile their real sources.
COPY vendor vendor
# Store registry data and compiled dependencies in the exported layer. Mutable
# BuildKit cache mounts are not preserved by the GitHub cache exporter.
RUN --mount=type=secret,id=ca_bundle \
    if [ -f /run/secrets/ca_bundle ]; then export CARGO_HTTP_CAINFO=/run/secrets/ca_bundle; fi; \
    cargo chef cook --release --locked --recipe-path recipe.json \
        -p qgenie-remote-gateway -p qgenie-remote-fixture-identity

FROM dependencies AS source
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY .cargo .cargo
COPY crates crates
COPY apps apps
COPY xtask xtask
COPY tests/fixtures/identity tests/fixtures/identity
COPY conformance/identity conformance/identity

# Match the cooked package set and compile shared crates only once.
FROM source AS binaries
RUN --mount=type=secret,id=ca_bundle \
    if [ -f /run/secrets/ca_bundle ]; then export CARGO_HTTP_CAINFO=/run/secrets/ca_bundle; fi; \
    cargo build --release --locked \
        -p qgenie-remote-gateway -p qgenie-remote-fixture-identity && \
    mkdir -p /out && \
    cp target/release/qgenie-remote-gateway /out/gateway && \
    cp target/release/qgenie-remote-fixture-identity /out/fixture-identity

# Explicit development-only image. The production image contains no fixture.
FROM gcr.io/distroless/static-debian13:nonroot@sha256:e2e927ec666bae08560abb3c55d0659eceabb657f56b6782ab500a9fc7f555e3 AS fixture-identity
COPY --from=binaries /out/fixture-identity /app/fixture-identity
USER 10001:10001
ENTRYPOINT ["/app/fixture-identity"]

# Default target is the production gateway; fixture identity is always explicit.
FROM gcr.io/distroless/static-debian13:nonroot@sha256:e2e927ec666bae08560abb3c55d0659eceabb657f56b6782ab500a9fc7f555e3 AS gateway
COPY --from=binaries /out/gateway /app/gateway
USER 10001:10001
EXPOSE 8080 9091
ENTRYPOINT ["/app/gateway"]
