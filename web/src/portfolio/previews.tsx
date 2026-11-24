import { memo } from "react";
import type { CatalogOption, GiftMedia } from "../market";
import type { ValuationSubject } from "../order-target";
import { portfolioRelation, type PortfolioItem } from "./model";
import { GiftCardView } from "../previews/gift-card";
import { GiftDetailsView } from "../previews/gift-details";
import { useOrderValuation } from "../valuation/use-order-valuation";
export function portfolioSubject(
  item: PortfolioItem,
  currency: string,
): ValuationSubject | undefined {
  const g = item.resolved;
  return g
    ? {
        book: { collection_id: g.collection_id, currency_id: currency },
        target: { kind: "gift", id: g.id, number: g.number, traits: g.traits },
      }
    : undefined;
}
function modelMedia(item: PortfolioItem): GiftMedia | undefined {
  const id = item.resolved?.traits.model;
  if (!id) return;
  const base = `/media/models/${encodeURIComponent(id)}`;
  return {
    thumbnail_url: `${base}/thumbnail`,
    preview_url: `${base}/preview`,
    animation_url: `${base}/animation`,
  };
}
export function PortfolioDetails({
  item,
  currency,
  options,
  names,
  close,
  share,
}: {
  item: PortfolioItem;
  currency: string;
  options: ReadonlyMap<string, CatalogOption>;
  names: ReadonlyMap<string, string>;
  close: () => void;
  share?: (slug: string, currency: string) => void;
}) {
  const gift = portfolioSubject(item, currency)!;
  const valuation = useOrderValuation(gift);
  const relation = portfolioRelation(item.gift.relation);
  return (
    <GiftDetailsView
      subject={gift}
      valuation={valuation}
      notice={
        relation && (
          <p className="portfolio-ownership-note">{relation.description}</p>
        )
      }
      collectionId={gift.book.collection_id}
      currency={currency}
      collectionName={item.gift.gift.slug.split("-")[0]}
      options={options}
      names={names}
      collectionPreview={modelMedia(item)}
      slug={item.gift.gift.slug}
      close={close}
      share={share}
    />
  );
}

/** Progress frames do not rebuild unchanged cards or their artwork props. */
export const PortfolioGiftCard = memo(function PortfolioGiftCard({
  item,
  currency,
  options,
  names,
  selected,
  select,
  position,
  total,
}: {
  item: PortfolioItem;
  currency: string;
  options: ReadonlyMap<string, CatalogOption>;
  names: ReadonlyMap<string, string>;
  selected: boolean;
  select: (id: string) => void;
  position: number;
  total: number;
}) {
  const gift = portfolioSubject(item, currency);
  const relation = portfolioRelation(item.gift.relation);
  return (
    <GiftCardView
      subject={gift}
      options={options}
      collectionName={
        names.get(item.resolved?.collection_id || "") ||
        item.gift.gift.slug.split("-")[0]
      }
      media={modelMedia(item)}
      identity={`#${item.gift.gift.number.toLocaleString()}`}
      meta={
        relation && (
          <span
            className="portfolio-relation"
            title={relation.description}
            aria-label={relation.description}
          >
            {relation.label}
          </span>
        )
      }
      price={item.estimate ?? undefined}
      currency={currency}
      format="rounded"
      position={position}
      total={total}
      className="portfolio-card"
      selected={selected}
      action={
        gift ? (
          <button
            className="card-open"
            aria-label={`View ${item.gift.gift.slug}`}
            title={relation?.description}
            onClick={() => select(item.gift.gift.id)}
            aria-haspopup="dialog"
          />
        ) : (
          <a
            className="card-open"
            aria-label={`Open ${item.gift.gift.slug} in Telegram`}
            href={`https://t.me/nft/${encodeURIComponent(item.gift.gift.slug)}`}
            target="_blank"
            rel="noreferrer"
          />
        )
      }
    />
  );
});
