import type { CSSProperties, ReactNode } from "react";
import type { Order, GiftMedia } from "../market";
import type { CatalogOption } from "../orderbook";
import { listingLink, modelName, serial } from "../orderbook";
import { GiftPreview } from "../previews/gift-preview";
import { GiftArtwork } from "../previews/gift-artwork";
import type { ValuationSubject } from "../order-target";
import { Modal } from "../ui/modal";
import { GiftPricing } from "./gift-pricing";
import { ShareIcon, VenueIcon } from "../ui/icons";
import { OrderAttributes } from "../orderbook/order-attributes";
import { BidPreview } from "../orderbook/bid-preview";
import { useOrderValuation } from "../valuation/use-order-valuation";
import { useBackgroundPlaybackSuspension } from "../previews/use-playback";
import { criteriaPresentation, giftPresentation } from "../order-target";
import { backdropPalette, giftAppearance } from "./gift-appearance";
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
  giftPreview,
  removed = false,
  notice,
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
  /** Exact gift artwork can load before the trait catalog without changing the collection icon. */
  giftPreview?: GiftMedia;
  removed?: boolean;
  notice?: ReactNode;
  close: () => void;
  share?: (slug: string, currency: string) => void;
  label?: string;
}) {
  useBackgroundPlaybackSuspension(true);
  const state = valuation;
  const selected = subject;
  const criteria = selected && criteriaPresentation(selected);
  const palette =
    selected && !criteria
      ? giftAppearance(selected, options).palette
      : criteria?.backdrops.length === 1
        ? backdropPalette(options.get(criteria.backdrops[0]))
        : undefined;
  const backdropStyle = palette
    ? ({ "--gift-backdrop-edge": palette.edge } as CSSProperties)
    : undefined;
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
      ? `${collectionSlug}-${giftPresentation(selected!)?.number}`
      : undefined);
  const telegramLink = telegramSlug
    ? `https://t.me/nft/${encodeURIComponent(telegramSlug)}`
    : undefined;
  const actions = [
    ...(marketLink && !removed && marketLink !== telegramLink
      ? [
          {
            kind: "market" as const,
            url: marketLink,
            name: venue,
            label: `Open on ${venue}`,
          },
        ]
      : []),
    ...(share && telegramSlug
      ? [{ kind: "share" as const, run: () => share(telegramSlug, currency) }]
      : []),
    ...(telegramLink
      ? [
          {
            kind: "telegram" as const,
            url: telegramLink,
            name: "Telegram",
            label: "Open in Telegram",
          },
        ]
      : []),
  ];
  const caption = (
    <div className="detail-caption">
      <h2 id="detail-title">
        {names.get(collectionId) || collectionName}
        {selectedSerial && (
          <>
            {" "}
            <span>#{selectedSerial}</span>
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
      swipeToClose
      scrollKey={slug ?? JSON.stringify([collectionId, selected?.target])}
      describedBy="detail-title"
    >
      <div
        className={`detail-art${criteria ? " detail-bid-art" : ""}${palette && !criteria ? " with-backdrop" : ""}`}
        style={backdropStyle}
      >
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
            fallbackMedia={giftPreview ?? collectionPreview}
            size="large"
            playback="detail"
          />
        ) : (
          <GiftArtwork
            media={collectionPreview}
            fallback="◇"
            size="large"
            playback="detail"
          />
        )}
        {!criteria && caption}
      </div>
      <div className="detail-content">
        <div className="detail-body">
          <div
            className={`detail-valuation${palette ? " with-backdrop" : ""}`}
            style={backdropStyle}
          >
            {notice}
            <GiftPricing
              order={order}
              state={state}
              currency={currency}
              criteria={!!criteria}
              removed={removed}
            />
          </div>
          {selected && (
            <OrderAttributes
              key={state.key}
              order={selected}
              options={options}
              names={names}
              collectionPreview={collectionPreview}
              valuation={state}
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
            {actions.map((action) =>
              action.kind === "share" ? (
                <button
                  key={action.kind}
                  className="detail-link detail-share"
                  onClick={action.run}
                >
                  <ShareIcon />
                  <span className="detail-link-label">Share</span>
                </button>
              ) : (
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
              ),
            )}
          </div>
        )}
      </div>
    </Modal>
  );
}
