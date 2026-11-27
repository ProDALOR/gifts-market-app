import type { components } from "../gen/api";
import type { TraitFilters } from "../market";
export type PortfolioState = components["schemas"]["PortfolioResponse"];
export type PortfolioItem = components["schemas"]["InventoryValue"];
export const portfolioRelations = {
  hosted: { label: "Hosted", description: "Hosted on this Telegram profile." },
  unverified: {
    label: "On-chain",
    description: "On-chain",
  },
};
export function portfolioRelation(relation: PortfolioItem["gift"]["relation"]) {
  return relation === "owned" ? undefined : portfolioRelations[relation];
}
export type PortfolioSort =
  | "value_desc"
  | "value_asc"
  | "number_asc"
  | "number_desc"
  | "position_asc"
  | "position_desc";
export type PortfolioSortKey = "value" | "number" | "position";
export const portfolioSortOptions = [
  { value: "position", label: "Position" },
  { value: "number", label: "Gift #" },
  { value: "value", label: "Estimate" },
] as const;
export function portfolioSort(
  key: PortfolioSortKey,
  ascending: boolean,
): PortfolioSort {
  return `${key}_${ascending ? "asc" : "desc"}`;
}
export type PortfolioFilters = {
  collection: string;
  traits: TraitFilters;
  number: string;
  sort: PortfolioSort;
};
/** Match the domain's nonzero u32 gift number without rounding or exponent notation. */
export function portfolioGiftNumber(value: string): number | undefined {
  if (!value) return undefined;
  const number = Number(value);
  if (
    !/^\d+$/.test(value) ||
    !Number.isInteger(number) ||
    number < 1 ||
    number > 0xffffffff
  )
    throw new Error("Enter a gift number from 1 to 4 294 967 295.");
  return number;
}
/** Canonical request identity matches the server's set semantics for trait alternatives. */
export function portfolioQuery(
  user: number | undefined,
  currency: string,
  filters: PortfolioFilters,
) {
  if (!user) return undefined;
  let number: number | undefined;
  try {
    number = portfolioGiftNumber(filters.number);
  } catch {
    return undefined;
  }
  const alternatives = (values?: string[]) => [...new Set(values)].sort();
  return {
    user_id: user,
    currency_id: currency,
    sort: filters.sort,
    filters: {
      collection_id: filters.collection || undefined,
      models: alternatives(filters.traits.models),
      backdrops: alternatives(filters.traits.backdrops),
      patterns: alternatives(filters.traits.patterns),
      number,
    },
  } satisfies Omit<
    components["schemas"]["PortfolioReadQuery"],
    "offset" | "limit" | "version"
  >;
}
export function progressText(state?: PortfolioState) {
  const p = state?.progress;
  if (!p) return "Connecting…";
  switch (p.stage) {
    case "idle":
      return "Not checked yet";
    case "queued":
      return "Waiting to check gifts…";
    case "loading_inventory":
      return `${p.checked.toLocaleString()}${p.total != null ? ` of ${p.total.toLocaleString()}` : ""} gifts checked`;
    case "valuing":
      return `${p.valued.toLocaleString()} of ${p.collectibles.toLocaleString()} gifts valued`;
    case "failed":
      return state?.error || "Refresh failed";
    case "ready":
      return state?.inventory?.complete
        ? "Inventory checked"
        : "Partial inventory";
  }
}
