# Gift floor valuation

This strategy estimates a gift from current asking prices in the selected currency, across available venues. It adds model and backdrop premiums to the collection floor. A matching model-and-backdrop floor caps the estimate when available. Symbol floors are shown independently and do not enter this formula.

## Floors and eligibility

- **Collection floor** $F_C$: lowest eligible ask in the collection.
- **Model floor** $F_M$: lowest eligible ask with the gift's exact model.
- **Backdrop floor** $F_B$: lowest eligible ask with its exact backdrop.
- **Model & backdrop floor** $F_{MB}$: lowest eligible ask matching both traits. This is a single price, not a product.

Eligible asks have positive prices and remaining quantity, are not expired at the observation time, and have supported gift targets. Trait IDs must resolve in the collection catalog. Broad criteria may supply prices; trait floors require a single exact choice for each selected trait. Unsupported targets, bids, history and other currencies do not supply these floors.

Collection, trait and pair floors use the **same immutable ask partitions and eligibility time** for that collection/currency. Only selected active ask partitions are retained; metadata, history and unrelated orderbooks are not pinned. Pair floors are calculated on demand using the existing attribute indexes, without preparing a matrix of combinations.

## Additive premiums

The model and backdrop premiums are nonnegative amounts above the collection floor:

$$
\begin{aligned}
P_M &= \max(0, F_M - F_C) \\
P_B &= \max(0, F_B - F_C) \\
E &= F_C + P_M + P_B
\end{aligned}
$$

For coherent floors, each selected trait is a subset of the collection, so $F_M,F_B\ge F_C$ and $E=F_M+F_B-F_C$. Clamping premiums to zero keeps the definition explicit and nonnegative.

The final mark is:

$$
V = \begin{cases}
\operatorname{round}_2(\min(F_{MB},E)), & \text{matching pair asks exist} \\
\operatorname{round}_2(E), & \text{no matching pair asks}
\end{cases}
$$

`basis` is `pair_capped` when pair evidence is available, even if the cap does not reduce the estimate, or `additive` when there is no pair floor. It is omitted when required floors are unavailable. The unavailable pair metric and its zero count remain visible alongside an uncapped additive estimate. Read failures, cancellation and capacity errors are not treated as absent pair data.

Collection, model and backdrop floors are required. Missing individual floors or unresolved traits produce explicit metric errors rather than zero premiums. The collection baseline must be positive.

## Explanatory multipliers and counts

$$
m=\frac{F_M}{F_C}, \qquad b=\frac{F_B}{F_C}
$$

Multipliers are display-only ratios; neither their values nor their availability controls the estimate. There is no minimum distinct-gift gate for floor arithmetic. Each floor contains its exact `value` or typed `error` beside `count`, the number of distinct identifiable gifts. There are no separate evidence objects or listing counts. The count does not claim statistical confidence. The same gift listed on several venues counts once. Unidentified criteria can supply a floor without increasing the distinct-gift count. Distinct gifts do not imply independent sellers or completed sales.

Symbol floors stay separate: a model/backdrop pair can have another symbol, so its floor cannot safely cap a symbol-inclusive estimate. Gift numbers and rarity premiums are not inferred.

## Examples

With collection **3**, model **10**, backdrop **11**:

```text
model premium = 10 - 3 = 7
backdrop premium = 11 - 3 = 8
additive estimate = 3 + 7 + 8 = 18

pair floor 100: final mark 18.00, basis pair_capped
pair floor 15.125: final mark 15.12, basis pair_capped
no pair floor: final mark 18.00, basis additive
```

If only the collection floor falls to **1**, the additive estimate becomes **20**, rather than the former multiplicative estimate of **110**. The result still responds to floor movements; it does not eliminate anomalous listings or guarantee a sale price. Accuracy has not been calibrated against historical executions.

## Precision and errors

Floors, premiums and `additive_estimate` retain available decimal precision. Only displayed multipliers and `final_mark` round to two decimals, using midpoint-to-even: 15.125 becomes 15.12; 15.135 becomes 15.14. The pair cap is applied before rounding.

Missing traits, absent required asks, invalid baselines and arithmetic overflow produce typed metric issues. Required-floor errors keep their original code and identify the failed floor in `dependency`. A multiplier overflow cannot block a valid additive estimate. If a nonnegative additive sum overflows but a finite pair floor exists, that floor safely caps the mark; without a pair cap, overflow remains explicit.

## Refresh and availability

Collection/currency observations refresh at most once per minute, on requested or watched work. Until then, collection, model, backdrop, symbol and newly requested pair floors all retain the previous observation generation, including its expiry cutoff. Changes, expirations and reconnect triggers are combined until the next refresh. Unchanged data can reuse the same observations without a rescan. The bounded trait cache retains shared partition/index references, not copied orders or a full replica snapshot; eviction releases them.

The report's calculation time is distinct from the collection observation time. A stale flag describes interrupted market updates, not the normal one-minute sampling interval. Availability changes do not reset price refresh deadlines. Direct `gift_attributes` input needs only a known collection and its supplied catalog traits; no listed identity or gift number is required. URL/number references resolve their facts from current listings or recorded executions. Unknown reference facts require a direct attribute input; attributes are never invented from a URL. Malformed source data can fail the request.
