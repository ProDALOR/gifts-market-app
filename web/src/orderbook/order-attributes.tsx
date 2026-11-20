import type { Order, CatalogOption } from "../market";
import { BidConditions } from "./bid-conditions";
import { giftDetails } from "../orderbook";
import { GiftArtwork } from "../previews/gift-artwork";
import { backdropPalette } from "../previews/gift-appearance";
import { useCollectionFloors, FloorSummary } from "../valuation/floors";
import { traitFloor } from "../valuation/floors-controller";

export function OrderAttributes({
  order,
  options,
  names,
}: {
  order: Order;
  options: ReadonlyMap<string, CatalogOption>;
  names: ReadonlyMap<string, string>;
}) {
  const shared = useCollectionFloors(
    order.book.collection_id,
    order.book.currency_id,
  );
  const pair = shared?.traits?.pair;
  const traits = order.target.kind === "gift" ? order.target.traits : undefined;
  const market = shared && (
    <section className="shared-floors" aria-label="Current market ask values">
      <div>
        <FloorSummary
          floor={shared.floor}
          currency={order.book.currency_id}
          stale={shared.stale}
          label="Collection floor"
        />
        {order.target.kind === "gift" &&
          pair &&
          pair.model_id === order.target.traits.model &&
          pair.backdrop_id === order.target.traits.backdrop && (
            <FloorSummary
              floor={pair.floor}
              currency={order.book.currency_id}
              stale={shared.stale}
              label="Model + backdrop floor"
            />
          )}
      </div>
    </section>
  );
  if (order.target.kind === "criteria")
    return (
      <>
        {market}
        <BidConditions
          criteria={order.target}
          options={options}
          names={names}
          quantity={order.quantity}
          collection={order.book.collection_id}
          currency={order.book.currency_id}
        />
      </>
    );
  const rows = giftDetails(order, names, options);
  return (
    <section
      className="gift-attributes"
      aria-label="Gift attributes and valuation"
    >
      {market}
      {rows.length > 0 && (
        <dl className="detail-attributes">
          {rows.map(({ label, value, attribute, option: visual }) => {
            return (
              <div key={label} data-attribute={attribute}>
                <dt>
                  {visual && (
                    <span
                      className={`attribute-visual attribute-${visual.kind}`}
                      aria-hidden="true"
                    >
                      <GiftArtwork
                        media={
                          visual.kind !== "backdrop"
                            ? visual.media || undefined
                            : undefined
                        }
                        palette={backdropPalette(visual)}
                        fallback={
                          visual.kind === "backdrop" && visual.palette
                            ? ""
                            : value[0] || "◇"
                        }
                        playback="static"
                        placeholderKind={
                          visual.kind === "pattern" ? "symbol" : "gift"
                        }
                      />
                    </span>
                  )}
                  <span>{label}</span>
                </dt>
                <dd>
                  <strong className="attribute-name">{value}</strong>
                  {attribute && (
                    <FloorSummary
                      compact
                      currency={order.book.currency_id}
                      stale={shared?.stale}
                      label="Current floor"
                      floor={traitFloor(shared, attribute, traits?.[attribute])}
                    />
                  )}
                </dd>
              </div>
            );
          })}
        </dl>
      )}
    </section>
  );
}
