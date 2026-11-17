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
export MARKET_HUB_TOKEN='<issued service token>'
npm --prefix web ci
npm --prefix web run build
cargo run --locked --bin market-app -- --hub-url http://127.0.0.1:50051 --data-dir data
```

Open `http://127.0.0.1:8080`. The interface provides collection search, separate quote currencies, unified or single-venue asks and bids, exact price/time/gift-number ordering, catalog trait filters, history and order details. Scrolling appends keyset pages automatically. Filters stay in the URL; pause/resume preserves the loaded scroll window. Executions are confirmed facts from adapters; disappearing listings do not imply sales.

The service token stays on the server. Visitors need no hub credentials. Native configuration uses `MARKET_HUB_URL`, `MARKET_HUB_TOKEN`, `MARKET_HTTP_ADDR` and `MARKET_DATA_DIR`, or their CLI options. Every app instance needs its own data directory. It is bound to the source hub URL and service identity; use a new directory when connecting to a different source.

For frontend development, start the Rust application and run `npm --prefix web run dev`. Vite proxies API and WebSocket traffic to port 8080. Rust serves `web/dist` normally; set `--web-dir` or `MARKET_WEB_DIR` for another location. Use `--no-web` or `MARKET_NO_WEB=true` to serve only the API, without frontend files or routes. Embedded `ApiConfig` defaults to `web_dir: None`; pass `Some(path)` to enable frontend hosting.

Deployment limits use `MARKET_HTTP_WORKERS` / `--http-workers` (default 2), `MARKET_MAX_CONCURRENT_QUERIES` / `--max-concurrent-queries` (32), and `MARKET_MAX_LIVE_QUERIES` / `--max-live-queries` (128). Live-query capacity counts distinct shared feeds; identical subscriptions share one slot. These limits are separate from WebSocket connection limits. CLI flags take precedence over environment values. Native runs do not automatically load `.env`; export its settings before starting the process. Compose loads `.env` automatically.

Configuration migration: `MARKET_CORE_URL` / `--core` became `MARKET_HUB_URL` / `--hub-url`; `MARKET_NODE_TOKEN` / `--node-token` became `MARKET_HUB_TOKEN` / `--hub-token`; `--workers` became `--http-workers`; `--max-streams` became `--max-live-queries`. Remove `MARKET_TELEGRAM_BOT_ID` and `MARKET_TELEGRAM_BOT_USERNAME` from worker deployments. For Mini App-only deployments, replace them with `MARKET_MINIAPP_BOT_ID` and `MARKET_MINIAPP_BOT_USERNAME`. Old names are no longer read. Keep the same hub URL, issued token and data directory when migrating to preserve the replica binding.

## Synchronization and recovery

A new replica reads one consistent, unfiltered `Read(SNAPSHOT)` view, including catalog, current orders and execution history. It opens `Watch` at the first page's cursor immediately and spools changes to disk while loading snapshot pages. The hub renews the snapshot's idle lifetime as paging progresses. The app builds RAM indexes incrementally and replays staged changes before atomically publishing the completed replica. Orders, history and checkpoints become visible together.

Normal operation needs one authenticated Watch stream through a reusable gRPC channel. Each delivery commits its changes, execution indexes and checkpoint in one redb transaction before publishing the new RAM/history view. Execution upserts remove the same identity from active orders. Immutable execution retries are idempotent. Execution timestamps determine history order; the opaque Watch cursor determines replication progress, so late backfills are retained.

Internal entity kinds, order modes, sort order and index tags use validated enums. Entity and execution keys have named fields; numeric codes and tuple layouts are converted at the protocol, database and continuation boundaries. Existing databases and valid pagination tokens keep their original format.

After app restart, saved state is immediately available with `sync.state: stale`. The app resumes Watch from its durable checkpoint. If the cursor expired or the hub restarted, it builds a replacement database while continuing to serve the previous copy. Completion swaps catalog, active orders and history together; facts absent from the replacement snapshot disappear too. An interrupted build is discarded on startup.

Hub outages leave public reads available from the last committed copy. WebSocket pages and REST responses carry `sync: {ready, state, updated_at}`; states are `starting`, `syncing`, `live` and `stale`. `updated_at` is the last durably received checkpoint time, not an execution timestamp or a guarantee of zero replication lag. The GUI shows **Stale** when current-order updates are delayed. `/readyz` succeeds once a complete saved or freshly synchronized replica exists. `/sync` reports upstream freshness separately.

The local database is a rebuildable cache, not the authoritative archive. Stop this app and remove `replica.redb` and `building.redb` from its data directory to rebuild it. Preserve `session.key`, which signs user sessions independently of market replication. Never share a redb directory between running app instances. Disk usage includes execution history and, during a rebuild, the replacement database and its change journal.

## Docker

Create the shared network once with `docker network create market`. On a new deployment, copy `.env.example` to `.env` and set the hub URL and issued token. Existing deployments can keep their `.env`. Start with:

```sh
docker compose config --quiet
docker compose up --build --detach
docker compose logs --follow --tail=100 market-app
```

