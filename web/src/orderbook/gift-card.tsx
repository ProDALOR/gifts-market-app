import { memo } from "react";
import type { Order } from "../market";
import type { CatalogOption } from "../orderbook";
import { serial, modelName } from "../orderbook";
import { GiftPreview } from "../previews/gift-preview";
import { VenueIcon } from "../ui/icons";
import { Quote } from "../ui/market-controls";
export const GiftCard = memo(function GiftCard({
  order,
  options,
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
  options: ReadonlyMap<string, CatalogOption>;
  collectionName: string;
  venueName: string;
  selected: boolean;
  open: (order: Order) => void;
}) {
  const number = serial(order);
  return (
    <li
      className={`gift-card${selected ? " selected" : ""}`}
      aria-posinset={position}
      aria-setsize={total}
    >
      <GiftPreview
        order={order}
        options={options}
        fallback={collectionName[0] || "◇"}
        className="card-preview"
      >
        <div className="card-head">
          <span className="serial">
            {number
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
          <div className={`quote price${order.mode === "bid" ? " bid" : ""}`}>
            <Quote price={order.price} currency={order.book.currency_id} />
          </div>
        </div>
        <button
          className="card-open"
          aria-label={`View ${collectionName}${number ? ` #${number}` : ""} on ${venueName}`}
          data-order-id={order.id}
          data-venue-id={order.book.venue_id}
          onClick={() => open(order)}
          aria-haspopup="dialog"
        />
      </GiftPreview>
    </li>
  );
});
