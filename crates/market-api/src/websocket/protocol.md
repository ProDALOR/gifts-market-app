# Typed market WebSocket API

Connect to `/ws`. The GUI and external clients share this public contract. HTTP and WebSocket reads use the same `GiftOrderQuery` and `OrderReadResponse`. OpenAPI is at `/openapi.json`, AsyncAPI at `/asyncapi.json`, and interactive documentation at `/docs` and `/docs/ws`. Public market reads do not require sign-in.

## One query

Send `{"type":"request","id":"read.1","method":"orders.query","params":{"filters":{"collection_id":"tg.gift.123","currency_id":"GRAM"},"source":"asks","limit":150}}`. The result is `{type:"result",id,method:"orders.query",data:{data,range,next,snapshot?}}`, identical to `POST /orders/query` without the request envelope.

`source` is `asks` (default), `bids`, or `history`. Default sorts are respectively `price_asc`, `price_desc`, and `time_desc`. History accepts an optional execution `period`; other sources reject it. A query also accepts `navigation` and `view`. Unknown fields are rejected.

Navigation defaults to `{type:"head"}`. `{type:"after",key}` and `{type:"before",key}` read strictly beyond the supplied ordering point; repricing its original order does not move that point. `{type:"around",key}` follows the current order identity when available, reserves one third of the slice before it, and fills available space at the edges. Each row has one opaque `key`. Compare keys using ordinary ASCII string order (`a < b` in JavaScript), with the same source and sort (including direction); never use locale-aware comparison. Keys contain no query fingerprint and can be reused across filters or execution periods. The supplied query still controls every returned row. Price positions use the query's currency units: reusing a key does not convert currencies or validate its former currency. Do not decode keys or reproduce ranking rules. History keys include execution-time ties. Order identity remains `(book.venue_id,id)`.

## Order payloads

HTTP queries, WebSocket queries, snapshots and upserts share the same `OrderedOrderResponse`. `book` supplies venue, collection and currency. Targets have a `kind` discriminator and flat fields:

```json
{"kind":"gift","id":"gift-id","number":42,"traits":{"model":"model-id"}}
{"kind":"criteria","models":["model-id"],"backdrops":[],"patterns":[]}
{"kind":"unsupported","code":"invalid_attributes","message":"This listing's gift details are unavailable."}
```

An order gift's collection is `book.collection_id`; standalone valuation gifts retain their own `collection_id`. Unknown gift numbers and trait IDs are omitted. Missing `market_url`, `expires_at` and `executed_at` are omitted too; executions always include `executed_at`. The dedicated `market_url` is a validated absolute HTTP(S) link without embedded credentials or control characters. Unsupported codes are typed; arbitrary source metadata is not exposed.

Protocol nulls retain meaning: `range.start`/`range.end` are always present and null denotes an open bound; finite query `next` is always present and null denotes traversal completion. An absent `snapshot` means no frozen view was retained.

## Range subscriptions

Send `{"type":"orders.watch","id":"orders.1","params":{"query":{"filters":{"collection_id":"tg.gift.123","currency_id":"GRAM"},"limit":150}}}`. Initialization and monitoring are one operation, so there is no separate-read/subscription gap. The initial slice establishes `range:{start,end}`. Bounds are inclusive ordering keys, with null extending to the open head or tail. Keys may represent the point immediately before or after an order. Coverage stays fixed as orders reprice: the row count can grow or shrink rather than refilling a fixed window. The query limit controls only the seed.

Optional `params.range` instead watches that explicit coverage within the supplied query from the outset. The server rejects inverted ranges and keys with a different sort or source. Keys from other filters are accepted as ordering boundaries. Replace coverage by cancelling the operation and starting a fresh ID, keeping visible data until the new snapshot arrives. Watches require `view:{kind:"latest"}` (the default).

The first data frame is `{type:"orders.snapshot",id,sequence:0,data,range}`. Subsequent frames are `{type:"orders.changed",id,sequence,upserts,removed}`. Require `sequence === lastSequence + 1` for changes. Apply each batch atomically: remove both deleted and replaced identities first, then merge all upserts in `key` order. Multiple orders can exchange ranks in the same batch. Removals and upserts have disjoint identities. Removal means an order left the matching range and does not imply a sale.

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

Operation IDs use 1..64 ASCII letters, digits, `_`, `-` or `.` and are unique while active. `{"type":"cancel","id":"orders.1"}` cancels without acknowledgement. A frame already being written can still arrive, so use fresh IDs for replacements and ignore retired IDs. Validation failures with a valid, unused ID return a correlated terminal `error`, so requests can fail immediately. A command that reuses an active ID returns nonterminal `command_error` and preserves the active operation. When no valid ID can be parsed, `command_error` has an empty ID. Every `error` has a nonempty operation ID; connection failures use WebSocket close frames. Method `valuation.evaluate` shares its handler and response with HTTP.

Subscriptions return an initial snapshot, catalog version manifest or error; there is no separate connecting acknowledgement.

The GUI bounds connection handshakes to ten seconds, cancels reconnect timers when its final observer leaves, and retries persistent subscriptions with fresh wire IDs. One-shot requests are never replayed after disconnect. Terminal operation IDs are retired before callbacks run, so late frames cannot revive completed work or suppress a retry. Query updates with unchanged parameters preserve the current subscription and its sequence.

Reads, scans, range membership, encoded bytes, concurrency, outgoing queues and retained views remain bounded. Budget failures are explicit errors, never incomplete ranges presented as complete. Narrow coverage or reduce the slice size on capacity errors. Slow consumers close with code 1013; reconnect and initialize again. Cancellation, command ingress and supervision continue while data frames are written. Shutdown cancels operations and drains tracked reads and encoders.

## Catalog updates

`{"type":"catalog.watch","id":"catalog","collection_ids":["tg.gift.123"]}` watches up to eight collections; an empty list watches the directory. Initial subscription and reconnect send complete `catalog.versions` manifests matching unquoted ETags from `/catalog` and `/collections/{id}`. Prices and freshness do not invalidate catalog content. Changed active collections revalidate; inactive collections become stale until needed. Reconnect comparisons recover missed changes without discarding unchanged content. Retries use bounded exponential backoff with jitter.

## Upgrading clients

This contract uses `catalog.watch` and `catalog.versions`; the former catalog message names and operation-level `status` frames are removed. Targets are flat, without `gift`, `criteria` or `reason` wrappers. Navigation keys now use version 4, without query fingerprints: unpadded Base64 with the ASCII-ordered, URL-unreserved alphabet `0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ_abcdefghijklmnopqrstuvwxyz~`. Sort and source are retained in a three-byte header; rank, identity and before/at/after edges preserve ordinary string ordering. No repeated-character grammar is needed. Treat the encoded token as opaque. Update backend and frontend together and restart existing watches; discard navigation keys from the previous format. Keys are positions interpreted within the supplied query, with sort/source validation rather than filter binding. Finite HTTP and WebSocket query results retain `next` and optional `snapshot` fields.
