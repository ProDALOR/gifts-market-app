import { useEffect, useState } from "react";
import { amount, valuation } from "../market";
import type { GiftValuation, Order } from "../market";
import { Quote } from "../ui/market-controls";
export function valuationSlug(order: Order, collectionSlug?: string) {
  if (
    order.target.kind !== "gift" ||
    !order.target.gift.number ||
    !collectionSlug
  )
    return;
  return `${collectionSlug}-${order.target.gift.number}`;
}
export const strategyLabel = (id: string) =>
  id.replaceAll("_", " ").replace(/^./, (letter) => letter.toUpperCase());
type Outcome = GiftValuation["strategies"][number];
function StrategyResult({ result }: { result: Outcome }) {
  if ("error" in result)
    return (
      <>
        <strong className="valuation-unavailable">
          {strategyLabel(result.error.code)}
        </strong>
        <p>{result.error.message}</p>
        {result.error.details?.required_samples != null && (
          <small>
            {result.error.details.available_samples ?? 0} /{" "}
            {result.error.details.required_samples} required listings
          </small>
        )}
      </>
    );
  const data = result.data;
  const stale = data.sync.state !== "live";
  return (
    <>
      <strong className="valuation-value">
        {"factor" in data ? (
          `${amount(data.factor)}×`
        ) : data.priced > 0 ? (
          <Quote price={data.total} currency={data.currency_id} />
        ) : (
          "No floor available"
        )}
      </strong>
      {"factor" in data && (
        <p>
          Trait floor ÷ collection floor ·{" "}
          {data.evidence.selected_trait.samples} /{" "}
          {data.evidence.collection.samples} listings
        </p>
      )}
      {!("factor" in data) && data.unpriced > 0 && (
        <p>Not enough market data</p>
      )}
      <small>
        {stale && <span className="valuation-stale">Stale · </span>}Updated{" "}
        <time dateTime={result.evaluated_at}>
          {new Date(result.evaluated_at).toLocaleTimeString([], {
            hour: "2-digit",
            minute: "2-digit",
          })}
        </time>
      </small>
    </>
  );
}
/** One on-demand request returns every registered strategy; no per-card requests or polling. */
export function GiftValuations({
  order,
  collectionSlug,
}: {
  order: Order;
  collectionSlug?: string;
}) {
  const slug = valuationSlug(order, collectionSlug);
  const giftId =
    order.target.kind === "gift" ? order.target.gift.id : undefined;
  const key = JSON.stringify([
    slug,
    giftId,
    order.book.collection_id,
    order.book.currency_id,
  ]);
  const [attempt, setAttempt] = useState(0);
  const [state, setState] = useState<{
    key: string;
    value?: GiftValuation;
    error?: string;
  }>({ key: "" });
  useEffect(() => {
    if (!slug || !giftId) return;
    const stop = new AbortController();
    setState({ key });
    void valuation(
      { gift: slug, currency_id: order.book.currency_id },
      stop.signal,
    )
      .then((value) => {
        if (stop.signal.aborted) return;
        if (
          value.gift.id !== giftId ||
          value.gift.collection_id !== order.book.collection_id ||
          value.currency_id !== order.book.currency_id
        )
          throw new Error(
            "Gift details changed. Refresh this market and try again.",
          );
        setState({ key, value });
      })
      .catch((error) => {
        if (!stop.signal.aborted)
          setState({
            key,
            error:
              error instanceof Error
                ? error.message
                : "Valuations are unavailable",
          });
      });
    return () => stop.abort();
  }, [key, attempt]);
  const current = state.key === key ? state : undefined;
  const loading = !!slug && !current?.value && !current?.error;
  return (
    <section
      className="gift-valuations"
      aria-label="Gift valuations"
      aria-busy={loading}
    >
      <div className="valuation-heading">
        <h3>Valuations</h3>
        {slug && (
          <button
            onClick={() => setAttempt((n) => n + 1)}
            disabled={loading}
            aria-label="Refresh valuations"
          >
            ↻
          </button>
        )}
      </div>
      {!slug ? (
        <p className="valuation-note">
          {order.target.kind === "gift"
            ? "Valuation isn’t available for this gift yet."
            : "Choose a collectible gift to see valuations."}
        </p>
      ) : loading ? (
        <p className="valuation-note" role="status">
          Loading all strategies…
        </p>
      ) : current?.error ? (
        <p className="valuation-note" role="alert">
          {current.error}
        </p>
      ) : (
        <>
          <ul>
            {current?.value?.strategies.map((result) => (
              <li key={result.strategy} data-strategy={result.strategy}>
                <h4>{strategyLabel(result.strategy)}</h4>
                <StrategyResult result={result} />
              </li>
            ))}
          </ul>
          <p className="valuation-note">
            Current asks, before fees. Multipliers are ratios, not prices.
          </p>
        </>
      )}
    </section>
  );
}
