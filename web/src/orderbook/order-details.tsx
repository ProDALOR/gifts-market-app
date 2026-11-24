import type { Order, GiftMedia } from "../market";
import type { CatalogOption } from "../orderbook";
import { useOrderValuation } from "../valuation/use-order-valuation";
import { GiftDetailsView } from "../previews/gift-details";
export function OrderDetails({
  selected,
  ...props
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
  const valuation = useOrderValuation(selected);
  return (
    <GiftDetailsView
      {...props}
      subject={selected}
      order={selected}
      valuation={valuation}
      collectionId={selected.book.collection_id}
      currency={selected.book.currency_id}
      label="Order details"
    />
  );
}
