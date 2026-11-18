# market-invocations

A Tokio scheduler for typed, caller-owned I/O. It knows no Telegram, gift, HTTP or actor types. Start one `Runtime` in application composition, share its `Handle`, and shut it down after service consumers stop. Runtime drop also closes admission and cancels consumers. A scheduler failure cancels its handles rather than leaving callers waiting indefinitely.

`Invocation` declares output/error types, rate policy and one repeatable attempt. `Handle::submit` reserves bounded admission immediately and returns a lazy `Ticket` future. Poll/await that ticket to queue and execute it; dropping it cancels local execution. Tickets never spawn detached request tasks. `Control::set_priority` updates queued priority without recreating the operation. `InvocationExt::map` preserves policy/retry hints when mapping transport results into application types.

The admission semaphore covers queued, active and unpolled tickets. The request mailbox, execution slots and rate-state table have independent hard bounds. Priorities get four/two/one turns per round; blocked work yields to unrelated ready scopes. All scopes are acquired atomically. Keys include account owner, group and optional resource; services sharing a rate must use identical scope policy. Live cooldowns are never evicted to admit new keys. Table saturation returns `Busy`.

`Replay::Safe` covers reads/idempotent calls, `Deduplicated` requires a stable mutation identifier, and `Never` executes only once. The transport classifies failures with `Retry::Never`, `Transient` or `After { scope, delay }`. Transient retry counts are bounded; flood retries wait in the queue until the caller deadline. Full server waits are recorded even when the operation cannot be replayed and survive cancellation and generation replacement. A deadline reports remaining rate wait where available; it never clears that wait. Pacing is measured between attempt starts; concurrency counts admitted attempts until completion or cancellation.

The scheduler owns admission and retry timing. Callers own futures, transport connections and workflow deadlines. Cancellation cannot undo a request already delivered to a remote server: any replayable mutation must deduplicate remotely. Shutdown stops scheduling and wakes consumer futures; service supervision is responsible for polling/dropping those futures before draining its transport.

Run deterministic tests with `CARGO_INCREMENTAL=0 cargo test --locked --offline -p market-invocations`.
