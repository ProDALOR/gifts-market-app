import type { MarketStatus } from "../market.ts";
export function estimateIsStale(
  report: { stale: boolean } | undefined,
  evaluatedGeneration: number | undefined,
  current: { status: MarketStatus; generation: number },
) {
  return (
    !report ||
    report.stale ||
    current.status !== "available" ||
    evaluatedGeneration !== current.generation
  );
}
