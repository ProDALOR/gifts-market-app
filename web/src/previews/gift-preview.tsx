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
  "media" | "palette" | "patternUrl" | "label"
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
  return (
    <GiftArtwork
      {...props}
      key={`${appearance.media?.thumbnail_url}|${appearance.media?.animation_url}`}
      media={appearance.media}
      palette={palette}
      patternUrl={appearance.patternUrl}
      label={appearance.label}
    />
  );
}
