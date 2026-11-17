import { collectionOptions } from "../src/catalog/selectors.ts";
import assert from "node:assert/strict";
import { test } from "node:test";
import { listingLink, serial, modelName } from "../src/orderbook.ts";
import type { Collection, CatalogOption, Order } from "../src/market.ts";
test("concrete gift numbers remain distinct from bid selection ranges", () => {
  const gift = {
    target: { kind: "gift", gift: { number: 42, traits: { model: "gold" } } },
  } as Order;
  const bid = {
    target: {
      kind: "criteria",
      criteria: { models: ["gold"], number: { min: 1, max: 100 } },
    },
  } as Order;
  assert.equal(serial(gift), "42");
  assert.equal(serial(bid), undefined);
  assert.equal(
    modelName(
      bid,
      new Map([
        ["gold", { id: "gold", name: "Gold", kind: "model", media: null }],
      ]),
      "Any",
    ),
    "Gold",
  );
});
test("trait choices use the active collection response and matching gift trait", () => {
  const collection: Collection = {
    id: "gift",
    name: "Gift",
    preview: null,
  };
  const options = new Map<string, CatalogOption>([
    ["gold", { id: "gold", name: "Gold", kind: "model", media: null }],
    ["silver", { id: "silver", name: "Silver", kind: "model", media: null }],
  ]);
  assert.deepEqual(
    collectionOptions(collection, "models", options).map((option) => option.id),
    ["gold", "silver"],
  );
  assert.deepEqual(collectionOptions(collection, "patterns", options), []);
});
test("market buttons require a public HTTP(S) source link", () => {
  for (const market_url of [
    "javascript:alert(1)",
    "data:text/html,bad",
    "https://user:secret@example.com",
    "not a url",
  ]) {
    assert.equal(listingLink({ market_url } as Order), undefined);
  }
  assert.equal(
    listingLink({
      market_url: "https://t.me/market/app?startapp=gift42",
    } as Order),
    "https://t.me/market/app?startapp=gift42",
  );
});
