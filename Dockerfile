# syntax=docker/dockerfile:1
ARG RUST_VERSION=1.98
FROM rust:${RUST_VERSION}-bookworm AS builder
WORKDIR /build
COPY Cargo.toml Cargo.lock market-hub.proto ./
COPY crates/ crates/

FROM builder AS api-builder
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/build/target,sharing=locked \
    cargo build --release --locked -p market-app --bin market-app \
    && install -Dm755 target/release/market-app /out/market-app

FROM debian:bookworm-slim AS runtime
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl && rm -rf /var/lib/apt/lists/*
WORKDIR /app
ENV RUST_LOG=info
RUN install -d -o 10001 -g 10001 /var/lib/market/node
USER 10001:10001
STOPSIGNAL SIGINT

FROM runtime AS market-app
COPY --from=api-builder /out/market-app /usr/local/bin/market-app
EXPOSE 8080 50053
HEALTHCHECK --interval=10s --timeout=3s --start-period=15s --retries=3 CMD ["curl", "--fail", "--silent", "http://127.0.0.1:8080/readyz"]
ENTRYPOINT ["market-app"]
CMD ["--bind", "0.0.0.0:8080"]
