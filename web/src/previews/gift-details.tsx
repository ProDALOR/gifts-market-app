import type { Order, GiftMedia } from "../market";
import type { CatalogOption } from "../orderbook";
import { listingLink, modelName, serial } from "../orderbook";
import { GiftPreview } from "../previews/gift-preview";
import { GiftArtwork } from "../previews/gift-artwork";
import type { ValuationSubject } from "../order-target";
import { Modal } from "../ui/modal";
import { OrderValuationSummary } from "../orderbook/order-valuation";
import { Quote } from "../ui/market-controls";
import { VenueIcon } from "../ui/icons";
import { OrderAttributes } from "../orderbook/order-attributes";
import { BidPreview } from "../orderbook/bid-preview";
import { useOrderValuation } from "../valuation/use-order-valuation";
import { useBackgroundPlaybackSuspension } from "../previews/use-playback";
import { ExecutionTime } from "../orderbook/execution-time";
import { criteriaPresentation, giftPresentation } from "../order-target";
export function GiftDetailsView({
  subject,
  order,
  valuation,
  options,
  names,
  collectionName,
  collectionId,
  currency,
  collectionSlug,
  slug,
  collectionPreview,
  removed = false,
  close,
  share,
  label = "Gift details",
}: {
  subject?: ValuationSubject;
  order?: Order;
  valuation: ReturnType<typeof useOrderValuation>;
  options: ReadonlyMap<string, CatalogOption>;
  names: ReadonlyMap<string, string>;
  collectionName: string;
  collectionId: string;
  currency: string;
  collectionSlug?: string | null;
  slug?: string;
  collectionPreview?: GiftMedia;
  removed?: boolean;
  close: () => void;
  share?: (slug: string, currency: string) => void;
  label?: string;
}) {
  useBackgroundPlaybackSuspension(true);
  const state = valuation;
  const selected = subject;
  const criteria = selected && criteriaPresentation(selected);
  const selectedSerial = selected ? serial(selected) : undefined,
    model =
      selected && giftPresentation(selected)
        ? modelName(selected, options, "")
        : "",
    marketLink = order ? listingLink(order) : undefined,
    venue = order ? names.get(order.book.venue_id) || order.book.venue_id : "";
  const telegramSlug =
    slug ||
    (selectedSerial &&
    collectionSlug &&
    /^[A-Za-z0-9]{1,128}$/.test(collectionSlug)
      ? `${collectionSlug}-${selectedSerial}`
      : undefined);
  const telegramLink = telegramSlug
    ? `https://t.me/nft/${encodeURIComponent(telegramSlug)}`
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
    order?.kind === "executed"
      ? "Execution price"
      : order?.kind === "bid"
        ? "Bid price"
        : "Asking price";
  const caption = (
    <div className="detail-caption">
      <h2 id="detail-title">
        {names.get(collectionId) || collectionName}
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
      label={label}
      closeLabel="Close details"
      viewportClose
      describedBy="detail-title"
    >
      <div className={`detail-art${criteria ? " detail-bid-art" : ""}`}>
        {order && (
          <div className="detail-art-meta">
            {(order.kind === "bid" || order.quantity !== 1) && (
              <span
                className="detail-quantity"
                aria-label={`Quantity: ${order.quantity.toLocaleString()}`}
                title={`${order.quantity.toLocaleString()} gift${order.quantity === 1 ? "" : "s"}`}
              >
                ×{order.quantity.toLocaleString()}
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
        )}
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
        ) : selected ? (
          <GiftPreview
            order={selected}
            options={options}
            fallback={collectionName[0] || "◇"}
            fallbackMedia={collectionPreview}
            size="large"
            playback="detail"
          />
        ) : (
          <GiftArtwork
            media={collectionPreview}
            fallback="◇"
            size="large"
            playback="static"
          />
        )}
        {!criteria && caption}
      </div>
      <div className="detail-content">
        <div className="detail-body">
          <div className="detail-pricing">
            {order && (
              <section className="listing-price" aria-label={priceLabel}>
                <div className="listing-price-row">
                  <div className="listing-price-label">
                    <h3>
                      {order?.kind === "executed"
                        ? "Paid"
                        : order?.kind === "bid"
                          ? "Bid price"
                          : "Ask price"}
                    </h3>
                    {order?.kind === "executed" && order.executed_at && (
                      <ExecutionTime
                        key={`${state.key}:${order.executed_at}`}
                        at={order.executed_at}
                      />
                    )}
                  </div>
                  <div className="detail-price">
                    <Quote price={order.price} currency={currency} />
                    {criteria && (
                      <small className="detail-price-unit">/ gift</small>
                    )}
                  </div>
                </div>
              </section>
            )}
            <OrderValuationSummary
              state={state}
              criteria={!!criteria}
              history={order?.kind === "executed"}
              price={
                !removed && order?.kind === "ask" ? order.price : undefined
              }
              currency={currency}
            />
          </div>
          {selected && (
            <OrderAttributes
              key={state.key}
              order={selected}
              options={options}
              names={names}
              collectionPreview={collectionPreview}
            />
          )}
          {removed && (
            <p className="listing-gone" role="status">
              No longer in these results.
            </p>
          )}
        </div>
        {actions.length > 0 && (
          <div className="detail-actions">
            {share && telegramSlug && (
              <button
                className="detail-link detail-share"
                onClick={() => share(telegramSlug, currency)}
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
