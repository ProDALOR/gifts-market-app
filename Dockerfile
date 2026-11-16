# syntax=docker/dockerfile:1
ARG RUST_VERSION=1.98
FROM rust:${RUST_VERSION}-bookworm AS builder
WORKDIR /build
COPY Cargo.toml Cargo.lock market-hub.proto ./
COPY crates/ crates/

FROM builder AS binaries
ARG MARKET_FEATURES=""
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/build/target,sharing=locked \
    cargo build --release --locked -p market-app --bin market-app --no-default-features --features "$MARKET_FEATURES" \
    && install -Dm755 target/release/market-app /out/market-app

FROM node:24-bookworm-slim AS web-builder
WORKDIR /web
COPY web/package.json web/package-lock.json ./
RUN --mount=type=cache,target=/root/.npm npm ci
COPY web/ ./
COPY --from=binaries /out/market-app /usr/local/bin/market-app
RUN MARKET_SCHEMA_BIN=/usr/local/bin/market-app npm run build

FROM debian:bookworm-slim AS runtime
RUN apt-get update && apt-get install -y --no-install-recommends curl && rm -rf /var/lib/apt/lists/*
WORKDIR /app
RUN mkdir /data /bot-data && chown 10001:10001 /data /bot-data
ENV MARKET_DATA_DIR=/data
ENV MARKET_BOT_DATA_DIR=/bot-data
ENV RUST_LOG=info
USER 10001:10001

FROM runtime AS service
COPY --from=binaries /out/market-app /usr/local/bin/market-app
EXPOSE 8080
HEALTHCHECK --interval=10s --timeout=3s --start-period=15s --retries=3 CMD ["curl", "--fail", "--silent", "http://127.0.0.1:8080/readyz"]
ENTRYPOINT ["market-app"]
CMD ["--bind", "0.0.0.0:8080"]

# API-only target skips the entire Node/frontend build graph.
FROM service AS market-api
CMD ["--bind", "0.0.0.0:8080", "--no-web"]

FROM service AS market-app
COPY --from=web-builder /web/dist /app/web/dist