Compose reads runtime configuration from `.env`; unset options use application defaults. Container data, bot, media and frontend paths are fixed to `/data`, `/bot-data`, `/data/media` and `/app/web/dist`, so native paths in `.env` do not override the mounted directories. Custom container paths require a Compose override with matching writable mounts. The listener is fixed to `0.0.0.0:8080`; `MARKET_APP_PORT` controls the published host port. After editing runtime settings, run `docker compose up --detach` to recreate the service with the updated environment.

Compose starts only this application, connects to `http://market-hub:50051`, publishes `0.0.0.0:8080` and persists its replica in `market-app-data:/data`. Configure `MARKET_HUB_URL`, `MARKET_NETWORK` or `MARKET_APP_PORT` for another deployment. Separate deployments use separate volumes. Multiple instances can sit behind a load balancer; retained snapshot pagination requires affinity to the instance holding the view. Live page boundaries can move between instances synchronized to the same market.

One Docker image includes the frontend, REST/WebSocket API, Telegram bot and Mini App support. A direct build uses `docker build -t market-app .`. The bot starts automatically when all three Telegram credentials are configured; serving the website and API requires none.

The image binds `0.0.0.0:8080`, stores the replica in `/data` and bot state in `/bot-data`, and runs as UID/GID 10001. Preserve both directories when running outside Compose. The image uses standard `SIGTERM` shutdown; Compose allows 15 seconds to drain workers.

The Dockerfile has three stages: Rust compilation, frontend compilation and the final runtime image. A single `.dockerignore` excludes credentials, local state and generated assets from the build context. The frontend build generates public OpenAPI and TypeScript types before building React. Generated clients, dependencies and built assets are ignored. The Node build exports its schema using the compiled Rust binary.

## Gift previews and media

The GUI uses a centered, mobile-first orderbook with a searchable collection rail with smoothly expanding collection tiles, multi-select Model/Backdrop/Symbol pickers, a virtual trait preview and a three-column gift grid. Order details open in a keyboard-accessible bottom sheet. Currency, venue, Asks/Bids/History and Time/Gift #/Price sorting use the live API; currency and sort menus share keyboard navigation and outside dismissal, GRAM/Stars use vector symbols, and the MRKT mark comes from its public app assets; filters are restored from the URL. Infinite scrolling uses one live head subscription and stateless live continuations, reconciling the loaded window when the head changes. Paused views and history use snapshot continuations. The preview appears only when traits are selected and shows only those selections. Model and backdrop rows balance their occupancy across the available width (twelve backdrops become six plus six). Models shrink from 52px to 40px, then 32px when needed to fit four rows; larger selections add rows at the minimum size. One continuous symbol layer uses a common tint and fixed tile spacing across every row. The footer stays at the viewport bottom on short pages and follows longer content. The layout adapts to Telegram safe areas and light/dark themes.

The GUI loads static model thumbnails through `GET /media/models/{catalog-option-id}`. The API resolves model document IDs through the catalog actor; clients cannot request arbitrary Telegram files. Responses contain only PNG, JPEG or WebP bytes, with content-type validation, ETags/304 responses and a one-day browser cache. Market JSON and WebSocket pages contain no image payloads or Telegram file credentials. Gift cards load thumbnails lazily. Collection tiles and selected models request autoplay after remaining visible briefly; a shared budget admits at most six players, with interaction and details taking priority. Opening a detail or account sheet suspends background playback. Initial collection positioning skips animation downloads for intervening tiles; subsequent selection movement stays smooth. Download producers are queued and limited to four concurrent HTTP requests. Collection metadata loads are cached across visits and failed loads expose a retry action. Selected models use a flat balanced grid, so resizing preserves their media nodes; order cards play on hover or focus. Animation requests share the same cache. Hovering or focusing an order card loads its TGS animation through `GET /media/models/{catalog-option-id}/animation`; order details play the animation automatically. Downloads and the expression-free canvas player are loaded only when needed. Leaving the preview immediately restores the thumbnail and pauses playback; a 350 ms grace period reuses its renderer on quick re-entry. Scrolling offscreen, closing details, hiding the tab or enabling reduced motion releases the renderer immediately. Runtime player errors also restore the thumbnail. Reduced-motion preferences keep the static thumbnail. Unsupported or unavailable animations retain the thumbnail; transient download failures retry briefly.

Gift previews compose the exact `tg.backdrop` and `tg.pattern` attributes around the model. Catalog backdrop metadata supplies the center/edge gradient colors and pattern tint; the selected pattern's transparent thumbnail is fetched from `GET /media/patterns/{catalog-option-id}` and repeated around the model. The background is painted once on a canvas and stays static during model playback. Pattern downloads are limited to visible previews, and the list does not load the animation player just to show patterns. Unknown or invalid palettes fall back to the ordinary model preview; conditional bids never invent a concrete gift appearance. Both gift cards and order details use the same composition, with a larger preview in details.

Thumbnails and animations are downloaded dynamically at runtime and are never committed or bundled into Docker images. `MARKET_MEDIA_DIR` / `--media-dir` selects the persistent disk cache; the default is `<MARKET_DATA_DIR>/media` (`/data/media` in Compose, on the replica volume). The bot’s shared grammers client downloads cache misses directly from Telegram. An instance without bot credentials serves existing cached files; uncached media returns 503. Use separate cache directories per writable node; never share the replica database directory between running instances. Custom container cache paths must be on a writable mounted volume. An uncached media asset also returns 503 while the Telegram connection is unavailable. Deleting the media directory safely rebuilds the cache on subsequent downloads.

