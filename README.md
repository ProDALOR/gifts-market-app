# Gifts Market App

Independent Rust API/GUI and history applications for Market Hub. This repository owns `market-app`, `history-node`, read projections, orderbook indexes, public JSON/OpenAPI contracts, and reader connection logic. It has no dependency on the hub or adapter Rust workspaces.

```text
adapters → market-hub → market-app → REST / WebSocket / public gRPC → GUI / services
                    └→ history-node → independent disk archive
```

Core owns authoritative validation and ordered state. API nodes maintain local query copies. Public requests never call core. History disk I/O never runs in core. No buying, custody, or execution is performed here.

## Start the MVP

Build this repository from any directory. The checked-in `market-hub.proto` is the gRPC boundary; `hub-protocol` generates local Rust bindings at build time. No sibling checkout, shared Rust model crate or core storage implementation is required.

Each application has its own Compose project. Start the hub from its repository first; it creates a private `.docker/join` bundle containing its public CA, node enrollment credential and Docker network name. No source files or private CA keys are shared. Create the shared `market` Docker network once with `docker network create market` (use your configured network name if changed).

From this repository:

```sh
sh docker/setup.sh /path/to/hub/.docker/join
docker compose up --build --detach --wait
```

This starts **only `market-app`**. The default hub endpoint is `https://market-hub:50052`; set `MARKET_CORE_URL` for a different hub. The project joins the existing network and reconnects independently, with no `depends_on` relationship to another project. HTTP/WebSocket is published on localhost:8080; public gRPC uses TLS on localhost:50053.

Bootstrap imports hub trust into `.docker/hub-ca.crt`, then creates its own API TLS issuer/certificate in `.docker/certs`. `.env` holds the enrollment and downstream API credentials. It does not generate a hub certificate, hub administrator credential or hub private key. Later `sh docker/setup.sh` calls reuse the configured trust and valid keys. Supply a new join bundle when changing hub trust or enrollment settings. Python 3 and OpenSSL are needed for local bootstrap.

The API Docker target builds only `market-app`; the default image target is also the API. The history target is independent. Builds use this repository as context and exclude accounts, credentials and local state.

Each API project keeps its own `api-identity` volume. To run another replica, choose a distinct Compose project and host ports, for example:

```sh
MARKET_API_PORT=8180 MARKET_API_GRPC_PORT=51053 docker compose -p market-app-2 up --build --detach
```

Use the same project name on subsequent starts so its identity volume is reused. Avoid `down --volumes` when preserving node identities. Existing deployments can retain their previous project name with `-p` or `COMPOSE_PROJECT_NAME`.

For local processes, start core using its own README, then:

```sh
cargo run -p market-app -- --core http://127.0.0.1:50052 --grpc-bind 127.0.0.1:50053 --cors-origin http://localhost:3000
```

Provide `MARKET_REGISTRATION_TOKEN` for first enrollment. API nodes create private credentials in `.market/market-app.json`; keep one persistent identity per process. Downstream consumers use the API credential, never the core reader credential.

## Replication and ownership

`MarketReplication.Sync(after?, projection)` streams a complete catalog/state snapshot when no cursor is supplied, then ordered changes. Snapshot records stay hidden until complete and replay has reached its checkpoint. Reconnect with the last applied cursor; gaps or generation changes explicitly require resnapshotting. Partial snapshots and delta chunks are discarded on interruption. Both catalog entries and records apply at one revision. Normal catalog additions preserve feed sessions, API replicas and existing pagination views; process restarts create a new generation.

`market-view` decodes protocol messages into application-owned typed projections. `market-query` owns one structurally shared row store and its price, item-observation and feed indexes; there is no imported core store or second canonical state layer. Complete deltas replace rows and indexes atomically. `market-reader` handles enrollment, private credentials and bounded snapshot/replay assembly. `market-json` owns public JSON codecs and schemas. Catalog revisions are opaque server identities, not hashes recomputed from Rust serialization.

## Independent history

The archive worker has a separate Compose file, image target, identity and archive volume. It connects directly to the hub and starts no API or hub container:

```sh
sh docker/setup.sh --history /path/to/hub/.docker/join
docker compose -f compose.history.yaml up --build --detach
```

Run `sh docker/setup.sh --history /path/to/join` first to import hub trust without generating an API token or server certificate. Stop this project independently with `docker compose -f compose.history.yaml down`. Its archives and identity remain in named volumes.

For native processes:

```sh
cargo run -p history-node -- --endpoint http://127.0.0.1:50052 --output .market/history
cargo run -p history-node -- --output .market/history --dump
```

History owns its reader credential (`.market/history.json`), independently reconnects and consumes the history projection of the same Sync protocol. Initial/resync snapshots establish an explicit archive boundary; subsequent order observations, removals, statuses and trades are stored in bounded checksummed protobuf segments, with compression on sealed files. Catalog updates are archived before referencing batches. Startup recovers the last complete frame and latest catalog. Missing replay is recorded as a gap rather than invented history. Initial startup begins at its consistent snapshot cursor; the archive does not promise events before that boundary.

