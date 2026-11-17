import assert from "node:assert/strict";
import { test } from "node:test";
import { amount, text, modelThumbnailUrl } from "../src/market.ts";
test("amount formatting preserves precision beyond JavaScript's numeric range", () => {
  assert.equal(
    amount("10000000000000000.000000001"),
    "10,000,000,000,000,000.000000001",
  );
  assert.equal(amount("0.500000001"), "0.500000001");
});
test("list and object displays preserve duplicate facts and catalog names", () => {
  const names = new Map([
    ["model", "Model"],
    ["gold", "Gold"],
  ]);
  assert.equal(
    text(
      {
        type: "list",
        value: [
          { type: "object", value: { model: { type: "enum", value: "gold" } } },
          { type: "object", value: { model: { type: "enum", value: "gold" } } },
        ],
      },
      names,
    ),
    "Model: Gold · Model: Gold",
  );
});

test("thumbnails use concrete catalog models and leave conditional bids as placeholders", () => {
  assert.equal(
    modelThumbnailUrl({
      attributes: {
        "tg.model": {
          op: "eq",
          value: { type: "enum", value: "tg.gift.123.model.42" },
        },
      },
    }),
    "/media/models/tg.gift.123.model.42",
  );
  assert.equal(modelThumbnailUrl({ attributes: {} }), undefined);
  assert.equal(
    modelThumbnailUrl({
      attributes: { "tg.model": { op: "one_of", values: [] } },
    }),
    undefined,
  );
  assert.equal(
    modelThumbnailUrl({
      attributes: {
        "tg.model": { op: "eq", value: { type: "text", value: "untrusted" } },
      },
    }),
    undefined,
  );
});
