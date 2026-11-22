import type { components } from "../gen/api.ts";
type CatalogFloors = components["schemas"]["CatalogFloorValuation"];
const record = (value: unknown): value is Record<string, unknown> =>
  !!value && typeof value === "object" && !Array.isArray(value);
const text = (value: unknown) => typeof value === "string";
const count = (value: unknown): value is number =>
  typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
function floor(value: unknown) {
  if (
    !record(value) ||
    !count(value.count) ||
    ["price", "evidence", "gifts", "listings"].some((key) =>
      Object.hasOwn(value, key),
    )
  )
    return false;
  return (
    (typeof value.value === "string" &&
      value.value.length <= 128 &&
      /^\d+(?:\.\d+)?$/.test(value.value) &&
      !Object.hasOwn(value, "error")) ||
    (record(value.error) &&
      text(value.error.code) &&
      text(value.error.message) &&
      !Object.hasOwn(value, "value"))
  );
}
/** Validate the projection at its typed consumer; the WS envelope remains strategy-independent. */
export function catalogFloors(value: unknown): value is CatalogFloors {
  if (
    !record(value) ||
    !Array.isArray(value.collections) ||
    value.collections.length < 1 ||
    value.collections.length > 8
  )
    return false;
  const ids = new Set();
  return value.collections.every((row) => {
    if (
      !record(row) ||
      !text(row.collection_id) ||
      ids.has(row.collection_id) ||
      !text(row.evaluated_at) ||
      !floor(row.floor)
    )
      return false;
    ids.add(row.collection_id);
    const traits = row.traits;
    return (
      traits == null ||
      (record(traits) &&
        traits.collection_id === undefined &&
        record(traits.models) &&
        Object.values(traits.models).every(floor) &&
        record(traits.backdrops) &&
        Object.values(traits.backdrops).every(floor) &&
        record(traits.patterns) &&
        Object.values(traits.patterns).every(floor) &&
        (traits.pair == null ||
          (record(traits.pair) &&
            text(traits.pair.model_id) &&
            text(traits.pair.backdrop_id) &&
            floor(traits.pair.floor))))
    );
  });
}
