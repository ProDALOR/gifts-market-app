import type { Order } from "../market";
import type { CatalogOption } from "../orderbook";
import { listingLink, modelName, serial } from "../orderbook";
import { GiftPreview } from "../previews/gift-preview";
import { Modal } from "../ui/modal";
import { GiftValuationSummary } from "./gift-valuations";
import { Quote } from "../ui/market-controls";
import { VenueIcon } from "../ui/icons";
import { OrderAttributes } from "./order-attributes";
import { useGiftValuation } from "../valuation/use-gift-valuation";
import { priceComparison } from "../valuation/price-comparison";
export function OrderDetails({
  selected,
  options,
  names,
  collectionName,
  removed = false,
  close,
}: {
  selected: Order;
  options: ReadonlyMap<string, CatalogOption>;
  names: Map<string, string>;
  collectionName: string;
  removed?: boolean;
  close: () => void;
}) {
  const state = useGiftValuation(selected);
  const criteria = selected.target.kind === "criteria";
  const selectedSerial = serial(selected),
    model =
      selected.target.kind === "gift" ? modelName(selected, options, "") : "",
    marketLink = listingLink(selected),
    venue = names.get(selected.book.venue_id) || selected.book.venue_id;
  const { primary } = state.fields;
  const estimate = primary.length === 1 ? primary[0].value : undefined;
  const comparison =
    !removed &&
    !state.loading &&
    !state.error &&
    selected.kind === "ask" &&
    !state.stale &&
    estimate?.kind === "currency" &&
    estimate.currency_id === selected.book.currency_id
      ? priceComparison(selected.price, estimate.amount)
      : undefined;
  return (
    <Modal
      close={close}
      className={`detail${criteria ? " detail-criteria" : ""}`}
      label="Order details"
      closeLabel="Close details"
      describedBy="detail-title"
    >
      {criteria ? (
        <header className="bid-detail-heading">
          <p>Bid conditions</p>
          <h2 id="detail-title">
            {names.get(selected.book.collection_id) || collectionName}
          </h2>
        </header>
      ) : (
        <div className="detail-art">
          <GiftPreview
            order={selected}
            options={options}
            fallback={collectionName[0] || "◇"}
            size="large"
            playback={selected.kind === "bid" ? "static" : "detail"}
          />
          <div className="detail-caption">
            <h2 id="detail-title">
              {names.get(selected.book.collection_id) || collectionName}
              {selectedSerial && (
                <>
                  {" "}
                  <span>
                    #{selectedSerial.replace(/\B(?=(\d{3})+(?!\d))/g, "\u00a0")}
                  </span>
                </>
              )}
            </h2>
            {model && <p>{model}</p>}
          </div>
        </div>
      )}
      <div className="detail-content">
        <div className="detail-body">
          <div className="detail-pricing">
            <section className="listing-price" aria-label="Listing price">
              <h3>
                {selected.kind === "executed"
                  ? "Execution price"
                  : selected.kind === "bid"
                    ? "Bid price"
                    : "Asking price"}
              </h3>
              <div className="detail-price">
                <Quote
                  price={selected.price}
                  currency={selected.book.currency_id}
                />
              </div>
              {criteria && <p className="valuation-note">Price per gift</p>}
              {comparison && <p className="price-comparison">{comparison}</p>}
            </section>
            {!criteria && <GiftValuationSummary state={state} />}
          </div>
          <OrderAttributes
            order={selected}
            options={options}
            names={names}
            fields={state.fields.secondary}
            summary={state.report?.display.summary}
          />
          {removed && (
            <p className="listing-gone" role="status">
              This listing left the refreshed results. A disappearance does not
              confirm a sale.
            </p>
          )}
        </div>
        {marketLink && !removed && (
          <div className="detail-actions">
            <a
              className="detail-market"
              href={marketLink}
              target="_blank"
              rel="noreferrer"
            >
              <VenueIcon name={venue} />
              Open on {venue} <span aria-hidden="true">↗</span>
            </a>
          </div>
        )}
      </div>
    </Modal>
  );
}
