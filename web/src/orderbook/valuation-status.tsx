import type { OrderValuationState } from "../valuation/use-order-valuation";

/** The popup and compact estimate describe the same report and refresh attempt. */
export function ValuationStatus({ state }: { state: OrderValuationState }) {
  const { report, loading, error } = state;
  if (!report) return null;
  return (
    <>
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
        {state.stale && (
          <span className="valuation-stale">Estimate may be outdated · </span>
        )}
        Calculated{" "}
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
  );
}
