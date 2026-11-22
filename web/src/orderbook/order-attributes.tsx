import type {
  Order,
  CatalogOption,
  GiftMedia,
  FloorValuation,
} from "../market";
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
import { criteriaPresentation, giftPresentation } from "../order-target";

export function OrderAttributes({
  order,
  options,
  names,
  collectionPreview,
}: {
  order: Order;
  options: ReadonlyMap<string, CatalogOption>;
  names: ReadonlyMap<string, string>;
  collectionPreview?: GiftMedia;
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
          <caption>Ask floors</caption>
          <tbody>
            {rows.map(({ label, value, attribute, visual, floor }) => (
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
                      <span className="attribute-label">{label}</span>
                      {value && (
                        <strong className="attribute-name">{value}</strong>
                      )}
                    </span>
                  </span>
                </th>
                <td>
                  <FloorValue
                    label={`${label} floor`}
                    floor={floor}
                    currency={order.book.currency_id}
                    stale={shared?.stale}
                    loading={detail.pending}
                    error={detail.error}
                  />
                </td>
              </tr>
            ))}
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