## Read-only client API

REST, WebSocket and the `MarketApi` gRPC service share one query layer over the API node's immutable local view. Clients never open connections to core. Run multiple API nodes independently; use a sticky route to the same node for pagination and replay, or restart from a fresh snapshot when changing nodes.

| Route | Purpose |
| --- | --- |
| `/healthz` | Process liveness |
| `/status` | Readiness, record/feed counts and synchronization metadata, including during startup |
| `/readyz` | The same status with HTTP 503 until a usable local view is available |
| `/catalog` | Definitions filtered by `kind`, `id`, `item_type_id`, `attribute_id` and name/ID search `q` |
| `/feeds` | Source availability and `last_sync_at`, optionally filtered by `feed_id` |
| `/items` | Observations filtered by feed, item type, item ID and exact typed attributes |
| `/orders` | Indexed side queries, or exact venue order lookup |
| `/orderbook` | Both asks and bids from the same revision and expiry evaluation time |
| `/ws` | WebSocket subscription upgrade |
| `/openapi.json` | Generated REST request and response schemas |

HTTP resources accept GET and HEAD; mutation methods return 405. `/records` has been removed. All JSON responses except liveness use `{data, meta, page?, included?}`. Errors use `{error:{code,message,field?}}`. JSON datetime fields are UTC RFC 3339; amounts, integer attributes and revisions use exact strings. Attribute scalars follow their catalog schemas. Items and orders are flat objects, without an additional `data` wrapper inside each observation.

`meta.cursor` identifies the local materialization. `meta.as_of` fixes expiry evaluation; `served_at` identifies this response. `meta.replica.state` is `syncing`, `live` or `cached`; `last_contact_at` measures contact with core. An observation's `observed_at` and a feed's `last_sync_at` describe the source, independently of replica connectivity. `/status` remains available while bootstrapping; market queries return 503 when no view is usable within `--max-stale`.

Indexed `/orders` requires `side=ask|bid`, `item_type_id` and `currency_id`. Exact lookup uses `feed_id` and `order_id`; optional `side` disambiguates reused ask/bid IDs. Both market endpoints support `feed_id`, `item_id`, catalog-aware JSON `attributes`, inclusive decimal `price_min`/`price_max`, and `include=item,feed`. Candidate attributes select matching asks or bids willing to accept that candidate. `included` uses the same view and observation-selection policy as the book: the newest ready source observation, deterministic source-ID tie-break, without merging traits across sources. Source availability and expiry affect indexed queries; exact order lookup can inspect a stored unavailable/expired observation.

```sh
curl --get http://localhost:8080/catalog --data-urlencode kind=item_type --data-urlencode q=Pepe
curl --get http://localhost:8080/orderbook \
  --data-urlencode item_type_id=telegram.gift.5936013938331222567 \
  --data-urlencode currency_id=TON --data-urlencode include=item,feed --data-urlencode limit=50
curl --get http://localhost:8080/orders --data-urlencode feed_id=telegram --data-urlencode order_id=VENUE_ORDER_ID
```

Catalog, items and orders use `limit=1..200` (default 50) and opaque `page.next_cursor`. Pass it unchanged as `cursor`, preserving filters, sort and `include`; limit may change. Both book sides return their own page cursor, usable on `/orders` with the corresponding side and identical filters. Continuations retain an immutable view and the original `as_of` even as live orders move or expire. Cursors are node-local, bounded by count/bytes and TTL (default 60 seconds); HTTP 409 `cursor_expired` means restart the query. Filter changes return `cursor_mismatch`. Bounded scans may return an empty page with a continuation. Do not parse cursors or use them as live stream cursors.

### WebSocket

Connect to `/ws` and send:

```json
{"type":"subscribe","subscription_id":"selected-market","topic":"orderbook","query":{"item_type_id":"telegram.gift.5936013938331222567","currency_id":"TON","limit":50,"include":"item,feed"}}
```

The server replies with `{type:"snapshot",subscription_id,response:<REST book envelope>}`, followed by `type:"replace"` with the complete bounded view whenever orders, item eligibility, included dependencies or availability change. Expiry is reevaluated at least once per second. Identical queries share book computation and JSON encoding. Intermediate replacements may coalesce; replace the displayed book atomically. A refresh is sent every 30 seconds. On reconnect, subscribe again for a fresh view. Send `{type:"unsubscribe",subscription_id:"selected-market"}` to release a subscription.

