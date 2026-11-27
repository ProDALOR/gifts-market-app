import type { CatalogOption, Collection } from "../orderbook.ts";
import { traits } from "../orderbook.ts";
const indexes = new WeakMap<
  ReadonlyMap<string, CatalogOption>,
  Map<string, CatalogOption[]>
>();
export function collectionOptions(
  collection: Collection | undefined,
  id: string,
  options: ReadonlyMap<string, CatalogOption>,
) {
  const trait = traits.find((trait) => trait.id === id);
  if (!collection || !trait) return [];
  let selections = indexes.get(options);
  if (!selections) {
    selections = new Map();
    indexes.set(options, selections);
  }
  const cached = selections.get(id);
  if (cached) return cached;
  const result = [...options.values()]
    .filter((option) => option.kind === trait.kind)
    .sort((a, b) => a.name.localeCompare(b.name));
  selections.set(id, result);
  return result;
}
