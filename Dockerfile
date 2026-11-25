# syntax=docker/dockerfile:1
FROM rust:1.98-bookworm AS binaries
WORKDIR /build
COPY Cargo.toml Cargo.lock market-hub.proto ./
COPY crates/ crates/

RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/build/target,sharing=locked \
    cargo build --release --locked -p market-app -p market-media --bin market-app --bin market-media-render --all-features \
    && install -Dm755 target/release/market-app /out/market-app \
    && install -Dm755 target/release/market-media-render /out/market-media-render

FROM node:24-bookworm-slim AS web-builder
WORKDIR /web
COPY web/package.json web/package-lock.json ./
RUN --mount=type=cache,target=/root/.npm npm ci
COPY web/ ./
COPY crates/market-media/assets/ /crates/market-media/assets/
COPY --from=binaries /out/market-app /usr/local/bin/market-app
RUN MARKET_SCHEMA_BIN=/usr/local/bin/market-app npm run build

FROM debian:bookworm-slim AS market-app
RUN apt-get update \
    && apt-get install -y --no-install-recommends curl \
    && rm -rf /var/lib/apt/lists/* \
    && install -d -o 10001 -g 10001 /data /bot-data
WORKDIR /app
COPY --from=binaries /out/market-app /usr/local/bin/market-app
COPY --from=binaries /out/market-media-render /usr/local/bin/market-media-render
COPY licenses/velato/ /usr/share/licenses/velato/
COPY --from=web-builder /web/dist /app/web/dist
ENV MARKET_DATA_DIR=/data \
    MARKET_BOT_DATA_DIR=/bot-data \
    RUST_LOG=info
USER 10001:10001
EXPOSE 8080
HEALTHCHECK --interval=10s --timeout=3s --start-period=15s --retries=3 CMD ["curl", "--fail", "--silent", "http://127.0.0.1:8080/readyz"]
ENTRYPOINT ["market-app"]
CMD ["--bind", "0.0.0.0:8080"]
