import { sumAmounts } from "../amount.ts";
import type { components } from "../gen/api";
import type { TraitFilters } from "../market";
export type PortfolioState = components["schemas"]["PortfolioState"];
export type PortfolioItem = components["schemas"]["InventoryValue"];
export type PortfolioSort =
  "value_desc" | "value_asc" | "number_asc" | "number_desc";
export type PortfolioFilters = {
  collection: string;
  traits: TraitFilters;
  number: string;
  min: string;
  max: string;
  sort: PortfolioSort;
};
export function compareValue(a: string, b: string) {
  const [ar, af = ""] = a.split("."),
    [br, bf = ""] = b.split(".");
  const aw = ar.replace(/^0+(?=\d)/, ""),
    bw = br.replace(/^0+(?=\d)/, "");
  return (
    aw.length - bw.length ||
    aw.localeCompare(bw) ||
    af
      .padEnd(Math.max(af.length, bf.length), "0")
      .localeCompare(bf.padEnd(Math.max(af.length, bf.length), "0"))
  );
}
export function selectItems(items: PortfolioItem[], f: PortfolioFilters) {
  return items
    .filter((item) => {
      const gift = item.resolved;
      if (f.collection && gift?.collection_id !== f.collection) return false;
      const t = gift?.traits;
      if (
        f.traits.models?.length &&
        (!t?.model || !f.traits.models.includes(t.model))
      )
        return false;
      if (
        f.traits.backdrops?.length &&
        (!t?.backdrop || !f.traits.backdrops.includes(t.backdrop))
      )
        return false;
      if (
        f.traits.patterns?.length &&
        (!t?.pattern || !f.traits.patterns.includes(t.pattern))
      )
        return false;
      if (f.number && item.gift.gift.number !== Number(f.number)) return false;
      if (
        f.min &&
        /^(?:0|[1-9]\d*)(?:\.\d+)?$/.test(f.min) &&
        (!item.estimate || compareValue(item.estimate, f.min) < 0)
      )
        return false;
      if (
        f.max &&
        /^(?:0|[1-9]\d*)(?:\.\d+)?$/.test(f.max) &&
        (!item.estimate || compareValue(item.estimate, f.max) > 0)
      )
        return false;
      return true;
    })
    .sort((a, b) => {
      if (f.sort.startsWith("value")) {
        if (a.estimate == null || b.estimate == null)
          return a.estimate == null ? (b.estimate == null ? 0 : 1) : -1;
        const n = compareValue(a.estimate, b.estimate);
        return (
          (f.sort === "value_desc" ? -n : n) ||
          a.gift.gift.number - b.gift.gift.number ||
          a.gift.gift.id.localeCompare(b.gift.gift.id)
        );
      }
      const n = a.gift.gift.number - b.gift.gift.number;
      return (
        (f.sort === "number_desc" ? -n : n) ||
        a.gift.gift.id.localeCompare(b.gift.gift.id)
      );
    });
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
      return `${p.checked.toLocaleString()}${p.total != null ? ` of ${p.total.toLocaleString()}` : ""} gifts checked · ${p.pages} pages`;
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

export function portfolioSummary(items: readonly PortfolioItem[]) {
  const priced: string[] = [];
  const collections = new Set<string>();
  for (const item of items) {
    if (item.estimate != null) priced.push(item.estimate);
    collections.add(item.gift.gift.collection);
  }
  return {
    count: items.length,
    unpriced: items.length - priced.length,
    collections: collections.size,
    total: priced.length || !items.length ? sumAmounts(priced) : undefined,
  };
}
