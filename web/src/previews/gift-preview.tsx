import { useMemo } from "react";
import type { ComponentProps } from "react";
import type { Order } from "../market";
import { giftAppearance } from "./gift-appearance";
import type { GiftOption } from "./gift-appearance";
import { GiftArtwork } from "./gift-artwork";

/** Adapt market orders at the boundary; artwork itself has no order dependency. */
export function GiftPreview({
  order,
  options,
  ...props
}: {
  order: Pick<Order, "target">;
  options: ReadonlyMap<string, GiftOption>;
} & Omit<
  ComponentProps<typeof GiftArtwork>,
  "media" | "palette" | "patternUrls" | "label"
>) {
  const appearance = giftAppearance(order, options);
  const palette = useMemo(
    () => appearance.palette,
    [
      appearance.palette?.center,
      appearance.palette?.edge,
      appearance.palette?.pattern,
    ],
  );
  const patternUrl = appearance.patternUrls[0];
  const patternUrls = useMemo(
    () => (patternUrl ? [patternUrl] : []),
    [patternUrl],
  );
  return (
    <GiftArtwork
      {...props}
      media={appearance.media}
      palette={palette}
      patternUrls={patternUrls}
      label={appearance.label}
    />
  );
}
