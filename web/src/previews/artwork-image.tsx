import { useCallback } from "react";
import type { useArtworkImage } from "./use-artwork-image";

export function ArtworkImage({
  source,
  eager,
  priority,
}: {
  source: ReturnType<typeof useArtworkImage>["sources"][number];
  eager: boolean;
  priority: "high" | "auto";
}) {
  const complete = useCallback(
    (image: HTMLImageElement | null) => {
      // Memory-cache completions can precede React's load listener. Reveal before the first paint.
      if (image?.complete && image.naturalWidth) source.loaded(image);
    },
    [source.loaded],
  );
  return (
    <img
      ref={complete}
      src={source.url}
      alt=""
      style={{ visibility: source.status !== "ready" ? "hidden" : undefined }}
      loading={eager ? "eager" : "lazy"}
      fetchPriority={priority}
      decoding="async"
      className={`gift-model-image${source.status === "ready" ? " loaded" : ""}`}
      onLoad={(event) => source.loaded(event.currentTarget)}
      onError={source.failed}
    />
  );
}
