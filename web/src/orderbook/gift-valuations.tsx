import { lazy, Suspense, useState } from "react";
import { useStrategy } from "../valuation/use-strategy";
import { guideKey } from "../valuation/strategies";
import { ValuationFields } from "../valuation/display";
import type { GiftValuationState } from "../valuation/use-gift-valuation";
const ValuationMethodology = lazy(() => import("./valuation-methodology"));

export function GiftValuationSummary({ state }: { state: GiftValuationState }) {
  const { available, loading, error, report, retry } = state;
  return (
    <section
      className="valuation-summary"
      aria-label="Estimated value"
      aria-busy={loading}
    >
      <div className="valuation-heading">
        <h3>Estimated value</h3>
        {available && (
          <button
            onClick={retry}
            disabled={loading}
            aria-label="Refresh valuation"
          >
            ↻
          </button>
        )}
      </div>
      {!available ? (
        <p className="valuation-note">
          Choose a numbered collectible to see an estimate.
        </p>
      ) : loading && !report ? (
        <p className="valuation-note" role="status">
          Calculating estimate…
        </p>
      ) : error && !report ? (
        <p className="valuation-note" role="alert">
          {error}
        </p>
      ) : (
        report && (
          <>
            <ValuationFields fields={state.fields.primary} />
            {!state.fields.primary.length && (
              <p className="valuation-note">
                See calculation breakdown for results.
              </p>
            )}
            {loading && (
              <p className="valuation-note" role="status">
                Refreshing estimate…
              </p>
            )}
            {error && (
              <div className="valuation-note" role="alert">
                <p>{error}</p>
                <p>Previous estimate shown.</p>
              </div>
            )}
            <p className="valuation-note">
              {report.sync.state !== "live" && (
                <span className="valuation-stale">Stale data · </span>
              )}
              Updated{" "}
              <time dateTime={report.evaluated_at}>
                {new Date(report.evaluated_at).toLocaleTimeString([], {
                  hour: "2-digit",
                  minute: "2-digit",
                })}
              </time>
            </p>
            {report.display.note && (
              <p className="valuation-note">{report.display.note}</p>
            )}
          </>
        )
      )}
    </section>
  );
}

/** Secondary fields stay collapsed; documentation is fetched only on expansion. */
export function GiftValuations({ state }: { state: GiftValuationState }) {
  const report = state.report;
  const [guideOpen, setGuideOpen] = useState(false);
  const {
    metadata: strategy,
    error: strategyError,
    retry: retryStrategies,
  } = useStrategy(report?.strategy);
  if (!report) return null;
  return (
    <section className="gift-valuations" aria-label="Valuation calculation">
      <div className="valuation-strategy">
        <h3>{strategy?.name ?? "Valuation"}</h3>
      </div>
      <details className="valuation-breakdown">
        <summary>Calculation breakdown</summary>
        {strategy && <p className="valuation-note">{strategy.description}</p>}
        <ValuationFields fields={state.fields.secondary} />
        {report.display.summary && (
          <p className="valuation-note">{report.display.summary}</p>
        )}
      </details>
      <details
        className="valuation-guide"
        onToggle={(event) => {
          setGuideOpen(event.currentTarget.open);
        }}
      >
        <summary>How is this calculated?</summary>
        {guideOpen &&
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
    </section>
  );
}
