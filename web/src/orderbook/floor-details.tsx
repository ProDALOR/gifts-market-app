import { useState } from "react";
import type { ReactNode } from "react";
import type { SharedFloor } from "../valuation/floors-controller";
import { FloorSummary } from "../valuation/floors";
import { DetailFactPopup } from "./detail-fact-popup";
import { Quote } from "../ui/market-controls";
import { fullTimestamp } from "../ui/time";

/** Inspect cached evidence; opening never performs a read or calculation. */
export function FloorDetails({
  label,
  name,
  floor,
  currency,
  stale,
  evaluatedAt,
  loading = false,
  error = "",
  className = "",
  children,
  title,
}: {
  label: string;
  name?: string;
  floor?: SharedFloor;
  currency: string;
  stale?: boolean;
  evaluatedAt?: string;
  loading?: boolean;
  error?: string;
  className?: string;
  children?: ReactNode;
  title?: string;
}) {
  const [open, setOpen] = useState(false);
  const value = floor && "value" in floor.price ? floor.price.value : undefined;
  const reason =
    floor && "error" in floor.price
      ? floor.price.error.message
      : error || "No floor information is available for this selection.";
  const updated = open && evaluatedAt ? fullTimestamp(evaluatedAt) : undefined;
  return (
    <>
      <button
        className={`floor-inspect ${className}`}
        aria-label={`View ${label.toLowerCase()} details${name ? ` for ${name}` : ""}`}
        aria-description={
          floor
            ? `${value ? `${value} ${currency}` : reason}; ${floor.evidence.gifts} unique gifts, ${floor.evidence.listings} listings`
            : loading
              ? "Loading"
              : reason
        }
        title={title}
        aria-haspopup="dialog"
        aria-expanded={open}
        disabled={loading && !floor}
        onClick={() => setOpen(true)}
      >
        {children ??
          (floor ? (
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
            >
              {error ? "!" : "—"}
            </span>
          ))}
      </button>
      {open && (
        <DetailFactPopup
          heading={label}
          label="Floor details"
          close={() => setOpen(false)}
        >
          {name && <p className="detail-fact-name">{name}</p>}
          {value ? (
            <dl className="detail-fact-values">
              <div>
                <dt>Exact ask floor</dt>
                <dd className="detail-fact-price">
                  <Quote price={value} currency={currency} />
                </dd>
              </div>
            </dl>
          ) : (
            <p className="detail-fact-reason">{reason}</p>
          )}
          {floor && (
            <>
              <dl className="detail-fact-values">
                <div>
                  <dt>Unique gifts</dt>
                  <dd>{floor.evidence.gifts.toLocaleString()}</dd>
                </div>
                <div>
                  <dt>Listings</dt>
                  <dd>{floor.evidence.listings.toLocaleString()}</dd>
                </div>
              </dl>
              <p className="detail-fact-note">
                Each gift is counted once, even when listed on several markets.
                Prices exclude fees.
              </p>
            </>
          )}
          {updated && (
            <p className="detail-fact-note">
              Calculated <time dateTime={evaluatedAt}>{updated}</time>
            </p>
          )}
        </DetailFactPopup>
      )}
    </>
  );
}
