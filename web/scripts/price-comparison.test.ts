import assert from "node:assert/strict";
import { test } from "node:test";
import { priceComparison } from "../src/valuation/price-comparison.ts";

test("price comparisons use the estimate as their baseline", () => {
  assert.equal(priceComparison("88", "100"), "About 12% below estimate");
  assert.equal(priceComparison("112.25", "100"), "About 12.3% above estimate");
  assert.equal(priceComparison("100.0", "100.00"), "At estimate");
  assert.equal(priceComparison("99.99999", "100"), "Near estimate");
  assert.equal(
    priceComparison("0.000088", "0.0001"),
    "About 12% below estimate",
  );
});

test("comparison keeps precision beyond the JavaScript safe integer range", () => {
  assert.equal(
    priceComparison("9007199254740992", "9007199254740993"),
    "Near estimate",
  );
  assert.equal(
    priceComparison(
      "10000000000000000.000000001",
      "20000000000000000.000000002",
    ),
    "About 50% below estimate",
  );
});

test("invalid and non-positive amounts cannot become price comparisons", () => {
  for (const value of [
    "0",
    "-1",
    "NaN",
    "Infinity",
    "1e10",
    "",
    "1.2.3",
    "9".repeat(101),
  ]) {
    assert.equal(priceComparison(value, "100"), undefined);
    assert.equal(priceComparison("100", value), undefined);
  }
});