For a raw local mirror, subscribe to `topic:"changes"` with optional structural `query:{feed_id,item_type_id}`. Without `after`, receive snapshot frames containing `response:{catalog,records,meta,complete}`; activate only after `complete:true`. Subsequent changes contain `previous`, `cursor`, `catalog`, `catalog_revision`, `events`, `complete` and `meta` under `response`. Catalog changes and large revisions are chunked: buffer catalog entries and events, validate the catalog revision, and apply both atomically only after `complete:true`, then save that cursor. Empty revisions still advance the cursor. `checkpoint` marks catch-up to the head captured at subscription; `heartbeat` carries current replica health. Resume by supplying `after:{generation,revision:"123"}` with exactly the same structural scope. Generation changes or evicted replay return `resync_required`; discard the partial view and subscribe without `after`. Snapshots include feed status dependencies. An update that leaves an item-type scope is projected as deletion.

A socket allows 16 subscriptions and 64 KiB input messages. Shared stream capacity defaults to 128, including sockets and subscriptions. Output queues and send deadlines are bounded; slow clients are disconnected and cannot stall replication. Ping/pong checks detect abandoned connections. Cross-origin GUIs must be enabled with repeated `--cors-origin https://gui.example`; the same origin is accepted automatically. Use an HTTPS reverse proxy for public HTTP/WebSocket hosting.

### Downstream gRPC

Enable `--grpc-bind 127.0.0.1:50053` for local services. `MarketApi` exposes `GetStatus`, `ListCatalog`, `ListFeeds`, `ListItems`, `ListOrders`, `GetOrderBook`, `WatchOrderBook`, `Snapshot` and `WatchChanges` in `api.proto` (namespace `market_app.api`). Queries share REST validation and pagination. Attributes use protobuf's typed value map; domain protobuf timestamps retain their declared millisecond units, while response metadata uses RFC 3339. Protobuf `uint64` cursors are exact native values. The public protocol imports canonical core types; adapter bindings contain only the private core protocol.

`Snapshot` streams a pinned view; record the cursor only after its complete marker. Start `WatchChanges(after=cursor)` with the same scope. The bounded local journal covers changes during export and replay; if it no longer covers that cursor, resnapshot. A stream never silently skips revisions. Discard incomplete snapshot or revision chunks if the stream ends. Book watches use the same snapshot/replacement model as WebSocket.

`MARKET_API_TOKEN` / `--grpc-token` optionally requires `authorization: Bearer <API key>`. This is a downstream credential, separate from enrollment and the API node's private reader identity. Consumers never enroll with core. `--grpc-cert` and `--grpc-key` configure TLS, required on non-loopback binds; `--grpc-client-ca` additionally requires client certificates. Compose enables TLS and a downstream token on port 50053. Downstream clients trust `.docker/certs/ca.crt`; `.docker/hub-ca.crt` is used only for the outbound connection to the hub. The API service exposes no registration or mutation RPCs.

Query concurrency and scan work are limited separately from streams. `--max-streams`, `--max-snapshots`, `--max-snapshot-bytes`, `--snapshot-ttl`, `--replay-batches` and `--replay-bytes` bound retained views and replay. Byte budgets use encoded record weights; catalog structures are shared. Snapshot exporters have a 60-second deadline and two queued chunks; the default replay journal retains up to 4096 revisions / 32 MiB. Responses cap each data section at 1 MiB; included dependencies and both book sides are bounded separately. Lower `limit` if a dependency set exceeds that budget. The public gRPC transport caps messages at 4 MiB.

## Workspace and verification

| Package | Responsibility |
| --- | --- |
| `market-app` | Local replication, query service, REST, WebSocket and public gRPC |
| `history-node` | Independent archive CLI and reconnecting consumer |
| `market-query` | Application row storage and read indexes |
| `market-view` | Typed read projections, catalog lookup and wire mapping |
| `hub-protocol` | Locally generated bindings from the vendored hub contract |
| `market-reader` | Reader enrollment, credentials, TLS and snapshot/delta assembly |
| `market-json` | Public JSON codecs and schemas |
| `market-history` | Archive storage and recovery; its schema is `archive.proto` |

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --locked -p market-app -p history-node --bins
```

Unit and protocol-peer tests are standalone and require no hub checkout, binary, Telegram account or marketplace connection. They cover public read responses, atomic replication, price indexes, exact JSON values and archive recovery.

A disposable TLS deployment check also exercises the Rust adapter and the independent history process:

```sh
cargo build --locked -p market-app -p history-node --bins --examples
python3 scripts/smoke.py --hub-bin /path/to/market-hub --adapter-manifest /path/to/adapters/Cargo.toml
```

The smoke test uses native-shaped in-process Telegram test data, temporary credentials and random ports. It does not load accounts or contact real marketplaces. `cargo run -p market-app --example read_api -- --help` shows a minimal downstream gRPC client.

## Updating the hub contract

Review and copy the hub release's `market-hub.proto` to this repository root, then run the checks above. Cargo regenerates bindings automatically. Commit the schema copy and lockfile with the consumer changes. `api.proto` and `archive.proto` import only the local schema; generated Rust files are build artifacts. The hub repository provides `scripts/check-contracts.py` to compare explicitly supplied copies before a coordinated release. An integration deployment is the place to test multiple service releases together; Cargo builds never discover or import another checkout.
