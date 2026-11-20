import { ValuationFields } from "../valuation/display";
import type { OrderValuationState } from "../valuation/use-order-valuation";
import { ValuationInfo } from "./valuation-info";
import { ValuationStatus } from "./valuation-status";

export function OrderValuationSummary({
  state,
  criteria = false,
  history = false,
}: {
  state: OrderValuationState;
  criteria?: boolean;
  history?: boolean;
}) {
  const { available, loading, error, report, retry } = state;
  return (
    <section
      className={criteria ? "matching-floor" : "valuation-summary"}
      aria-label={criteria ? "Matching current ask floor" : "Estimated value"}
      aria-busy={loading}
    >
      <div className="valuation-heading">
        <h3>
          {criteria
            ? "Matching current ask floor"
            : history
              ? "Current estimate"
              : "Estimate"}
        </h3>
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
