import type { components } from "../gen/api";
import type { TraitFilters } from "../market";
export type PortfolioState = components["schemas"]["PortfolioResponse"];
export type PortfolioItem = components["schemas"]["InventoryValue"];
const relations = {
  hosted: { label: "Hosted", description: "Hosted on this Telegram profile." },
  unverified: {
    label: "Unverified",
    description: "Visible on this profile; ownership unverified.",
  },
};
export function portfolioRelation(relation: PortfolioItem["gift"]["relation"]) {
  return relation === "owned" ? undefined : relations[relation];
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
