import type {
  OrderValuationState,
  OrderValuation,
} from "../valuation/use-order-valuation";

/** The popup and compact estimate describe the same report and refresh attempt. */
export function ValuationStatus({
  state,
  detailed = false,
  valuation,
}: {
  state: OrderValuationState;
  valuation?: OrderValuation;
  detailed?: boolean;
}) {
  const { report, loading, error } = state;
  if (!report) return null;
  return (
    <>
      {loading && detailed && (
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
      {detailed && (
        <>
          <p className="valuation-note">
            Calculated{" "}
            <time dateTime={report.evaluated_at}>
              {new Date(report.evaluated_at).toLocaleTimeString([], {
                hour: "2-digit",
                minute: "2-digit",
              })}
            </time>
          </p>
          {valuation?.strategy.display.note && (
            <p className="valuation-note">{valuation.strategy.display.note}</p>
          )}
        </>
      )}
    </>
  );
}
