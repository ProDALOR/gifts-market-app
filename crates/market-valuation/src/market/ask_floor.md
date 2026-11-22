# Ask floor

A `gift_attributes` target supplies known catalog traits without a listed gift identity; its optional number narrows the query to that exact collection/number. Without a number, asks match all supplied traits. A missing exact ask returns a metric issue without blocking another strategy’s attribute-based estimate.

The target selects a collection, a model within that collection, a backdrop, a model-and-backdrop pair, an individual gift, or accepted gift criteria. A gift target selects that gift's own asks; the separate `gift_floor` strategy estimates its value from collection and trait evidence.

The result is the lowest eligible positive asking price in the selected currency, before fees. Expired, exhausted and unsupported listings are excluded. Trait floors use exact resolved trait choices. The result is flat: `{value:"10.8",count:42}` or `{error:{code:"not_enough_data",message:"..."},count:0}`. `count` counts distinct identifiable gifts; cross-listings count once. No listing count or separate evidence object is returned. Broad offers can contribute prices, but have no identifiable gift.

$$F=\min_i p_i$$

One eligible ask is sufficient. With no matching asks, the price carries `not_enough_data`; missing price data never becomes a zero price. Prices retain their original decimal precision. This is an asking price, not a completed sale or guaranteed realizable value.

Fresh reads use one pinned market snapshot. Cached reports and collection/currency floor statistics are held for at least 60 seconds, so changed or expired asks can remain in the displayed quote until the next refresh. At that boundary, relevant dependencies and expiry are checked, including expiry in narrow indexed selections; unchanged data is reused without another scan. Up to eight completed reports are retained in an LRU without a TTL, so polling can validate the previous result instead of starting cold. Retention never extends the one-minute freshness deadline. Availability changes update the status flag independently. New selections calculate on demand and can use cached floor baselines. Collection-only requests do not load new trait choices; already prepared traits refresh together with their summary. Model and backdrop floors are independent, while combinations use a bounded indexed orderbook read on demand.

For accepted criteria, each nonempty model/backdrop/symbol group is an OR selection and groups combine with AND; gift-number bounds further restrict eligible asks. Empty groups are unrestricted. This returns the floor of the complete selection, rather than combining separate trait minima. It is useful for valuing bid conditions. The listing browser's Any/All bid matching mode does not change valuation semantics.

The shared market-values projection batches collection floors and all selected collection model/backdrop floors from these same cached statistics. They describe current asks in every tab, including History; they do not reconstruct historical prices.

Discovery classifies this strategy as `purpose:"reference"`. Bid previews use it for their matching conditions; gift previews and bot reports show estimate strategies instead. Explicit API queries can still request `ask_floor` for any supported target.
