import type { Order, Predicate } from "../market";
import { text } from "../market";
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
          {names.get(selected.book.item_type_id) || selected.book.item_type_id}
          {selectedSerial && ` #${selectedSerial}`}
        </h2>
        <div className="detail-price">
          <Quote price={selected.price} currency={selected.book.currency_id} />
        </div>
        <dl>
          {Object.entries(selected.attributes).map(([id, predicate]) => (
            <div key={id}>
              <dt>{id === "tg.pattern" ? "Symbol" : names.get(id) || id}</dt>
              <dd>{describe(predicate, names)}</dd>
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
                ? "Offer"
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
              ["Item ID", selected.item_id || "—"],
              ["Currency", selected.book.currency_id],
              ...(selected.parent_order_id
                ? [["Parent order", selected.parent_order_id]]
                : []),
              ...Object.entries(selected.meta),
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
function describe(p: Predicate, names: Map<string, string>): string {
  switch (p.op) {
    case "eq":
      return text(p.value, names);
    case "one_of":
      return p.values.map((v) => text(v, names)).join(" / ");
    case "range":
      return `${p.min ? text(p.min, names) : "Any"} – ${p.max ? text(p.max, names) : "Any"}`;
    case "object":
      return Object.entries(p.fields || {})
        .map(([k, v]) => `${names.get(k) || k}: ${describe(v, names)}`)
        .join(" · ");
    case "contains":
      return `Contains ${p.min_count ?? 1}${p.max_count == null ? "+" : "–" + p.max_count}: ${describe(p.predicate, names)}`;
    case "all_of":
      return p.predicates.map((v) => describe(v, names)).join("; ");
  }
}
