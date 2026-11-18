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
  return order.target.kind === "gift" && order.target.gift.number != null
    ? String(order.target.gift.number)
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
      ? order.target.gift.traits.model
        ? [order.target.gift.traits.model]
        : []
      : order.target.kind === "criteria"
        ? order.target.criteria.models
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

/** Core gift appearance and bid constraints, without redundant listing metadata. */
export function giftDetails(
  order: Order,
  names: ReadonlyMap<string, string>,
): [string, string][] {
  const target = order.target;
  if (target.kind === "unsupported")
    return [["Conditions", target.reason.message]];
  const criteria =
    target.kind === "criteria"
      ? target.criteria
      : {
          models: target.gift.traits.model ? [target.gift.traits.model] : [],
          backdrops: target.gift.traits.backdrop
            ? [target.gift.traits.backdrop]
            : [],
          patterns: target.gift.traits.pattern
            ? [target.gift.traits.pattern]
            : [],
          number: null,
        };
  const rows: [string, string][] = (
    [
      ["Model", criteria.models],
      ["Backdrop", criteria.backdrops],
      ["Symbol", criteria.patterns],
    ] as const
  )
    .filter(([, ids]) => target.kind === "criteria" || ids.length > 0)
    .map(([label, ids]) => [
      label,
      ids.length
        ? ids.map((id) => names.get(id) || "Unavailable").join(" / ")
        : target.kind === "criteria"
          ? "Any"
          : "Unknown",
    ]);
  if (
    target.kind === "criteria" &&
    criteria.number &&
    (criteria.number.min != null || criteria.number.max != null)
  )
    rows.push([
      "Gift #",
      criteria.number.min === criteria.number.max
        ? String(criteria.number.min)
        : `${criteria.number.min ?? "Any"} – ${criteria.number.max ?? "Any"}`,
    ]);
  if (target.kind === "criteria" && order.quantity > 1)
    rows.push(["Quantity", String(order.quantity)]);
  return rows;
}
