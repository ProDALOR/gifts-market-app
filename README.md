# Gifts Market App

A stateless Rust REST, WebSocket and gRPC gateway for Market Hub. Core owns the current book, market filtering, query indexes, pagination views, subscriptions and durable trade history. This repository owns public transport policy, JSON/OpenAPI formatting and the GUI boundary. Each request uses the same authenticated core connection; gateways keep no market replicas or history archives.

```text
venue adapters → market-hub (one writer + shared-state read workers)
                             ↑
                 market-app gateway → GUI / other services
```

The copied `market-hub.proto` is the service boundary. All crates build independently of other repositories; no hub model, state or history crate is imported.

## Run

Start the hub from its repository and create the external Docker network once (`docker network create market`). Import its private join bundle:

```sh
sh docker/setup.sh /path/to/hub/.docker/join
docker compose up --build --detach --wait
```

This project starts only `market-app`. It uses the bundle's core URL, public CA trust, join token and network name. Its own API certificate and downstream credential are generated separately. HTTP/WebSocket is published on loopback port 8080 and downstream TLS gRPC on 50053. The default core URL is `https://market-hub:50051`; `MARKET_CORE_URL` overrides it.

The `api-identity` volume retains the node's private identity. Use the same Compose project name on restart. Another gateway needs its own identity volume and host ports:

```sh
MARKET_API_PORT=8180 MARKET_API_GRPC_PORT=51053 docker compose -p market-app-2 up --build --detach
```

For native development:

```sh
cargo run --locked -p market-app -- --core http://127.0.0.1:50051
```

Supply `MARKET_JOIN_TOKEN` for first enrollment. The gateway saves role-free `{id, token}` credentials with an independently generated node token in `.market/market-app.json` before connecting. A normal request uses that node token; only an unknown credential triggers enrollment. Established nodes need no join token, including after a core restart or join-token rotation. Public HTTPS certificates use system trust; `--core-ca` supplies private CA trust. Client certificate/key options remain available for deployments requiring mutual TLS.

Core treats every enrolled node as a trusted application. The gateway exposes only read operations to downstream clients; its own private core credential permits reads and writes within its node namespace. The join token, node token and downstream API token stay separate.

The gateway reuses one HTTP/2 channel. Unary attempts and stream establishment have five-second deadlines; healthy subscriptions have no short lifetime limit. Query/stream admission is bounded independently of core's read workers. Core unavailability returns a service-unavailable response; there is no local stale-market fallback. Core's writer remains independent of its read-worker budget.

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

All routes are read-only. Responses contain `data`, optional `page`/`included`, and `meta` with the core cursor, catalog revision, serving time and fixed query `as_of`. IDs, decimals, quantities, integer attributes and cursor revisions are lossless strings. All HTTP datetimes are UTC RFC 3339 strings.

```sh
curl --get http://localhost:8080/orderbook \
  --data-urlencode item_type_id=telegram.gift.5936013938331222567 \
  --data-urlencode currency_id=TON \
  --data-urlencode limit=20 \
  --data-urlencode include=item,feed
```

Candidate `attributes` are a catalog-aware JSON map. Enum IDs, exact decimals and integers use strings; booleans use JSON booleans. Core validates their types and applicability. Price filters are exact strings. `limit` is 1–200. `include=item,feed` expands dependencies from the same core view.

Catalog continuations pin catalog revision and filters independently of live views; they survive book changes and core restarts while those definitions remain unchanged. Book/order continuations pin a bounded core view, filter scope and `as_of` for up to 60 seconds. Each book side has its own order-page continuation. Reuse it on `/orders` with the corresponding side and identical filters. HTTP 409 means reload the query. Cursors belong to core, so requests may switch gateway instances connected to that same core.

`/trades` supports `feed_id`, `item_type_id`, `currency_id`, `item_id`, `executed_from`, `executed_until`, `limit` and `cursor`. Date filters use RFC 3339. Results sort by execution time, newest first; ties use ingestion sequence. Historical pagination pins a durable watermark, excludes later arrivals, and survives core restarts. Trade IDs are feed-scoped. A current-record snapshot is separate from the durable trade dataset; historical continuations do not act as raw `Sync` cursors.

