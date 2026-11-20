import type { Order, CatalogOption, ValuationView } from "../market";
import { BidConditions } from "./bid-conditions";
import { giftDetails } from "../orderbook";
import { GiftArtwork } from "../previews/gift-artwork";
import { ValuationFields } from "../valuation/display";
import { backdropPalette } from "../previews/gift-appearance";
import { useCollectionValues, FloorBadge } from "../valuation/market-values";

export function OrderAttributes({
  order,
  options,
  names,
  fields = [],
  summary,
}: {
  order: Order;
  options: ReadonlyMap<string, CatalogOption>;
  names: ReadonlyMap<string, string>;
  fields?: ValuationView["display"]["fields"];
  summary?: string | null;
}) {
  const shared = useCollectionValues(
    order.book.collection_id,
    order.book.currency_id,
  );
  const pair = shared?.traits?.pair;
  const market = shared && (
    <section
      className="shared-market-values"
      aria-label="Current market ask values"
    >
      <h3>Current market asks</h3>
      <div>
        <FloorBadge
          floor={shared.floor}
          currency={order.book.currency_id}
          stale={shared.stale}
          label="Collection floor"
        />
        {order.target.kind === "gift" &&
          pair &&
          pair.model_id === order.target.traits.model &&
          pair.backdrop_id === order.target.traits.backdrop && (
            <FloorBadge
              floor={pair.floor}
              currency={order.book.currency_id}
              stale={shared.stale}
              label="Model + backdrop floor"
            />
          )}
      </div>
      {order.kind === "executed" && (
        <p className="valuation-note">
          These are current asks, not prices at the time of execution.
        </p>
      )}
    </section>
  );
  const displayFields = shared?.traits
    ? fields.filter((field) => !field.basis)
    : fields;
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
  const attributes = new Set(rows.map((row) => row.attribute));
  const general = displayFields.filter(
    (field) => !field.attribute || !attributes.has(field.attribute),
  );
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
                    <FloorBadge
                      currency={order.book.currency_id}
                      stale={shared?.stale}
                      label="Current floor"
                      floor={
                        attribute === "model"
                          ? shared?.traits?.models[
                              order.target.kind === "gift"
                                ? order.target.traits.model || ""
                                : ""
                            ]
                          : attribute === "backdrop"
                            ? shared?.traits?.backdrops[
                                order.target.kind === "gift"
                                  ? order.target.traits.backdrop || ""
                                  : ""
                              ]
                            : undefined
                      }
                    />
                  )}
                  <ValuationFields
                    fields={
                      attribute
                        ? displayFields.filter(
                            (field) => field.attribute === attribute,
                          )
                        : []
                    }
                  />
                </dd>
              </div>
            );
          })}
        </dl>
      )}
      <div className="valuation-context">
        <ValuationFields fields={general} />
      </div>
      {summary && <p className="valuation-note">{summary}</p>}
    </section>
  );
}
