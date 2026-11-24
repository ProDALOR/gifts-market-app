import { memo, useMemo } from "react";
import type { ReactNode } from "react";
import type { GiftMedia } from "../market";
import type { PreviewOrder } from "../order-target";
import { criteriaPresentation } from "../order-target";
import type { CatalogOption } from "../orderbook";
import { giftNumberRange, modelName } from "../orderbook";
import { GiftPreview } from "./gift-preview";
import { GiftArtwork } from "./gift-artwork";
import { BidPreview } from "../orderbook/bid-preview";
import { Quote } from "../ui/market-controls";
import { CurrencySymbol } from "../ui/icons";
import { amount } from "../amount";
import { FRESHNESS_MS } from "../orderbook/order-activity";
import type { OrderActivity } from "../orderbook/order-activity";
import { usePreviewEnvironment } from "./preview-environment";
/** One card layout for listings and inventory gifts. Market adapters supply only listing-specific content. */
export const GiftCardView = memo(function GiftCardView({
  subject,
  options,
  collectionName,
  media,
  identity,
  identityDetail,
  meta,
  price,
  currency,
  format = "listing",
  action,
  selected = false,
  activity,
  position,
  total,
  className = "",
}: {
  subject?: PreviewOrder;
  options: ReadonlyMap<string, CatalogOption>;
  collectionName: string;
  media?: GiftMedia;
  identity: ReactNode;
  identityDetail?: ReactNode;
  meta?: ReactNode;
  price?: string;
  currency: string;
  format?: "listing" | "rounded";
  action: ReactNode;
  selected?: boolean;
  activity?: OrderActivity;
  position?: number;
  total?: number;
  className?: string;
}) {
  const { pageVisible, reducedMotion } = usePreviewEnvironment();
  const clockStyle = useMemo(
    () => ({
      animationDuration: `${FRESHNESS_MS}ms`,
      animationDelay: `-${Math.max(0, Date.now() - (activity?.since ?? Date.now()))}ms`,
    }),
    [activity?.since, pageVisible, reducedMotion],
  );
  const criteria = subject && criteriaPresentation(subject);
  const name = subject
    ? modelName(subject, options, collectionName)
    : collectionName;
  const content = (
    <>
      {activity && (
        <>
          <span
            className="activity-ribbon"
            title={
              activity.state === "removed"
                ? "Left the refreshed results; this does not confirm a sale"
                : "New in this live view"
            }
          >
            {activity.state === "removed" ? "Gone" : "New"}
          </span>
          {pageVisible && !reducedMotion && (
            <span
              aria-hidden="true"
              className="card-clock"
              key={`${activity.state}:${activity.since}`}
              style={clockStyle}
            />
          )}
        </>
      )}
      <div className="card-head">
        <div className="card-identity">
          <span
            className="serial"
            title={
              subject?.target.kind === "unsupported"
                ? subject.target.message
                : typeof identity === "string"
                  ? identity
                  : undefined
            }
          >
            {identity}
          </span>
          {identityDetail}
        </div>
        {meta}
      </div>
      <div className="card-copy">
        {criteria?.number && (
          <span className="bid-number">
            #{giftNumberRange(criteria.number)}
          </span>
        )}
        {!criteria && <strong title={name}>{name}</strong>}
        <div className={`quote price${subject?.kind === "bid" ? " bid" : ""}`}>
          {price != null ? (
            <Quote price={price} currency={currency} format={format} />
          ) : (
            <>
              <CurrencySymbol currency={currency} />
              <span className="quote-amount">—</span>
            </>
          )}
        </div>
      </div>
      {action}
    </>
  );
  return (
    <li
      className={`gift-card${selected ? " selected" : ""}${activity ? ` is-${activity.state}` : ""}${className ? ` ${className}` : ""}`}
      title={[
        name,
        typeof identity === "string" ? identity : undefined,
        price != null ? `${amount(price)} ${currency}` : undefined,
      ]
        .filter(Boolean)
        .join("\n")}
      data-activity={activity?.state}
      style={
        activity?.state === "new"
          ? {
              animationDelay: clockStyle.animationDelay,
              animationName: pageVisible && !reducedMotion ? undefined : "none",
            }
          : undefined
      }
      aria-posinset={position}
      aria-setsize={total}
    >
      {criteria ? (
        <div className="card-preview bid-card">
          <BidPreview criteria={criteria} options={options} />
          {content}
        </div>
      ) : subject ? (
        <GiftPreview
          order={subject}
          options={options}
          fallback={collectionName[0] || "◇"}
          fallbackMedia={media}
          className="card-preview"
          patternLayout="card"
          playback={
            subject.kind === "bid" || activity?.state === "removed"
              ? "static"
              : "interaction"
          }
        >
          {content}
        </GiftPreview>
      ) : (
        <GiftArtwork
          media={media}
          className="card-preview"
          patternLayout="card"
          fallback="◇"
          playback="static"
        >
          {content}
        </GiftArtwork>
      )}
    </li>
  );
});
