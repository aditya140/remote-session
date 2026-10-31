# syntax=docker/dockerfile:1.7
# Local builders keep mutable Cargo caches. This avoids copying a complete Rust
# toolchain and target directory into many layers on nested Docker VFS engines.
# CI uses Dockerfile's exported dependency layers instead.
FROM rust:1.99.1-alpine@sha256:557a94c82b485a14b569120e00867267108c734efe33f10cc31ca0cd5dd1eca9 AS binaries
ENV RUSTUP_TOOLCHAIN=1.99.1
WORKDIR /source
COPY . .
RUN --mount=type=secret,id=ca_bundle \
    --mount=type=cache,id=qgr-local-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=qgr-local-release,target=/source/target \
    if [ -f /run/secrets/ca_bundle ]; then export CARGO_HTTP_CAINFO=/run/secrets/ca_bundle; fi; \
    cargo build --release --locked -p qgenie-remote-gateway -p qgenie-remote-fixture-identity && \
    mkdir -p /out && \
    cp target/release/qgenie-remote-gateway /out/gateway && \
    cp target/release/qgenie-remote-fixture-identity /out/fixture-identity

FROM gcr.io/distroless/static-debian13:nonroot@sha256:e2e927ec666bae08560abb3c55d0659eceabb657f56b6782ab500a9fc7f555e3 AS fixture-identity
COPY --from=binaries /out/fixture-identity /app/fixture-identity
USER 10001:10001
ENTRYPOINT ["/app/fixture-identity"]

FROM gcr.io/distroless/static-debian13:nonroot@sha256:e2e927ec666bae08560abb3c55d0659eceabb657f56b6782ab500a9fc7f555e3 AS gateway
COPY --from=binaries /out/gateway /app/gateway
USER 10001:10001
EXPOSE 8080 9091
ENTRYPOINT ["/app/gateway"]
