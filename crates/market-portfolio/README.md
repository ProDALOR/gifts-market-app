# market-portfolio

Shared inventory workflow, implemented as an Actix actor. The bot's `/portfolio` command uses this service; API/GUI endpoints are not added. Shared inventory data and skip/partial reasons live in `market-gifts::inventory`.

`PortfolioService::start(inventory_provider, valuation_provider, config, invocation_handle)` starts one actor thread and returns a `PortfolioHandle` and `PortfolioTask`. Run the task under supervision, or stop and join with `task.shutdown().await`. Drop requests shutdown and rejects new work. `task.run(stop)` reports unexpected termination.

An `InventoryProvider` builds one unsubmitted typed page operation for a verified user, opaque cursor and requested page limit. The actor submits it to the supplied `market-invocations::Handle`; providers do not paginate, queue again or sleep through flood waits. Operations carry rate scopes, replay policy, attempt deadlines and generation cancellation. `market-app::portfolio` adapts the bot's authenticated `InventoryClient` without exposing TL types, peer hashes or credentials. Authorization remains with the caller: derive `UserId` from verified identity.

The actor owns pagination, per-user coalescing, caller priorities, cancellation and inventory caching. There is one outstanding page per user. Independent users can overlap according to the transport's shared policy (Telegram defaults: two active users, 300 ms between starts, one active page per user). Coalesced waiters promote/demote queued page priority. Dropping one caller cancels only its interest; the last waiter cancels unused inventory work. Flood waits survive cancellation and connection replacement in the invocation runtime.

Complete inventories share an LRU cache independent of quote currency: 64 users, 30-second TTL. Defaults bound each inventory to 100 items per page, 1,000 scanned items and 20 pages. Every scanned item must be accounted for; IDs deduplicate across pages. Bad/missing/repeated cursors, exhausted limits and later page failures yield explicit partial inventories. Partial inventories are never cached as complete. Authorization and permission failures remain errors, including failures on later pages.

`ValuationProvider` returns a shared `market-valuation::Mark<Valuation>`. The production provider is the generic valuation handle backed by `ModelFloorStrategy`. The actor groups gifts by collection/model document identity, preserving exact quantities and explicit owned/hosted/unverified relations. Market revisions invalidate marks without invalidating inventory; a subsequent request can revalue cached gifts without Telegram calls. Currency stays explicit and unknown/unpriced models stay outside the estimate. The portfolio actor does not implement or periodically scan valuation state.

Workflow deadlines default to 60 seconds; the bot's 40-second command deadline may cancel its caller earlier. Page attempts use invocation policy, and valuation reads have a 10-second workflow timeout. Admission is bounded to 128 callers including duplicates. Inventory and marks use `Arc`; simultaneous reads for the same user/snapshot/currency share their valuation. An active valuation retains its slot until completion/timeout even if callers leave.

Run regression tests: `CARGO_INCREMENTAL=0 cargo test --locked --offline -p market-portfolio`.
