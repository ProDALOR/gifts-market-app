import { ValuationFields } from "../valuation/display";
import type { GiftValuationState } from "../valuation/use-gift-valuation";
import { ValuationInfo } from "./valuation-info";
import { ValuationStatus } from "./valuation-status";

export function GiftValuationSummary({ state }: { state: GiftValuationState }) {
  const { available, loading, error, report, retry } = state;
  return (
    <section
      className="valuation-summary"
      aria-label="Estimated value"
      aria-busy={loading}
    >
      <div className="valuation-heading">
        <h3>Estimate</h3>
        <div className="valuation-controls">
          <ValuationInfo key={state.key} state={state} />
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
            <ValuationFields fields={state.fields.primary} compact />
            {!state.fields.primary.length && (
              <p className="valuation-note">
                See attribute values below for results.
              </p>
            )}
            <ValuationStatus state={state} />
          </>
        )
      )}
    </section>
  );
}
