import type { components } from "../gen/api";
export type GiftRarity = components["schemas"]["GiftRarity"];
type Choice = {
  id: string;
  name: string;
  rarity?: GiftRarity | null;
};

const tiers = {
  legendary: "Legendary",
  epic: "Epic",
  rare: "Rare",
  uncommon: "Uncommon",
} as const;
export function craftTier(
  rarity: Choice["rarity"],
): keyof typeof tiers | undefined {
  return rarity && Object.hasOwn(tiers, rarity.kind)
    ? (rarity.kind as keyof typeof tiers)
    : undefined;
}
function rarityGroup(rarity: Choice["rarity"]) {
  if (rarity?.kind === "percentage") {
    const value = rarity.permille;
    if (Number.isInteger(value) && value >= 0 && value <= 1000) {
      return {
        key: `percentage:${value}`,
        order: value,
        label: `${Math.floor(value / 10)}${value % 10 ? `.${value % 10}` : ""}%`,
        craft: undefined,
      };
    }
  }
  const craft = craftTier(rarity);
  if (craft) {
    return {
      key: craft,
      order: 1001 + Object.keys(tiers).indexOf(craft),
      label: tiers[craft],
      craft,
    };
  }
  return {
    key: "unknown",
    order: Infinity,
    label: "Unknown",
    craft: undefined,
  };
}

/** Reuse picker formatting without displaying an unknown rarity as a value. */
export function rarityLabel(rarity: Choice["rarity"]) {
  const group = rarityGroup(rarity);
  return group.key === "unknown" ? undefined : group.label;
}

/** Keep exact percentages and craft tiers distinct; unknown rarity comes last. */
export function rarityGroups<T extends Choice>(
  options: readonly T[],
  search = "",
) {
  const query = search.trim().toLowerCase();
  const groups = new Map<
    string,
    ReturnType<typeof rarityGroup> & { options: T[] }
  >();
  for (const option of options) {
    if (!option.name.toLowerCase().includes(query)) continue;
    const group = rarityGroup(option.rarity);
    const existing = groups.get(group.key);
    if (existing) existing.options.push(option);
    else groups.set(group.key, { ...group, options: [option] });
  }
  return [...groups.values()].sort((a, b) => a.order - b.order);
}

export function groupSelection(
  options: readonly Choice[],
  chosen: readonly string[],
) {
  const selected = new Set(chosen);
  const count = options.filter((option) => selected.has(option.id)).length;
  return count === 0 ? false : count === options.length ? true : "mixed";
}

/** Search limits group actions to visible matches; unrelated selections survive. */
export function toggleRarityGroup(
  options: readonly Choice[],
  chosen: readonly string[],
) {
  const ids = new Set(options.map((option) => option.id));
  const result =
    groupSelection(options, chosen) === true
      ? chosen.filter((id) => !ids.has(id))
      : [...new Set([...chosen, ...ids])];
  return result.length ? result : undefined;
}
