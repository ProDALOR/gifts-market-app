import assert from "node:assert/strict";
import { test } from "node:test";
import { giftAppearance } from "../src/gift-appearance.ts";
import type { GiftOption } from "../src/gift-appearance.ts";
import type { Order } from "../src/market.ts";
function options(): Map<string, GiftOption> {
  return new Map([
    [
      "backdrop",
      {
        kind: "attribute_option",
        id: "backdrop",
        name: "Lavender",
        attribute_id: "tg.backdrop",
        aliases: [],
        meta: {
          "colors.center_color": "#B789E4",
          "colors.edge_color": "#8a5abc",
          "colors.pattern_color": "#5b10ab",
        },
      },
    ],
    [
      "pattern",
      {
        kind: "attribute_option",
        id: "pattern",
        name: "Lion Head",
        attribute_id: "tg.pattern",
        aliases: [],
        meta: {},
      },
    ],
  ]);
}
function order(): Pick<Order, "attributes"> {
  return {
    attributes: Object.fromEntries(
      ["model", "backdrop", "pattern"].map((key) => [
        `tg.${key}`,
        { op: "eq", value: { type: "enum", value: key } },
      ]),
    ),
  };
}
test("gift previews resolve the exact backdrop and symbol, preserving catalog RGB colors", () => {
  assert.deepEqual(giftAppearance(order(), options()), {
    modelUrl: "/media/models/model",
    palette: { center: "#b789e4", edge: "#8a5abc", pattern: "#5b10ab" },
    patternUrl: "/media/patterns/pattern",
    label: "Gift preview, Lavender backdrop, Lion Head pattern",
  });
});
test("missing metadata and conditions never fabricate a gift appearance", () => {
  assert.equal(giftAppearance(order(), new Map()).palette, undefined);
  const conditional = order();
  conditional.attributes["tg.model"] = { op: "one_of", values: [] };
  assert.equal(giftAppearance(conditional, options()).palette, undefined);
  assert.equal(giftAppearance(conditional, options()).patternUrl, undefined);
  const exactBackdrop = order();
  exactBackdrop.attributes["tg.pattern"] = { op: "one_of", values: [] };
  assert.ok(giftAppearance(exactBackdrop, options()).palette);
  assert.equal(giftAppearance(exactBackdrop, options()).patternUrl, undefined);
});
test("palettes validate RGB values and distinguish backdrop and pattern catalog entries", () => {
  const catalog = options();
  catalog.get("backdrop")!.meta["colors.center_color"] =
    "url(https://example.com)";
  assert.equal(giftAppearance(order(), catalog).palette, undefined);
  const swapped = options();
  swapped.get("backdrop")!.attribute_id = "tg.pattern";
  assert.equal(giftAppearance(order(), swapped).palette, undefined);
  const invalidPattern = options();
  invalidPattern.get("pattern")!.attribute_id = "tg.model";
  assert.equal(giftAppearance(order(), invalidPattern).patternUrl, undefined);
});
