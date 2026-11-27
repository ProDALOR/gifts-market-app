import type { CatalogOption } from "../market";
import { memo, useEffect, useRef, useState } from "react";
import type { RefObject } from "react";
import { ArtworkImage } from "./artwork-image";
import { useArtworkImage } from "./use-artwork-image";
import { GiftPlaceholder } from "./gift-placeholder";
import { useDisplayMode } from "../app/preferences";
import { artworkPolicy } from "./artwork-policy";
import {
  usePreviewVisibility,
  usePreviewEnvironment,
} from "./preview-environment";

/** Static catalog visuals shared by pickers and order conditions. */
type OptionProps = {
  option: CatalogOption;
  visibilityRoot?: RefObject<HTMLElement | null>;
  active?: boolean;
};
export const OptionArt = memo(function OptionArt({
  option,
  visibilityRoot,
  active = true,
}: OptionProps) {
  if (option.kind === "backdrop") {
    const colors = option.palette;
    return (
      <span
        className="option-art swatch"
        style={
          colors
            ? {
                background: `radial-gradient(ellipse at 40% 30%,${colors.center},${colors.edge})`,
              }
            : undefined
        }
      />
    );
  }
  return (
    <OptionImage
      option={option}
      visibilityRoot={visibilityRoot}
      active={active}
    />
  );
});

/** Mount native images only after visibility admission; retain them for scroll-back reuse. */
function OptionImage({
  option,
  visibilityRoot,
  active = true,
}: {
  option: Exclude<CatalogOption, { kind: "backdrop" }>;
} & Omit<OptionProps, "option">) {
  const host = useRef<HTMLSpanElement>(null);
  const visible = usePreviewVisibility(
    host,
    visibilityRoot ? 96 : 256,
    active,
    visibilityRoot,
  );
  const { pageVisible } = usePreviewEnvironment();
  const mode = useDisplayMode();
  const format = artworkPolicy(mode, "item", "static").format;
  const media = option.media;
  const preferred =
    format === "thumbnail"
      ? media?.thumbnail_url
      : (media?.preview_url ?? media?.thumbnail_url);
  const [requested, setRequested] = useState(false);
  useEffect(() => {
    if (visible && pageVisible) setRequested(true);
  }, [visible, pageVisible]);
  return (
    <span
      ref={host}
      className={`option-art${option.kind === "pattern" ? " symbol-thumb" : ""}`}
      aria-hidden="true"
    >
      {requested && (
        <OptionStill
          key={JSON.stringify([preferred, media?.thumbnail_url])}
          preferred={preferred}
          fallback={media?.thumbnail_url}
          label={option.name}
          active={visible && pageVisible}
          placeholderKind={option.kind === "pattern" ? "symbol" : "gift"}
        />
      )}
    </span>
  );
}

/** Static icons share image recovery and decoding without allocating playback state or another observer. */
function OptionStill({
  preferred,
  fallback,
  label,
  active,
  placeholderKind,
}: {
  preferred?: string | null;
  fallback?: string | null;
  label: string;
  active: boolean;
  placeholderKind: "gift" | "symbol";
}) {
  const image = useArtworkImage(
    preferred ?? undefined,
    fallback ?? undefined,
    active,
  );
  return (
    <span className="item-icon gift-preview" role="img" aria-label={label}>
      {!image.ready && (
        <GiftPlaceholder
          kind={placeholderKind}
          active={active}
          loading={image.loading}
        />
      )}
      {image.sources.map((source) => (
        <ArtworkImage
          key={`${source.url}:${source.attempt}`}
          source={source}
          eager={active}
          priority="auto"
        />
      ))}
    </span>
  );
}
