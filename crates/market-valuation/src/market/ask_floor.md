# Ask floor

The target selects a collection, a model within that collection, a backdrop, a model-and-backdrop pair, or an individual gift. A gift target selects that gift's own asks; the separate `gift_floor` strategy estimates its value from collection and trait evidence.

The result is the lowest eligible positive asking price in the selected currency, before fees. Expired, exhausted and unsupported listings are excluded. Trait floors use exact resolved trait choices. Evidence reports listing rows and distinct identifiable gifts; cross-listings count once toward the gift count. Broad offers can contribute prices, but have no identifiable gift.

$$F=\min_i p_i$$

One eligible ask is sufficient. With no matching asks, the price carries `not_enough_data`; missing evidence never becomes zero. Prices retain their original decimal precision. This is an asking price, not a completed sale or guaranteed realizable value.

All requested strategies use the same pinned market snapshot. Cached collection statistics follow relevant data changes, listing expiry and a five-minute safety age.
