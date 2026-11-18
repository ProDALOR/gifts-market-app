# market-valuation

A bounded message-driven valuation actor over an injected `Strategy`. This crate knows no gifts, Telegram, market storage, HTTP or currencies. Strategies define a normalized key, a typed value (including its unit/currency), typed failures, a coalescing source-change signal and an evaluation against a consistent source snapshot.

`Service::start(strategy, config)` returns a clonable `Handle` and an owned `Task`. `handle.mark(key).await` returns an `Arc<Mark<Value>>` containing the value, revision, evaluation timestamp and priced/unpriced coverage. Requests for the same normalized key share one evaluation and one immutable mark. Source errors, saturation, timeouts and shutdown remain explicit errors.

One evaluation runs at a time. Pending requests and retained keys are each bounded to 128 by default; cache admission evicts the least recently requested inactive key. Idle keys expire after five minutes; background refresh never extends their lifetime. Market changes invalidate all retained keys with a 100 ms coalescing window. A 30-second reconciliation interval handles time-dependent changes even without a source event. Only requested keys are evaluated, with waiting callers ahead of background refresh. Results crossing an invalidation are discarded and re-evaluated, preventing an old in-flight result from repopulating a current cache. Failed evaluations are not cached or replaced with old marks. Attempts time out after 10 seconds and callers after 30 seconds.

Cancellation releases that caller's interest. Queued work with no callers or previous mark is removed. An active evaluation retains its execution slot until it completes or times out, since a source may continue underlying work after its result is dropped. Dropping `Task` closes admission and cancels local work. Run `task.run(stop)` under application supervision or explicitly join it with `task.shutdown().await`.

The first concrete strategy is `market_actors::valuation::ModelFloorStrategy`. It groups equivalent positions, validates service limits and obtains exact model-floor valuations through `MarketDataHandle`. The strategy uses the replica's change signal through the actor facade, while the source revision and all valuation rows come from the same pinned view. The existing floor policy (positive current asks, no currency conversion or fallback, explicit unpriced positions) remains intact.

Run deterministic tests: `CARGO_INCREMENTAL=0 cargo test --locked --offline -p market-valuation`.
