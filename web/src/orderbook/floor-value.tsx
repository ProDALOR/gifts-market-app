import type { SharedFloor } from "../valuation/floors-controller";
import { FloorSummary } from "../valuation/floors";

/** Read-only cached floor, including loading and unavailable states. */
export function FloorValue({
  label,
  floor,
  currency,
  stale,
  loading = false,
  error = "",
}: {
  label: string;
  floor?: SharedFloor;
  currency: string;
  stale?: boolean;
  loading?: boolean;
  error?: string;
}) {
  return (
    <span className="floor-value">
      {floor ? (
        <FloorSummary
          compact
          rounded
          floor={floor}
          currency={currency}
          stale={stale}
          label={label}
        />
      ) : loading ? (
        <span
          className="floor-loading"
          role="status"
          aria-label={`Loading ${label.toLowerCase()}`}
        >
          <span />
          <span />
        </span>
      ) : (
        <span
          className={`attribute-floor-missing${error ? " floor-load-error" : ""}`}
          aria-label={
            error ? `${label} could not be loaded` : `${label} unavailable`
          }
          title={error || undefined}
        >
          {error ? "!" : "—"}
        </span>
      )}
    </span>
  );
}
