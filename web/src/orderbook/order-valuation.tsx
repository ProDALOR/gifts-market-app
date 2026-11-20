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
          {criteria ? "Ask floor" : history ? "Current estimate" : "Estimate"}
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
      {state.strategies.length > 1 && (
        <label className="valuation-strategy-picker">
          <span className="sr">Valuation strategy</span>
          <select
            aria-label="Valuation strategy"
            value={state.strategy?.id}
            onChange={(event) => state.selectStrategy(event.target.value)}
          >
            {state.strategies.map((row) => (
              <option key={row.id} value={row.id}>
                {row.name}
              </option>
            ))}
          </select>
        </label>
      )}
      {!available ? (
        <p className="valuation-note">Estimate unavailable</p>
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
            <ValuationFields fields={state.primaryFields} compact />
            {!state.primaryFields.length && (
              <p className="valuation-note">See calculation details</p>
            )}
            <ValuationStatus state={state} />
          </>
        )
      )}
    </section>
  );
}
