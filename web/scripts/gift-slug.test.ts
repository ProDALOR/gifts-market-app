import { test } from "node:test";
import assert from "node:assert/strict";
import { normalizeSlug, sameSlug } from "../src/valuation/slug.ts";
test("gift preview slugs match canonical casing and positive edition numbers", () => {
  assert.equal(normalizeSlug("ResistanceDog-014029"), "resistancedog-14029");
  assert.equal(normalizeSlug("3DGlasses-1"), "3dglasses-1");
  assert.ok(sameSlug("ResistanceDog-14029", "resistancedog-014029"));
  for (const slug of [
    "Gift-0",
    "Gift-4294967296",
    "Gift--1",
    "https://t.me/nft/Gift-1",
    "Gift-1?token=private",
    "<script>-1",
  ])
    assert.equal(normalizeSlug(slug), undefined, slug);
});
