# Observed execution prices

Select an explicit half-open period: `from` is inclusive and `until` is exclusive. Both bounds are required, and the period cannot end in the future. Targets may select a collection, model, backdrop, model-and-backdrop pair, or individual gift. Individual-gift identity resolves from observed asks or execution history. When an authorized Telegram user-session resolver is configured, URL/slug queries can also resolve unlisted gifts; Telegram facts are mapped to catalog traits and existing market identities before evaluation. Collection and trait targets do not require a listed individual gift. All prices are per gift in the requested currency. No currency conversion is performed.

Only supported positive confirmed execution records matching the target and period contribute. Disappearing asks are never treated as executions. The result describes history available in this service; the requested period does not guarantee complete market coverage.

## Arithmetic mean

`execution_mean` weights every execution record equally, independently of quantity. This is deliberately distinct from a quantity-weighted mean.

$$\bar p=\frac{\sum_{i=1}^{n}p_i}{n}$$

## Median

`execution_median` sorts the observed prices and takes the middle value. With an even count it averages the two central prices.

Both estimators require at least three execution records. Collection and trait targets require three distinct identifiable gifts; an individual-gift target uses that one gift's own execution history and requires one distinct gift. A repeated sale or cross-venue record of the same gift does not increase the distinct-gift count. Without that evidence, the price carries `not_enough_data`. Counts do not establish independent sellers, and records without a common transaction identity are not asserted to be independent trades.

Evidence includes observed execution count, distinct gifts, venues, the first and last eligible execution times and the sample requirements. Typed prices use checked decimal arithmetic without a fixed two-decimal reporting round. The strategy's display projection rounds execution estimates to two decimal places using midpoint-to-even. Price histories are bounded to 50,000 candidate observations per request; shorten the period if that budget is exceeded. A failed history read does not discard successfully computed ask-based strategies.

Mean and median reuse one history traversal within a report and one pinned snapshot with the other requested strategies. Neither estimator is a guarantee of a future sale price.
