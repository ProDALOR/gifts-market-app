import assert from "node:assert/strict";
import { test } from "node:test";
import { artworkPolicy } from "../src/previews/artwork-policy.ts";
import type { ArtworkSurface } from "../src/previews/artwork-policy.ts";

test("quality keeps scrolling artwork sharp while animated previews use only TGS", () => {
  for (const surface of [
    "item",
    "collection",
    "active_collection",
    "preview",
  ] as ArtworkSurface[])
    for (const playback of [
      "static",
      "interaction",
      "collection",
      "selected",
      "detail",
    ] as const)
      assert.deepEqual(artworkPolicy("quality", surface, playback), {
        format:
          surface === "preview" &&
          (playback === "detail" || playback === "selected")
            ? "animation"
            : "preview",
        playback,
      });
});
test("performance uses raster cards with hover playback and active-only collection autoplay", () => {
  assert.deepEqual(artworkPolicy("performance", "item", "interaction"), {
    format: "thumbnail",
    playback: "interaction",
  });
  assert.deepEqual(artworkPolicy("performance", "collection", "collection"), {
    format: "thumbnail",
    playback: "static",
  });
  assert.deepEqual(
    artworkPolicy("performance", "active_collection", "collection"),
    { format: "thumbnail", playback: "collection" },
  );
  assert.deepEqual(artworkPolicy("performance", "preview", "detail"), {
    format: "animation",
    playback: "detail",
  });
});
