import type { DisplayMode } from "../app/preferences.ts";
import type { PlaybackPolicy } from "./use-playback.ts";

export type ArtworkSurface =
  "preview" | "item" | "collection" | "active_collection";
/** Animated previews download only TGS. Scrolling surfaces keep a cheap resting image. */
export function artworkPolicy(
  mode: DisplayMode,
  surface: ArtworkSurface,
  playback: PlaybackPolicy,
) {
  const raster = mode === "performance" && surface !== "preview";
  return {
    format:
      surface === "preview" &&
      (playback === "selected" || playback === "detail")
        ? ("animation" as const)
        : raster
          ? ("thumbnail" as const)
          : ("preview" as const),
    playback:
      raster && surface === "collection" ? ("static" as const) : playback,
  };
}
