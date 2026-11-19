# Typed market WebSocket API

Connect to `/ws`. The GUI and external clients use this same public contract. HTTP and WebSocket finite reads share query and response types. OpenAPI is served at `/openapi.json`; AsyncAPI is at `/asyncapi.json`. No hub credentials or Telegram login are needed for public market reads.

## Query and watch

A finite request uses `{"type":"request","id":"read.1","method":"orders.query","params":{"query":{"filters":{"collection_id":"tg.gift.123","currency_id":"GRAM"},"source":"asks","sort":"price_asc","position":null,"limit":150,"direction":"around"}}}`. Its result is `{type:"result", id, method:"orders.query", data:{result, sync, revision}}`, matching `POST /orders/read` after removing the request envelope. `source` is `asks`, `bids` or `history`. History accepts an optional execution `period`; active queries reject it. A null position reads the head. Forward/backward reads include the resolved position and stay in their requested direction. Around reads allocate one third of their requested rows before it and fill available space at boundaries. Positions are opaque and bound to filters, source, period and sort.

A watch uses `{"type":"orders.watch","id":"orders.1","params":{"query":{"filters":{"collection_id":"tg.gift.123","currency_id":"GRAM"},"source":"asks","sort":"price_asc","position":null,"limit":150,"direction":"around"}}}`. Initial reading and monitoring are one operation: no change can fall between a separate query and subscription. Each watch owns its coverage and lifetime. To replace coverage, cancel the old operation and start a fresh ID; keep the old visible data until the new snapshot arrives.

## Range coverage

`result` is `{data, range:{start,end}}`. Each row has the public order fields plus `position` and `sort_key`. A boundary is `{position,sort_key}` and is inclusive. Null start/end extends to the open head/tail of this query's ordering. Boundaries are fixed ordering positions, not moving order identities. A watch seeded at the head includes new leading orders. Its row count can grow or shrink; it does not refill or evict rows to maintain the original count. The query limit controls initial slice selection, not ongoing range size.

An explicit `params.range` watches that query-bound coverage from the outset. Its initial result contains the complete matching range within server budgets. The server validates boundary positions and their corresponding keys, and rejects inverted ranges or boundaries from another query. Compare ASCII `sort_key` values lexicographically; do not decode them or duplicate ranking rules. History keys include execution-time tie breaking. Identity remains `(book.venue_id,id)`.

## Atomic state and changes

The first data frame is `{type:"orders.snapshot", id, sequence:0, reset:false, result, sync, revision}`. Subsequent frames are `{type:"orders.changed", id, base_sequence, sequence, upserts, removed, sync, revision}`. Verify `base_sequence` matches the last applied data sequence, then apply the entire batch atomically. Remove identities in `removed`, replace upserts by identity, and sort by `sort_key`. An order repriced outside coverage is removed; an order entering it is upserted. Removal means it left the matching range and does not imply a sale. Removals and upserts within one batch have disjoint identities.

Watches represent current replica state; they may coalesce intermediate observations. They are not an execution-event journal. A dataset replacement emits a new `orders.snapshot` with `reset:true` and the next sequence. Sequences belong to this operation lifetime. On a gap or reconnect, start a fresh watch and replace from its sequence-zero snapshot. There is no durable replay token. `revision` is an opaque equality token, not a replay sequence. A metadata-only `sync` frame updates freshness/revision without advancing the data sequence.

Live history receives newly observed executions, including late backfills whose keys fall within coverage. Frozen history and bulk traversal remain available through HTTP snapshot pagination.

## Frozen and bulk reads

`POST /orders/read` defaults to latest consistency and retains no server view. Set `consistency:"snapshot"` to create a frozen read; its response additionally includes `snapshot`. Send that token on later snapshot reads and use `DELETE /orders/read` with `{snapshot}` when finished. Latest reads reject snapshot tokens. Retained views expire after 60 seconds idle or application restart. Expired/released tokens return HTTP 409 `restart_read`; reopening at the visible position establishes a fresh baseline. Release is idempotent and an in-flight read cannot resurrect a released view.

`POST /orders/query` and `POST /history/query` retain cursor pagination for bulk traversal. Follow `next_page_token` until empty. Their `cursor` is a replica checkpoint, not a navigation or watch replay token. The GUI keeps all visited listings in memory and mounts nearby cards only. Its active range is live; cached regions refresh when revisited. Pausing retains visited rows and refreshes revisited regions against the frozen root.

## Operations and budgets

Operation IDs use 1..64 ASCII letters, digits, `_`, `-` or `.` and are unique while active. `{"type":"cancel","id":"orders.1"}` cancels a request or watch without an acknowledgement. Duplicate IDs produce `command_error` and leave the original operation running. Terminal `error` frames end the referenced operation; an empty ID denotes a connection-level failure. Typed valuation requests use method `valuation.evaluate` and share the HTTP valuation response.

Reads, range membership, encoded messages, concurrent work, outgoing queues and retained views remain bounded. Budget failures are explicit errors, never partial ranges presented as complete results. Narrow coverage or reduce the initial slice on capacity errors. Slow consumers close with code 1013; reconnect and initialize again. Writes run concurrently with command handling, cancellation and supervision. Fresh wire IDs protect replacement operations from late frames. Shutdown cancels operations and drains tracked reads and encoders.

## Catalog updates

`{"type":"watch_catalog","id":"catalog","collection_ids":["tg.gift.123"]}` watches up to eight collections; an empty list watches the directory. Initial subscription and reconnection send complete `catalog` versions. Versions match unquoted ETags from `/catalog` and `/collections/{id}`. Prices and freshness ticks do not invalidate catalog content. Catalog watches share computation independently of order watches. Changed active collections revalidate; inactive collections become stale until needed. Reconnect comparisons recover missed changes without clearing unchanged data. Catalog and transport retries use bounded exponential backoff with jitter.
