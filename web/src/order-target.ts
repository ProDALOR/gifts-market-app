import type { Order } from "./market.ts";

type Gift = Extract<Order["target"], { kind: "gift" }>;
export type PreviewOrder = Pick<Order, "target"> & Partial<Pick<Order, "kind">>;
const single = (ids: readonly string[]) =>
  ids.length === 1 ? ids[0] : undefined;

/** An execution uses gift visuals, but criteria never supply a fabricated identity. */
export function giftPresentation(
  order: PreviewOrder,
): Pick<Gift, "traits" | "number"> | undefined {
  if (order.target.kind === "gift") return order.target;
  if (order.kind !== "executed" || order.target.kind !== "criteria") return;
  const { models, backdrops, patterns, number } = order.target;
  return {
    traits: {
      model: single(models),
      backdrop: single(backdrops),
      pattern: single(patterns),
    },
    number:
      number?.min != null && number.min === number.max ? number.min : undefined,
  };
}

export function criteriaPresentation(order: PreviewOrder) {
  return order.kind !== "executed" && order.target.kind === "criteria"
    ? order.target
    : undefined;
}
