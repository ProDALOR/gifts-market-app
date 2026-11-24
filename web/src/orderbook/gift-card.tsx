import { memo } from "react";
import type { Order, GiftMedia } from "../market";
import type { CatalogOption } from "../orderbook";
import { serial } from "../orderbook";
import { criteriaPresentation } from "../order-target";
import type { OrderActivity } from "./order-activity";
import { VenueIcon } from "../ui/icons";
import { ExecutionAge } from "./execution-age";
import { GiftCardView } from "../previews/gift-card";
export const GiftCard = memo(function GiftCard({
  order,
  options,
  activity,
  collectionName,
  collectionPreview,
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
  collectionPreview?: GiftMedia;
  venueName: string;
  selected: boolean;
  open: (order: Order) => void;
}) {
  const number = serial(order),
    criteria = criteriaPresentation(order);
  const identity = criteria
    ? `×${order.quantity.toLocaleString()}`
    : order.target.kind === "unsupported"
      ? "Unsupported"
      : number
        ? `#${number}`
        : order.quantity !== 1
          ? `×${order.quantity}`
          : "Gift";
  return (
    <GiftCardView
      subject={order}
      options={options}
      collectionName={collectionName}
      media={
        order.kind === "executed" && order.target.kind === "criteria"
          ? collectionPreview
          : undefined
      }
      identity={identity}
      identityDetail={
        order.kind === "executed" && order.executed_at ? (
          <ExecutionAge at={order.executed_at} />
        ) : undefined
      }
      meta={
        <span className="venue-icon" title={venueName}>
          <VenueIcon name={venueName} />
        </span>
      }
      price={order.price}
      currency={order.book.currency_id}
      activity={activity}
      selected={selected}
      position={position}
      total={total}
      action={
        <button
          className="card-open"
          title={
            order.kind === "executed" && order.executed_at
              ? `Executed ${new Date(order.executed_at).toLocaleString()}`
              : undefined
          }
          aria-label={`View ${order.kind === "bid" ? "bid for " : ""}${collectionName}${number ? ` #${number}` : ""} on ${venueName}${activity ? (activity.state === "removed" ? ", left refreshed results" : ", new") : ""}`}
          data-order-id={order.id}
          data-venue-id={order.book.venue_id}
          onClick={() => open(order)}
          aria-haspopup="dialog"
        />
      }
    />
  );
});
