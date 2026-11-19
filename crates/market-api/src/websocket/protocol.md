# Typed market WebSocket API

Connect to `/ws`. The GUI and external clients share this public contract. HTTP and WebSocket reads use the same `GiftOrderQuery` and `OrderReadResponse`. OpenAPI is at `/openapi.json`, AsyncAPI at `/asyncapi.json`, and interactive documentation at `/docs` and `/docs/ws`. Public market reads do not require sign-in.

## One query

Send `{"type":"request","id":"read.1","method":"orders.query","params":{"filters":{"collection_id":"tg.gift.123","currency_id":"GRAM"},"source":"asks","limit":100}}`. The result is `{type:"result",id,method:"orders.query",data:{data,range,next,snapshot?}}`, identical to `POST /orders/query` without the request envelope.

`source` is `asks` (default), `bids`, or `history`. Default sorts are respectively `price_asc`, `price_desc`, and `time_desc`. History accepts an optional execution `period`; other sources reject it. A query also accepts inclusive `range:{start,end}`, `limit` (default and maximum 100), and `view`. The default range has two null bounds. Unknown fields are rejected.

Queries read forward in sort order within inclusive key bounds. Null starts at the open head or extends to the open tail. A finite query returns up to `limit` rows; `next` is an exclusive continuation point. To continue, set `range.start` to `next` and preserve the original `range.end`. Repricing an order does not move a stored boundary. Each row has one opaque `key`. Compare keys using ordinary ASCII string order (`a < b` in JavaScript), with the same source and sort (including direction); never use locale-aware comparison. Keys contain no query fingerprint and can be reused across filters or execution periods. The supplied query still controls every returned row. Price positions use the query's currency units: reusing a key does not convert currencies or validate its former currency. Do not decode keys or reproduce ranking rules. History keys include execution-time ties. Order identity remains `(book.venue_id,id)`.

## Order payloads

HTTP queries, WebSocket queries, snapshots and upserts share the same `OrderedOrderResponse`. `book` supplies venue, collection and currency. Targets have a `kind` discriminator and flat fields:

```json
{"kind":"gift","id":"gift-id","number":42,"traits":{"model":"model-id"}}
{"kind":"criteria","models":["model-id"],"backdrops":[],"patterns":[]}
{"kind":"unsupported","code":"invalid_attributes","message":"This listing's gift details are unavailable."}
```

An order gift's collection is `book.collection_id`; standalone valuation gifts retain their own `collection_id`. Unknown gift numbers and trait IDs are omitted. Missing `market_url`, `expires_at` and `executed_at` are omitted too; executions always include `executed_at`. The dedicated `market_url` is a validated absolute HTTP(S) link without embedded credentials or control characters. Unsupported codes are typed; arbitrary source metadata is not exposed.

Protocol nulls retain meaning: `range.start`/`range.end` are always present and null denotes an open bound; query and watch-snapshot `next` is always present and null denotes traversal completion. An absent `snapshot` means no frozen view was retained.

## Range subscriptions

REST `POST /orders/query`, WebSocket method `orders.query`, and `orders.watch` accept the same query model. Start a watch directly; no preliminary query is needed:

```json
{"type":"orders.watch","id":"orders.1","params":{"filters":{"collection_id":"tg.gift.123","currency_id":"GRAM"},"source":"asks","range":{"start":null,"end":null},"limit":80}}
```

A watch reads up to `limit` rows (default and maximum 100) and atomically establishes the actual covered range from that read. The initial snapshot includes `range` and the exclusive continuation `next`, just like a finite query. Follow `next` with `orders.query` to load more without replacing the watch. `limit` controls initialization; later updates reconcile the complete fixed coverage, up to the 100-row live cap. Leave headroom for arrivals by initializing fewer rows, such as 80. If coverage grows beyond capacity, the operation terminates with a capacity error; restart a watch with narrower coverage. Updates never silently clip coverage.

Bounds stay fixed as prices change, even if a boundary listing disappears. Keep the returned range rather than deriving it from first/last rows: open bounds, filtered scans and empty results carry meaningful coverage. Watching is registered before initialization so changes during that read are reconciled. There is no `navigation`, `around` operation, nested `params.query` or separate watch range. Watches require the default latest view. Inverted bounds and sort/source mismatches are rejected.

The first data frame is `{type:"orders.snapshot",id,sequence:0,data,range,next}`. Subsequent frames are `{type:"orders.changed",id,sequence,upserts,removed}`. Require `sequence === lastSequence + 1` for changes. Apply each batch atomically: remove both deleted and replaced identities first, then merge all upserts in `key` order. Multiple orders can exchange ranks in the same batch. Removals and upserts have disjoint identities. Removal means an order left the matching range and does not imply a sale.

Live snapshots include `next` (null when traversal is complete); `orders.changed` frames do not repeat coverage or continuation. Frozen-view IDs belong only to finite query results.

Every `orders.snapshot` replaces the complete subscribed state, including when a dataset is replaced. Its sequence advances within that operation. Reconnect or sequence gaps require a new watch initialized from sequence zero. Watches can coalesce intermediate observations; they are current state, not a durable event journal. Live history includes late backfills within coverage.

## Frozen traversal

`view` defaults to `{kind:"latest"}` and retains no server state. Set `{kind:"snapshot"}` to pin an immutable view. The response includes a `snapshot` ID; send `view:{kind:"snapshot",id}` for subsequent reads. Follow `next` by setting `range.start` to that key until null, keeping the original end bound. A bounded scan may return a short or empty slice with a non-null `next`; continue until null even if `data` is empty. After initialization, watched ranges always return complete coverage or an explicit capacity error. No separate history endpoint or page token is needed.

Release with `DELETE /orders/query` and `{snapshot}`. Views expire after 60 seconds idle or application restart, share bounded capacity, and require instance affinity behind a load balancer. An expired/released ID returns HTTP 409 `restart_read`; reopening at the visible key establishes a fresh baseline. Release is idempotent and an in-flight read cannot resurrect a released view.

The GUI keeps all visited listings and virtualizes mounted cards. The GUI initializes with one 80-row watch, uses WebSocket queries for additional live pages, watches cached viewport coverage with headroom below the 100-row cap, and re-seeds a watch after capacity errors. Only the active range is live; cached regions refresh on revisit. Pausing retains visited rows and uses a frozen view. The History tab uses forward snapshot traversal through the same query endpoint.

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

This contract uses `catalog.watch` and `catalog.versions`; the former catalog message names and operation-level `status` frames are removed. Targets are flat, without `gift`, `criteria` or `reason` wrappers. Navigation keys have no format-version marker or query fingerprint: unpadded Base64 with the ASCII-ordered, URL-unreserved alphabet `0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ_abcdefghijklmnopqrstuvwxyz~`. Sort/direction and source are packed into one header byte; rank, identity and before/at/after edges preserve ordinary string ordering. No repeated-character grammar is needed. Treat the encoded token as opaque. Update backend and frontend together and restart existing watches; discard navigation keys from the previous format. Keys are positions interpreted within the supplied query, with sort/source validation rather than filter binding. Finite HTTP and WebSocket query results retain `next` and optional `snapshot` fields.
