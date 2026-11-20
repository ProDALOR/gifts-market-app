# Typed market WebSocket API

Connect to `/ws`. The GUI and external clients share this public contract. HTTP and WebSocket reads use the same `GiftOrderQuery` and `OrderReadResponse`. OpenAPI is at `/openapi.json`, AsyncAPI at `/asyncapi.json`, and interactive documentation at `/docs` and `/docs/ws`. Public market reads do not require sign-in.

## One query

Send `{"type":"request","id":"read.1","method":"orders.query","params":{"filters":{"collection_id":"tg.gift.123","currency_id":"GRAM"},"source":"asks","limit":100}}`. The result is `{type:"result",id,method:"orders.query",data,range,snapshot?}`, identical to `POST /orders/query` without the request envelope.

`source` is `asks` (default), `bids`, or `history`. Default sorts are respectively `price_asc`, `price_desc`, and `time_desc`. History accepts an optional execution `period`; other sources reject it. A query also accepts half-open `range:{start,end}`, `limit` (default and maximum 100), and `view`. The default range has two null bounds. Unknown fields are rejected.

Queries read forward in sort order within half-open key bounds. Null starts at the open head or extends to the open tail. A finite query returns up to `limit` rows and its covered range. The start is inclusive and the end is exclusive. Continue with `range.start = response.range.end`, preserving the original requested end. Stop when the returned end equals that original end; for an open-ended query this is null. Empty pages can still advance coverage. There is no `next` field. Repricing an order does not move a stored boundary. Each row has one opaque `key`. Compare keys using ordinary ASCII string order (`a < b` in JavaScript), with the same source and sort (including direction); never use locale-aware comparison. Keys contain no query fingerprint and can be reused across filters or execution periods. The supplied query still controls every returned row. Price positions use the query's currency units: reusing a key does not convert currencies or validate its former currency. Do not decode keys or reproduce ranking rules. History keys include execution-time ties. Order identity remains `(book.venue_id,id)`.

## Order payloads

HTTP queries, WebSocket queries, snapshots and upserts share the same `OrderedOrderResponse`. `book` supplies venue, collection and currency. Targets have a `kind` discriminator and flat fields:

```json
{"kind":"gift","id":"gift-id","number":42,"traits":{"model":"model-id"}}
{"kind":"criteria","models":["model-id"],"backdrops":[],"patterns":[]}
{"kind":"unsupported","code":"invalid_attributes","message":"This listing's gift details are unavailable."}
```

An order gift's collection is `book.collection_id`; standalone valuation gifts retain their own `collection_id`. Unknown gift numbers and trait IDs are omitted. Missing `market_url`, `expires_at` and `executed_at` are omitted too; executions always include `executed_at`. The dedicated `market_url` is a validated absolute HTTP(S) link without embedded credentials or control characters. Unsupported codes are typed; arbitrary source metadata is not exposed.

Protocol nulls retain meaning: `range.start`/`range.end` are always present and null denotes an open bound; An absent `snapshot` means no frozen view was retained.

## Range subscriptions

REST `POST /orders/query`, WebSocket method `orders.query`, and `orders.watch` accept the same query model. Start a watch directly; no preliminary query is needed:

```json
{"type":"orders.watch","id":"orders.1","params":{"filters":{"collection_id":"tg.gift.123","currency_id":"GRAM"},"source":"asks","range":{"start":null,"end":null},"limit":80}}
```

A watch reads up to `limit` rows (default and maximum 100) and atomically establishes the actual covered range from that read. The initial snapshot includes the covered `range`, just like a finite query. Continue from its exclusive end with `orders.query` to load more without replacing the watch. Identical resolved coverage shares a producer even while it is catching up; an exact, current range can initialize directly from that producer without another read. Each caller retains its own first snapshot before consuming coalesced shared updates, so changes during setup cannot enlarge or replace that baseline. Initialization slots and live producers are bounded separately by the live-query limit. `limit` controls initialization; later updates reconcile the complete fixed coverage, up to the 100-row live cap. Leave headroom for arrivals by initializing fewer rows, such as 80. If coverage grows beyond capacity, the operation terminates with a capacity error; restart a watch with narrower coverage. Updates never silently clip coverage.

