// Synthetic page server for browser/controller tests. Production matching lives in Rust.
import { sumAmounts } from "../src/amount.ts";
export const portfolioVersion = (report) =>
  report.refresh_id.toString(16).padStart(32, "0");
export function selection(items) {
  const priced = items.filter((item) => item.estimate != null);
  return {
    count: items.length,
    priced: priced.length,
    unpriced: items.length - priced.length,
    unverified: items.filter((item) => item.gift.relation === "unverified")
      .length,
    collections: [
      ...new Set(
        items.flatMap((item) =>
          item.resolved ? [item.resolved.collection_id] : [],
        ),
      ),
    ],
    total:
      priced.length || !items.length
        ? sumAmounts(priced.map((item) => item.estimate))
        : null,
  };
}
export function portfolioSnapshot(report) {
  if (!report.valuation) return report;
  const { items, ...metadata } = report.valuation;
  return {
    ...report,
    valuation: {
      ...metadata,
      ...selection(items),
      version: portfolioVersion(report),
    },
  };
}
export function portfolioPage(report, params) {
  const version = portfolioVersion(report);
  if (params.version && params.version !== version)
    throw { code: "restart_read", message: "Portfolio changed" };
  const filters = params.filters || {};
  const items = report.valuation.items.filter((item) => {
    const gift = item.resolved;
    return (
      (!filters.collection_id ||
        gift?.collection_id === filters.collection_id) &&
      (!filters.number || item.gift.gift.number === filters.number) &&
      [
        ["models", "model"],
        ["backdrops", "backdrop"],
        ["patterns", "pattern"],
      ].every(
        ([many, one]) =>
          !filters[many]?.length || filters[many].includes(gift?.traits[one]),
      )
    );
  });
  const sort = params.sort || "value_desc";
  const asc = sort.endsWith("asc");
  const decimal = (value) => {
    const [whole, fraction = ""] = value.split(".");
    return BigInt(whole) * 10n ** 28n + BigInt(fraction.padEnd(28, "0"));
  };
  if (sort === "position_desc") items.reverse();
  else if (sort !== "position_asc")
    items.sort((a, b) => {
      if (sort.startsWith("value")) {
        if (a.estimate == null || b.estimate == null)
          return a.estimate == null ? (b.estimate == null ? 0 : 1) : -1;
        const left = decimal(a.estimate),
          right = decimal(b.estimate);
        if (left !== right) return (left < right ? -1 : 1) * (asc ? 1 : -1);
      }
      return (
        (a.gift.gift.number - b.gift.gift.number) *
        (sort.startsWith("number") && !asc ? -1 : 1)
      );
    });
  const offset = params.offset || 0,
    limit = params.limit || 100;
  return {
    version,
    offset,
    items: items.slice(offset, offset + limit),
    has_more: offset + limit < items.length,
    selection: selection(items),
  };
}
