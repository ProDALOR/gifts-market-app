# Catalog ask floors

For one to eight collections in one currency, return the lowest eligible current ask and evidence counts. Optional details include every known model/backdrop/symbol in one selected collection and an optional model/backdrop pair.

Collection summaries prepare only the collection floor and counts, without loading trait choices. Selecting a collection prepares independent model, backdrop and symbol floors. Matching model/backdrop pairs are queried directly from the indexed orderbook only when requested; combinations are never precomputed. Independent values share preparation with ask-floor and gift-floor strategies. Missing asks return `not_enough_data`; a removed selected pair returns `missing_trait`. Amounts retain exact decimal precision, before fees. Collection calculation times remain stable when another collection changes.

Use the same `ValuationQuery` for a finite query or `valuation.watch`. Watches sample relevant data changes and listing expiry once per minute and publish changed results atomically. Availability changes are reported promptly without recalculating prices.

Symbol floors and unique gift counts are collected in the same bounded ask traversal. They are independent reference values and do not change the gift-floor estimate formula.

One cache entry per collection/currency owns the summary, optional trait statistics and one 60-second refresh deadline. There are at most 512 entries and eight retained trait projections. Trait LRU eviction drops only the trait maps, keeping the summary and its deadline. Summary sweeps do not promote trait recency, and trait-bearing entries are protected from summary capacity pressure. Summary-only requests do not prepare new trait maps. Refreshing existing traits also produces the summary in one scan; trait-only metadata changes preserve an unchanged summary and its calculation time. Cached statistics are immutable; report reads capture the entry's deadline and cannot have it changed by later cache hits. Order/catalog changes and listing expiry are checked at the next requested or watched refresh; unchanged values are reused. Cold or evicted trait selections prepare on demand. No background sweep runs for unused collections.

Completed query reports use a separate eight-entry LRU without a TTL. Active watches share through bounded weak entries and retain independent ownership after LRU eviction. Reports are validated at the one-minute freshness deadline before reuse; a cache hit never resets that deadline.
