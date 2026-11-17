import assert from "node:assert/strict";
import { test } from "node:test";
import { amount, modelMedia } from "../src/market.ts";
import type { Order, CatalogOption } from "../src/market.ts";
test("amount formatting preserves precision beyond JavaScript numeric range", () => {
  assert.equal(
    amount("10000000000000000.000000001"),
    "10,000,000,000,000,000.000000001",
  );
  assert.equal(amount("0.500000001"), "0.500000001");
});
test("concrete previews use supplied media, never guessed paths or hypothetical targets", () => {
  const gift: Pick<Order, "target"> = {
    target: {
      kind: "gift",
      gift: {
        id: "42",
        collection_id: "gift",
        number: 42,
        traits: { model: "model", backdrop: null, pattern: null },
      },
    },
  };
  const media = {
    thumbnail_url: "/assets/thumbnail?v=2",
    animation_url: "/assets/animation.tgs?v=3",
  };
  const options = new Map<string, CatalogOption>([
    ["model", { id: "model", name: "Model", kind: "model", media }],
  ]);
  assert.equal(modelMedia(gift, options), media);
  assert.equal(modelMedia(gift, new Map()), undefined);
  const missing = new Map<string, CatalogOption>([
    ["model", { id: "model", name: "Model", kind: "model", media: null }],
  ]);
  assert.equal(modelMedia(gift, missing), undefined);
  assert.equal(
    modelMedia(
      {
        target: {
          kind: "criteria",
          criteria: {
            models: ["model"],
            backdrops: [],
            patterns: [],
            number: null,
          },
        },
      },
      options,
    ),
    undefined,
  );
  assert.equal(
    modelMedia(
      { target: { kind: "unsupported", reason: "unsupported" } },
      options,
    ),
    undefined,
  );
});
