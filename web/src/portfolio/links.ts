/** Stateless portfolio return links share the market's readable attribute codec. */
import { selectionFromParams, selectionParams } from "../market-selection.ts";
import type { PortfolioFilters, PortfolioSort } from "./model.ts";
export function portfolioFiltersFromParams(
  params: URLSearchParams,
): PortfolioFilters {
  for (const key of ["user", "sort"])
    if (params.getAll(key).length > 1)
      throw Error("Repeated portfolio parameter");
  const sort = (params.get("sort") || "value_desc").replace(
    /^price_/,
    "value_",
  );
  if (!/^(position|number|value)_(asc|desc)$/.test(sort))
    throw Error("Invalid portfolio sort");
  const input = new URLSearchParams(params);
  input.delete("sort");
  const { filters } = selectionFromParams(input);
  const number = filters.number;
  if (number && (number.min == null || number.min !== number.max))
    throw Error("Portfolio gift number must be exact");
  return {
    collection: filters.collection_id || "",
    traits: {
      models: filters.models,
      backdrops: filters.backdrops,
      patterns: filters.patterns,
    },
    number: number?.min == null ? "" : String(number.min),
    sort: sort as PortfolioSort,
  };
}
export function portfolioParams(filters: PortfolioFilters, currency: string) {
  const number = filters.number ? Number(filters.number) : undefined;
  const market = selectionParams({
    filters: {
      collection_id: filters.collection || undefined,
      currency_id: currency,
      venue_id: null,
      models: filters.traits.models ?? [],
      backdrops: filters.traits.backdrops ?? [],
      patterns: filters.traits.patterns ?? [],
      number: number == null ? null : { min: number, max: number },
      price: null,
    },
  });
  return new URLSearchParams({
    currency,
    f: market.get("f") || "",
    sort: filters.sort,
  });
}