## WebSocket

Subscribe to a bounded orderbook:

```json
{"type":"subscribe","subscription_id":"book","topic":"orderbook","query":{"item_type_id":"telegram.gift.5936013938331222567","currency_id":"TON","limit":20}}
```

Core streams plain `BookResult` messages. The gateway labels the initial view `snapshot` and subsequent full views `replace` for WebSocket clients. Core evaluates expiry, coalesces updates and shares identical book computations. Reconnect by subscribing again. Send `{"type":"unsubscribe","subscription_id":"book"}` to release a subscription.

For the raw ordered market stream, use `topic:"changes"` with an optional `after:{generation,revision:"123"}`. Frames have `{type:"sync",subscription_id,response}`. `response.kind` is `snapshot_start`, `snapshot_data`, `snapshot_complete`, `delta`, `checkpoint` or `heartbeat`; records/catalog/events use the public JSON codecs. Activate snapshots only at `snapshot_complete`, and delta revisions only at `complete:true`. Persist only complete cursors. Empty revisions still advance the cursor. Invalid generations or lost replay require a fresh subscription without `after`. Raw subscriptions include all market events and preserve core's protocol boundaries.

Each socket accepts at most 16 subscriptions and 64 KiB input messages. Queues and send deadlines are bounded; slow clients disconnect without blocking core. Browser origins must match the gateway or be allowed with repeated `--cors-origin`. Use an HTTPS reverse proxy for public hosting and any customer authentication policy.

## Downstream gRPC

Enable `--grpc-bind` for other services. The gateway exposes core's `market.MarketRead` service, using the copied `market-hub.proto`. `MarketRead` provides status, catalog/items/feeds/orders, orderbooks, durable trades, book watches and raw synchronization. `ReadStatus` includes core's storage, replay and workload counters. `ListStoredOrders` inspects orders in storage-key order, including stale/expired entries and unavailable feeds; it reuses `OrdersQuery`/`OrdersResult` and supports side, feed, type, currency, exact order ID, limit and cursor. `Replay` reuses `SyncRequest`/`SyncFrame`, emits up to ten complete retained revisions, then a checkpoint and EOF; resume from that checkpoint. Omit `after` to start at the retained replay floor. These inspection methods have no additional REST routes.

gRPC book watches return plain full `BookResult` messages: initialize with the first and replace with each subsequent result. `MarketRead.Sync` provides current snapshots and ordered event replay. Ingestion, catalog administration and node registration remain on core.

`MARKET_API_TOKEN` / `--grpc-token` controls downstream gRPC access. It is independent of the gateway's private node token and the join token. Inbound credentials are never forwarded to core. Remote gRPC listeners require TLS; Compose supplies API-owned certificates. Downstream clients trust `.docker/certs/ca.crt`; outbound core calls trust `.docker/hub-ca.crt`.

## Workspace and verification

| Crate | Responsibility |
| --- | --- |
| `market-app` | REST/WebSocket/gRPC gateway, bounded transport admission and OpenAPI |
| `market-client` | Private node identity, enrollment and reusable authenticated channel |
| `hub-protocol` | Generated bindings from the local contract copy |
| `market-view` | Response projections used for JSON formatting |
| `market-json` | Public JSON codecs and schemas |

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --locked --bins --examples
python3 scripts/smoke.py --hub-bin /path/to/market-hub --adapter-manifest /path/to/adapters/Cargo.toml
```

Standalone tests use protocol peers. The integration check uses disposable TLS services, random ports, temporary credentials and synthetic venue facts. It verifies indexed books, a slow source scan with independent lease renewal, durable trades after a hard core crash, backfill deduplication, operational status, stored-order inspection, bounded replay, read subscriptions and token-free gateway/adapter reconnects. It never opens Telegram accounts or contacts real markets.

Review and copy each hub contract update into this repository, then regenerate bindings through the normal build. Core's `scripts/check-contracts.py` compares explicit schema paths before a coordinated release. Ordinary builds use only this repository.
