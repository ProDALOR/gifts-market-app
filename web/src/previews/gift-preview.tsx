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
  order: Pick<Order, "attributes">;
  options: ReadonlyMap<string, GiftOption>;
} & Omit<
  ComponentProps<typeof GiftArtwork>,
  "url" | "palette" | "patternUrl" | "label"
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
      key={appearance.modelUrl || "none"}
      url={appearance.modelUrl}
      palette={palette}
      patternUrl={appearance.patternUrl}
      label={appearance.label}
    />
  );
}
