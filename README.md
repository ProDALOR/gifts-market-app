# Gifts Market App

A stateless Rust REST and WebSocket gateway for Market Hub. Core owns the current book, market filtering, query indexes, pagination views, subscriptions and durable trade history. This repository owns public transport policy, JSON/OpenAPI formatting and the GUI boundary. Each request uses the same authenticated core connection; gateways keep no market replicas or history archives.

```text
venue adapters → market-hub (one writer + shared-state read workers)
                             ↑
                 market-app gateway → GUI / other services
```

The copied `market-hub.proto` is the service boundary. All crates build independently of other repositories; no hub model, state or history crate is imported.

## Run

Start the hub from its repository and create the external Docker network once (`docker network create market`). Configure the gateway with its own issued service token:

```sh
cp .env.example .env
# Set MARKET_NODE_TOKEN to the output of market-hub token --id market-app.
docker compose up --build --detach
```

This project starts only `market-app`. It needs a hub URL and its own issued service token; the hub signing secret stays with the operator. Compose defaults to `http://market-hub:50051` on the external `market` network. Set `MARKET_CORE_URL` and `MARKET_NETWORK` for another deployment. There are no setup scripts or connection bundles. HTTP/WebSocket listens on loopback port 8080. The gateway has no persistent volume.

For native development, export `MARKET_NODE_TOKEN` and run `cargo run --locked --bin market-app -- --core http://127.0.0.1:50051`. The application uses one immutable, validated client and cached authorization metadata for every core call.

## Read API

| Route | Purpose |
| --- | --- |
| `/healthz` | Gateway process liveness |
| `/status`, `/readyz` | Core readiness and current record/feed counts |
| `/catalog` | Filtered catalog definitions |
| `/feeds` | Current source status |
| `/items` | Current item observations |
| `/orders` | Indexed market queries or exact venue order lookup |
| `/orderbook` | Asks and bids from the same core view and expiry evaluation time |
| `/trades` | Durable confirmed executions |
| `/ws` | Core-backed subscriptions |
| `/openapi.json` | Generated HTTP request/response schemas |

All routes are read-only. Responses contain `data`, optional `page`/`included`, and `meta` with the core cursor, serving time and fixed query `as_of`. IDs, decimals, quantities, integer attributes and cursor revisions are lossless strings. All HTTP datetimes are UTC RFC 3339 strings.

```sh
curl --get http://localhost:8080/orderbook \
  --data-urlencode item_type_id=telegram.gift.5936013938331222567 \
  --data-urlencode currency_id=TON \
  --data-urlencode limit=20 \
  --data-urlencode include_items=true \
  --data-urlencode include_feeds=true
```

Candidate `attributes` are a catalog-aware JSON map. The gateway looks up the applicable attribute schemas only when attributes are supplied, then sends an explicitly typed `AttributeMap` to core. Enum IDs, exact decimals and integers use strings; booleans use JSON booleans. Core validates their types and applicability. Price filters are exact strings. `limit` is 1–200. `include_items=true` and `include_feeds=true` expand dependencies from the same core view.

Catalog continuations pin the current definitions and filters independently of live views; they survive book changes and core restarts while those definitions remain unchanged. Book/order continuations pin a bounded core view, filter scope and `as_of` for up to 60 seconds. Each book side has its own order-page continuation. Reuse it on `/orders` with the corresponding side and identical filters. HTTP 409 means reload the query. Cursors belong to core, so requests may switch gateway instances connected to that same core.

`/trades` supports `feed_id`, `item_type_id`, `currency_id`, `item_id`, `executed_from`, `executed_until`, `limit` and `cursor`. Date filters use RFC 3339. Results sort by execution time, newest first; ties use ingestion sequence. Historical pagination pins a durable watermark, excludes later arrivals, and survives core restarts. Trade IDs are feed-scoped. A current-record snapshot is separate from the durable trade dataset; historical continuations do not act as raw `Sync` cursors.

## WebSocket

Subscribe to a bounded orderbook:

```json
{"type":"subscribe","subscription_id":"book","topic":"orderbook","query":{"item_type_id":"telegram.gift.5936013938331222567","currency_id":"TON","limit":20}}
```

Core streams plain `BookResult` messages. The gateway labels the initial view `snapshot` and subsequent full views `replace` for WebSocket clients. Core evaluates expiry, coalesces updates and shares identical book computations. Reconnect by subscribing again. Send `{"type":"unsubscribe","subscription_id":"book"}` to release a subscription.

For the raw ordered market stream, use `topic:"changes"` with an optional `after:{generation,revision:"123"}`. Frames have `{type:"sync",subscription_id,response}`. `response.kind` is `snapshot_start`, `snapshot_data`, `snapshot_complete`, `delta`, `checkpoint` or `heartbeat`; records/catalog/events use the public JSON codecs. Activate snapshots only at `snapshot_complete`, and delta revisions only at `complete:true`. Persist only complete cursors. Empty revisions still advance the cursor. Invalid generations or lost replay require a fresh subscription without `after`. Raw subscriptions include all market events and preserve core's protocol boundaries.

Each socket accepts at most 16 subscriptions and 64 KiB input messages. Queues and send deadlines are bounded; slow clients disconnect without blocking core. Browser origins must match the gateway or be allowed with repeated `--cors-origin`. Use an HTTPS reverse proxy for public hosting and any customer authentication policy.

## Other services

Trusted services connect directly to core’s native gRPC endpoint with their own CLI-issued service token and generated `market-hub.proto` client. The gateway exposes REST and WebSocket only; it has no duplicate gRPC listener or separate downstream token. Core owns status, catalogs, indexed queries, durable trades, book subscriptions, raw stored-order inspection, replay and snapshot/delta synchronization.

`crates/app/examples/read_core.rs` demonstrates direct core access. Set `MARKET_NODE_TOKEN` to a token issued for that service, then use `cargo run --locked --example read_core -- --endpoint http://127.0.0.1:50051`.

## Workspace and verification

| Crate | Responsibility |
| --- | --- |
| `market-app` | REST/WebSocket gateway, bounded transport admission and OpenAPI |
| `hub-protocol` | Generated bindings from the local contract copy |
| `market-view` | Response projections used for JSON formatting |
| `market-json` | Public response formatting, query scalar parsing and schemas |

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --locked --bins --examples
python3 scripts/smoke.py --hub-bin /path/to/market-hub --adapter-manifest /path/to/adapters/Cargo.toml
```

Standalone tests use protocol peers. The integration check uses disposable token-authenticated HTTP/2 services, random ports, temporary credentials and synthetic venue facts. It verifies indexed books, a slow order scan with independent lease renewal and concurrent trade ingestion, durable trades after a hard core crash, persistent source-cursor resume and overlapping-page deduplication, operational status, stored-order inspection, bounded replay, read subscriptions and gateway/adapter reconnects without access-secret distribution or registration. It never opens Telegram accounts or contacts real markets.

The gateway decodes hub responses and formats the public JSON shape. It has no catalog compiler, order matching engine or record-import parsers. Query attributes are decoded using the applicable attribute schemas returned by core; core enforces catalog and trading rules. Exact numeric strings, flattened records and RFC 3339 dates remain part of the public format.

Review and copy each hub contract update into this repository, then regenerate bindings through the normal build. Core's `scripts/check-contracts.py` compares explicit schema paths before a coordinated release. Ordinary builds use only this repository.
