import assert from "node:assert/strict";
import { test } from "node:test";
import {
  aliasFilters,
  matchesSelector,
  preferredSelector,
  resolveSelector,
  resolveFilterSelectors,
  availableFilterSelectors,
} from "../src/catalog/aliases.ts";
test("selectors emit public slugs and names while accepting legacy collection links", () => {
  const one = {
    id: "c1",
    name: "One",
    aliases: [{ namespace: "tg.slug", id: "One" }],
  };
  assert.equal(resolveSelector([one], "@tg.slug:one"), one);
  assert.equal(preferredSelector(one), "@slug:One");
  assert(!matchesSelector(one, "@tg.slug:Two"));
  assert.equal(
    resolveSelector([one, { ...one, id: "c2" }], "@name:One"),
    undefined,
  );
  const option = {
    id: "b1",
    name: "Black",
    kind: "backdrop" as const,
    aliases: [{ namespace: "tg", id: "42" }],
  };
  const options = new Map([["b1", option]]);
  assert.deepEqual(aliasFilters({ backdrops: ["b1"] }, options), {
    backdrops: ["@name:Black"],
  });
  assert.deepEqual(
    resolveFilterSelectors(
      { backdrops: ["@tg:42"], patterns: ["@tg:42"] },
      options,
    ),
    { backdrops: ["b1"], patterns: ["@tg:42"] },
  );
  assert.equal(resolveSelector([option], "@malformed"), undefined);
});
test("market filters keep only unambiguous traits available in the selected collection", () => {
  const options = new Map([
    ["m", { id: "m", name: "Aura", kind: "model" as const }],
    ["b", { id: "b", name: "Black", kind: "backdrop" as const }],
    ["p", { id: "p", name: "Sun", kind: "pattern" as const }],
  ]);
  assert.deepEqual(
    availableFilterSelectors(
      {
        models: ["m", "@name:Old model"],
        backdrops: ["@name:Carmine", "@name:black"],
        patterns: ["@name:Sun", "@name:Black"],
      },
      options,
    ),
    { models: ["m"], backdrops: ["@name:black"], patterns: ["@name:Sun"] },
  );
  assert.deepEqual(
    availableFilterSelectors({ backdrops: ["@name:Carmine"] }, options),
    {},
    "An absent backdrop must not block the market",
  );
  const filters = { backdrops: ["@name:Black"] };
  assert.equal(availableFilterSelectors(filters, options), filters);
  assert.deepEqual(
    availableFilterSelectors(
      filters,
      new Map([
        ...options,
        ["b2", { id: "b2", name: "Black", kind: "backdrop" as const }],
      ]),
    ),
    {},
    "Ambiguous names are never chosen arbitrarily",
  );
});
test("new catalog selectors use canonical slugs or unique names without numeric source identities", () => {
  const collection = {
    id: "gift",
    name: "Gift",
    aliases: [{ namespace: "slug", id: "CanonicalGift" }],
  };
  assert.equal(preferredSelector(collection), "@slug:CanonicalGift");
  assert.equal(
    resolveSelector([collection], "@tg.slug:canonicalgift"),
    collection,
  );
  const one = { id: "m1", name: "Same", kind: "model" as const };
  const two = { id: "m2", name: "Same", kind: "model" as const };
  assert.deepEqual(
    aliasFilters(
      { models: ["m1"] },
      new Map([
        ["m1", one],
        ["m2", two],
      ]),
    ),
    { models: ["m1"] },
  );
  assert.deepEqual(aliasFilters({ models: ["m1"] }, new Map([["m1", one]])), {
    models: ["@name:Same"],
  });
  assert.equal(
    preferredSelector({
      id: "m1",
      name: "Model",
      aliases: [{ namespace: "tg", id: "123" }],
    }),
    "m1",
  );
  const oversized = {
    id: "long",
    name: "é".repeat(126),
    kind: "model" as const,
  };
  assert.deepEqual(
    aliasFilters({ models: ["long"] }, new Map([["long", oversized]])),
    { models: ["long"] },
  );
});
