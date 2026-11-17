import assert from "node:assert/strict";
import { test } from "node:test";
import { giftAppearance } from "../src/previews/gift-appearance.ts";
import type { GiftOption } from "../src/previews/gift-appearance.ts";
import type { Order } from "../src/market.ts";
function options(): Map<string, GiftOption> {
  return new Map([
    [
      "model",
      {
        kind: "model",
        id: "model",
        name: "Model",
        media: {
          thumbnail_url: "/custom/model.webp",
          animation_url: "/custom/model.tgs",
        },
      },
    ],
    [
      "backdrop",
      {
        kind: "backdrop",
        id: "backdrop",
        name: "Lavender",
        palette: { center: "#b789e4", edge: "#8a5abc", pattern: "#5b10ab" },
      },
    ],
    [
      "pattern",
      {
        kind: "pattern",
        id: "pattern",
        name: "Lion Head",
        media: {
          thumbnail_url: "/media/patterns/pattern",
          animation_url: null,
        },
      },
    ],
  ]);
}
function order(): Pick<Order, "target"> {
  return {
    target: {
      kind: "gift",
      gift: {
        id: "gift-42",
        collection_id: "collection",
        number: 42,
        traits: { model: "model", backdrop: "backdrop", pattern: "pattern" },
      },
    },
  };
}
test("gift previews use named traits and validated catalog appearance", () => {
  assert.deepEqual(giftAppearance(order(), options()), {
    media: {
      thumbnail_url: "/custom/model.webp",
      animation_url: "/custom/model.tgs",
    },
    palette: { center: "#b789e4", edge: "#8a5abc", pattern: "#5b10ab" },
    patternUrl: "/media/patterns/pattern",
    label: "Gift preview, Lavender backdrop, Lion Head pattern",
  });
});
test("missing catalog appearance and bid criteria never fabricate concrete artwork", () => {
  assert.equal(giftAppearance(order(), new Map()).palette, undefined);
  const conditional: Pick<Order, "target"> = {
    target: {
      kind: "criteria",
      criteria: {
        models: ["model"],
        backdrops: ["backdrop"],
        patterns: ["pattern"],
        number: null,
      },
    },
  };
  assert.equal(giftAppearance(conditional, options()).media, undefined);
  assert.equal(giftAppearance(conditional, options()).palette, undefined);
  const noPattern = order();
  if (noPattern.target.kind === "gift")
    noPattern.target.gift.traits.pattern = null;
  assert.ok(giftAppearance(noPattern, options()).palette);
  assert.equal(giftAppearance(noPattern, options()).patternUrl, undefined);
});
test("typed catalog option kinds and unavailable palettes preserve the placeholder", () => {
  const catalog = options();
  catalog.set("backdrop", {
    kind: "backdrop",
    id: "backdrop",
    name: "Unknown",
    palette: null,
  });
  assert.equal(giftAppearance(order(), catalog).palette, undefined);
  catalog.set("backdrop", {
    kind: "model",
    id: "backdrop",
    name: "Model",
    media: null,
  });
  assert.equal(giftAppearance(order(), catalog).palette, undefined);
  const invalidPattern = options();
  invalidPattern.set("pattern", {
    kind: "model",
    id: "pattern",
    name: "Model",
    media: null,
  });
  assert.equal(giftAppearance(order(), invalidPattern).patternUrl, undefined);
});
