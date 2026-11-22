import assert from "node:assert/strict";
import { test } from "node:test";
import {
  sameCatalogTarget,
  sameCriteria,
  sameGiftAttributes,
} from "../src/valuation/targets.ts";

test("catalog scope identity normalizes omitted and null options without accepting another scope", () => {
  const target = {
    kind: "catalog" as const,
    collection_ids: ["gift"],
    details: { collection_id: "gift", pair: null },
  };
  assert.equal(
    sameCatalogTarget(
      { ...target, details: { collection_id: "gift" } },
      target,
    ),
    true,
  );
  assert.equal(
    sameCatalogTarget({ ...target, collection_ids: ["other"] }, target),
    false,
  );
  assert.equal(
    sameCatalogTarget(
      {
        ...target,
        details: {
          collection_id: "gift",
          pair: { model_id: "m", backdrop_id: "b" },
        },
      },
      target,
    ),
    false,
  );
});

test("criteria scope identity ignores omission and field order but retains every condition", () => {
  const criteria = {
    models: ["model"],
    backdrops: [],
    patterns: [],
    number: null,
  };
  assert.equal(
    sameCriteria({ patterns: [], backdrops: [], models: ["model"] }, criteria),
    true,
  );
  assert.equal(
    sameCriteria({ ...criteria, patterns: ["symbol"] }, criteria),
    false,
  );
  assert.equal(
    sameCriteria({ ...criteria, number: { min: 1 } }, criteria),
    false,
  );
  assert.equal(
    sameCriteria(
      { ...criteria, number: { max: 10, min: 1 } },
      { ...criteria, number: { min: 1, max: 10 } },
    ),
    true,
  );
});

test("canonical selection alternatives tolerate server sorting and deduplication", () => {
  assert.equal(
    sameCriteria(
      { models: ["a", "b"], backdrops: [], patterns: ["p"] },
      { models: ["b", "a", "b"], backdrops: [], patterns: ["p", "p"] },
    ),
    true,
  );
  assert.equal(
    sameCriteria(
      { models: ["a"], backdrops: [], patterns: [] },
      { models: ["a", "b"], backdrops: [], patterns: [] },
    ),
    false,
  );
});

test("attribute responses must match every supplied trait, collection and optional number", () => {
  const target = {
    kind: "gift_attributes" as const,
    collection_id: "gift",
    traits: { model: "m", backdrop: "b", pattern: "s" },
  };
  assert.equal(sameGiftAttributes({ ...target, number: null }, target), true);
  for (const traits of [
    { model: "other", backdrop: "b", pattern: "s" },
    { model: "m", backdrop: "other", pattern: "s" },
    { model: "m", backdrop: "b", pattern: "other" },
    { model: "m", backdrop: "b" },
  ])
    assert.equal(sameGiftAttributes({ ...target, traits }, target), false);
  assert.equal(
    sameGiftAttributes({ ...target, collection_id: "other" }, target),
    false,
  );
  assert.equal(sameGiftAttributes({ ...target, number: 42 }, target), false);
});
