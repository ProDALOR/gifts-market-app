import type { ValuationResponse } from "../market.ts";
import type { FloorsEntry } from "./floors-controller.ts";
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

/** Compare the original gift estimate with current inputs; the popup retains its own basis. */
export function giftBasisChanged(
  report: ValuationResponse,
  shared: Pick<FloorsEntry, "floor" | "traits">,
  strategy = "gift_floor",
) {
  if (report.target.kind !== "gift") return undefined;
  const result = report.results.find((row) => row.strategy === strategy);
  if (!result || "error" in result || result.strategy !== "gift_floor")
    return undefined;
  const data = result.data;

  const equal = (
    metric: typeof data.collection_floor,
    floor: typeof shared.floor | undefined,
    proof: typeof shared.floor.evidence | undefined,
  ) =>
    !floor ||
    !metric ||
    !proof ||
    JSON.stringify([metric, proof]) ===
      JSON.stringify([floor.price, floor.evidence]);
  const model = report.target.gift.traits.model,
    backdrop = report.target.gift.traits.backdrop;
  const pair = shared.traits?.pair;
  if (
    shared.traits &&
    ((model && !shared.traits.models[model]) ||
      (backdrop && !shared.traits.backdrops[backdrop]))
  )
    return true;
  return (
    !equal(data.collection_floor, shared.floor, data.evidence.collection) ||
    !equal(
      data.model_floor,
      model ? shared.traits?.models[model] : undefined,
      data.evidence.model,
    ) ||
    !equal(
      data.backdrop_floor,
      backdrop ? shared.traits?.backdrops[backdrop] : undefined,
      data.evidence.backdrop,
    ) ||
    !equal(
      data.model_backdrop_floor,
      pair && pair.model_id === model && pair.backdrop_id === backdrop
        ? pair.floor
        : undefined,
      data.evidence.model_backdrop,
    )
  );
}

/** Keep native sub-millisecond precision when comparing collection calculation times. */
export function evaluatedAfter(current: string, previous: string) {
  const parts = (value: string) => {
    const match = /^(.*:\d{2})(?:\.(\d{1,9}))?Z$/.exec(value);
    return match
      ? {
          second: Date.parse(match[1] + "Z"),
          fraction: (match[2] ?? "").padEnd(9, "0"),
        }
      : undefined;
  };
  const a = parts(current),
    b = parts(previous);
  return (
    !a ||
    !b ||
    !Number.isFinite(a.second) ||
    !Number.isFinite(b.second) ||
    a.second > b.second ||
    (a.second === b.second && a.fraction > b.fraction)
  );
}
