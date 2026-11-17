import type { CatalogOption, Collection } from "../orderbook.ts";
const indexes = new WeakMap<
  ReadonlyMap<string, CatalogOption>,
  WeakMap<Collection, Map<string, CatalogOption[]>>
>();
export function collectionOptions(
  collection: Collection | undefined,
  id: string,
  options: ReadonlyMap<string, CatalogOption>,
) {
  if (!collection?.attributes[id]) return [];
  let collections = indexes.get(options);
  if (!collections) {
    collections = new WeakMap();
    indexes.set(options, collections);
  }
  let attributes = collections.get(collection);
  if (!attributes) {
    attributes = new Map();
    collections.set(collection, attributes);
  }
  const cached = attributes.get(id);
  if (cached) return cached;
  const allowed = collection.attributes[id].allowed_values;
  const candidates = allowed
    ? [...new Set(allowed)].flatMap((key) => options.get(key) || [])
    : [...options.values()];
  const result = candidates
    .filter((option) => option.attribute_id === id)
    .sort((a, b) => a.name.localeCompare(b.name));
  attributes.set(id, result);
  return result;
}
