# Best bid

The highest positive, unexpired bid with remaining quantity that accepts the target, before fees. A quote is an observed order, not a guaranteed execution: venue eligibility, custody, fees and changes after the snapshot still apply.

For a gift, all model, backdrop, symbol and number conditions must match. Unknown attributes never satisfy restrictions. Concrete-gift bids require the same collectible identity or an exact collection/number lookup.

For a selection, a bid must accept **every** permitted combination. Unselected dimensions are unrestricted. A model + backdrop bid therefore cannot become a model-only reference. Broad collection bids may supply any narrower reference.

Gift results include independent collection, model, backdrop, model + backdrop and symbol references. They share one bounded descending-price book read, with cancellation and the existing valuation cache/refresh policy. Missing bids return `not_enough_data`, never zero. No trait combination table is prepared. Bid prices do not change the gift estimate formula.
