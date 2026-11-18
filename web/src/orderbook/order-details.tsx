import type { Order } from "../market";
import type { CatalogOption } from "../orderbook";
import { listingLink, serial, orderDetails } from "../orderbook";
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
          {orderDetails(selected, names).map(([label, value]) => (
            <div key={label}>
              <dt>{label}</dt>
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
      </div>
    </DetailSheet>
  );
}
