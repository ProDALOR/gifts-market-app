import { collectionCatalog } from "../market.ts";
import type { CatalogOption, Collection } from "../market.ts";
/** Bounded LRU of complete collection responses; only active choices reach renderers. */
export class CollectionCatalogCache {
  private completed = new Map<
    string,
    { key: string; options: ReadonlyMap<string, CatalogOption> }
  >();
  options: ReadonlyMap<string, CatalogOption> = new Map();
  constructor(private query: typeof collectionCatalog = collectionCatalog) {}
  async load(collection: Collection, signal: AbortSignal) {
    const key = JSON.stringify(collection);
    signal.throwIfAborted();
    const cached = this.completed.get(collection.id);
    if (cached?.key === key) {
      this.completed.delete(collection.id);
      this.completed.set(collection.id, cached);
      return (this.options = cached.options);
    }
    const catalog = await this.query(collection.id, signal);
    signal.throwIfAborted();
    const rows: CatalogOption[] = [
      ...catalog.models.map((model) => ({ ...model, kind: "model" as const })),
      ...catalog.backdrops.map((backdrop) => ({
        ...backdrop,
        kind: "backdrop" as const,
      })),
      ...catalog.patterns.map((pattern) => ({
        ...pattern,
        kind: "pattern" as const,
      })),
    ];
    const options = new Map(rows.map((row) => [row.id, row]));
    this.completed.delete(collection.id);
    this.completed.set(collection.id, { key, options });
    while (
      this.completed.size > 8 ||
      [...this.completed.values()].reduce(
        (total, entry) => total + entry.options.size,
        0,
      ) > 10_000
    ) {
      this.completed.delete(this.completed.keys().next().value!);
    }
    return (this.options = options);
  }
}
