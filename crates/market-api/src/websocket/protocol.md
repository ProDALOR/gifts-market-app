# Typed market WebSocket API

Connect to `/ws`. The GUI and external clients share this public contract. HTTP and WebSocket reads use the same `GiftOrderQuery` and `OrderReadResponse`. OpenAPI is at `/openapi.json`, AsyncAPI at `/asyncapi.json`, and interactive documentation at `/docs` and `/docs/ws`. Public market reads do not require sign-in.

## One query

Send `{"type":"request","id":"read.1","method":"orders.query","params":{"filters":{"collection_id":"tg.gift.123","currency_id":"GRAM"},"source":"asks","limit":150}}`. The result is `{type:"result",id,method:"orders.query",data:{data,range,next,snapshot?}}`, identical to `POST /orders/query` without the request envelope.

`source` is `asks` (default), `bids`, or `history`. Default sorts are respectively `price_asc`, `price_desc`, and `time_desc`. History accepts an optional execution `period`; other sources reject it. A query also accepts `navigation` and `view`. Unknown fields are rejected.

Navigation defaults to `{type:"head"}`. `{type:"after",key}` and `{type:"before",key}` read strictly beyond the supplied ordering point; repricing its original order does not move that point. `{type:"around",key}` follows the current order identity when available, reserves one third of the slice before it, and fills available space at the edges. Each row has one opaque `key`. Compare keys lexicographically within the same filters, source, period and sort; do not decode keys or reproduce ranking rules. History keys include execution-time ties. Order identity remains `(book.venue_id,id)`.

## Range subscriptions

Send `{"type":"orders.watch","id":"orders.1","params":{"query":{"filters":{"collection_id":"tg.gift.123","currency_id":"GRAM"},"limit":150}}}`. Initialization and monitoring are one operation, so there is no separate-read/subscription gap. The initial slice establishes `range:{start,end}`. Bounds are inclusive ordering keys, with null extending to the open head or tail. Keys may represent the point immediately before or after an order. Coverage stays fixed as orders reprice: the row count can grow or shrink rather than refilling a fixed window. The query limit controls only the seed.

Optional `params.range` instead watches that explicit query-bound coverage from the outset. The server rejects inverted ranges and keys from another query. Replace coverage by cancelling the operation and starting a fresh ID, keeping visible data until the new snapshot arrives. Watches require `view:{kind:"latest"}` (the default).

The first data frame is `{type:"orders.snapshot",id,sequence:0,data,range}`. Subsequent frames are `{type:"orders.changed",id,sequence,upserts,removed}`. Require `sequence === lastSequence + 1` for changes. Apply each batch atomically: remove identities, replace upserts by identity, and sort by `key`. Removals and upserts have disjoint identities. Removal means an order left the matching range and does not imply a sale.

Live snapshots contain only `type`, `id`, `sequence`, `data` and `range`. Continuations and frozen-view IDs belong to finite query results.

Every `orders.snapshot` replaces the complete subscribed state, including when a dataset is replaced. Its sequence advances within that operation. Reconnect or sequence gaps require a new watch initialized from sequence zero. Watches can coalesce intermediate observations; they are current state, not a durable event journal. Live history includes late backfills within coverage.

## Frozen traversal

`view` defaults to `{kind:"latest"}` and retains no server state. Set `{kind:"snapshot"}` to pin an immutable view. The response includes a `snapshot` ID; send `view:{kind:"snapshot",id}` for subsequent reads. Follow `next` using `navigation:{type:"after",key:next}` until null. Before-navigation returns the key for continuing backwards. A bounded scan may return a short or empty slice with a non-null `next`; continue until null even if `data` is empty. Watched ranges always return complete coverage or an explicit capacity error. No separate history endpoint or page token is needed.

Release with `DELETE /orders/query` and `{snapshot}`. Views expire after 60 seconds idle or application restart, share bounded capacity, and require instance affinity behind a load balancer. An expired/released ID returns HTTP 409 `restart_read`; reopening at the visible key establishes a fresh baseline. Release is idempotent and an in-flight read cannot resurrect a released view.

The GUI keeps all visited listings and virtualizes mounted cards. Only the active range is live; cached regions refresh on revisit. Pausing retains visited rows and uses a frozen view. The History tab uses forward snapshot traversal through the same query endpoint.

## Status, operations and budgets

Each connection receives a `market.status` message initially and when availability changes, for example `{type:"market.status",status:"available"}`. Status is `available` when normal market updates are available, `delayed` when previously received data can still be served, or `unavailable` when market data is not available. This message has no operation ID and does not advance order sequences. Unchanged availability does not produce duplicate status messages. A connected socket does not itself establish market availability.

Valuation results contain `strategy`, `evaluated_at`, `stale`, typed calculation fields and their display projection. `stale` describes delayed inputs at evaluation time. Clients should also react to current availability: after a delay or connection loss, treat a previously displayed estimate as potentially outdated until a fresh evaluation succeeds. A response to a request started before the interruption remains potentially outdated even if it arrives after recovery. Calculation time is not an assertion about when every underlying listing changed. Status changes do not require automatically issuing valuations.

`GET /sync` is removed. Operational `/healthz` and `/readyz` remain for deployments and stay out of the consumer schema. Readiness remains successful while saved market data can be served during delays.

Operation IDs use 1..64 ASCII letters, digits, `_`, `-` or `.` and are unique while active. `{"type":"cancel","id":"orders.1"}` cancels without acknowledgement. A frame already being written can still arrive, so use fresh IDs for replacements and ignore retired IDs. Malformed commands and duplicate IDs yield `command_error` while active operations continue unchanged; its ID is empty when no valid command identity was parsed. Terminal `error` ends the operation; an empty ID denotes a connection-level failure. Method `valuation.evaluate` shares its handler and response with HTTP.

Subscriptions return an initial snapshot, catalog version manifest or error; there is no separate connecting acknowledgement.

The GUI bounds connection handshakes to ten seconds, cancels reconnect timers when its final observer leaves, and retries persistent subscriptions with fresh wire IDs. One-shot requests are never replayed after disconnect. Terminal operation IDs are retired before callbacks run, so late frames cannot revive completed work or suppress a retry. Query updates with unchanged parameters preserve the current subscription and its sequence.

Reads, scans, range membership, encoded bytes, concurrency, outgoing queues and retained views remain bounded. Budget failures are explicit errors, never incomplete ranges presented as complete. Narrow coverage or reduce the slice size on capacity errors. Slow consumers close with code 1013; reconnect and initialize again. Cancellation, command ingress and supervision continue while data frames are written. Shutdown cancels operations and drains tracked reads and encoders.

## Catalog updates

`{"type":"catalog.watch","id":"catalog","collection_ids":["tg.gift.123"]}` watches up to eight collections; an empty list watches the directory. Initial subscription and reconnect send complete `catalog.versions` manifests matching unquoted ETags from `/catalog` and `/collections/{id}`. Prices and freshness do not invalidate catalog content. Changed active collections revalidate; inactive collections become stale until needed. Reconnect comparisons recover missed changes without discarding unchanged content. Retries use bounded exponential backoff with jitter.

## Upgrading clients

This contract uses `catalog.watch` and `catalog.versions`; the former catalog message names and operation-level `status` frames are removed. Navigation keys now use a compact versioned encoding. Update backend and frontend together and restart existing watches; discard navigation keys from the previous format. Keys remain opaque, query-bound and lexicographically ordered. Finite HTTP and WebSocket query results retain `next` and optional `snapshot` fields.
