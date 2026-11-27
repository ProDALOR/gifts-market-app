import type { MarketSelection } from "../market-selection.ts";
import { giftPresentation, type ValuationSubject } from "../order-target.ts";
import type { AttributeArtKind } from "./attribute-art.tsx";

/** Navigate to the displayed reference scope; unrelated traits and venue restrictions do not apply. */
export function attributeMarketSelection(
  subject: ValuationSubject,
  attribute: AttributeArtKind | undefined,
  source: "asks" | "bids",
): MarketSelection | undefined {
  if (!attribute) return;
  const traits = giftPresentation(subject)?.traits;
  if (
    ((attribute === "model" || attribute === "pair") && !traits?.model) ||
    ((attribute === "backdrop" || attribute === "pair") && !traits?.backdrop) ||
    (attribute === "pattern" && !traits?.pattern)
  )
    return;
  return {
    source,
    sort: source === "asks" ? "price_asc" : "price_desc",
    ...(source === "bids" ? { bid_match: "any" as const } : {}),
    filters: {
      collection_id: subject.book.collection_id,
      currency_id: subject.book.currency_id,
      venue_id: null,
      models:
        attribute === "model" || attribute === "pair" ? [traits!.model!] : [],
      backdrops:
        attribute === "backdrop" || attribute === "pair"
          ? [traits!.backdrop!]
          : [],
      patterns: attribute === "pattern" ? [traits!.pattern!] : [],
      number: null,
      price: null,
    },
  };
}
