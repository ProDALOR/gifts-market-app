import type { Order, CatalogOption } from "./market.ts";
export type { Collection, CatalogOption } from "./market.ts";
export const traits = [
  { id: "models", label: "Model", kind: "model" },
  {
    id: "backdrops",
    label: "Backdrop",
    kind: "backdrop",
  },
  { id: "patterns", label: "Symbol", kind: "pattern" },
] as const;
export function serial(order: Order): string | undefined {
  return order.target.kind === "gift" && order.target.number != null
    ? String(order.target.number)
    : undefined;
}
export function modelName(
  order: Order,
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
  return models.length === 1
    ? options.get(models[0])?.name || fallback
    : fallback;
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
  return range.min === range.max
    ? String(range.min)
    : `${range.min ?? "Any"} – ${range.max ?? "Any"}`;
}

export type GiftDetail = {
  label: string;
  value: string;
  option?: CatalogOption;
  attribute?: CatalogOption["kind"];
};

/** Resolve labels and single-trait visuals together, without reconstructing selections in the UI. */
export function giftDetails(
  order: Order,
  names: ReadonlyMap<string, string>,
  options?: ReadonlyMap<string, CatalogOption>,
): GiftDetail[] {
  const target = order.target;
  if (target.kind === "unsupported")
    return [{ label: "Conditions", value: target.message }];
  const rows: GiftDetail[] = [];
  for (const trait of traits) {
    const id = target.kind === "gift" ? target.traits[trait.kind] : undefined;
    const ids = target.kind === "criteria" ? target[trait.id] : id ? [id] : [];
    if (target.kind === "gift" && !ids.length) continue;
    const option = ids.length === 1 ? options?.get(ids[0]) : undefined;
    rows.push({
      label: trait.label,
      attribute: trait.kind,
      value: ids.length
        ? ids.map((id) => names.get(id) || "Unavailable").join(" / ")
        : "Any",
      ...(option?.kind === trait.kind ? { option } : {}),
    });
  }
  const criteria = target.kind === "criteria" ? target : undefined;
  const number = giftNumberRange(criteria?.number);
  if (number) rows.push({ label: "Gift #", value: number });
  if (target.kind === "criteria" && order.quantity > 1)
    rows.push({ label: "Quantity", value: String(order.quantity) });
  return rows;
}
