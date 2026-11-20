import type { Order } from "../market";
import type { CatalogOption } from "../orderbook";
import { listingLink, modelName, serial } from "../orderbook";
import { GiftPreview } from "../previews/gift-preview";
import { Modal } from "../ui/modal";
import { OrderValuationSummary } from "./order-valuation";
import { Quote } from "../ui/market-controls";
import { VenueIcon } from "../ui/icons";
import { OrderAttributes } from "./order-attributes";
import { BidPreview } from "./bid-preview";
import { useOrderValuation } from "../valuation/use-order-valuation";
import { priceComparison } from "../valuation/price-comparison";
export function OrderDetails({
  selected,
  options,
  names,
  collectionName,
  collectionSlug,
  removed = false,
  close,
}: {
  selected: Order;
  options: ReadonlyMap<string, CatalogOption>;
  names: Map<string, string>;
  collectionName: string;
  collectionSlug?: string | null;
  removed?: boolean;
  close: () => void;
}) {
  const state = useOrderValuation(selected);
  const criteria = selected.target.kind === "criteria";
  const selectedSerial = serial(selected),
    model =
      selected.target.kind === "gift" ? modelName(selected, options, "") : "",
    marketLink = listingLink(selected),
    venue = names.get(selected.book.venue_id) || selected.book.venue_id;
  const telegramLink =
    selectedSerial &&
    collectionSlug &&
    /^[A-Za-z0-9]{1,128}$/.test(collectionSlug)
      ? `https://t.me/nft/${collectionSlug}-${selectedSerial}`
      : undefined;
  const showMarketLink = marketLink && !removed && marketLink !== telegramLink;
  const priceLabel =
    selected.kind === "executed"
      ? "Execution price"
      : selected.kind === "bid"
        ? "Bid price"
        : "Asking price";
  const primary = state.primaryFields;
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
      viewportClose
      describedBy="detail-title"
    >
      <div className={`detail-art${criteria ? " detail-bid-art" : ""}`}>
        {selected.target.kind === "criteria" ? (
          <div className="bid-detail-preview">
            <BidPreview
              criteria={selected.target}
              options={options}
              names={names}
              compact={false}
            />
          </div>
        ) : (
          <GiftPreview
            order={selected}
            options={options}
            fallback={collectionName[0] || "◇"}
            size="large"
            playback={selected.kind === "bid" ? "static" : "detail"}
          />
        )}
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
      <div className="detail-content">
        <div className="detail-body">
          <div className="detail-pricing">
            <section className="listing-price" aria-label={priceLabel}>
              <h3>{priceLabel}</h3>
              <div className="detail-price">
                <Quote
                  price={selected.price}
                  currency={selected.book.currency_id}
                />
                {criteria && (
                  <small className="detail-price-unit">/ gift</small>
                )}
              </div>
              {comparison && <p className="price-comparison">{comparison}</p>}
            </section>
            <OrderValuationSummary
              state={state}
              criteria={criteria}
              history={selected.kind === "executed"}
            />
          </div>
          <OrderAttributes order={selected} options={options} names={names} />
          {removed && (
            <p className="listing-gone" role="status">
              No longer in these results.
            </p>
          )}
        </div>
        {(showMarketLink || telegramLink) && (
          <div className="detail-actions">
            {showMarketLink && (
              <a
                className="detail-link detail-market"
                href={marketLink}
                target="_blank"
                rel="noreferrer"
                aria-label={`Open on ${venue}`}
                title={`Open on ${venue}`}
              >
                <VenueIcon name={venue} />
                <span className="detail-link-label">{venue}</span>
                <span aria-hidden="true">↗</span>
              </a>
            )}
            {telegramLink && (
              <a
                className="detail-link detail-telegram"
                href={telegramLink}
                target="_blank"
                rel="noreferrer"
                aria-label="Open in Telegram"
              >
                <VenueIcon name="Telegram" />
                <span className="detail-link-label">Telegram</span>
                <span aria-hidden="true">↗</span>
              </a>
            )}
          </div>
        )}
      </div>
    </Modal>
  );
}
