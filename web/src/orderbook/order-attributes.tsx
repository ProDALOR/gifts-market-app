import type { CatalogOption, GiftMedia, FloorValuation } from "../market";
import { BidConditions } from "./bid-conditions";
import { giftDetails } from "../orderbook";
import { AttributeArt } from "./attribute-art";
import type { AttributeArtKind } from "./attribute-art";
import {
  useCollectionFloors,
  useCollectionFloorDetails,
} from "../valuation/floors";
import { traitFloor } from "../valuation/floors-controller";
import { FloorValue } from "./floor-value";
import type { ValuationSubject } from "../order-target";
import { criteriaPresentation, giftPresentation } from "../order-target";
import type { OrderValuationState } from "../valuation/use-order-valuation";
import { BidValue } from "./bid-value";
import { RarityLabel } from "../ui/rarity-label";
import { MarketLink } from "../app/navigation";
import { attributeMarketSelection } from "./attribute-market";

export function OrderAttributes({
  order,
  options,
  names,
  collectionPreview,
  valuation,
}: {
  order: ValuationSubject;
  options: ReadonlyMap<string, CatalogOption>;
  names: ReadonlyMap<string, string>;
  collectionPreview?: GiftMedia;
  valuation?: OrderValuationState;
}) {
  const shared = useCollectionFloors(
    order.book.collection_id,
    order.book.currency_id,
  );
  const detail = useCollectionFloorDetails(
    order.book.collection_id,
    order.book.currency_id,
  );
  const traits = giftPresentation(order)?.traits;
  const criteria = criteriaPresentation(order);
  const bidResult = valuation?.report?.results.find(
    (row) => row.strategy === "best_bid",
  );
  const bids =
    bidResult && "data" in bidResult && bidResult.strategy === "best_bid"
      ? bidResult.data.references
      : undefined;
  const bidState = valuation?.valuations.find(
    (row) => row.strategy.id === "best_bid",
  );
  const bidError =
    bidResult && "error" in bidResult
      ? bidResult.error.message
      : valuation?.error;
  const pair = shared?.traits?.pair;
  const matchingPair =
    pair &&
    pair.model_id === traits?.model &&
    pair.backdrop_id === traits?.backdrop
      ? pair.floor
      : undefined;
  const details = criteria ? [] : giftDetails(order, names, options);
  const model = details.find((row) => row.attribute === "model")?.option;
  const backdrop = details.find((row) => row.attribute === "backdrop")?.option;
  const rows: {
    label: string;
    value?: string;
    attribute?: AttributeArtKind;
    visual?: CatalogOption;
    floor?: FloorValuation;
  }[] = [
    {
      label: "Collection",
      value: names.get(order.book.collection_id) || order.book.collection_id,
      attribute: "collection" as const,
      floor: shared?.floor,
    },
    ...(traits?.model && traits.backdrop
      ? [
          {
            label: "Model & Backdrop",
            attribute: "pair" as const,
            floor: matchingPair,
          },
        ]
      : []),
    ...details.map(({ label, value, attribute, option }) => ({
      label,
      value,
      attribute,
      visual: option,
      floor: attribute
        ? traitFloor(shared, attribute, traits?.[attribute])
        : undefined,
    })),
  ];
  return (
    <section
      className="gift-attributes"
      aria-label="Gift attributes and valuation"
    >
      <div className="attribute-floor-table">
        <table className="detail-attributes">
          <thead>
            <tr>
              <th scope="col">
                <span className="sr">Attribute</span>
              </th>
              <th scope="col">Best asks</th>
              <th scope="col">Best bids</th>
            </tr>
          </thead>
          <tbody>
            {rows.map(({ label, value, attribute, visual, floor }) => {
              const bid =
                attribute === "collection"
                  ? bids?.collection
                  : attribute === "pair"
                    ? bids?.model_backdrop
                    : attribute
                      ? bids?.[attribute]
                      : undefined;
              const asks =
                floor && "value" in floor
                  ? attributeMarketSelection(order, attribute, "asks")
                  : undefined;
              const bidSelection =
                bid && "value" in bid
                  ? attributeMarketSelection(order, attribute, "bids")
                  : undefined;
              const askValue = (
                <FloorValue
                  label={`${label} floor`}
                  floor={floor}
                  currency={order.book.currency_id}
                  stale={shared?.stale}
                  loading={detail.pending}
                  error={detail.error}
                />
              );
              const bidValue = (
                <BidValue
                  label={`${label} best bid`}
                  bid={bid}
                  currency={order.book.currency_id}
                  stale={bidState?.stale}
                  loading={valuation?.loading}
                  error={bidError}
                  names={names}
                />
              );
              return (
                <tr key={label} data-attribute={attribute}>
                  <th scope="row">
                    <span className="attribute-identity">
                      {(visual ||
                        attribute === "collection" ||
                        attribute === "pair") && (
                        <span
                          className={`attribute-visual attribute-${visual?.kind ?? attribute}`}
                          aria-hidden="true"
                        >
                          <AttributeArt
                            kind={attribute}
                            option={visual}
                            model={model}
                            backdrop={backdrop}
                            collectionPreview={collectionPreview}
                          />
                        </span>
                      )}
                      <span className="attribute-copy">
                        <span className="attribute-label">
                          {label}
                          <RarityLabel rarity={visual?.rarity} />
                        </span>
                        {value && (
                          <strong className="attribute-name">{value}</strong>
                        )}
                      </span>
                    </span>
                  </th>
                  <td>
                    {asks ? (
                      <MarketLink
                        className="market-quote-link"
                        selection={asks}
                        aria-label={`View ${label.toLowerCase()} asks`}
                        title={`View ${label.toLowerCase()} asks`}
                      >
                        {askValue}
                      </MarketLink>
                    ) : (
                      askValue
                    )}
                  </td>
                  <td>
                    {bidSelection ? (
                      <MarketLink
                        className="market-quote-link"
                        selection={bidSelection}
                        aria-label={`View ${label.toLowerCase()} bids`}
                        title={`View compatible ${label.toLowerCase()} bids`}
                      >
                        {bidValue}
                      </MarketLink>
                    ) : (
                      bidValue
                    )}
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
      {criteria && (
        <BidConditions
          criteria={criteria}
          options={options}
          names={names}
          collection={order.book.collection_id}
          currency={order.book.currency_id}
        />
      )}
    </section>
  );
}
