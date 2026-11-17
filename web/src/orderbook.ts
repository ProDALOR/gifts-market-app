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
