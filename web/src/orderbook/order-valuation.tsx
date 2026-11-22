import { ValuationFields } from "../valuation/display";
import type {
  OrderValuationState,
  OrderValuation,
} from "../valuation/use-order-valuation";
import { priceComparison } from "../valuation/price-comparison";
import { ValuationInfo } from "./valuation-info";
import { ValuationStatus } from "./valuation-status";

export function OrderValuationSummary({
  state,
  criteria = false,
  history = false,
  price,
  currency,
}: {
  state: OrderValuationState;
  criteria?: boolean;
  history?: boolean;
  price?: string;
  currency?: string;
}) {
  const { available, loading, error, report, valuations } = state;
  return (
    <section
      className={criteria ? "matching-floor" : "valuation-summary"}
      aria-label={
        criteria
          ? "Matching current ask floor"
          : history
            ? "Current estimates"
            : "Estimated value"
      }
      aria-busy={loading}
    >
      {history && report && (
        <h3 className="valuation-context">Current estimates</h3>
      )}
      {!report ? (
        <div className="valuation-result-row">
          <p
            className="valuation-note"
            role={error ? "alert" : loading ? "status" : undefined}
          >
            {!available ? "Unavailable" : error || "Calculating…"}
          </p>
        </div>
      ) : (
        <>
          <div className="valuation-strategies">
            {valuations.map((valuation) => (
              <StrategyRow
                key={`${state.key}:${valuation.strategy.id}`}
                state={state}
                valuation={valuation}
                multiple={valuations.length > 1}
                price={price}
                currency={currency}
              />
            ))}
          </div>
          <ValuationStatus state={state} />
        </>
      )}
    </section>
  );
}
function StrategyRow({
  state,
  valuation,
  multiple,
  price,
  currency,
}: {
  state: OrderValuationState;
  valuation: OrderValuation;
  multiple: boolean;
  price?: string;
  currency?: string;
}) {
  const { strategy, primaryFields, stale } = valuation;
  const primary =
    primaryFields.length === 1 ? primaryFields[0].value : undefined;
  const comparison =
    price &&
    !state.error &&
    !stale &&
    primary?.kind === "currency" &&
    primary.currency_id === currency
      ? priceComparison(price, primary.amount)
      : undefined;
  return (
    <section
      className="valuation-strategy-row"
      data-strategy={strategy.id}
      aria-label={strategy.name}
    >
      <div className="valuation-result-row">
        <div className="valuation-heading">
          <h4>{strategy.name}</h4>
          <ValuationInfo
            state={state}
            valuation={valuation}
            label={multiple ? `How is ${strategy.name} calculated?` : undefined}
          />
        </div>
        <div className="valuation-primary-result">
          <ValuationFields fields={primaryFields} compact />
          {!primaryFields.length && (
            <p className="valuation-note">See calculation details</p>
          )}
          {comparison && (
            <span
              className="price-comparison"
              title={comparison.description}
              aria-label={comparison.description}
            >
              {comparison.label}
            </span>
          )}
        </div>
      </div>
    </section>
  );
}
