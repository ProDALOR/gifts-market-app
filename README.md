# Gifts Market App

A public React market browser with a Rust REST/WebSocket API and a persistent local read replica. Market Hub receives and validates authoritative facts. Each app instance synchronizes those facts once, then serves catalog, order and execution queries locally. Public requests never query the hub.

```text
Adapters → Market Hub ── snapshot + Watch ──→ App replica → REST / WebSocket → React
                          └───────────────→ Another app replica → more visitors
```

Each app process shares one replica across its HTTP workers. Current catalog, feeds and active-order indexes live in RAM. Execution payloads remain in local redb, with chronological, book and typed attribute indexes. Current entities are also persisted so a saved checkpoint always describes fully restorable state. There is no sibling Rust dependency: the copied `market-hub.proto` is the service boundary and the replica stores its generated messages directly.

## Run

Issue a service token on the hub with `market-hub token --id market-app`, then configure this application:

```sh
export MARKET_NODE_TOKEN='<issued service token>'
npm --prefix web ci
npm --prefix web run build
cargo run --locked --bin market-app -- --core http://127.0.0.1:50051 --data-dir data
```

Open `http://127.0.0.1:8080`. The interface provides collection search, separate quote currencies, unified or single-venue offers and bids, exact price ordering, typed catalog filters, execution history and order details. It receives only its visible page. Filters stay in the URL; pagination and pause/resume retain the selected boundary. Executions are confirmed facts from adapters; disappearing listings do not imply sales.

The service token stays on the server. Visitors need no hub credentials. Native configuration uses `MARKET_CORE_URL`, `MARKET_NODE_TOKEN`, `MARKET_HTTP_ADDR` and `MARKET_DATA_DIR`, or their CLI options. Every app instance needs its own data directory. It is bound to the source hub URL and service identity; use a new directory when connecting to a different source.

For frontend development, start the Rust application and run `npm --prefix web run dev`. Vite proxies API and WebSocket traffic to port 8080. Rust serves `web/dist` normally; set `--web-dir` for another location.

## Synchronization and recovery

A new replica reads one consistent, unfiltered `Read(SNAPSHOT)` view, including catalog, current orders and execution history. It opens `Watch` at the first page's cursor immediately and spools changes to disk while loading snapshot pages. The hub renews the snapshot's idle lifetime as paging progresses. The app builds RAM indexes incrementally and replays staged changes before atomically publishing the completed replica. Orders, history and checkpoints become visible together.

Normal operation needs one authenticated Watch stream through a reusable gRPC channel. Each delivery commits its changes, execution indexes and checkpoint in one redb transaction before publishing the new RAM/history view. Execution upserts remove the same identity from active orders. Immutable execution retries are idempotent. Execution timestamps determine history order; the opaque Watch cursor determines replication progress, so late backfills are retained.

After app restart, saved state is immediately available with `sync.state: stale`. The app resumes Watch from its durable checkpoint. If the cursor expired or the hub restarted, it builds a replacement database while continuing to serve the previous copy. Completion swaps catalog, active orders and history together; facts absent from the replacement snapshot disappear too. An interrupted build is discarded on startup.

Hub outages leave public reads available from the last committed copy. WebSocket pages and REST responses carry `sync: {ready, state, updated_at}`; states are `starting`, `syncing`, `live` and `stale`. `updated_at` is the last durably received checkpoint time, not an execution timestamp or a guarantee of zero replication lag. The GUI shows **Stale** when current-order updates are delayed. `/readyz` succeeds once a complete saved or freshly synchronized replica exists. `/sync` reports upstream freshness separately.

The local database is a rebuildable cache, not the authoritative archive. Stop this app and delete its data directory to rebuild it. Never share a redb directory between running app instances. Disk usage includes execution history and, during a rebuild, the replacement database and its change journal.

## Docker

Create the shared network once with `docker network create market`. Copy `.env.example` to `.env`, set `MARKET_NODE_TOKEN`, and run `docker compose up --build --detach`.

Compose starts only this application, connects to `http://market-hub:50051`, publishes `0.0.0.0:8080` and persists its replica in `market-app-data:/data`. Configure `MARKET_CORE_URL`, `MARKET_NETWORK` or `MARKET_APP_PORT` for another deployment. Separate deployments use separate volumes. Multiple instances can sit behind a load balancer; retained snapshot pagination requires affinity to the instance holding the view. Live page boundaries can move between instances synchronized to the same market.

The image generates public OpenAPI and TypeScript types before building React. Generated clients, dependencies and built assets are ignored. The Node build exports its schema using the compiled Rust binary.

## Public API

| Route | Purpose |
| --- | --- |
| `GET /books` | Paged concrete venue / item-type / currency books |
| `GET /catalog/{kind}` | Paged reference definitions |
| `POST /catalog/query` | Reference definitions, optionally by exact IDs |
| `POST /orders/query` | One frozen page of current asks/bids |
| `POST /executions/query` | One frozen page of confirmed executions |
| `GET /ws` | Live current-order pages from the local replica |
| `GET /sync` | Replica readiness and synchronization status |
| `GET /openapi.json` | Generated public contract, including WebSocket frames |
| `GET /healthz`, `GET /readyz` | Process and replica readiness |

