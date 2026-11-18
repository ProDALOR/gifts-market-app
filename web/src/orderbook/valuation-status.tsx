import type { GiftValuationState } from "../valuation/use-gift-valuation";

/** The popup and compact estimate describe the same report and refresh attempt. */
export function ValuationStatus({ state }: { state: GiftValuationState }) {
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
  );
}
