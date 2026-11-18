import { memo, useMemo } from "react";
import type { Order } from "../market";
import type { CatalogOption } from "../orderbook";
import { serial, modelName } from "../orderbook";
import { GiftPreview } from "../previews/gift-preview";
import type { OrderActivity } from "./order-activity";
import { FRESHNESS_MS } from "./order-activity";
import { usePreviewEnvironment } from "../previews/preview-environment";
import { VenueIcon } from "../ui/icons";
import { Quote } from "../ui/market-controls";
export const GiftCard = memo(function GiftCard({
  order,
  options,
  activity,
  collectionName,
  venueName,
  selected,
  open,
  position,
  total,
}: {
  position?: number;
  total?: number;
  order: Order;
  activity?: OrderActivity;
  options: ReadonlyMap<string, CatalogOption>;
  collectionName: string;
  venueName: string;
  selected: boolean;
  open: (order: Order) => void;
}) {
  const { pageVisible, reducedMotion } = usePreviewEnvironment();
  const clockStyle = useMemo(
    () => ({
      animationDuration: `${FRESHNESS_MS}ms`,
      animationDelay: `-${Math.max(0, Date.now() - (activity?.since ?? Date.now()))}ms`,
    }),
    [activity?.since, pageVisible, reducedMotion],
  );
  const number = serial(order),
    unsupported = order.target.kind === "unsupported";
  return (
    <li
      className={`gift-card${selected ? " selected" : ""}${activity ? ` is-${activity.state}` : ""}`}
      data-activity={activity?.state}
      aria-posinset={position}
      aria-setsize={total}
    >
      <GiftPreview
        order={order}
        options={options}
        fallback={collectionName[0] || "◇"}
        className="card-preview"
        playback={activity?.state === "removed" ? "static" : "interaction"}
      >
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
          <span
            className="serial"
            title={
              order.target.kind === "unsupported"
                ? order.target.reason.message
                : undefined
            }
          >
            {unsupported
              ? "Unsupported"
              : number
                ? `#${number}`
                : order.quantity !== 1
                  ? `×${order.quantity}`
                  : "Gift"}
          </span>
          <span className="venue-icon" title={venueName}>
            <VenueIcon name={venueName} />
          </span>
        </div>
        <div className="card-copy">
          <strong>{modelName(order, options, collectionName)}</strong>
          <div className={`quote price${order.kind === "bid" ? " bid" : ""}`}>
            <Quote price={order.price} currency={order.book.currency_id} />
          </div>
        </div>
        <button
          className="card-open"
          aria-label={`View ${collectionName}${number ? ` #${number}` : ""} on ${venueName}${activity ? (activity.state === "removed" ? ", left refreshed results" : ", new") : ""}`}
          data-order-id={order.id}
          data-venue-id={order.book.venue_id}
          onClick={() => open(order)}
          aria-haspopup="dialog"
        />
      </GiftPreview>
    </li>
  );
});
