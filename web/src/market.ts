import { ApiFailure } from "./api-error.ts";
import { giftPresentation, type PreviewOrder } from "./order-target.ts";
export { ApiFailure } from "./api-error.ts";
import { liveSocket } from "./live-socket.ts";
import type { components } from "./gen/api";
export type ValuationStrategy = components["schemas"]["StrategyResponse"];
export const valuationStrategies = (
  signal?: AbortSignal,
): Promise<ValuationStrategy[]> =>
  request("/valuations/strategies", undefined, signal);
export type ValuationQuery = components["schemas"]["ValuationQuery"];
export type ValuationResponse = components["schemas"]["ValuationResponse"];
export const valuation = (
  query: ValuationQuery,
  signal?: AbortSignal,
): Promise<ValuationResponse> =>
  liveSocket.request({ method: "valuation.query", params: query }, signal);
export type MarketStatus = components["schemas"]["MarketStatus"];
export type Order = components["schemas"]["OrderResponse"];
export type Sort = components["schemas"]["GiftSort"];
export type OrderQuery = components["schemas"]["GiftOrderQuery"];
export type BidMatch = components["schemas"]["BidMatch"];
/** One source-oriented selection for live and explicitly frozen GUI reads. */
export type FeedQuery = Pick<
  OrderQuery,
  "filters" | "sort" | "period" | "bid_match"
> & {
  source: NonNullable<OrderQuery["source"]>;
};
export type GiftMedia = components["schemas"]["GiftMedia"];
export type CatalogVersions = components["schemas"]["CatalogVersions"];
export type Catalog = components["schemas"]["GiftCatalog"];
export type CollectionCatalog = components["schemas"]["GiftCollectionCatalog"];
export type Collection = components["schemas"]["GiftCollection"];
export type Model = components["schemas"]["GiftModel"];
export type Backdrop = components["schemas"]["GiftBackdrop"];
export type Pattern = components["schemas"]["GiftPattern"];
export type BackdropPalette = components["schemas"]["BackdropPalette"];
export type OrderQueryResult = components["schemas"]["OrderReadResponse"];
export type TraitKey = "models" | "backdrops" | "patterns";
export type TraitFilters = Partial<Record<TraitKey, string[]>>;
export type CatalogOption =
  | (Model & { kind: "model" })
  | (Backdrop & { kind: "backdrop" })
  | (Pattern & { kind: "pattern" });
export async function request<T>(
  path: string,
  body?: unknown,
  signal?: AbortSignal,
  token?: string,
  revision?: (value: string) => void,
): Promise<T> {
  const response = await fetch(path, {
    method: body === undefined ? "GET" : "POST",
    headers: {
      ...(body === undefined ? {} : { "Content-Type": "application/json" }),
      ...(token ? { Authorization: `Bearer ${token}` } : {}),
    },
    body: body === undefined ? undefined : JSON.stringify(body),
    signal,
  });
  const value = await response.json();
  if (!response.ok)
    throw new ApiFailure(
      value.error?.code || "unavailable",
      value.error?.message || "Market data is unavailable",
    );
  // Catalog manifests carry the same opaque token without HTTP's ETag quotes.
  const etag = response.headers.get("etag")?.match(/^(?:W\/)?"([^"]*)"$/);
  if (etag) revision?.(etag[1]);
  return value;
}
export const catalog = (
  signal: AbortSignal,
  revision?: (value: string) => void,
): Promise<Catalog> =>
  request("/catalog", undefined, signal, undefined, revision);
export const collectionCatalog = (
  id: string,
  signal: AbortSignal,
  revision?: (value: string) => void,
): Promise<CollectionCatalog> =>
  request(
    `/collections/${encodeURIComponent(id)}`,
    undefined,
    signal,
    undefined,
    revision,
  );
export { amount, roundedAmount } from "./amount.ts";
/** Concrete gifts and unambiguous executions supply artwork; bids stay criteria. */
export function modelMedia(
  order: PreviewOrder,
  options: ReadonlyMap<string, CatalogOption>,
): GiftMedia | undefined {
  const id = giftPresentation(order)?.traits.model;
  const model = id ? options.get(id) : undefined;
  return model?.kind === "model" ? model.media || undefined : undefined;
}
