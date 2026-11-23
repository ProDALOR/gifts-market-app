import type { Order, GiftMedia } from "../market";
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
import { useBackgroundPlaybackSuspension } from "../previews/use-playback";
import { ExecutionTime } from "./execution-time";
import { criteriaPresentation, giftPresentation } from "../order-target";
export function OrderDetails({
  selected,
  options,
  names,
  collectionName,
  collectionSlug,
  collectionPreview,
  removed = false,
  close,
  share,
}: {
  selected: Order;
  options: ReadonlyMap<string, CatalogOption>;
  names: ReadonlyMap<string, string>;
  collectionName: string;
  collectionSlug?: string | null;
  collectionPreview?: GiftMedia;
  removed?: boolean;
  close: () => void;
  share?: (slug: string, currency: string) => void;
}) {
  useBackgroundPlaybackSuspension(true);
  const state = useOrderValuation(selected);
  const criteria = criteriaPresentation(selected);
  const selectedSerial = serial(selected),
    model = giftPresentation(selected) ? modelName(selected, options, "") : "",
    marketLink = listingLink(selected),
    venue = names.get(selected.book.venue_id) || selected.book.venue_id;
  const telegramLink =
    selectedSerial &&
    collectionSlug &&
    /^[A-Za-z0-9]{1,128}$/.test(collectionSlug)
      ? `https://t.me/nft/${collectionSlug}-${selectedSerial}`
      : undefined;
  const actions = [
    ...(marketLink && !removed && marketLink !== telegramLink
      ? [
          {
            kind: "market",
            url: marketLink,
            name: venue,
            label: `Open on ${venue}`,
          },
        ]
      : []),
    ...(telegramLink
      ? [
          {
            kind: "telegram",
            url: telegramLink,
            name: "Telegram",
            label: "Open in Telegram",
          },
        ]
      : []),
  ];
  const priceLabel =
    selected.kind === "executed"
      ? "Execution price"
      : selected.kind === "bid"
        ? "Bid price"
        : "Asking price";
  const caption = (
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
  );
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
        <div className="detail-art-meta">
          {(selected.kind === "bid" || selected.quantity !== 1) && (
            <span
              className="detail-quantity"
              aria-label={`Quantity: ${selected.quantity.toLocaleString()}`}
              title={`${selected.quantity.toLocaleString()} gift${selected.quantity === 1 ? "" : "s"}`}
            >
              ×{selected.quantity.toLocaleString()}
            </span>
          )}
          <span
            className="detail-venue"
            role="img"
            aria-label={`Venue: ${venue}`}
            title={venue}
          >
            <VenueIcon name={venue} />
          </span>
        </div>
        {criteria ? (
          <div className="bid-detail-preview">
            <BidPreview
              criteria={criteria}
              options={options}
              names={names}
              compact={false}
              caption={caption}
            />
          </div>
        ) : (
          <GiftPreview
            order={selected}
            options={options}
            fallback={collectionName[0] || "◇"}
            fallbackMedia={
              selected.kind === "executed" &&
              selected.target.kind === "criteria"
                ? collectionPreview
                : undefined
            }
            size="large"
            playback="detail"
          />
        )}
        {!criteria && caption}
      </div>
      <div className="detail-content">
        <div className="detail-body">
          <div className="detail-pricing">
            <section className="listing-price" aria-label={priceLabel}>
              <div className="listing-price-row">
                <div className="listing-price-label">
                  <h3>
                    {selected.kind === "executed"
                      ? "Paid"
                      : selected.kind === "bid"
                        ? "Bid price"
                        : "Ask price"}
                  </h3>
                  {selected.kind === "executed" && selected.executed_at && (
                    <ExecutionTime
                      key={`${state.key}:${selected.executed_at}`}
                      at={selected.executed_at}
                    />
                  )}
                </div>
                <div className="detail-price">
                  <Quote
                    price={selected.price}
                    currency={selected.book.currency_id}
                  />
                  {criteria && (
                    <small className="detail-price-unit">/ gift</small>
                  )}
                </div>
              </div>
            </section>
            <OrderValuationSummary
              state={state}
              criteria={!!criteria}
              history={selected.kind === "executed"}
              price={
                !removed && selected.kind === "ask" ? selected.price : undefined
              }
              currency={selected.book.currency_id}
            />
          </div>
          <OrderAttributes
            key={state.key}
            order={selected}
            options={options}
            names={names}
            collectionPreview={collectionPreview}
          />
          {removed && (
            <p className="listing-gone" role="status">
              No longer in these results.
            </p>
          )}
        </div>
        {actions.length > 0 && (
          <div className="detail-actions">
            {share && collectionSlug && selectedSerial && (
              <button
                className="detail-link detail-share"
                onClick={() =>
                  share(
                    `${collectionSlug}-${selectedSerial}`,
                    selected.book.currency_id,
                  )
                }
              >
                Share valuation ↗
              </button>
            )}
            {actions.map((action) => (
              <a
                key={action.kind}
                className={`detail-link detail-${action.kind}`}
                href={action.url}
                target="_blank"
                rel="noreferrer"
                aria-label={action.label}
                title={action.label}
              >
                <VenueIcon name={action.name} />
                <span className="detail-link-label">{action.name}</span>
                <span aria-hidden="true">↗</span>
              </a>
            ))}
          </div>
        )}
      </div>
    </Modal>
  );
}
