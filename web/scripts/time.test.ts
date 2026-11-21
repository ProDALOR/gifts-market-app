import assert from "node:assert/strict";
import { test } from "node:test";
import { compactAge } from "../src/ui/time.ts";

test("execution ages use one elapsed unit rounded down", () => {
  const now = Date.parse("2026-11-21T12:00:00Z");
  for (const [minutes, expected] of [
    [-1, "<1m"],
    [0, "<1m"],
    [0.99, "<1m"],
    [1, "1m"],
    [10.9, "10m"],
    [59.99, "59m"],
    [60, "1h"],
    [359, "5h"],
    [1439, "23h"],
    [1440, "1d"],
    [4320, "3d"],
    [10080, "7d"],
  ] as const) {
    assert.equal(compactAge(now - minutes * 60_000, now), expected);
  }
});
