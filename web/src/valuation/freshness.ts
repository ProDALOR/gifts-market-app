import type { ValuationView } from "../market.ts";
import type { ValuesEntry } from "./market-values-controller.ts";
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
  report: ValuationView,
  shared: Pick<ValuesEntry, "floor" | "traits">,
) {
  if (!report.evidence || !report.gift || !report.collection_floor)
    return false;
  const equal = (
    metric: ValuationView["collection_floor"],
    floor: typeof shared.floor | undefined,
    proof: typeof shared.floor.evidence | undefined,
  ) =>
    !floor ||
    !metric ||
    !proof ||
    JSON.stringify([metric, proof]) ===
      JSON.stringify([floor.price, floor.evidence]);
  const model = report.gift.traits.model,
    backdrop = report.gift.traits.backdrop;
  const pair = shared.traits?.pair;
  if (
    shared.traits &&
    ((model && !shared.traits.models[model]) ||
      (backdrop && !shared.traits.backdrops[backdrop]))
  )
    return true;
  return (
    !equal(report.collection_floor, shared.floor, report.evidence.collection) ||
    !equal(
      report.model_floor,
      model ? shared.traits?.models[model] : undefined,
      report.evidence.model,
    ) ||
    !equal(
      report.backdrop_floor,
      backdrop ? shared.traits?.backdrops[backdrop] : undefined,
      report.evidence.backdrop,
    ) ||
    !equal(
      report.model_backdrop_floor,
      pair && pair.model_id === model && pair.backdrop_id === backdrop
        ? pair.floor
        : undefined,
      report.evidence.model_backdrop,
    )
  );
}