For `source:"bids"`, the same flat query accepts `bid_match:"any"` (default) or `"all"`. Any requires an accepted value in common with each selected trait group; omitted bid conditions accept any value. All requires exactly the selected trait sets and gift-number bounds. Empty/omitted selections require absent restrictions under All: selecting models without a backdrop excludes backdrop-restricted bids. Equivalent singleton equality/set/range encodings match; set order and duplicates do not matter. Groups combine with AND. Number ranges overlap for Any and require equal bounds for All, including unbounded ends. Collection/currency/venue and bid-price filters retain their ordinary meaning. With no gift selections, Any returns all eligible bids, while All returns only unrestricted bids. Unsupported complex conditions remain visible only in unfiltered Any browsing. All matching is only valid for Bids. Matching occurs before finite pagination and live diffs; keep `bid_match` with filters when paging or reconnecting. Changing it creates a new subscription/query selection while ordering keys retain the same sort/source semantics.

```json
{"type":"orders.watch","id":"bids.1","params":{"source":"bids","bid_match":"all","filters":{"collection_id":"tg.gift.123","currency_id":"GRAM","models":["gold","silver"]},"limit":80}}
```

Bounds stay fixed as prices change, even if a boundary listing disappears. Keep the returned range rather than deriving it from first/last rows: open bounds, filtered scans and empty results carry meaningful coverage. Initialization captures data and its observation together; the producer reconciles changes since that read before waiting for new updates. Older completed reads cannot roll shared state back. There is no `navigation`, `around` operation, nested `params.query` or separate watch range. Watches require the default latest view. Inverted bounds and sort/source mismatches are rejected.

The first data frame is `{type:"orders.snapshot",id,sequence:0,data,range}`. Subsequent frames are `{type:"orders.changed",id,sequence,upserts,removed}`. Require `sequence === lastSequence + 1` for changes. Apply each batch atomically: remove both deleted and replaced identities first, then merge all upserts in `key` order. Multiple orders can exchange ranks in the same batch. Removals and upserts have disjoint identities. Removal means an order left the matching range and does not imply a sale.

Finite WebSocket order results flatten `data`, `range` and optional `snapshot` alongside `type`, `method` and `id`; there is no `data.data` wrapper. `orders.changed` frames do not repeat coverage. Frozen-view IDs belong only to finite query results.

Every `orders.snapshot` replaces the complete subscribed state, including when a dataset is replaced. Its sequence advances within that operation. Reconnect or sequence gaps require a new watch initialized from sequence zero. Watches can coalesce intermediate observations; they are current state, not a durable event journal. Live history includes late backfills within coverage.

## Frozen traversal

`view` defaults to `{kind:"latest"}` and retains no server state. Set `{kind:"snapshot"}` to pin an immutable view. The response includes a `snapshot` ID; send `view:{kind:"snapshot",id}` for subsequent reads. Follow the returned range end, keeping the original end bound, until the two ends match. A bounded scan may return short or empty data with an advancing range end; keep reading until the requested end is reached. After initialization, watched ranges always return complete coverage or an explicit capacity error. No separate history endpoint or page token is needed.

Release with `DELETE /orders/query` and `{snapshot}`. Views expire after 60 seconds idle or application restart, share bounded capacity, and require instance affinity behind a load balancer. An expired/released ID returns HTTP 409 `restart_read`; reopening at the visible key establishes a fresh baseline. Release is idempotent and an in-flight read cannot resurrect a released view.

The GUI keeps all visited listings and virtualizes mounted cards. It initializes with one 80-row watch and uses the same query model for asks, bids and history, preserving execution-period filters across paging and reconnects. Cached rows and loaded coverage are separate: a partial replacement baseline retains older rows but reopens the interval before the next cached row. Additional WebSocket queries fill unread gaps using bounded ranges; an old open-tail result does not imply that newly reopened gaps are loaded. Viewport watches stay within contiguous covered intervals below the 100-row cap. Only the active range is live; cached regions refresh on revisit. New executions and late backfills appear within watched coverage without polling or opening snapshot leases.

The latest visible anchor is remembered for reconnects without restarting healthy watches. Pagination requested while reconnecting waits for the fresh baseline. A replacement snapshot cancels an outstanding page read and preserves its intent; continuation uses current unread coverage after cancellation completes. Delayed pages cannot overwrite the current watched state. Empty filtered pages continue scanning, while retryable failures use backoff. Capacity errors prepare narrower coverage for the socket's delayed retry. Subscription state remains connecting, reconnecting or failed until a valid baseline arrives; market availability alone cannot make it Live. Explicitly paused reads use one snapshot pager; rebuilding after expiry reads through the visited tail, including cached gaps, before publishing a new view.

## Status, operations and budgets

