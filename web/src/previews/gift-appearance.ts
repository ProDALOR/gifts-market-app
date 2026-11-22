import type { CatalogOption, BackdropPalette } from "../market.ts";
import { modelMedia } from "../market.ts";
import { giftPresentation, type PreviewOrder } from "../order-target.ts";
export type GiftOption = CatalogOption;
export type { BackdropPalette } from "../market.ts";
export function backdropPalette(
  backdrop?: GiftOption,
): BackdropPalette | undefined {
  return backdrop?.kind === "backdrop"
    ? backdrop.palette || undefined
    : undefined;
}
/** Use known gift/execution traits; open criteria orders keep their bid composition. */
export function giftAppearance(
  order: PreviewOrder,
  options: ReadonlyMap<string, GiftOption>,
) {
  const media = modelMedia(order, options);
  const selected = giftPresentation(order)?.traits;
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
