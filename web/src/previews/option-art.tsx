import type { CatalogOption } from "../market";
import { GiftArtwork } from "./gift-artwork";
import { useEffect, useRef, useState } from "react";
import {
  usePreviewVisibility,
  usePreviewEnvironment,
} from "./preview-environment";

/** Static catalog visuals shared by pickers and order conditions. */
export function OptionArt({ option }: { option: CatalogOption }) {
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
  return <OptionImage option={option} />;
}

/** Mount native images only after visibility admission; retain them for scroll-back reuse. */
function OptionImage({
  option,
}: {
  option: Exclude<CatalogOption, { kind: "backdrop" }>;
}) {
  const host = useRef<HTMLSpanElement>(null);
  const visible = usePreviewVisibility(host);
  const { pageVisible } = usePreviewEnvironment();
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
        <GiftArtwork
          media={option.media || undefined}
          fallback=""
          label={option.name}
          playback="static"
          placeholderKind={option.kind === "pattern" ? "symbol" : "gift"}
        />
      )}
    </span>
  );
}
