# Catalog ask floors

For one to eight collections in one currency, return the lowest eligible current ask and evidence counts. Optional details include every known model/backdrop/symbol in one selected collection and an optional model/backdrop pair.

Prices and evidence share the same cached statistics as ask-floor and gift-floor strategies. Missing asks return `not_enough_data`; a removed selected pair returns `missing_trait`. Amounts retain exact decimal precision, before fees. Collection calculation times remain stable when another collection changes.

Use the same `ValuationQuery` for a finite query or `valuation.watch`. Watches update atomically on relevant data changes, availability changes and listing expiry.

Symbol floors and unique gift counts are collected in the same bounded ask traversal. They are independent reference values and do not change the gift-floor estimate formula.
