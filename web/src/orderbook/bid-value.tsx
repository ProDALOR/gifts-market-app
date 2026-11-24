import type { components } from "../gen/api";
import { Quote } from "../ui/market-controls";
import { formatDecimal, trimDecimalZeros } from "../amount";

type BidQuote = components["schemas"]["BidQuote"];
/** Bid quantity is order capacity, never the distinct-gift count shown for asks. */
export function BidValue({
  bid,
  currency,
  label,
  names,
  stale,
  loading,
  error,
}: {
  bid?: BidQuote | null;
  currency: string;
  label: string;
  names: ReadonlyMap<string, string>;
  stale?: boolean;
  loading?: boolean;
  error?: string;
}) {
  const value = bid && "value" in bid ? bid.value : undefined;
  const order = bid?.order;
  const venue = order ? names.get(order.venue_id) || order.venue_id : "";
  const title = value
    ? `${label}: ${value} ${currency}${venue ? ` · ${venue}` : ""}${order ? ` · ${order.quantity} available` : ""}${stale ? " · updates pending" : ""}`
    : bid && "error" in bid
      ? bid.error.message
      : error;
  return (
    <span
      className={`floor-value bid-value${stale ? " stale" : ""}`}
      title={title}
      aria-label={
        title ||
        (loading ? `Loading ${label.toLowerCase()}` : `${label} unavailable`)
      }
    >
      {value ? (
        <span className="floor-badge">
          <Quote
            price={trimDecimalZeros(formatDecimal(value, 2))}
            currency={currency}
          />
        </span>
      ) : loading && !bid ? (
        <span className="floor-loading" role="status">
          <span />
          <span />
        </span>
      ) : (
        <span
          className={error ? "floor-load-error" : "attribute-floor-missing"}
        >
          {error ? "!" : "—"}
        </span>
      )}
    </span>
  );
}
