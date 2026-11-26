/** Stateless portfolio return links share the market's readable attribute codec. */
import { selectionFromParams, selectionParams } from "../market-selection.ts";
import type { PortfolioFilters, PortfolioSort } from "./model.ts";
export function portfolioFiltersFromParams(
  params: URLSearchParams,
): PortfolioFilters {
  const sort = params.get("sort") || "value_desc";
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
export function decodePortfolioLaunch(value: string): URLSearchParams | null {
  if (!/^p_[A-Za-z0-9_-]+$/.test(value) || value.length > 512) return null;
  try {
    const bytes = Uint8Array.from(
      atob(value.slice(2).replaceAll("-", "+").replaceAll("_", "/")),
      (c) => c.charCodeAt(0),
    );
    const input = new URLSearchParams(
      new TextDecoder("utf-8", { fatal: true }).decode(bytes),
    );
    for (const key of ["user", "currency", "f", "sort"])
      if (input.getAll(key).length !== 1) return null;
    const params = portfolioParams(
      portfolioFiltersFromParams(input),
      input.get("currency")!,
    );
    params.set("user", input.get("user")!);
    return params;
  } catch {
    return null;
  }
}
export function encodePortfolioLaunch(input: URLSearchParams): string | null {
  try {
    const user = input.get("user");
    if (
      !user ||
      !/^[1-9][0-9]*$/.test(user) ||
      !Number.isSafeInteger(Number(user))
    )
      return null;
    const params = portfolioParams(
      portfolioFiltersFromParams(input),
      input.get("currency") || "GRAM",
    );
    params.set("user", user);
    const bytes = new TextEncoder().encode(params.toString());
    if (bytes.length > Math.floor(((512 - 2) * 3) / 4)) return null;
    return (
      "p_" +
      btoa(String.fromCharCode(...bytes))
        .replaceAll("+", "-")
        .replaceAll("/", "_")
        .replace(/=+$/, "")
    );
  } catch {
    return null;
  }
}
