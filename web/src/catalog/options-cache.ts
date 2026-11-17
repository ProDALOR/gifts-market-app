import { catalog } from "../market.ts";
import type { Entry } from "../market.ts";
import type { Collection, CatalogOption } from "../orderbook.ts";
import { traits } from "../orderbook.ts";
/** Commit complete metadata loads only; aborted/failed requests remain retryable. */
export class CatalogOptionsCache {
  private completed = new Map<string, string>();
  options: ReadonlyMap<string, CatalogOption> = new Map();
  constructor(private query: typeof catalog = catalog) {}
  async load(collection: Collection, signal: AbortSignal) {
    const key = JSON.stringify([collection.id, collection.attributes]);
    signal.throwIfAborted();
    if (this.completed.get(collection.id) === key) return this.options;
    // Only UI choice traits have catalog options. Numeric facts such as tg.number
    // have no allowed_values and must not trigger an unfiltered catalog scan.
    const rules = traits.flatMap(({ id }) =>
      collection.attributes[id] ? [collection.attributes[id]] : [],
    );
    const ids = [
      ...new Set(rules.flatMap((rule) => rule.allowed_values || [])),
    ];
    const rows: Entry[] = [];
    if (rules.some((rule) => rule.allowed_values == null)) {
      rows.push(...(await this.query("attribute_option", signal)));
    } else {
      const batches: string[][] = [];
      for (let index = 0; index < ids.length; index += 200)
        batches.push(ids.slice(index, index + 200));
      // Match the API page size and overlap independent batches, rather than
      // requesting 500 IDs and following three serial continuation requests.
      for (let index = 0; index < batches.length; index += 3) {
        const pages = await Promise.all(
          batches
            .slice(index, index + 3)
            .map((batch) => this.query("attribute_option", signal, batch)),
        );
        rows.push(...pages.flat());
      }
    }
    signal.throwIfAborted();
    this.options = new Map([
      ...this.options,
      ...rows
        .filter((row): row is CatalogOption => row.kind === "attribute_option")
        .map((row) => [row.id, row] as const),
    ]);
    this.completed.set(collection.id, key);
    return this.options;
  }
}
