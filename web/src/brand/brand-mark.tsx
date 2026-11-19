import { GiftArtwork } from "../previews/gift-artwork";
import poster from "./liberty-figure.svg?url";
import animation from "./liberty-figure.tgs?url";
import "./brand.css";

// Fixed branding from https://t.me/nft/LibertyFigure-11, model Columbus.
// The SVG poster was generated with market-media-render from the same TGS.
const media = {
  thumbnail_url: poster,
  preview_url: poster,
  animation_url: animation,
};

export function BrandMark({ animated = false }: { animated?: boolean }) {
  return (
    <a className="brand" href="/" aria-label="IGIFTS home" title="IGIFTS">
      <GiftArtwork
        media={media}
        fallback=""
        label="Liberty Figure · Columbus"
        playback={animated ? "collection" : "static"}
      />
    </a>
  );
}