All operations are read-only. POST bodies carry filters. Orders require an item type and quote currency; omit venue to combine markets. Default mode is `ask`, sort is `price_asc`, and page size is 50 (maximum 200).

```sh
curl http://localhost:8080/orders/query \
  -H 'Content-Type: application/json' \
  -d '{"scope":{"item_type_id":"tg.gift.5167939598143193218","currency_id":"GRAM","modes":["ask"],"price":{"max":"20"}},"sort":"price_asc","limit":25}'
```

Pages contain `{data, next_page_token, cursor, sync}`. Each order carries its concrete `book`, direct item ID, scalar price, quantity and typed attributes. Prices, quantities and integer attributes are exact strings. Dates are RFC 3339 strings. Values use `{type, value}` and predicates use `{op, ...}`. Nested lists and objects preserve duplicates and match object fields within the same occurrence.

```json
{"scope":{"item_type_id":"tg.gift.5167939598143193218","currency_id":"GRAM","attributes":{"tg.number":{"op":"range","min":{"type":"integer","value":"1"},"max":{"type":"integer","value":"100"}}}},"limit":25}
```

Sort is `price_asc`, `price_desc` or `identity`. Price trees compare exact decimals, then venue/order identity; descending reverses the complete key. Optional price bounds are inclusive. Executions use chronological `(executed_at, venue_id, order_id)` order and optional `period: {from, until}` with inclusive start and exclusive end. They use the same Order model with `mode: executed`.

Catalog kinds are `venue`, `currency`, `item_type`, `attribute` and `attribute_option`. Exact-ID requests accept at most 500 IDs. `/books` returns `{data, next_after}`; pass its JSON BookKey as the next `after` query parameter. Even an empty filtered page can have a continuation after a bounded scan.

## Pagination and WebSockets

REST pages pin a local RAM/history view for consistent forward pagination. Views expire after 60 seconds of inactivity or app restart, and are released when traversal completes. Hub restart alone does not invalidate an already retained local view.

Live subscriptions use local keyset boundaries and replace only the requested page. Identical subscriptions share one page computation within an app. The same continuation format works across live and snapshot reads: a live continuation used in REST captures a new snapshot; a snapshot continuation used live ignores its frozen-view ID and resumes its boundary against current data. An expired snapshot returns HTTP 409 in REST but remains a usable live boundary. Changing filters or sort starts a new traversal.

Public continuations are bounded, validated read positions, not credentials. They contain a normalized query fingerprint, sort, keyset position and optional local snapshot ID. They never contain upstream page tokens or the service token. Stateless live boundaries are portable across replicas; frozen views remain local. `cursor` identifies the replicated hub checkpoint behind that page. It is not a public query continuation or a query-specific native Watch cursor.

```json
{"type":"subscribe","id":"offers","query":{"scope":{"item_type_id":"tg.gift.5167939598143193218","currency_id":"GRAM","modes":["ask"]},"sort":"price_asc","limit":25,"page_token":""}}
```

Frames are `{type:"status", id, state:"connecting"}`, then `{type:"page", id, page:{data,next_page_token,cursor,sync}}`. Each page fully replaces the previous one, including refills, execution exits and freshness changes. Errors use `{type:"error", id, error:{code,message}}`. Unsubscribe with `{"type":"unsubscribe","id":"offers"}`. Browser reconnection uses the same query and page boundary; upstream reconnection is handled independently by the replica.

Each socket permits 16 subscriptions and 64 KiB input messages. Queues, send deadlines, local query concurrency and retained views are bounded. Browser origins must match this application or be allowed with repeated `--cors-origin`. Writer endpoints and raw authenticated hub forwarding are absent. Trusted ingestion services still connect directly to the hub.

## Workspace and checks

- `crates/hub-protocol`: native bindings generated from the local proto copy.
- `crates/market-replica`: protocol-native RAM indexes, redb storage, local pages and synchronization.
- `crates/app`: public DTOs/OpenAPI, JSON conversion, REST/WebSockets and React hosting.
- `web`: React interface and generated public API types.

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
npm --prefix web test
npm --prefix web run build
cargo build --locked --bins
python3 scripts/smoke.py --hub-bin /path/to/market-hub --adapter-manifest /path/to/adapters/Cargo.toml
```

Tests cover snapshot/Watch overlap, durable checkpoint recovery, replaced datasets, exact prices, typed duplicate list matching, chronological backfills and local-only public reads. The integration check starts a disposable real hub, adapter fixture and app on random local ports. It verifies browser layouts, live updates, execution transitions, hub restart rebuilds and persisted reads while the hub is offline. It never opens account exports or contacts markets. Install its browser with `cd web && npx playwright install chromium`. Optional `--keep target/replica-preview` retains a synthetic preview; use a fresh directory.

Copy coordinated contract updates into the client repositories and use the hub's `scripts/check-contracts.py` before release. All builds use only their own repository. No legacy gateway routes or copied hub implementation crates remain.
