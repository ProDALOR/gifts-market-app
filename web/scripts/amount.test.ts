import assert from "node:assert/strict";
import { test } from "node:test";
import {
  amount,
  compactAmount,
  listingAmount,
  roundedAmount,
  trimDecimalZeros,
} from "../src/amount.ts";

test("collection floors compact thousands and handle rounding across magnitude boundaries", () => {
  for (const [value, expected] of [
    ["999", "999"],
    ["999.994", "999.99"],
    ["999.995", "1K"],
    ["1000", "1K"],
    ["1234.5678", "1.23K"],
    ["999999.995", "1M"],
    ["12345678.123456789", "12.35M"],
    ["1234567890", "1.23B"],
    ["1234567890000", "1.23T"],
    ["999999999999999.999", "1e15"],
    ["10000000000000000.123456789", "1e16"],
  ])
    assert.equal(compactAmount(value), expected);
  assert.equal(roundedAmount("999.995"), "1000");
  assert.equal(amount(roundedAmount("1000.125")), "1,000.13");
  assert.equal(
    amount(roundedAmount("10000000000000000.135")),
    "10,000,000,000,000,000.14",
  );
});

test("floor labels trim only trailing fractional zeros without losing precision", () => {
  for (const [value, expected] of [
    ["0.000", "0"],
    ["1000", "1000"],
    ["12.500", "12.5"],
    ["10000000000000000.00000000100", "10000000000000000.000000001"],
  ])
    assert.equal(trimDecimalZeros(value), expected);
  assert.equal(roundedAmount("12.501"), "12.5");
  assert.equal(roundedAmount("12.006"), "12.01");
});

test("item cards hide fractional digits after two places without altering the price", () => {
  for (const [value, expected] of [
    ["12", "12"],
    ["12.3", "12.3"],
    ["12.34", "12.34"],
    ["12.3456", "12.34.."],
    ["999.9999", "999.99.."],
    ["0.000000001", "0.00.."],
    ["10000000000000000.000000001", "10,000,000,000,000,000.00.."],
  ])
    assert.equal(listingAmount(value), expected);
});

test("portfolio totals sum exact decimal amounts beyond JavaScript numeric precision", async () => {
  const { sumAmounts } = await import("../src/amount.ts");
  assert.equal(
    sumAmounts(["9999999999999999.01", "0.02"]),
    "9999999999999999.03",
  );
  assert.equal(sumAmounts(["0.1", "0.2"]), "0.3");
  assert.equal(sumAmounts([]), "0");
  assert.equal(
    sumAmounts([
      "0.0000000000000000000000000001",
      "0.0000000000000000000000000001",
    ]),
    "0.0000000000000000000000000002",
  );
});
