import assert from "node:assert/strict";
import { test } from "node:test";
import {
  rarityGroups,
  groupSelection,
  toggleRarityGroup,
} from "../src/catalog/rarity.ts";

const rows = [
  { id: "unknown", name: "Unknown" },
  {
    id: "one",
    name: "Amber",
    rarity: { kind: "percentage" as const, permille: 5 },
  },
  {
    id: "two",
    name: "Azure",
    rarity: { kind: "percentage" as const, permille: 5 },
  },
  {
    id: "three",
    name: "Blue",
    rarity: { kind: "percentage" as const, permille: 10 },
  },
  {
    id: "four",
    name: "Copper",
    rarity: { kind: "percentage" as const, permille: 11 },
  },
];

test("rarity grouping uses exact percentages in ascending order, with unknown last", () => {
  const groups = rarityGroups(rows);
  assert.deepEqual(
    groups.map((g) => g.label),
    ["0.5%", "1%", "1.1%", "Unknown"],
  );
  assert.deepEqual(
    groups[0].options.map((o) => o.id),
    ["one", "two"],
  );
  assert.deepEqual(
    rarityGroups([
      {
        id: "zero",
        name: "Zero",
        rarity: { kind: "percentage" as const, permille: 0 },
      },
      {
        id: "all",
        name: "All",
        rarity: { kind: "percentage" as const, permille: 1000 },
      },
    ]).map((g) => g.label),
    ["0%", "100%"],
  );
  assert.equal(
    rows[0].id,
    "unknown",
    "Grouping does not reorder the source catalog",
  );
});

test("malformed or missing rarity is unknown, never a rounded or invented percentage", () => {
  const options = [undefined, null, -1, 1001, 1.5, NaN, Infinity, "5"].map(
    (rarity, index) => ({
      id: String(index),
      name: "Choice",
      rarity: { kind: "percentage" as const, permille: rarity as number },
    }),
  );
  const groups = rarityGroups(options);
  assert.equal(groups.length, 1);
  assert.equal(groups[0].key, "unknown");
  assert.equal(groups[0].options.length, 8);
});

test("group selection supports empty, mixed and all, preserving other and missing selections", () => {
  const group = rarityGroups(rows)[0].options;
  assert.equal(groupSelection(group, []), false);
  assert.equal(groupSelection(group, ["one"]), "mixed");
  assert.equal(groupSelection(group, ["two", "one"]), true);
  assert.deepEqual(toggleRarityGroup(group, ["one", "unavailable", "three"]), [
    "one",
    "unavailable",
    "three",
    "two",
  ]);
  assert.deepEqual(toggleRarityGroup(group, ["one", "two", "three"]), [
    "three",
  ]);
  assert.equal(
    toggleRarityGroup(group, ["one", "two"]),
    undefined,
    "Clearing the final group restores Any",
  );
});

test("search hides empty groups and limits group actions to visible matches", () => {
  const groups = rarityGroups(rows, "  AMBER  ");
  assert.equal(groups.length, 1);
  assert.deepEqual(
    groups[0].options.map((o) => o.id),
    ["one"],
  );
  assert.deepEqual(toggleRarityGroup(groups[0].options, ["one", "two"]), [
    "two",
  ]);
  assert.deepEqual(toggleRarityGroup(groups[0].options, ["three"]), [
    "three",
    "one",
  ]);
  assert.deepEqual(rarityGroups(rows, "absent"), []);
  assert.equal(rarityGroups(rows, "   ").length, 4);
});

test("rarity groups recompute from updated collection metadata without changing selections", () => {
  const chosen = ["one", "missing"];
  const changed = rows.map((o) =>
    o.id === "one"
      ? { ...o, rarity: { kind: "percentage" as const, permille: 10 } }
      : o,
  );
  assert.deepEqual(
    rarityGroups(changed)[1].options.map((o) => o.id),
    ["one", "three"],
  );
  assert.equal(
    groupSelection(rarityGroups(changed)[1].options, chosen),
    "mixed",
  );
  assert.deepEqual(chosen, ["one", "missing"]);
  assert.equal(
    groupSelection(
      rarityGroups([
        {
          id: "other",
          name: "Other",
          rarity: { kind: "percentage" as const, permille: 5 },
        },
      ])[0].options,
      chosen,
    ),
    false,
  );
});

test("craft enum tiers remain distinct from percentages, ordered rarest first", () => {
  const tiers = ["uncommon", "rare", "epic", "legendary"] as const;
  const crafts = tiers.map((kind) => ({
    id: kind,
    name: kind,
    rarity: { kind },
  }));
  assert.deepEqual(
    rarityGroups([...rows, ...crafts]).map((group) => group.label),
    ["0.5%", "1%", "1.1%", "Legendary", "Epic", "Rare", "Uncommon", "Unknown"],
  );
  const group = rarityGroups(crafts).find((group) => group.key === "rare")!;
  assert.deepEqual(toggleRarityGroup(group.options, []), ["rare"]);
});
