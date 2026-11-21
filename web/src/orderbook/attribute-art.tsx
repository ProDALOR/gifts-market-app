import type { CatalogOption, GiftMedia } from "../market";
import { OptionArt } from "../previews/option-art";
import { GiftArtwork } from "../previews/gift-artwork";
import { GiftPlaceholder } from "../previews/gift-placeholder";
export type AttributeArtKind = CatalogOption["kind"] | "collection" | "pair";

/** Static attribute artwork shares the preview cache; no animation or pattern renderer. */
export function AttributeArt({
  kind,
  option,
  model,
  backdrop,
  collectionPreview,
}: {
  kind?: AttributeArtKind;
  option?: CatalogOption;
  model?: CatalogOption;
  backdrop?: CatalogOption;
  collectionPreview?: GiftMedia;
}) {
  if (option) return <OptionArt option={option} />;
  if (kind === "collection")
    return collectionPreview ? (
      <GiftArtwork media={collectionPreview} fallback="" playback="static" />
    ) : (
      <GiftPlaceholder active={false} loading={false} />
    );
  if (kind === "pair")
    return (
      <>
        {backdrop?.kind === "backdrop" ? (
          <OptionArt option={backdrop} />
        ) : (
          <span className="option-art swatch" />
        )}
        <span className="attribute-pair-model">
          {model?.kind === "model" ? (
            <OptionArt option={model} />
          ) : (
            <GiftPlaceholder active={false} loading={false} />
          )}
        </span>
      </>
    );
  return null;
}
