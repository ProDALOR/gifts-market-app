import assert from "node:assert/strict";
import { test } from "node:test";
import { isBestBid } from "../src/valuation/best-bid.ts";
import { previewStrategies } from "../src/valuation/strategies.ts";
import {
  presentResult,
  validateDefinitions,
} from "../src/valuation/presentation.ts";
import { bestBidResult, fixtureDisplay } from "./valuation-fixture.mjs";

test("best bid payloads keep native quotes and capacity, reject malformed nested references", () => {
  const data = bestBidResult().data;
  assert(isBestBid(data));
  assert.equal(data.best.order.quantity, 3);
  for (const change of [
    (v) => {
      delete v.references.collection;
    },
    (v) => {
      v.references.model.value = "NaN";
    },
    (v) => {
      v.best.order.quantity = 0;
    },
    (v) => {
      v.best.value = "0";
    },
    (v) => {
      v.best.error = { code: "not_enough_data", message: "Missing" };
    },
    (v) => {
      v.references.pattern.order.expires_at = "bad";
    },
  ]) {
    const bad = structuredClone(data);
    change(bad);
    assert.equal(isBestBid(bad), false);
  }
  const absent = {
    error: { code: "not_enough_data", message: "No matching bid" },
  };
  assert(isBestBid({ best: absent, references: { collection: absent } }));
});
test("discovered quote strategies appear alongside estimates without adding reference floor requests", () => {
  const metadata = (id, purpose) => ({
    id,
    name: id,
    purpose,
    description: "test",
    revision: "r1",
    input: { targets: ["gift_attributes"], period: "unsupported" },
    display: fixtureDisplay("best_bid"),
    documentation_url: "/guide",
    documentation_revision: "d1",
  });
  const rows = [
    metadata("renamed_quote", "quote"),
    metadata("estimate", "estimate"),
    metadata("floor", "reference"),
  ];
  validateDefinitions(rows);
  assert.deepEqual(
    previewStrategies(rows, "gift_attributes").map((v) => v.id),
    ["renamed_quote", "estimate"],
  );
  assert.equal(previewStrategies(rows, "gift_attributes", true).length, 3);
  const strategy = metadata("best_bid", "quote");
  const result = bestBidResult("1.545");
  const fields = presentResult(strategy, result, "GRAM").fields;
  assert.deepEqual(fields[0].value, {
    kind: "currency",
    currency_id: "GRAM",
    amount: "1.54",
  });
});
