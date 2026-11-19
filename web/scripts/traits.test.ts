import assert from "node:assert/strict";
import { test } from "node:test";
import { resolveTraits } from "../src/catalog/traits.ts";
import type { CatalogOption, TraitFilters } from "../src/market.ts";

const model: CatalogOption = { kind: "model", id: "model", name: "Gold" };
const pattern: CatalogOption = { kind: "pattern", id: "pattern", name: "Star" };

test("trait resolution preserves order and restricted IDs in partial catalogs", () => {
  const selection: TraitFilters = { models: ["unknown", "model", "pattern"] };
  const options = new Map([
    [model.id, model],
    [pattern.id, pattern],
  ]);
  const [models, backdrops, patterns] = resolveTraits(selection, options);
  assert.deepEqual(
    models.entries.map(({ id }) => id),
    selection.models,
  );
  assert.deepEqual(models.available, [model]);
  assert.equal(models.missing, 2);
  assert.equal(models.entries[2].name, "Unavailable model");
  assert.equal(models.entries[2].option, undefined);
  assert.equal(backdrops.entries.length, 0);
  assert.equal(patterns.entries.length, 0);
  assert.deepEqual(selection.models, ["unknown", "model", "pattern"]);
});

test("cached names never make missing or wrong-kind catalog entries available", () => {
  const [models] = resolveTraits(
    { models: ["unknown", "pattern"] },
    new Map([[pattern.id, pattern]]),
    new Map([
      ["unknown", "Remembered model"],
      ["pattern", "Wrong model name"],
    ]),
  );
  assert.equal(models.entries[0].name, "Remembered model");
  assert.equal(models.entries[1].name, "Unavailable model");
  assert.equal(models.missing, 2);
  assert.deepEqual(models.available, []);
});

test("catalog recovery resolves previous missing entries without stale cached groups", () => {
  const selection = { models: ["model"], patterns: ["pattern"] };
  const options = new Map<string, CatalogOption>();
  const missing = resolveTraits(selection, options);
  options.set(model.id, model);
  options.set(pattern.id, pattern);
  const fresh = resolveTraits(selection, options);
  assert.equal(missing[0].missing, 1);
  assert.equal(fresh[0].missing, 0);
  assert.deepEqual(fresh[0].available, [model]);
  assert.deepEqual(fresh[2].available, [pattern]);
});
