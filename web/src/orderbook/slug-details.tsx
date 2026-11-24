import { useEffect, useMemo } from "react";
import type { CatalogOption, GiftMedia } from "../market";
import type { ValuationSubject } from "../order-target";
import { useOrderValuation } from "../valuation/use-order-valuation";
import { GiftDetailsView } from "../previews/gift-details";

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
  return (
    <GiftDetailsView
      subject={subject}
      valuation={state}
      collectionId={gift?.collection_id || collection}
      currency={currency}
      collectionName={slug.split("-")[0]}
      options={options}
      names={names}
      collectionPreview={preview}
      slug={slug}
      close={close}
      share={share}
    />
  );
}
