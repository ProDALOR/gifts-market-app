import { collectionOptions } from "../src/catalog/selectors.ts";
import assert from "node:assert/strict";
import { test } from "node:test";
import {
  selectedOptions,
  optionPredicate,
  listingLink,
} from "../src/orderbook.ts";
import type { Collection, CatalogOption } from "../src/orderbook.ts";
import type { Order, Predicate } from "../src/market.ts";
test("trait selections use exact or one-of API predicates and clear to any", () => {
  assert.equal(optionPredicate([]), undefined);
  assert.deepEqual(optionPredicate(["gold", "gold"]), {
    op: "eq",
    value: { type: "enum", value: "gold" },
  });
  assert.deepEqual(selectedOptions(optionPredicate(["gold", "silver"])), [
    "gold",
    "silver",
  ]);
  for (const malformed of [
    { op: "eq" },
    { op: "one_of" },
    { op: "one_of", values: [null, { type: "integer", value: "7" }] },
  ]) {
    assert.deepEqual(selectedOptions(malformed as Predicate), []);
  }
});
test("trait choices stay within the collection and matching attribute", () => {
  const collection = {
    attributes: {
      "tg.model": { required: true, allowed_values: ["gold", "silver"] },
    },
  } as Collection;
  const options = new Map<string, CatalogOption>([
    [
      "gold",
      { id: "gold", name: "Gold", attribute_id: "tg.model" } as CatalogOption,
    ],
    [
      "silver",
      {
        id: "silver",
        name: "Silver",
        attribute_id: "tg.model",
      } as CatalogOption,
    ],
    [
      "foreign",
      {
        id: "foreign",
        name: "Other collection",
        attribute_id: "tg.model",
      } as CatalogOption,
    ],
    [
      "pattern",
      {
        id: "pattern",
        name: "Pattern",
        attribute_id: "tg.pattern",
      } as CatalogOption,
    ],
  ]);
  assert.deepEqual(
    collectionOptions(collection, "tg.model", options).map(
      (option) => option.id,
    ),
    ["gold", "silver"],
  );
  assert.deepEqual(collectionOptions(collection, "tg.pattern", options), []);
});
test("market buttons require a public HTTP(S) source link", () => {
  for (const url of [
    "javascript:alert(1)",
    "data:text/html,bad",
    "https://user:secret@example.com",
    "not a url",
  ]) {
    assert.equal(listingLink({ meta: { url } } as Order), undefined);
  }
  assert.equal(
    listingLink({
      meta: { listing_url: "https://t.me/market/app?startapp=gift42" },
    } as Order),
    "https://t.me/market/app?startapp=gift42",
  );
});
