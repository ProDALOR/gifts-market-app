# Gift floor valuation

This strategy estimates a gift from available market asking prices. All prices use the currency selected in the market. Listings from all available venues are considered.

## The four floors

A floor is the lowest eligible asking price for a selection:

- **Collection floor:** any supported listing in the gift's collection.
- **Model floor:** listings with the gift's exact model.
- **Backdrop floor:** listings with the gift's exact backdrop.
- **Model × backdrop floor:** listings matching both the gift's model and backdrop. This is one asking price, not the product of two prices.

An eligible ask has a positive price and remaining quantity, has not expired, and has a supported gift target. Unsupported targets do not supply floors. A model or backdrop must resolve to a catalog choice. Broad criteria can contribute to the collection floor; trait floors require a single exact choice for that trait.

## Multipliers

The multipliers compare each trait's floor with the collection floor:

Let $F_C$ be the collection floor, $F_M$ the model floor, $F_B$ the backdrop floor, and $F_{MB}$ the matching model-and-backdrop floor. The multipliers are $m$ and $b$:

$$
m = \frac{F_M}{F_C}, \qquad b = \frac{F_B}{F_C}
$$

They are dimensionless: a model multiplier of 1.50× means its floor is 1.5 times the collection floor.

Each multiplier requires at least **3 distinct identifiable gifts** in the collection and at least **3** matching its selected trait. The collection floor must be positive. The matching model-and-backdrop floor needs at least one eligible matching listing.

Evidence reports both eligible listing rows and distinct gifts for the collection, model, backdrop and pair. The same gift listed on multiple venues counts once toward the distinct-gift requirement. All its eligible asks still contribute to the floor price. Broad criteria without a concrete identity can supply price evidence, but do not count as identifiable gifts. Distinct gifts do not imply independent sellers or completed sales.

## Final mark

The estimate is $E$, and the final mark is $V$:

$$
\begin{aligned}
E &= F_C \cdot m \cdot b \\
V &= \operatorname{round}_2\!\left(\min(F_{MB}, E)\right)
\end{aligned}
$$

The estimate can also be written as:

$$
E = \frac{F_M \cdot F_B}{F_C}
$$

The lower of the matching floor and the estimate wins. Both multipliers and the matching floor must be available; a missing input does not produce a fallback mark.

This is a **floor-based heuristic**. Holding the model and backdrop floors fixed, lowering the collection floor increases the multiplier estimate. For example, changing the collection floor from 3 to 1 changes an estimate based on model 10 and backdrop 11 from 36.67 to 110, before applying the matching-floor cap. A cheap baseline listing can therefore materially affect the result. This formula has not been calibrated against historical executions; no statistical confidence or expected sale price is implied. Alternative estimators should be compared against executions as separately named strategies before replacing this formula.

## Rounding

Calculations use the available decimal precision, without substituting the rounded multipliers shown in the app. Only displayed multipliers and the final mark are rounded to **2 decimal places**. The four floor prices retain their original precision.

Rounding uses **midpoint-to-even**: an exact halfway value rounds toward the nearest even last digit. For example, 20.125 becomes 20.12, while 20.135 becomes 20.14. The minimum is chosen before rounding.

## Worked example

Assume all sample requirements are met, with these floors in the selected currency:

- Collection: **3**
- Model: **10**
- Backdrop: **11**
- Matching model and backdrop: **100**

```text
model multiplier = 10 / 3 → displayed as 3.33×
backdrop multiplier = 11 / 3 → displayed as 3.67×

estimate = 10 × 11 / 3 = 36.666…
final mark = round2(min(100, 36.666…)) = 36.67
```

Using the displayed multipliers would give a different result. If the matching floor instead were 20.125, it would cap the estimate and the final mark would be **20.12**.

## When values are unavailable

A metric shows an explanation rather than treating missing data as zero:

- **Not enough data:** no matching asks or too few distinct identifiable gifts.
- **Missing trait:** the gift's model or backdrop is absent or cannot resolve to a catalog choice.
- **Invalid baseline:** the collection floor cannot serve as a positive denominator.
- **Calculation overflow:** a required value cannot be represented safely.

The final mark preserves the original issue, evidence and a typed `dependency` identifying the failed component, such as `backdrop_multiplier` or `model_backdrop_floor`.

Gift identity must be present in available market listings. Gifts missing from that data cannot be resolved for valuation. Malformed source data can make the request fail rather than supply a price.

## Reading the result

The calculation time shows when the estimate was computed. Cached reports and collection/currency floor statistics are held for at least 60 seconds. Related order/catalog changes and expired asks are combined until the next requested or watched refresh; unchanged statistics are reused without a full scan. Cached quotes can therefore include listings that expired during that minute. Availability changes update the status flag promptly without resetting the price refresh deadline. Preparation contains independent collection/model/backdrop/symbol values only. A new gift or selection calculates on demand using its requested indexed orderbook observations and cached floor baselines; other combinations are not prepared. A stale indicator describes interrupted market updates, rather than the normal one-minute cache interval. Calculation time does not indicate when every underlying listing changed.
