import assert from "node:assert/strict";
import { test } from "node:test";
import {
  amount,
  compactAmount,
  listingAmount,
  roundedAmount,
} from "../src/amount.ts";

test("collection floors compact thousands and handle rounding across magnitude boundaries", () => {
  for (const [value, expected] of [
    ["999", "999.00"],
    ["999.994", "999.99"],
    ["999.995", "1.00K"],
    ["1000", "1.00K"],
    ["1234.5678", "1.23K"],
    ["999999.995", "1.00M"],
    ["12345678.123456789", "12.35M"],
    ["1234567890", "1.23B"],
    ["1234567890000", "1.23T"],
    ["999999999999999.999", "1.00e15"],
    ["10000000000000000.123456789", "1.00e16"],
  ])
    assert.equal(compactAmount(value), expected);
  assert.equal(roundedAmount("999.995"), "1000.00");
  assert.equal(amount(roundedAmount("1000.125")), "1,000.13");
  assert.equal(
    amount(roundedAmount("10000000000000000.135")),
    "10,000,000,000,000,000.14",
  );
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
