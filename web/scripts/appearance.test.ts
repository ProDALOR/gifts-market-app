import assert from "node:assert/strict";
import { test } from "node:test";
import { parseAppearance } from "../src/app/appearance.ts";

test("appearance settings validate persisted palettes and modes independently", () => {
  for (const value of [
    null,
    "invalid",
    "[]",
    "null",
    "42",
    '{"palette":"bad","mode":"bad"}',
  ])
    assert.deepEqual(parseAppearance(value), { palette: "rose", mode: "auto" });
  assert.deepEqual(parseAppearance('{"palette":"ocean","mode":"light"}'), {
    palette: "ocean",
    mode: "light",
  });
  assert.deepEqual(parseAppearance('{"palette":"mint","mode":"invalid"}'), {
    palette: "mint",
    mode: "auto",
  });
  assert.deepEqual(parseAppearance('{"palette":"invalid","mode":"dark"}'), {
    palette: "rose",
    mode: "dark",
  });
});
