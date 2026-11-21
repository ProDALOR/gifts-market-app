import { validateDefinitions } from "./presentation.ts";
import { valuationStrategies } from "../market.ts";
import type { ValuationQuery, ValuationStrategy } from "../market.ts";
import { ResourceCache } from "../resources/cache.ts";

/** Deployment metadata changes on use, without polling or per-detail duplicate requests. */
export class StrategyClient {
  private catalog: ResourceCache<ValuationStrategy[]>;
  constructor(load = valuationStrategies, now: () => number = Date.now) {
    this.catalog = new ResourceCache(
      async (_key, signal) => {
        const value = await load(signal);
        validateDefinitions(value);
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
  list(signal: AbortSignal): Promise<ValuationStrategy[]> {
    return this.catalog.get("catalog", signal);
  }
}
export const strategyClient = new StrategyClient();
/** Capability selection only; the server validates periods and resolves domain facts. */
export function compatibleStrategies(
  rows: readonly ValuationStrategy[],
  target: ValuationQuery["target"]["kind"],
  hasPeriod = false,
) {
  return rows.filter(
    (row) =>
      row.targets.includes(target) && (hasPeriod || !row.requires_period),
  );
}
export const guideKey = (strategy: ValuationStrategy) =>
  JSON.stringify([strategy.documentation_url, strategy.documentation_revision]);
