import type { CatalogOption, Collection } from "../orderbook.ts";
import { traits } from "../orderbook.ts";
const indexes = new WeakMap<
  ReadonlyMap<string, CatalogOption>,
  WeakMap<Collection, Map<string, CatalogOption[]>>
>();
export function collectionOptions(
  collection: Collection | undefined,
  id: string,
  options: ReadonlyMap<string, CatalogOption>,
) {
  const trait = traits.find((trait) => trait.id === id);
  if (!collection || !trait) return [];
  let collections = indexes.get(options);
  if (!collections) {
    collections = new WeakMap();
    indexes.set(options, collections);
  }
  let selections = collections.get(collection);
  if (!selections) {
    selections = new Map();
    collections.set(collection, selections);
  }
  const cached = selections.get(id);
  if (cached) return cached;
  const result = [...options.values()]
    .filter((option) => option.kind === trait.kind)
    .sort((a, b) => a.name.localeCompare(b.name));
  selections.set(id, result);
  return result;
}
