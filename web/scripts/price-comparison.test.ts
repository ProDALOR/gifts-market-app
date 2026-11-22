import assert from "node:assert/strict";
import { test } from "node:test";
import { priceComparison } from "../src/valuation/price-comparison.ts";

test("price comparisons use the estimate as their baseline", () => {
  assert.deepEqual(priceComparison("88", "100"), {
    label: "−12%",
    description: "Ask price is about 12% below this estimate",
  });
  assert.deepEqual(priceComparison("112.25", "100"), {
    label: "+12.3%",
    description: "Ask price is about 12.3% above this estimate",
  });
  assert.equal(priceComparison("100.0", "100.00"), undefined);
  assert.equal(priceComparison("99.99999", "100"), undefined);
  assert.equal(priceComparison("0.000088", "0.0001")?.label, "−12%");
});

test("comparison keeps precision beyond the JavaScript safe integer range", () => {
  assert.equal(
    priceComparison("9007199254740992", "9007199254740993"),
    undefined,
  );
  assert.equal(
    priceComparison(
      "10000000000000000.000000001",
      "20000000000000000.000000002",
    )?.label,
    "−50%",
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
