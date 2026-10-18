# syntax=docker/dockerfile:1.7
FROM rust:1.99.1-alpine@sha256:557a94c82b485a14b569120e00867267108c734efe33f10cc31ca0cd5dd1eca9 AS build-base
ENV RUSTUP_TOOLCHAIN=1.99.1
WORKDIR /source
COPY . .
# Optional build-only enterprise CA. No credentials, compiler cache or CA override
# are copied into runtime images. Docker's predefined proxy args stay build-only.
FROM build-base AS gateway-build
RUN --mount=type=secret,id=ca_bundle \
    --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/source/target \
    if [ -f /run/secrets/ca_bundle ]; then export CARGO_HTTP_CAINFO=/run/secrets/ca_bundle; fi; \
    cargo build --release --locked -p qgenie-remote-gateway && \
    mkdir -p /out && cp target/release/qgenie-remote-gateway /out/gateway

FROM build-base AS fixture-build
RUN --mount=type=secret,id=ca_bundle \
    --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/source/target \
    if [ -f /run/secrets/ca_bundle ]; then export CARGO_HTTP_CAINFO=/run/secrets/ca_bundle; fi; \
    cargo build --release --locked -p qgenie-remote-fixture-identity && \
    mkdir -p /out && cp target/release/qgenie-remote-fixture-identity /out/fixture-identity

# Explicit development-only target; absent from the production gateway target.
FROM gcr.io/distroless/static-debian13:nonroot@sha256:e2e927ec666bae08560abb3c55d0659eceabb657f56b6782ab500a9fc7f555e3 AS fixture-identity
COPY --from=fixture-build /out/fixture-identity /app/fixture-identity
USER 10001:10001
ENTRYPOINT ["/app/fixture-identity"]

# Default target is the production gateway; fixture identity is always explicit.
FROM gcr.io/distroless/static-debian13:nonroot@sha256:e2e927ec666bae08560abb3c55d0659eceabb657f56b6782ab500a9fc7f555e3 AS gateway
COPY --from=gateway-build /out/gateway /app/gateway
USER 10001:10001
EXPOSE 8080 9091
ENTRYPOINT ["/app/gateway"]
