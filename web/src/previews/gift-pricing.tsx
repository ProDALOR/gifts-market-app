import type { Order } from "../market";
import type { OrderValuationState } from "../valuation/use-order-valuation";
import { OrderValuationSummary } from "../orderbook/order-valuation";
import { ExecutionTime } from "../orderbook/execution-time";
import { Quote } from "../ui/market-controls";

/** The same exact-price and estimate layout serves market, history and portfolio previews. */
export function GiftPricing({
  order,
  state,
  currency,
  criteria = false,
  removed = false,
}: {
  order?: Order;
  state: OrderValuationState;
  currency: string;
  criteria?: boolean;
  removed?: boolean;
}) {
  const priceLabel =
    order?.kind === "executed"
      ? "Execution price"
      : order?.kind === "bid"
        ? "Bid price"
        : "Asking price";
  return (
    <div className="detail-pricing">
      {order && (
        <section className="listing-price" aria-label={priceLabel}>
          <div className="listing-price-row">
            <div className="listing-price-label">
              <h3>
                {order.kind === "executed"
                  ? "Paid"
                  : order.kind === "bid"
                    ? "Bid price"
                    : "Ask price"}
              </h3>
              {order.kind === "executed" && order.executed_at && (
                <ExecutionTime
                  key={`${state.key}:${order.executed_at}`}
                  at={order.executed_at}
                />
              )}
            </div>
            <div className="detail-price">
              <Quote price={order.price} currency={currency} wrap />
              {criteria && <small className="detail-price-unit">/ gift</small>}
            </div>
          </div>
        </section>
      )}
      <OrderValuationSummary
        state={state}
        criteria={criteria}
        history={order?.kind === "executed"}
        price={!removed && order?.kind === "ask" ? order.price : undefined}
        currency={currency}
      />
    </div>
  );
}
