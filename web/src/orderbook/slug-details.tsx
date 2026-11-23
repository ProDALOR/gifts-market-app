import { useEffect, useMemo } from "react";
import type { CatalogOption, GiftMedia } from "../market";
import type { ValuationSubject } from "../order-target";
import { GiftArtwork } from "../previews/gift-artwork";
import { GiftPreview } from "../previews/gift-preview";
import { useBackgroundPlaybackSuspension } from "../previews/use-playback";
import { useOrderValuation } from "../valuation/use-order-valuation";
import { OrderValuationSummary } from "./order-valuation";
import { OrderAttributes } from "./order-attributes";
import { Modal } from "../ui/modal";

/** Slug previews share artwork, floors and valuation lifecycle without inventing an order. */
export function SlugDetails({
  slug,
  collection,
  currency,
  options,
  names,
  preview,
  onGift,
  close,
  share,
}: {
  slug: string;
  collection: string;
  currency: string;
  options: ReadonlyMap<string, CatalogOption>;
  names: ReadonlyMap<string, string>;
  preview?: GiftMedia;
  onGift: (gift: ValuationSubject | undefined) => void;
  close: () => void;
  share?: (slug: string, currency: string) => void;
}) {
  useBackgroundPlaybackSuspension(true);
  const state = useOrderValuation({ slug, collection, currency });
  const resolved = state.report?.target;
  const gift = resolved?.kind === "gift" ? resolved.gift : undefined;
  const subject = useMemo<ValuationSubject | undefined>(
    () =>
      gift
        ? {
            book: { collection_id: gift.collection_id, currency_id: currency },
            target: {
              kind: "gift",
              id: gift.id,
              number: gift.number,
              traits: gift.traits,
            },
          }
        : undefined,
    [gift, currency],
  );
  useEffect(() => {
    onGift(subject);
    return () => onGift(undefined);
  }, [subject, onGift]);
  const model = gift?.traits.model ? options.get(gift.traits.model) : undefined;
  return (
    <Modal
      className="detail"
      label="Gift details"
      closeLabel="Close details"
      viewportClose
      describedBy="detail-title"
      close={close}
    >
      <div className="detail-art">
        {subject ? (
          <GiftPreview
            order={subject}
            options={options}
            fallbackMedia={preview}
            fallback="◇"
            size="large"
            playback="detail"
          />
        ) : (
          <GiftArtwork
            media={preview}
            fallback="◇"
            size="large"
            playback="static"
          />
        )}
        <div className="detail-caption">
          <h2 id="detail-title">
            {names.get(collection) || slug.split("-")[0]}{" "}
            {gift?.number != null && (
              <span>#{gift.number.toLocaleString()}</span>
            )}
          </h2>
          {model && <p>{model.name}</p>}
        </div>
      </div>
      <div className="detail-content">
        <div className="detail-body">
          <div className="detail-pricing">
            <OrderValuationSummary state={state} />
          </div>
          {subject && (
            <OrderAttributes
              order={subject}
              options={options}
              names={names}
              collectionPreview={preview}
            />
          )}
        </div>
        <div className="detail-actions">
          {share && (
            <button
              className="detail-link detail-share"
              onClick={() => share(slug, currency)}
            >
              Share valuation ↗
            </button>
          )}
          <a
            className="detail-link detail-telegram"
            href={`https://t.me/nft/${encodeURIComponent(slug)}`}
            target="_blank"
            rel="noreferrer"
          >
            Telegram ↗
          </a>
        </div>
      </div>
    </Modal>
  );
}
