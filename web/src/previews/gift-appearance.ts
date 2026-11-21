import type { Order, CatalogOption, BackdropPalette } from "../market.ts";
import { modelMedia } from "../market.ts";
export type GiftOption = CatalogOption;
export type { BackdropPalette } from "../market.ts";
export function backdropPalette(
  backdrop?: GiftOption,
): BackdropPalette | undefined {
  return backdrop?.kind === "backdrop"
    ? backdrop.palette || undefined
    : undefined;
}
/** Catalog colors are validated by the gift projection; only concrete gifts get artwork. */
export function giftAppearance(
  order: Pick<Order, "target">,
  options: ReadonlyMap<string, GiftOption>,
) {
  const media = modelMedia(order, options);
  const selected =
    order.target.kind === "gift" ? order.target.traits : undefined;
  const backdrop = selected?.backdrop
    ? options.get(selected.backdrop)
    : undefined;
  const pattern = selected?.pattern ? options.get(selected.pattern) : undefined;
  const palette = backdropPalette(backdrop);
  const patternUrl =
    palette && pattern?.kind === "pattern"
      ? pattern.media?.thumbnail_url
      : undefined;
  return {
    media,
    palette,
    patternUrls: patternUrl ? [patternUrl] : [],
    label: [
      "Gift preview",
      ...(palette ? [`${backdrop!.name} backdrop`] : []),
      ...(patternUrl ? [`${pattern!.name} pattern`] : []),
    ].join(", "),
  };
}
