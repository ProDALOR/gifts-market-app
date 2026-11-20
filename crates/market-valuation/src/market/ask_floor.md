# Ask floor

The target selects a collection, a model within that collection, a backdrop, a model-and-backdrop pair, an individual gift, or accepted gift criteria. A gift target selects that gift's own asks; the separate `gift_floor` strategy estimates its value from collection and trait evidence.

The result is the lowest eligible positive asking price in the selected currency, before fees. Expired, exhausted and unsupported listings are excluded. Trait floors use exact resolved trait choices. Evidence reports listing rows and distinct identifiable gifts; cross-listings count once toward the gift count. Broad offers can contribute prices, but have no identifiable gift.

$$F=\min_i p_i$$

One eligible ask is sufficient. With no matching asks, the price carries `not_enough_data`; missing evidence never becomes zero. Prices retain their original decimal precision. This is an asking price, not a completed sale or guaranteed realizable value.

All requested strategies use the same pinned market snapshot. Cached collection statistics follow relevant data changes, listing expiry and a five-minute safety age.

For accepted criteria, each nonempty model/backdrop/symbol group is an OR selection and groups combine with AND; gift-number bounds further restrict eligible asks. Empty groups are unrestricted. This returns the floor of the complete selection, rather than combining separate trait minima. It is useful for valuing bid conditions. The listing browser's Any/All bid matching mode does not change valuation semantics.

The shared market-values projection batches collection floors and all selected collection model/backdrop floors from these same cached statistics. They describe current asks in every tab, including History; they do not reconstruct historical prices.