Each connection receives a `market.status` message initially and when availability changes, for example `{type:"market.status",status:"available"}`. Status is `available` when normal market updates are available, `delayed` when previously received data can still be served, or `unavailable` when market data is not available. This message has no operation ID and does not advance order sequences. Unchanged availability does not produce duplicate status messages. A connected socket does not itself establish market availability.

Valuation results contain `strategy`, `evaluated_at`, `stale`, typed calculation fields and their display projection. `stale` describes delayed inputs at evaluation time. Clients should also react to current availability: after a delay or connection loss, treat a previously displayed estimate as potentially outdated until a fresh evaluation succeeds. A response to a request started before the interruption remains potentially outdated even if it arrives after recovery. Calculation time is not an assertion about when every underlying listing changed. Status changes do not require automatically issuing valuations.

`GET /sync` is removed. Operational `/healthz` and `/readyz` remain for deployments and stay out of the consumer schema. Readiness remains successful while saved market data can be served during delays.

Operation IDs use 1..64 ASCII letters, digits, `_`, `-` or `.` and are unique while active. `{"type":"cancel","id":"orders.1"}` cancels without acknowledgement. A frame already being written can still arrive, so use fresh IDs for replacements and ignore retired IDs. Validation failures with a valid, unused ID return a correlated terminal `error`, so requests can fail immediately. A command that reuses an active ID returns nonterminal `command_error` and preserves the active operation. When no valid ID can be parsed, `command_error` has an empty ID. Every `error` has a nonempty operation ID; connection failures use WebSocket close frames. Method `valuation.evaluate` shares its handler and response with HTTP.

Subscriptions return an initial snapshot, catalog version manifest or error; there is no separate connecting acknowledgement.

The GUI bounds connection handshakes to ten seconds, cancels reconnect timers when its final observer leaves, and retries persistent subscriptions with fresh wire IDs. One-shot requests are never replayed after disconnect. Terminal operation IDs are retired before callbacks run, so late frames cannot revive completed work or suppress a retry. Query updates with unchanged parameters preserve the current subscription and its sequence.

Reads, scans, range membership, encoded bytes, concurrency, outgoing queues and retained views remain bounded. Initialization and finite reads report their actual coverage and continuation. Later watch budget failures are explicit errors, never incomplete ranges presented as complete. Narrow coverage or reduce the slice size on capacity errors. Slow consumers close with code 1013; reconnect and initialize again. Cancellation, command ingress and supervision continue while data frames are written. Shutdown cancels operations and drains tracked reads and encoders.

## Catalog updates

`{"type":"catalog.watch","id":"catalog","collection_ids":["tg.gift.123"]}` watches up to eight collections; an empty list watches the directory. Initial subscription and reconnect send complete `catalog.versions` manifests matching unquoted ETags from `/catalog` and `/collections/{id}`. Public versions are opaque 22-character tokens: an unpadded Base64url encoding of a 128-bit digest, without a format-version prefix, for example `f3MgDfNxr-4kEnhwdAqItQ`. One presentation-layer converter namespaces the complete domain content revision and produces both HTTP ETags and WebSocket tokens. HTTP quotes the token (`"f3MgDfNxr-4kEnhwdAqItQ"`); compare the unquoted value for equality, without parsing, sorting or decoding it. A null collection value still means the collection is unavailable. Older prefixed ETags cause one normal 200 response with the new token, then ordinary 304 revalidation resumes. Prices and freshness do not invalidate catalog content. Changed active collections revalidate; inactive collections become stale until needed. Reconnect comparisons recover missed changes without discarding unchanged content. Retries use bounded exponential backoff with jitter.

## Upgrading clients

This contract uses `catalog.watch` and `catalog.versions`; the former catalog message names and operation-level `status` frames are removed. Targets are flat, without `gift`, `criteria` or `reason` wrappers. Navigation keys have no format-version marker or query fingerprint: unpadded Base64 with the ASCII-ordered, URL-unreserved alphabet `0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ_abcdefghijklmnopqrstuvwxyz~`. Sort/direction and source are packed into one header byte; rank, identity and before/at/after edges preserve ordinary string ordering. No repeated-character grammar is needed. Treat the encoded token as opaque. Update backend and frontend together and restart existing watches; discard navigation keys from the previous format. Keys are positions interpreted within the supplied query, with sort/source validation rather than filter binding. Order results use half-open ranges and omit `next`; finite queries support an optional `snapshot` lease. Discard cached coverage from the former inclusive-range contract.


