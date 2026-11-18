import { valuationStrategies } from "../market.ts";
import type { ValuationStrategy } from "../market.ts";
import { ResourceCache } from "../resources/cache.ts";

/** Deployment metadata changes on use, without polling or per-detail duplicate requests. */
export class StrategyClient {
  private missing = new WeakMap<ValuationStrategy[], Set<string>>();
  private catalog: ResourceCache<ValuationStrategy[]>;
  constructor(load = valuationStrategies, now: () => number = Date.now) {
    this.catalog = new ResourceCache(
      async (_key, signal) => {
        const value = await load(signal);
        return { value, bytes: JSON.stringify(value).length * 2 };
      },
      128 * 1024,
      1,
      60_000,
      now,
    );
  }
  invalidate() {
    this.catalog.invalidate("catalog");
  }
  async get(id: string, signal: AbortSignal): Promise<ValuationStrategy> {
    const cached = this.catalog.hasFresh("catalog");
    let rows = await this.catalog.get("catalog", signal);
    // An unfamiliar result ID can come from a newer deployment. Revalidate cached metadata once.
    if (
      !rows.some((row) => row.id === id) &&
      cached &&
      !this.missing.get(rows)?.has(id)
    ) {
      this.invalidate();
      rows = await this.catalog.get("catalog", signal);
    }
    const metadata = rows.find((row) => row.id === id);
    if (!metadata) {
      const missing = this.missing.get(rows) ?? new Set<string>();
      if (missing.size >= 64) missing.delete(missing.values().next().value!);
      missing.add(id);
      this.missing.set(rows, missing);
      throw new Error("This strategy's guide is unavailable.");
    }
    return metadata;
  }
}
export const strategyClient = new StrategyClient();
export const guideKey = (strategy: ValuationStrategy) =>
  JSON.stringify([strategy.documentation_url, strategy.documentation_revision]);
