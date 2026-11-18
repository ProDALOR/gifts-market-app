# market-portfolio

Shared portfolio application service, implemented as an Actix actor. This crate is not connected to the bot, API, GUI or Telegram transport yet. Existing applications and configuration remain unchanged.

`PortfolioService::start(provider, market_data_handle, config, invocation_handle)` starts an isolated actor thread and returns a `PortfolioHandle` plus a `PortfolioTask`. Run the task under application supervision, or call `task.shutdown().await` to cancel and join it. Dropping the task rejects new requests and requests shutdown. Unexpected actor termination is reported by `task.run(stop)`.

The caller supplies an `InventoryProvider`; `page` builds an **unsubmitted typed invocation for one page**, using the requested user, opaque continuation cursor and limit. The actor submits it to the supplied shared `market-invocations::Handle`. The operation defines rate scopes, retry safety and attempt timeout; it reports flood waits using `AttemptError::retry`. Providers must not paginate, sleep through flood waits or submit to another scheduler internally. `InvocationExt::map` converts transport data/errors while preserving rate and retry metadata. Dropping the operation future must release local work. A transport adapter will be added during a separate integration step; neither this service nor its contracts expose MTProto peers, sessions or credentials. Authorization stays with callers: derive `UserId` from a verified identity before calling the handle.

The `MarketDataHandle` implements `ValuationProvider`. Alternate providers allow deterministic tests without a hub, Telegram connection or real market data. The service deduplicates gifts by ID, groups collectible positions by collection/document identity and delegates model resolution, quote currency checks and exact floor calculation to the existing market actors. Owned, hosted and unverified relations remain explicit in the inventory.

## Queue and rate limits

Requests choose `Interactive`, `Normal` or `Background`. The queue prefers higher priorities with four/two/one work turns per round, preserving FIFO within each class. Every inventory page yields its turn. Coalesced callers promote pending work to the highest requested priority; cancellation removes that caller's priority too.

There is one outstanding page per user and one active valuation. The shared invocation runtime controls execution, pacing and flood waits across every service using its handle. All rate scopes required by a page are acquired together; unrelated ready groups can progress. Cooldowns outlive the request that observed them and survive transport reconnects. Cached valuations continue during waits. Coalesced callers promote or demote the submitted page through its ticket control; active I/O is never preempted.

The admission bound covers queued and active callers, including duplicate requests. Saturation returns `PortfolioError::Busy`. Cancellation removes only the caller's interest; shared work continues for other waiters. An unneeded page fetch is cancelled, while an active valuation retains its slot until it completes or hits its timeout. The queue never preempts an active page to serve a higher priority request.

## Caching and completeness

Complete inventories share a bounded LRU cache by user, independent of quote currency. Defaults are 64 users and 30 seconds. Inventory and valuation snapshots use `Arc`; simultaneous requests for the same user/snapshot/currency share a valuation, but completed valuations are not cached. The next read uses current market state without re-fetching a still-fresh inventory.

Limits default to 100 items per page, 1,000 scanned gifts and 20 pages. The final provider limit shrinks to the remaining budget. Pages exceeding their declared bounds are rejected. Every scanned gift must be accounted for as a collectible, a regular gift or an explicit skip. Missing/repeated continuation cursors, scan/page limits and later read failures produce explicit partial inventories. Partial inventories and provider failures are not cached as complete results. Permission and authorization failures remain errors rather than empty portfolios.

Request and valuation deadlines bound workflow work; invocation policy owns page-attempt deadlines. Request expiry cancels that caller; it never shortens an already observed flood wait. `PortfolioConfig` controls these limits without adding environment variables or application configuration.

## Validation

`CARGO_INCREMENTAL=0 cargo test --locked --offline -p market-portfolio`

The tests cover priorities, fairness, pagination, promotion, shared requests, cancellation, deadlines, flood waits, caching, partial inventories and exact valuation through real market actors. Virtual-clock tests avoid real rate-limit sleeps.
