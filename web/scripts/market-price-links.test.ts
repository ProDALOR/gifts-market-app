import assert from "node:assert/strict";
import { test } from "node:test";
import { attributeMarketSelection } from "../src/orderbook/attribute-market.ts";
import {
  selectionFromParams,
  selectionParams,
} from "../src/market-selection.ts";
import type { ValuationSubject } from "../src/order-target.ts";
const gift: ValuationSubject = {
  book: { collection_id: "collection", currency_id: "XTR" },
  target: {
    kind: "gift",
    id: "g",
    number: 42,
    traits: { model: "model", backdrop: "back", pattern: "symbol" },
  },
};
test("table price links use each independent valuation scope and price direction", () => {
  for (const [scope, models, backdrops, patterns] of [
    ["collection", [], [], []],
    ["model", ["model"], [], []],
    ["backdrop", [], ["back"], []],
    ["pattern", [], [], ["symbol"]],
    ["pair", ["model"], ["back"], []],
  ] as const)
    for (const source of ["asks", "bids"] as const) {
      const selection = attributeMarketSelection(gift, scope, source)!;
      assert.deepEqual(selection.filters, {
        collection_id: "collection",
        currency_id: "XTR",
        venue_id: null,
        models,
        backdrops,
        patterns,
        number: null,
        price: null,
      });
      assert.equal(
        selection.sort,
        source === "asks" ? "price_asc" : "price_desc",
      );
      assert.equal(
        selectionFromParams(selectionParams(selection)).source,
        source,
      );
      if (source === "bids")
        assert.equal(
          selection.bid_match,
          "any",
          "compatible bids include broad criteria",
        );
    }
});
test("unknown traits never become broad links or exact gift-number filters", () => {
  const partial: ValuationSubject = {
    ...gift,
    target: { kind: "gift", id: "partial", traits: { model: "known" } },
  };
  assert.equal(attributeMarketSelection(partial, "pair", "asks"), undefined);
  assert.equal(
    attributeMarketSelection(partial, "backdrop", "asks"),
    undefined,
  );
  assert.equal(attributeMarketSelection(partial, undefined, "asks"), undefined);
  assert.equal(attributeMarketSelection(partial, "pattern", "bids"), undefined);
  assert.deepEqual(
    attributeMarketSelection(partial, "model", "asks")?.filters.models,
    ["known"],
  );
  assert.equal(
    attributeMarketSelection(partial, "collection", "asks")?.filters.number,
    null,
  );
});
