import type { Entry, Order } from "./market.ts";
import { modelThumbnailUrl } from "./market.ts";
export type GiftOption = Extract<Entry, { kind: "attribute_option" }>;
export type BackdropPalette = { center: string; edge: string; pattern: string };
function exactOption(
  order: Pick<Order, "attributes">,
  attribute: string,
): string | undefined {
  const predicate = order.attributes[attribute];
  return predicate?.op === "eq" && predicate.value.type === "enum"
    ? predicate.value.value
    : undefined;
}
function color(value: string | undefined): string | undefined {
  return value && /^#[\da-f]{6}$/i.test(value)
    ? value.toLowerCase()
    : undefined;
}
export function backdropPalette(
  backdrop?: GiftOption,
): BackdropPalette | undefined {
  if (backdrop?.attribute_id !== "tg.backdrop") return;
  const center = color(backdrop.meta["colors.center_color"]);
  const edge = color(backdrop.meta["colors.edge_color"]);
  const pattern = color(backdrop.meta["colors.pattern_color"]);
  return center && edge && pattern ? { center, edge, pattern } : undefined;
}
/** Compose only concrete catalog attributes; conditional bids never invent gift artwork. */
export function giftAppearance(
  order: Pick<Order, "attributes">,
  options: ReadonlyMap<string, GiftOption>,
) {
  const modelUrl = modelThumbnailUrl(order);
  const backdropId = exactOption(order, "tg.backdrop");
  const patternId = exactOption(order, "tg.pattern");
  const backdrop = backdropId ? options.get(backdropId) : undefined;
  const pattern = patternId ? options.get(patternId) : undefined;
  const palette = modelUrl ? backdropPalette(backdrop) : undefined;
  const patternUrl =
    palette && pattern?.attribute_id === "tg.pattern"
      ? `/media/patterns/${encodeURIComponent(pattern.id)}`
      : undefined;
  return {
    modelUrl,
    palette,
    patternUrl,
    label: [
      "Gift preview",
      ...(palette ? [`${backdrop!.name} backdrop`] : []),
      ...(patternUrl ? [`${pattern!.name} pattern`] : []),
    ].join(", "),
  };
}
