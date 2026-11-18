import { useEffect, useState } from "react";
import { amount, valuation } from "../market";
import type { GiftValuation, Order } from "../market";
import { Quote } from "../ui/market-controls";
const issueLabel = (id: string) =>
  id.replaceAll("_", " ").replace(/^./, (letter) => letter.toUpperCase());
type Metric = GiftValuation["collection_floor"];
const metrics = [
  ["collection_floor", "Collection floor"],
  ["model_floor", "Model floor"],
  ["backdrop_floor", "Backdrop floor"],
  ["model_backdrop_floor", "Model × backdrop floor"],
  ["model_multiplier", "Model multiplier"],
  ["backdrop_multiplier", "Backdrop multiplier"],
] as const;
function twoDecimals(value: string) {
  const [whole, fraction = ""] = value.split(".");
  return `${whole}.${fraction.padEnd(2, "0")}`;
}
function MetricResult({
  metric,
  currency,
  ratio = false,
  rounded = false,
}: {
  metric: Metric;
  currency: string;
  ratio?: boolean;
  rounded?: boolean;
}) {
  if ("error" in metric)
    return (
      <>
        <strong className="valuation-unavailable">
          {issueLabel(metric.error.code)}
        </strong>
        <p>{metric.error.message}</p>
        {metric.error.required_samples != null && (
          <small>
            {metric.error.available_samples ?? 0} /{" "}
            {metric.error.required_samples} required listings
          </small>
        )}
      </>
    );
  return (
    <strong className="valuation-value">
      {ratio ? (
        `${amount(twoDecimals(metric.value))}×`
      ) : (
        <Quote
          price={rounded ? twoDecimals(metric.value) : metric.value}
          currency={currency}
        />
      )}
    </strong>
  );
}
/** One on-demand request returns the unified report; no per-card requests or polling. */
export function GiftValuations({ order }: { order: Order }) {
  const gift = order.target.kind === "gift" ? order.target.gift : undefined;
  const number = gift?.number;
  const giftId = gift?.id;
  const key = JSON.stringify([
    number,
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
    if (!number || !giftId) return;
    const stop = new AbortController();
    setState({ key });
    void valuation(
      {
        gift: { collection_id: order.book.collection_id, number },
        currency_id: order.book.currency_id,
      },
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
  const report = current?.value;
  const loading = !!number && !report && !current?.error;
  return (
    <section
      className="gift-valuations"
      aria-label="Gift valuation"
      aria-busy={loading}
    >
      <div className="valuation-heading">
        <h3>Valuation</h3>
        {number && (
          <button
            onClick={() => setAttempt((n) => n + 1)}
            disabled={loading}
            aria-label="Refresh valuation"
          >
            ↻
          </button>
        )}
      </div>
      {!number ? (
        <p className="valuation-note">
          {order.target.kind === "gift"
            ? "This listing has no gift number; valuation is unavailable."
            : "Choose a collectible gift to see valuations."}
        </p>
      ) : loading ? (
        <p className="valuation-note" role="status">
          Calculating valuation…
        </p>
      ) : current?.error ? (
        <p className="valuation-note" role="alert">
          {current.error}
        </p>
      ) : (
        <>
          {report && (
            <>
              <div className="valuation-final" data-metric="final_mark">
                <h4>Final mark</h4>
                <MetricResult
                  metric={report.final_mark}
                  currency={report.currency_id}
                  rounded
                />
              </div>
              <ul>
                {metrics.map(([key, label]) => (
                  <li key={key} data-metric={key}>
                    <h4>{label}</h4>
                    <MetricResult
                      metric={report[key]}
                      currency={report.currency_id}
                      ratio={key.endsWith("multiplier")}
                    />
                  </li>
                ))}
              </ul>
              <p className="valuation-note">
                {report.sync.state !== "live" && (
                  <span className="valuation-stale">Stale · </span>
                )}
                Updated{" "}
                <time dateTime={report.evaluated_at}>
                  {new Date(report.evaluated_at).toLocaleTimeString([], {
                    hour: "2-digit",
                    minute: "2-digit",
                  })}
                </time>
                {` · ${report.evidence.collection_samples} collection listings`}
              </p>
            </>
          )}
          <p className="valuation-note">
            Min(combined floor, collection floor × unrounded multipliers),
            rounded to 2 decimals. Current asks, before fees.
          </p>
        </>
      )}
    </section>
  );
}
