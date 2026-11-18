import { lazy, Suspense, useEffect, useState } from "react";
import { valuation } from "../market";
import type { ValuationView, Order } from "../market";
import { useStrategy } from "../valuation/use-strategy";
import { guideKey } from "../valuation/strategies";
import { ValuationFields } from "../valuation/display";
const ValuationMethodology = lazy(() => import("./valuation-methodology"));
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
  const [guideOpened, setGuideOpened] = useState(false);
  const [attempt, setAttempt] = useState(0);
  const [state, setState] = useState<{
    key: string;
    value?: ValuationView;
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
        if (!value.display || !Array.isArray(value.display.fields))
          throw new Error(
            "This server did not provide a strategy display. Refresh the app and try again.",
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
  const {
    metadata: strategy,
    error: strategyError,
    retry: retryStrategies,
  } = useStrategy(report?.strategy);
  const loading = !!number && !report && !current?.error;
  return (
    <section
      className="gift-valuations"
      aria-label="Gift valuation"
      aria-busy={loading}
    >
      <div className="valuation-heading">
        <h3>{strategy?.name ?? "Valuation"}</h3>
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
      {strategy && <p className="valuation-note">{strategy.description}</p>}
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
              <ValuationFields fields={report.display.fields} />
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
                {report.display.summary && ` · ${report.display.summary}`}
              </p>
            </>
          )}
          {report?.display.note && (
            <p className="valuation-note">{report.display.note}</p>
          )}
        </>
      )}
      {report && (
        <details
          className="valuation-guide"
          onToggle={(event) => {
            if (event.currentTarget.open) setGuideOpened(true);
          }}
        >
          <summary>How is this calculated?</summary>
          {guideOpened &&
            (strategy ? (
              <Suspense
                fallback={
                  <p className="valuation-note" role="status">
                    Loading calculation guide…
                  </p>
                }
              >
                <ValuationMethodology
                  key={guideKey(strategy)}
                  strategy={strategy}
                />
              </Suspense>
            ) : strategyError ? (
              <div className="valuation-guide-error" role="alert">
                <p>{strategyError}</p>
                <button onClick={retryStrategies}>
                  Retry strategy information
                </button>
              </div>
            ) : (
              <p className="valuation-note" role="status">
                Loading strategy information…
              </p>
            ))}
        </details>
      )}
    </section>
  );
}
