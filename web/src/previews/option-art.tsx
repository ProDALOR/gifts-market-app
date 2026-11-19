import type { CatalogOption } from "../market";
import { GiftArtwork } from "./gift-artwork";

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
  return (
    <span
      className={`option-art${option.kind === "pattern" ? " symbol-thumb" : ""}`}
      aria-hidden="true"
    >
      <GiftArtwork
        media={option.media || undefined}
        fallback=""
        label={option.name}
        playback="static"
        placeholderKind={option.kind === "pattern" ? "symbol" : "gift"}
      />
    </span>
  );
}
