import type { Order, CatalogOption, TraitFilters } from "./market.ts";
import { resolveTraits } from "./catalog/traits.ts";
import { giftPresentation, type PreviewOrder } from "./order-target.ts";
import { formatGiftNumber } from "./amount.ts";
export { traits } from "./catalog/traits.ts";
export type { Collection, CatalogOption } from "./market.ts";
export function serial(order: PreviewOrder): string | undefined {
  const number = giftPresentation(order)?.number;
  return number != null ? formatGiftNumber(number) : undefined;
}
export function modelName(
  order: PreviewOrder,
  options: ReadonlyMap<string, CatalogOption>,
  fallback: string,
) {
  if (order.target.kind === "unsupported") return "Unsupported conditions";
  const models =
    order.target.kind === "gift"
      ? order.target.traits.model
        ? [order.target.traits.model]
        : []
      : order.target.kind === "criteria"
        ? order.target.models
        : [];
  const model = models.length === 1 ? options.get(models[0]) : undefined;
  return model?.kind === "model" ? model.name : fallback;
}
export function listingLink(order: Order): string | undefined {
  if (!order.market_url) return;
  try {
    const url = new URL(order.market_url);
    if (
      ["http:", "https:"].includes(url.protocol) &&
      !url.username &&
      !url.password
    )
      return url.href;
  } catch {}
}

export type BidCriteria = Extract<Order["target"], { kind: "criteria" }>;
export function giftNumberRange(range: BidCriteria["number"]) {
  if (!range || (range.min == null && range.max == null)) return;
  const bound = (value: number | null | undefined) =>
    value == null ? "Any" : formatGiftNumber(value);
  return range.min === range.max
    ? bound(range.min)
    : `${bound(range.min)} – ${bound(range.max)}`;
}

export type GiftDetail = {
  label: string;
  value: string;
  option?: CatalogOption;
  attribute?: CatalogOption["kind"];
};

/** Resolve labels and single-trait visuals together, without reconstructing selections in the UI. */
export function giftDetails(
  order: PreviewOrder,
  names: ReadonlyMap<string, string>,
  options?: ReadonlyMap<string, CatalogOption>,
): GiftDetail[] {
  const target = order.target;
  if (target.kind === "unsupported")
    return [{ label: "Conditions", value: target.message }];
  const rows: GiftDetail[] = [];
  const gift = giftPresentation(order);
  const selection: TraitFilters =
    !gift && target.kind === "criteria"
      ? target
      : {
          models: gift?.traits.model ? [gift.traits.model] : [],
          backdrops: gift?.traits.backdrop ? [gift.traits.backdrop] : [],
          patterns: gift?.traits.pattern ? [gift.traits.pattern] : [],
        };
  for (const trait of resolveTraits(selection, options ?? new Map(), names)) {
    if (gift && !trait.entries.length) continue;
    const option =
      trait.entries.length === 1 ? trait.entries[0].option : undefined;
    rows.push({
      label: trait.label,
      attribute: trait.kind,
      value: trait.entries.length
        ? trait.entries.map((entry) => entry.name).join(" / ")
        : "Any",
      ...(option?.kind === trait.kind ? { option } : {}),
    });
  }
  const criteria = !gift && target.kind === "criteria" ? target : undefined;
  const number = giftNumberRange(criteria?.number);
  if (number) rows.push({ label: "Gift #", value: number });
  return rows;
}