Method `valuation.query` shares `POST /valuations/query`: pass `target`, `currency_id`, an explicit nonempty `strategies` list and an explicit half-open `period` when selecting execution methods. Targets are collection, model, backdrop, model_backdrop or gift. Discovery at `/valuations/strategies` advertises capabilities. The result contains one resolved target, currency, evaluation time, stale flag and independent typed strategy results with display projections. Successful results have `strategy` and `data`; failures have `strategy` and `error`. Missing samples are structured price issues. Mean/median share one history scan, and the entire report uses one pinned snapshot. Cancellation and request deadlines use the ordinary request lifecycle; reconnect does not replay the request. The former gift-only method has been removed; gift queries use the same method with `target:{kind:"gift",gift:"Gift-123"}` and `strategies:["gift_floor"]`.


## Valuation subscriptions

`valuation.watch` accepts exactly the same `ValuationQuery` as HTTP and `valuation.query`. Send, for example:

```json
{"type":"valuation.watch","id":"prices","params":{"currency_id":"GRAM","strategies":["catalog_floor"],"target":{"kind":"catalog","collection_ids":["tg.gift.123"],"details":{"collection_id":"tg.gift.123"}}}}
```

Each `valuation.snapshot` has `{type,id,data}`; `data` is the same `ValuationResponse` as a finite query, including resolved target, currency, calculation time, stale state and independent typed strategy results/display projections. A snapshot atomically replaces the report. Any registered strategy can be selected, subject to its input contract; fixed execution periods observe late backfills as market history arrives. There is no automatic rolling period or durable replay.

For picker values, `catalog_floor` accepts a `catalog` target with one to eight distinct collection IDs. Optional `details` must select one of those collections and adds all known model/backdrop floors and an optional pair. Its result data is `{collections:[{collection_id,evaluated_at,floor,traits?}]}`. Traits contain model/backdrop maps and an optional pair. Each floor has an exact price or typed unavailable issue plus evidence. Currency and stale state are on the report, and each collection retains its own calculation time when other collections change.

Observed data changes, availability and listing expiry refresh subscriptions with 200 ms coalescing. Internal dependencies suppress unrelated updates; availability-only changes reuse calculation data and preserve calculation time. Identical active queries share completed reads through a bounded weak cache while cancellation remains independent. Up to 32 valuation watches can exist application-wide, in addition to connection limits. Cancel, replace or disconnect releases the lease. Reconnect with a fresh watch for an atomic baseline; retained client values stay stale until it arrives. The former floor-specific endpoint and protocol are removed.

The `valuation.query` target may also be `{kind:"criteria",collection_id,criteria:{models:[],backdrops:[],patterns:[],number:null}}`. `ask_floor` returns the lowest ask matching all conditions together, useful for bids. Alternatives within each group use OR; different groups combine with AND; omitted restrictions accept any value. This differs from the Any/All mode for browsing bid orders. Execution strategies accept the same target with an explicit period. At most 500 trait alternatives are allowed. Individual-gift `gift_floor` remains gift-only.

Each display field contains `id`, `label`, `primary` and a typed `value`; field order and precision belong to the strategy. Attribute previews use the independent catalog valuation, while the calculation popup retains all fields from the gift estimate's original report. A historical execution's price and a present-day estimate are distinct values.


Valuation reports use one tagged resolved target and one `{currency_id,evaluated_at,stale}` context. A resolved gift is `{kind:"gift",gift:{id,collection_id,number?,traits},slug?}`; its strategy data contains only metrics and evidence. There is no gift-specific response format. WebSocket frames are encoded once before queue admission; the retained byte buffer is used for both byte accounting and fragmented writes.


Strategy discovery includes `input`: `catalog`, `selection`, `selection_with_execution_period` or `resolved_gift`. The accompanying `targets` and `requires_period` are derived from that contract. Query parameters are unchanged: callers submit a target/lookup, currency, explicit strategies and, for execution strategies, a nonempty historical period with both bounds. Target resolution happens once; typed input preparation and per-strategy failures use the same path for every strategy, including `gift_floor`. Missing optional gift attributes retain typed partial metric issues.

Clients may filter discovery by `targets` and `requires_period` to build an explicit selection. The bot and GUI order details select compatible strategies without periods; the GUI target panel lets users choose period-dependent strategies. These projections come from each typed input contract. Domain facts are resolved by the service, and callers do not submit `SelectionInput`, `ExecutionInput` or resolved gift facts directly. Strategy names, display fields and notes belong to the registry/results, not transport handlers.
