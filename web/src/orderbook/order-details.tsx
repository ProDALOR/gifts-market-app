import type { Order } from "../market";
import type { CatalogOption } from "../orderbook";
import { listingLink, serial } from "../orderbook";
import { GiftPreview } from "../previews/gift-preview";
import { DetailSheet } from "../ui/detail-sheet";
import { Quote } from "../ui/market-controls";
export function OrderDetails({
  selected,
  options,
  names,
  collectionName,
  close,
}: {
  selected: Order;
  options: ReadonlyMap<string, CatalogOption>;
  names: Map<string, string>;
  collectionName: string;
  close: () => void;
}) {
  const selectedSerial = serial(selected),
    marketLink = listingLink(selected);
  return (
    <DetailSheet close={close}>
      <div className="detail-art">
        <GiftPreview
          order={selected}
          options={options}
          fallback={collectionName[0] || "◇"}
          size="large"
          playback="detail"
        />
        <button
          className="detail-close"
          aria-label="Close details"
          onClick={close}
          autoFocus
        >
          ×
        </button>
      </div>
      <div className="detail-body">
        <h2 id="detail-title">
          {names.get(selected.book.collection_id) ||
            selected.book.collection_id}
          {selectedSerial && ` #${selectedSerial}`}
        </h2>
        <div className="detail-price">
          <Quote price={selected.price} currency={selected.book.currency_id} />
        </div>
        <dl>
          {describeTarget(selected, names).map(([label, value]) => (
            <div key={label}>
              <dt>{label}</dt>
              <dd>{value}</dd>
            </div>
          ))}
          {[
            [
              "Venue",
              names.get(selected.book.venue_id) || selected.book.venue_id,
            ],
            ["Quantity", selected.quantity],
            [
              "Mode",
              selected.mode === "ask"
                ? "Ask"
                : selected.mode === "bid"
                  ? "Bid"
                  : "Executed",
            ],
            [
              selected.executed_at ? "Executed" : "Observed",
              new Date(
                selected.executed_at || selected.observed_at,
              ).toLocaleString(),
            ],
          ].map(([key, value]) => (
            <div key={key}>
              <dt>{key}</dt>
              <dd>{value}</dd>
            </div>
          ))}
        </dl>
        {marketLink && (
          <a
            className="detail-market"
            href={marketLink}
            target="_blank"
            rel="noreferrer"
          >
            Open on{" "}
            {names.get(selected.book.venue_id) || selected.book.venue_id} ↗
          </a>
        )}
        <details className="source-details">
          <summary>Listing information</summary>
          <dl>
            {[
              ["Order ID", selected.id],
              [
                "Gift ID",
                selected.target.kind === "gift"
                  ? selected.target.gift.id
                  : "Selection",
              ],
              ["Currency", selected.book.currency_id],
              ...(selected.parent_order_id
                ? [["Parent order", selected.parent_order_id]]
                : []),
            ].map(([key, value]) => (
              <div key={key}>
                <dt>{key}</dt>
                <dd>{value}</dd>
              </div>
            ))}
          </dl>
        </details>
      </div>
    </DetailSheet>
  );
}
function describeTarget(
  order: Order,
  names: Map<string, string>,
): [string, string][] {
  const target = order.target;
  if (target.kind === "unsupported") return [["Conditions", target.reason]];
  const criteria =
    target.kind === "criteria"
      ? target.criteria
      : {
          models: target.gift.traits.model ? [target.gift.traits.model] : [],
          backdrops: target.gift.traits.backdrop
            ? [target.gift.traits.backdrop]
            : [],
          patterns: target.gift.traits.pattern
            ? [target.gift.traits.pattern]
            : [],
          number:
            target.gift.number == null
              ? null
              : { min: target.gift.number, max: target.gift.number },
        };
  const rows: [string, string][] = (
    [
      ["Model", criteria.models],
      ["Backdrop", criteria.backdrops],
      ["Symbol", criteria.patterns],
    ] as const
  ).map(([label, ids]) => [
    label,
    ids.length
      ? ids.map((id) => names.get(id) || id).join(" / ")
      : target.kind === "criteria"
        ? "Any"
        : "Unknown",
  ]);
  if (criteria.number)
    rows.push([
      "Gift #",
      criteria.number.min === criteria.number.max
        ? String(criteria.number.min)
        : `${criteria.number.min ?? "Any"} – ${criteria.number.max ?? "Any"}`,
    ]);
  return rows;
}