An enabled bot worker shares its authenticated grammers client with the media downloader; no second Telegram login or update consumer is created. Startup warms up to 64 catalog model thumbnails after replica readiness, without delaying the API. Further GUI requests fetch missing thumbnails and animations on demand; startup does not prefetch TGS. Cache entries and pending requests are keyed by document ID and media kind, so each variant is downloaded once and failures do not affect the other variant. The pending budget is 64 assets, with at most four concurrent downloads paced 75 ms apart. Interactive requests take priority over startup warming; abandoned queued requests are skipped. Downloads time out after 15 seconds and callers after 30 seconds. Compressed files are limited to 256 KiB. Negative caching and Telegram flood-wait cooldowns prevent retry storms. A 16 MiB memory LRU with a five-minute TTL avoids repeated disk reads, hashing and TGS validation, including for conditional HTTP requests. Response bodies share the cached byte buffer. The disk cache evicts its least recently used files above 512 MiB; startup and write-time maintenance remove recognized temporary files older than one hour. Atomic writes keep partial files out of responses; shutdown drains tracked disk work. Transport reconnects replace the media fetch connection together with the command connection; disk hits continue to work during outages.

The Telegram adapter coalesces document resolution across thumbnail and TGS misses, caches up to 128 private references for five minutes, and refreshes an expired file reference once. It downloads either a static thumbnail or the original `application/x-tgsticker` document. Model and pattern document IDs share the same cache without exposing Telegram download credentials. The cache stores `<document-id>.thumb` and `<document-id>.tgs` independently. TGS is validated as gzip-compressed vector animation JSON before storage: expanded JSON is capped at 2 MiB, with bounded dimensions/duration and no external assets, fonts or expressions. The API serves original compressed TGS bytes without `Content-Encoding: gzip`; the lazy frontend decoder expands them for the player. Both variants support ETags/304 and a one-day browser cache. Browser previews share pending downloads and a bounded status-aware retry policy, and cancel when the last interested preview leaves. Animation data has a 16 MiB weighted cache; decoded pattern images and tinted frames each have a 2 MiB RGBA cache. Concurrent pattern requests share decoding and tinting. Each renderer gets its own copy of animation data. Visibility, motion and resize observation are pooled across previews. Offscreen backdrops release their canvas buffers and local tinted-frame references; selected backdrop color bands use CSS gradients. The transport-independent `market-media` crate owns disk caching and request coordination; `market-telegram` owns authenticated downloads, `market-api` owns catalog lookup and HTTP delivery, and `market-app` wires them together. API-only builds retain no Telegram dependencies.

Frontend code is organized into `web/src/collections`, `catalog`, `previews`, `orderbook` and shared `ui` modules. `GiftArtwork` renders resolved media; `GiftPreview` adapts market orders. Feature styles live beside their components, while `style.css` imports them and owns shared tokens and foundations.

## Telegram Mini App and bot

Native Cargo builds keep Telegram support opt-in. Docker images include both `bot` and `miniapp` features; configuring all three Telegram credentials starts the worker automatically. Native builds without these features serve the market API and ordinary website without Telegram dependencies. Disabled features omit their CLI flags and subcommands; an API-only process ignores Telegram environment variables and creates no Telegram session or authentication storage.

| Cargo features on `market-app` | Included support |
| --- | --- |
| none (default) | Market API, WebSocket queries and ordinary website |
| `bot` | MTProto bot worker and `bot inspect` / `bot configure` commands |
| `miniapp` | Mini App launch settings, HTTP sign-in/session endpoints and `session-key` command |
| `bot,miniapp` | Both integrations |

`market-api` also exposes its own `miniapp` feature for embedding. The `bot` feature includes bot launch URL validation but does not enable Mini App HTTP routes. Compiling `bot` makes the worker available; configuring all three Telegram credentials starts it automatically.

```sh
# Default: market API and website, with no Telegram settings.
npm --prefix web run build
cargo run --locked -p market-app -- --data-dir data
# Mini App HTTP support without a bot worker:
MARKET_SCHEMA_FEATURES=miniapp npm --prefix web run build
cargo run --locked -p market-app --features miniapp -- --data-dir data
# Both integrations (set the runtime credentials described below):
cargo run --locked -p market-app --features bot,miniapp -- --data-dir data
```

Build the frontend against the same Mini App capability as the server. `MARKET_SCHEMA_FEATURES=miniapp` selects that capability when the frontend generates its schema through Cargo; with `MARKET_SCHEMA_BIN`, the provided binary's compiled features determine it. The browser build reads the schema and disables Telegram SDK initialization, sign-in requests and account controls when Mini App routes are absent.

Docker and Compose always build with `--all-features`. Bot startup is automatic when `TELEGRAM_BOT_TOKEN`, `TELEGRAM_API_ID` and `TELEGRAM_API_HASH` are all configured. Leave all three unset or blank to run without a bot; partial or invalid credentials fail startup with a configuration error. Native Cargo builds still support selecting features explicitly.

