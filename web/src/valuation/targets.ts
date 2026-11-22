import type { ValuationQuery, ValuationResponse } from "../market.ts";
type Catalog = Extract<ValuationQuery["target"], { kind: "catalog" }>;
type Criteria = Extract<
  ValuationQuery["target"],
  { kind: "selection" }
>["criteria"];

/** Resolved responses omit absent options; requests may explicitly send null. */
export function sameCatalogTarget(
  actual: ValuationResponse["target"],
  expected: Catalog,
) {
  return (
    actual.kind === "catalog" &&
    JSON.stringify([...actual.collection_ids].sort()) ===
      JSON.stringify([...expected.collection_ids].sort()) &&
    actual.details?.collection_id === expected.details?.collection_id &&
    actual.details?.pair?.model_id === expected.details?.pair?.model_id &&
    actual.details?.pair?.backdrop_id === expected.details?.pair?.backdrop_id
  );
}
export function sameCriteria(actual: Criteria, expected: Criteria) {
  const key = (value: Criteria) =>
    JSON.stringify([
      [...new Set(value.models)].sort(),
      [...new Set(value.backdrops)].sort(),
      [...new Set(value.patterns)].sort(),
      value.number?.min ?? null,
      value.number?.max ?? null,
    ]);
  return key(actual) === key(expected);
}

/** Attribute valuation does not require a listed identity; the entire supplied basis must echo back. */
export function sameGiftAttributes(
  actual: Extract<ValuationResponse["target"], { kind: "gift_attributes" }>,
  expected: Extract<ValuationQuery["target"], { kind: "gift_attributes" }>,
) {
  return (
    actual.collection_id === expected.collection_id &&
    (actual.number ?? null) === (expected.number ?? null) &&
    (["model", "backdrop", "pattern"] as const).every(
      (key) => (actual.traits[key] ?? null) === (expected.traits[key] ?? null),
    )
  );
}
