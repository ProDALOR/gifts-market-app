import type { FloorValuation, ValuationResponse } from "../market.ts";
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
  const traits =
    report.target.kind === "gift"
      ? report.target.gift.traits
      : report.target.kind === "gift_attributes"
        ? report.target.traits
        : undefined;
  if (!traits) return undefined;
  const result = report.results.find((row) => row.strategy === strategy);
  if (!result || "error" in result || result.strategy !== "gift_floor")
    return undefined;
  const data = result.data;

  const model = traits.model,
    backdrop = traits.backdrop;
  const pair = shared.traits?.pair;
  if (
    shared.traits &&
    ((model && !shared.traits.models[model]) ||
      (backdrop && !shared.traits.backdrops[backdrop]))
  )
    return true;
  return (
    !sameFloor(data.collection_floor, shared.floor) ||
    !sameFloor(
      data.model_floor,
      model ? shared.traits?.models[model] : undefined,
    ) ||
    !sameFloor(
      data.backdrop_floor,
      backdrop ? shared.traits?.backdrops[backdrop] : undefined,
    ) ||
    !sameFloor(
      data.model_backdrop_floor,
      pair && pair.model_id === model && pair.backdrop_id === backdrop
        ? pair.floor
        : undefined,
    )
  );
}

function sameFloor(basis: FloorValuation, current?: FloorValuation) {
  // A scope that has not loaded yet supplies no evidence of a change.
  if (!current) return true;
  if (basis.count !== current.count) return false;
  if ("value" in basis)
    return "value" in current && basis.value === current.value;
  if (!("error" in current)) return false;
  const a = basis.error,
    b = current.error;
  return (
    a.code === b.code &&
    a.message === b.message &&
    a.required_samples === b.required_samples &&
    a.available_samples === b.available_samples &&
    a.dependency === b.dependency
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