The same React interface works as an ordinary website and as a Telegram Mini App. The bridge loads only for Telegram launches, integrates safe areas and theme changes, and connects Telegram's Back button to order details, filters and the account menu. A small account control displays the server-verified identity. Market browsing stays public.

For a real Telegram launch, host the application at a public HTTPS origin. A deployment or HTTPS tunnel can forward to the existing HTTP listener. Configure these values in `.env` for Compose:

```dotenv
MARKET_PUBLIC_URL=https://markets.example.com
TELEGRAM_BOT_TOKEN=<BotFather token>
TELEGRAM_API_ID=<application ID from my.telegram.org>
TELEGRAM_API_HASH=<application hash from my.telegram.org>
```

`market-app` hosts one supervised bot worker when all three Telegram credentials are configured. It uses MTProto through grammers 0.12, logging in as the bot with its BotFather token and a Telegram application API ID/hash from [my.telegram.org](https://my.telegram.org). Supply these three credentials only to the application instance running the worker. Native runs read environment variables or CLI flags; Compose reads `.env`. Keep `.env` private; it is excluded from Git and Docker builds. These credentials and the saved MTProto authentication key are private and never part of public configuration or browser state.

Enable the **Main Mini App** in BotFather for this bot and set its URL to `MARKET_PUBLIC_URL` when launch buttons are wanted. The worker registers `/start`, `/market`, `/help`, `/status`, `/floor` and `/portfolio`; it also configures the chat's **Open market** menu button when a URL is supplied. Commands work in private chats. `/status` reads synchronization status through the shared data actor. Startup verifies the saved session belongs to the token’s bot, then discovers the ID, current username and active aliases from the authenticated Telegram profile. The worker does not manage HTTP Bot API webhooks.

```text
/floor Bunny Muffin | Red Alert
/floor Bunny Muffin | Red Alert | GRAM
/portfolio
/portfolio GRAM
```

`/floor` resolves exact collection/model names (case-insensitive) or catalog IDs, then returns the lowest non-expired, positive-quantity ask across loaded venues. Bids and executions are excluded. Quote currency defaults to `GRAM`; `TON` is accepted as its alias. A missing listing is reported explicitly.

`/portfolio` reads the command sender's Telegram-visible profile gifts through [payments.getSavedStarGifts](https://core.telegram.org/method/payments.getSavedStarGifts). It never accepts another user's ID as its target. The model document and original gift IDs are matched to catalog aliases, then the actors value each supported collectible at its model floor in one quote currency. Regular gifts and refunded or burned gifts are not valued. Hosted collectibles linked from TON wallets are included when their host matches the sender, and counted separately; linking to a profile does not prove wallet ownership. On-chain holdings not linked to the profile are outside this estimate. Saved and unsaved gifts are requested; Telegram decides which gifts this bot may see. Gifts hidden from the bot remain outside the result. Telegram permission errors produce a per-command explanation instead of a zero balance. Reads are bounded to 1000 saved gifts or 20 pages and 30 seconds overall, deduplicate collectible IDs across pages, and label truncated inventories with the reason. If a later page times out or fails, the estimate retains successfully read gifts and is explicitly partial; authorization failures are never hidden. Skipped gifts report separate reasons for refunds, burns, ownership/hosting mismatches and missing IDs or models. Telegram flood waits are shared across inventory requests, and shorter waits never shorten an existing cooldown.

Portfolio output reports Telegram's returned gift count, advertised total and pages read, alongside priced and unpriced gift counts. A missing continuation cursor before the advertised total is reached is marked partial. Only priced gifts contribute to the estimate; no collection-floor fallback or currency conversion is used. Decimal multiplication and addition are exact and checked for overflow. Quotes within one valuation use one pinned replica view, and stale market data is marked. Results are ask-based estimates before fees, not sale proceeds. Only the first ten model groups are displayed; additional priced groups remain included in the total.

```sh
# Log in/resume the local session and inspect identity; no updates consumed or messages sent.
cargo run --locked -p market-app --features bot -- bot inspect
# Set commands/menu without consuming updates or starting the hub replica/HTTP server:
cargo run --locked -p market-app --features bot -- bot configure
# Start the application, including its enabled worker:
docker compose up --build --detach
```

Bot transport, command handling, session persistence and worker lifecycle live in separate modules under `crates/market-telegram/src/bot`; application startup builds configuration and registers the worker once outside the HTTP-worker factory. Shutdown stops and joins the MTProto sender, then drains session writes before releasing the writer lock. Temporary connection failures retry; Telegram flood waits are honored in full. A sender-task failure closes the old connection, drains session writes and reconnects using the saved session. Authorization, identity/configuration or session-storage failures stop the bot and are logged; the market API and replica continue running. The application still owns and drains the bot task on shutdown.

Bot commands use `CommandKind` for menu metadata and a typed `Command` request enum for parsed arguments. `BotWorker::run(handler, stop)` is generic over the native async `CommandHandler` trait; the default `MarketCommands` implementation receives a `MarketDataHandle`. The parser builds `CommandRequest` values from the authenticated private peer and message ID, with validated floor queries and quote currencies. Each request receives a read-only `GiftSource` scoped to that peer. Handlers receive no bot token, Telegram API credentials or saved session. Parsing, floor/portfolio services and reply rendering live in separate command modules. The private `bot/mtproto` module contains all TL requests, response mapping, grammers calls and session storage. `TelegramActions` supplies a cloneable action handle; `Transport` receives updates, and `UpdateCheckpoint` persists progress. Profile gifts are Telegram data with an explicit owned/hosted/unverified relation; only the portfolio service converts their identifiers into actor catalog aliases. The bot validates its own launch URL and has no dependency on Mini App authentication.

The dispatcher runs up to eight commands concurrently across private chats, with one active command per chat and at most 64 received, uncheckpointed deliveries. Commands in the same chat retain their order. Checkpoints advance only through the completed prefix of received updates, so a later fast command cannot skip an earlier unfinished command after restart. Backpressure stops receiving when that window is full. In-progress receive futures survive other command completions. Command handlers have a 40-second deadline and at most three attempts; failures and panics produce a command-level response. Permanent reply rejections finish only that delivery. Transient sends retry with a 15-second timeout per attempt and honor full flood waits; cancellation leaves unfinished deliveries uncheckpointed. Replies retain stable random IDs and are bounded to Telegram's text limit without splitting characters.

MTProto worker authentication requires `TELEGRAM_BOT_TOKEN`, `TELEGRAM_API_ID` and `TELEGRAM_API_HASH`; its saved MTProto session is reused across restarts. The worker has no separately configured identity. When Mini App support is also compiled, the application publishes its authenticated ID and username to all HTTP workers. The public API starts without waiting for Telegram; Mini App sign-in becomes available after login. Discovered identity remains available during subsequent bot transport outages and is refreshed on reconnect. `MARKET_PUBLIC_URL` configures launch buttons. Bot users do not provide a Mini App session token, `initData`, phone number or login code to send commands; the worker gets their identity from Telegram's incoming private message. The Mini App's application session key serves HTTP sign-in separately.

The bot can run without `MARKET_PUBLIC_URL`: it registers its commands and sends plain text replies, including `/status`, without Mini App buttons. When a public HTTPS URL is supplied, it also configures the launch menu and attaches launch buttons. Existing menu settings are left unchanged when no URL is configured.

Run the worker on exactly one instance per bot. Instances without a worker leave all three Telegram credentials unset or blank. Mini App-only deployments and API replicas configure the intended public identity explicitly, without Telegram credentials:

```dotenv
MARKET_MINIAPP_BOT_ID=123456789
MARKET_MINIAPP_BOT_USERNAME=your_bot
MARKET_PUBLIC_URL=https://markets.example.com
```

These identity settings are rejected when a bot worker is enabled; remove them from that instance. The ID binds signature verification to the intended bot, while the username supplies launch-link metadata. Omitting the ID leaves Mini App sign-in disabled. Neither value replaces `MARKET_PUBLIC_URL`, which remains a deployment setting.

`MARKET_BOT_DATA_DIR` defaults to `<MARKET_DATA_DIR>/bot` for native runs; Compose mounts `market-bot-data` at `/bot-data`. The private `<bot-id>-<api-id>.mtproto.sqlite` session stores authentication keys, peer access hashes and update progress, separately from the rebuildable market cache. Preserve this directory across restarts. An exclusive `<bot-id>.lock` also prevents a legacy worker from sharing the directory; it does not coordinate separate deployment volumes.

For migration, stop the previous HTTP polling worker and remove any HTTP webhook before enabling MTProto. Old Bot API `.offset` checkpoints cannot be converted: the first MTProto login starts from Telegram’s current update state and leaves old files untouched. Later starts catch up from the saved MTProto state. Progress is saved through each consecutively completed update; replies use a stable MTProto `random_id` across retries and replays to reduce duplicates, without claiming exactly-once delivery. The standalone bot executable and Compose service have been removed. Notification subscriptions and buying remain outside this integration.

Sign-in uses `POST /auth/telegram` with `{init_data: "<raw Telegram.WebApp.initData>"}`. The API verifies Telegram's Ed25519 signature against its production public key and the discovered or explicitly configured bot ID, checks the signed launch time (one hour, with 30 seconds of future clock tolerance), and issues a 24-hour application session. It never trusts `initDataUnsafe.user`. `GET /auth/me` accepts the resulting `Authorization: Bearer <session>` and returns the verified profile. IDs are strings and expiration dates are RFC 3339.

These HTTP endpoints and public launch settings live in `market-api/src/miniapp.rs` behind `MiniAppApi`. Verification and application session logic live in `market-miniapp`; the HTTP API does not run a Telegram bot. The provider-specific public routes and schema names keep their existing Telegram names.

The browser keeps only its application session and launch fingerprint in tab-scoped session storage. Refresh validates the saved session; a different launch revalidates Telegram's proof, preventing cached identity from carrying over when the Telegram account changes. Sign-out clears that tab's session and pauses automatic sign-in for the current launch. Browsing works without signing in. `/telegram` exposes public configuration and whether sign-in is enabled.

Session signatures use a generated private `data/session.key`, preserved across app restarts and replica rebuilds. For multiple Mini App API replicas, supply the same `MARKET_SESSION_KEY` to every instance, generated offline with `cargo run --locked -p market-app --features miniapp -- session-key`. This key is separate from hub service credentials and the bot token. It does not require a registration database. API-only builds never load or generate it. Permanent user preferences or future alert definitions must use their own application storage, rather than the rebuildable replica.

The **Telegram** link on the standalone website opens a configured Main Mini App with the selected collection, currency, mode and filters. Launch state uses `startapp=v_<base64url query>` with only market-view fields and Telegram's 512-character limit. Very large selections open the app's default view. Launch parameters choose a view; they never authorize an action or identify a user. The browser policy permits the official bridge and Telegram Web embedding while retaining same-origin API/WebSocket connections.

The official [Mini App documentation](https://core.telegram.org/bots/webapps) describes launch configuration and signature validation. The [Telegram MTProto bot documentation](https://core.telegram.org/api/bots) describes bot authorization and available methods.

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
| `GET /media/models/{id}` | Cached model thumbnail |
| `GET /media/models/{id}/animation` | Cached TGS model animation |
| `GET /media/patterns/{id}` | Cached gift pattern thumbnail |
| `GET /telegram` | Public Mini App configuration |
| `POST /auth/telegram`, `GET /auth/me` | Verified Telegram sign-in and session identity |
| `GET /openapi.json` | Generated public contract, including WebSocket frames |
| `GET /healthz`, `GET /readyz` | Process and replica readiness |

Market operations are read-only. Query POST bodies carry filters; the authentication POST issues a user session. Orders require an item type and quote currency; omit venue to combine markets. Default mode is `ask`, sort is `price_asc`, and page size is 50 (maximum 200).

```sh
curl http://localhost:8080/orders/query \
  -H 'Content-Type: application/json' \
  -d '{"scope":{"item_type_id":"tg.gift.5167939598143193218","currency_id":"GRAM","modes":["ask"],"price":{"max":"20"}},"sort":"price_asc","limit":25}'
```

Pages contain `{data, next_page_token, cursor, sync}`. Each order carries its concrete `book`, direct item ID, scalar price, quantity and typed attributes. Prices, quantities and integer attributes are exact strings. Dates are RFC 3339 strings. Values use `{type, value}` and predicates use `{op, ...}`. Nested lists and objects preserve duplicates and match object fields within the same occurrence.

```json
{"scope":{"item_type_id":"tg.gift.5167939598143193218","currency_id":"GRAM","attributes":{"tg.number":{"op":"range","min":{"type":"integer","value":"1"},"max":{"type":"integer","value":"100"}}}},"limit":25}
```

Sort is `price_asc`, `price_desc`, `time_asc`, `time_desc`, `number_asc`, `number_desc` or `identity`. Sorting applies to the full query before pagination. Active orders use observed time; history uses execution time. Gift-number ranking uses exact `tg.number` integer facts; missing numbers follow concrete numbers in ascending order and precede them in descending order. Ties use venue/order identity (execution time also disambiguates history); descending reverses the complete key. Price ranks compare exact decimals. Optional price bounds are inclusive. Executions retain chronological identity order by default and support the same optional `sort` keys plus `period: {from, until}` with inclusive start and exclusive end. Their derived rank indexes are created/backfilled locally on startup. They use the same Order model with `mode: executed`.

Catalog kinds are `venue`, `currency`, `item_type`, `attribute` and `attribute_option`. Exact-ID requests accept at most 500 IDs. `/books` returns `{data, next_after}`; pass its JSON BookKey as the next `after` query parameter. Even an empty filtered page can have a continuation after a bounded scan.

## Pagination and WebSockets

REST pages default to `consistency: "snapshot"`, pinning a local RAM/history view for consistent forward pagination. Active-order reads also accept `consistency: "live"` to use stateless keyset boundaries without retaining server views; the scrolling live feed uses this mode. Views expire after 60 seconds of inactivity or app restart, and are released when traversal completes. Hub restart alone does not invalidate an already retained local view.

Live subscriptions use local keyset boundaries and replace only the requested page. Identical subscriptions share one page computation within an app. The same continuation format works across live and snapshot reads: a live continuation used in REST captures a new snapshot; a snapshot continuation used live ignores its frozen-view ID and resumes its boundary against current data. An expired snapshot returns HTTP 409 in REST but remains a usable live boundary. Changing filters or sort starts a new traversal.

Public continuations are bounded, validated read positions, not credentials. They contain a normalized query fingerprint, sort, keyset position and optional local snapshot ID. They never contain upstream page tokens or the service token. Stateless live boundaries are portable across replicas; frozen views remain local. `cursor` identifies the replicated hub checkpoint behind that page. It is not a public query continuation or a query-specific native Watch cursor.

```json
{"type":"subscribe","id":"offers","query":{"scope":{"item_type_id":"tg.gift.5167939598143193218","currency_id":"GRAM","modes":["ask"]},"sort":"price_asc","limit":25,"page_token":""}}
```

Frames are `{type:"status", id, state:"connecting"}`, then `{type:"page", id, page:{data,next_page_token,cursor,sync}}`. Each page fully replaces the previous one, including refills, execution exits and freshness changes. Errors use `{type:"error", id, error:{code,message}}`. Unsubscribe with `{"type":"unsubscribe","id":"offers"}`. Browser reconnection uses the same query and page boundary; upstream reconnection is handled independently by the replica.

API errors use `{error:{code,message}}`, including JSON, query/path extraction and framework errors. Invalid parameters return 400, oversized JSON returns 413, unsupported content types return 415, unknown routes return 404 and unsupported methods return 405. JSON query bodies are limited to 64 KiB by default. The same extractor policy is installed by the public route configurator and the full server.

WebSockets reserve subscription IDs during asynchronous setup, so pings, unsubscribe and shutdown remain responsive. Unsubscribe cancels pending setup or forwarding; an internal generation prevents queued frames and late completions from affecting a reused ID. An oversized page produces a terminal subscription error and releases the ID. Slow consumers close with code 1013 so clients can reconnect and resubscribe. An API-scoped, bounded cache shares encoded actor pages across subscribers; encoding runs on four tracked blocking workers, converts one order at a time and enforces the output limit while writing. Shutdown drains encoding jobs after closing connections.

`ApiLimits` and `WebSocketLimits` centralize service bounds. Defaults are 256 WebSocket connections across HTTP workers, 16 pending/active subscriptions per socket, 64 KiB input messages, 4 MiB encoded pages, and an outgoing application queue bounded to eight messages and 8 MiB. Setup and send deadlines are 10 and 5 seconds; heartbeat and idle timeouts are 15 and 35 seconds. Large outgoing messages are fragmented at the configured frame size, preserving one JSON message without buffering dozens of complete pages inside the socket library. Local query concurrency and retained views remain bounded by the actor layer. CORS and WebSocket handshakes share one canonical origin policy; configured origins are validated at bind time. Browser origins must match this application's host or be allowed with repeated `--cors-origin` or comma-separated `MARKET_CORS_ORIGINS`. Writer endpoints and raw authenticated hub forwarding are absent. Trusted ingestion services still connect directly to the hub.

`MarketRuntime` owns process startup, supervision and shutdown. Synchronization is an ordinary worker in `market-replica`: it owns the sole storage writer, commits batches and snapshot replacements, then publishes immutable views. The Telegram MTProto loop and the `market-api` HTTP service are also supervised workers. Each starts once, and the application runtime stays outside the HTTP-worker factory.

Actors provide user-facing data access through one `MarketDataService` and a shared `MarketDataHandle` for API and bot callers. The service starts three actors on its dedicated Actix thread: `OrderbooksActor` owns order, execution, book-directory and freshness queries; `CatalogActor` owns reference-definition reads; `UserSubscriptionsActor` owns per-caller subscription leases and shared live-query feeds. Orderbook and catalog requests use separate mailboxes and one shared read budget. Expensive reads run on tracked blocking workers. Bot `/status` uses the orderbooks actor's freshness interface, as does the API's `/sync`.

Actor requests, pages, catalog entries, synchronization status and errors are native Rust contracts owned by `market-actors`. Query validation and the private hub/replica adapters live there. API serialization and HTTP error mapping remain in `market-api`. The API and bot do not receive hub read messages, replica result objects or RPC error types through the data interface; shared native live pages are published by the actors, and the API reuses bounded page encodings across subscribers.

Each subscription call creates its own lease. Identical orderbook queries share page computation across callers; dropping one lease leaves the other callers subscribed. Dropping the last lease stops that feed and releases its capacity, even if an obsolete receiver remains. These are live connection subscriptions; saved preferences and durable alerts remain separate features.

The API and bot use `MarketDataHandle` directly; `market-api::MarketApi` maps public queries and responses to that actor interface. The API and bot receive no storage writer. Within the API crate, WebSocket handlers receive a small `ConnectionSupervisor` that registers connection tasks and supplies a separate cancellation token for each connection. The application supervisor tracks the actor group, synchronization worker, bot and API service. The API drains its connection tasks, and the actor group drains its query and subscription jobs. An API, replica or data-actor failure stops the application; bot failures and failed WebSocket connections remain isolated. Shutdown rejects new reads, cancels subscriptions, drains in-flight commits/reads, stops HTTP workers and joins the actor thread. Normal hub outages still use the synchronization worker's reconnect/rebuild loop.

## Workspace and checks

- `crates/market-hub-protocol`: native bindings generated from the local proto copy.
- `crates/market-hub-client`: authenticated snapshot/Watch transport, service identity, RPC deadlines and bounded capacity retries over one reusable channel.
- `crates/market-replica`: state, indexes, storage, immutable views and an ordinary snapshot/Watch synchronization worker. It has no Actix dependency.
- `crates/market-actors`: actor-owned data contracts and errors, query validation, private replica/protocol adapters, read-only data-serving actors, shared subscriptions and actor lifecycles. It has no direct hub-client dependency.
- `crates/market-api`: public DTOs/OpenAPI, REST/WebSocket query conversion, Telegram sign-in endpoints, React hosting, HTTP configuration and connection supervision. It receives only actor access and a service shutdown token.
- `crates/market-media`: bounded model/pattern media requests, content validation, disk cache and a transport-independent downloader.
- `crates/market-miniapp`: Telegram Mini App signature verification, application sessions, session keys and public launch URL validation. It has no actor, bot, HTTP-server or market-storage dependency; API schemas are an optional feature.
- `crates/market-telegram`: embeddable MTProto bot worker with typed commands, native profile inventory, bounded dispatch, session persistence and a private grammers adapter. It uses actors for market data and owns its launch configuration without Mini App authentication dependencies.
- `crates/market-app`: CLI/configuration, replica and actor assembly, API/bot startup, process supervision and shutdown. Actor read workers retain their capacity permits even if the requesting connection closes.
- `web`: React interface and generated public API types.

Actor implementations live under `market-actors/src/orderbooks/`, `catalog/` and `subscriptions/`. The crate's `models.rs` and `error.rs` define its public data contract; `query.rs` and `protocol.rs` adapt it to the private replica requests and results. `handle.rs` routes requests to the responsible actor, `runtime.rs` shares lifecycle and query-worker support, and `service.rs` starts and drains the actor group. All three use the same published replica views and coordinated shutdown; failure of any actor stops the group and is reported to the application supervisor. Synchronization stays in `market-replica/src/replication.rs` and writes storage directly, without actor messages.

The data path is `hub → synchronization worker → replica views → responsibility-specific actors → API / bot`. Production dependencies are `market-actors → market-replica → market-hub-client`, `market-api → market-actors + market-media + market-miniapp` and `market-telegram → market-actors + market-media`. The API and bot have no direct production dependency on replica storage, the hub client or protocol, or one another. `market-app` wires settings, constructs these services and supervises their lifecycles. Interface tests may use replica writers and fake hub servers to build fixtures; the dependency-boundary test checks production dependencies separately.

Crate folders match their Cargo package names. `market-hub-*` contains upstream transport and protocol code; `market-replica` owns local state, `market-actors` owns the data-serving interface and actors; `market-api` serves HTTP/WebSocket clients; `market-miniapp` contains Mini App authentication and launch support; `market-telegram` runs the bot; `market-app` assembles them.

Within `market-app/src`, `market/` contains `runtime.rs` and the private `supervisor.rs`. `api_config.rs` and `bot_config.rs` translate application settings into the API service and Telegram worker. `MarketRuntime` owns their lifecycles and returns the shared `MarketDataHandle`. The API crate separates public models, REST routes, schema generation, browser policy and optional frontend hosting. `conversion.rs` maps API DTOs to actor contracts, `encoding.rs` shares bounded live-page encodings, `server.rs` owns the service, and `connections.rs` admits and drains WebSocket tasks. The `websocket/` modules own handshake, session, generation registry, outgoing queue and cancellable subscription work. `config.rs` centralizes validated limits. `ApiServer::bind` validates configuration and reserves the listener without starting HTTP workers; `ApiServer::run` starts them under application supervision and drains them on cancellation. `ReplicaReader` and `ReplicationWorker` separate read access from synchronization; `MarketDataHandle` and `MarketDataTask` separate actor access from lifecycle. Public route, configuration, DTO and persisted-data names retain their existing formats. OpenAPI uses an explicit `telegram` tag so documentation groups remain independent of Rust module paths. The `market-app schema` command delegates to `market-api` for frontend type generation. Frontend market helpers live in `web/src/market.ts`; Telegram hooks and controls live in `web/src/telegram-ui.tsx`.

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo test -p market-app -p market-api --no-default-features
cargo test -p market-app -p market-api --no-default-features --features market-app/bot
cargo test -p market-app -p market-api --no-default-features --features market-app/miniapp
npm --prefix web test
# Isolated real React/Lottie browser regression; needs Playwright Chromium:
npm --prefix web run test:previews
# Isolated orderbook interactions, filters, exact prices and responsive layout:
npm --prefix web run test:orderbook
# Preview row balance, fixed symbol tiling, model spacing and footer placement:
npm --prefix web run test:traits
npm --prefix web run build
# Verify API-only browsing against an API-only React build:
cargo test -p market-api --test market_api api_only_browser -- --ignored
# Build Mini App assets, then verify its bridge using a local SDK simulation and real test signatures:
MARKET_SCHEMA_FEATURES=miniapp npm --prefix web run build
cargo test -p market-api --features miniapp --test market_api telegram_react_bridge -- --ignored
cargo build --locked --bins
python3 scripts/smoke.py --hub-bin /path/to/market-hub --adapter-manifest /path/to/adapters/Cargo.toml
```

Tests cover snapshot/Watch overlap, durable checkpoint recovery, replaced datasets, exact prices, typed duplicate list matching, chronological backfills and local-only public reads. The integration check starts a disposable real hub, adapter fixture and app on random local ports. It verifies browser layouts, live updates, execution transitions, hub restart rebuilds and persisted reads while the hub is offline. It never opens account exports or contacts markets. Install its browser with `cd web && npx playwright install chromium`. Optional `--keep target/replica-preview` retains a synthetic preview; use a fresh directory.

Copy coordinated contract updates into the client repositories and use the hub's `scripts/check-contracts.py` before release. All builds use only their own repository. No legacy gateway routes or copied hub implementation crates remain.
