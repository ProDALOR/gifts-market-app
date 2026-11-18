import type { components } from "./gen/api";
export type GiftValuationRequest =
  components["schemas"]["GiftValuationRequest"];
export type GiftValuation = components["schemas"]["GiftValuationResponse"];
export type ValuationErrorDetails =
  components["schemas"]["ValuationErrorDetails"];
export type Order = components["schemas"]["OrderResponse"];
export type Book = components["schemas"]["GiftBook"];
export type Query = components["schemas"]["GiftOrdersQuery"];
export type HistoryQuery = components["schemas"]["GiftHistoryQuery"];
export type FeedQuery =
  { kind: "orders"; query: Query } | { kind: "history"; query: HistoryQuery };
export type GiftMedia = components["schemas"]["GiftMedia"];
export type CatalogVersions = components["schemas"]["CatalogVersions"];
export type Catalog = components["schemas"]["GiftCatalog"];
export type CollectionCatalog = components["schemas"]["GiftCollectionCatalog"];
export type Collection = components["schemas"]["GiftCollection"];
export type Model = components["schemas"]["GiftModel"];
export type Backdrop = components["schemas"]["GiftBackdrop"];
export type Pattern = components["schemas"]["GiftPattern"];
export type BackdropPalette = components["schemas"]["BackdropPalette"];
export type Page<T> = Omit<
  components["schemas"]["OrderPageResponse"],
  "data"
> & {
  data: T[];
};
export type TraitKey = "models" | "backdrops" | "patterns";
export type TraitFilters = Partial<Record<TraitKey, string[]>>;
export type CatalogOption =
  | (Model & { kind: "model" })
  | (Backdrop & { kind: "backdrop" })
  | (Pattern & { kind: "pattern" });
export class ApiFailure extends Error {
  constructor(
    public code: string,
    message: string,
    public details?: ValuationErrorDetails,
  ) {
    super(message);
  }
}
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
      value.error?.details,
    );
  const etag = response.headers.get("etag");
  if (etag) revision?.(etag.replace(/^W\//, "").replace(/^"|"$/g, ""));
  return value;
}
export const valuation = (
  query: GiftValuationRequest,
  signal?: AbortSignal,
): Promise<GiftValuation> => request("/valuations/gift", query, signal);
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
/** Exact string formatting; no conversion to JavaScript floating point. */
export function amount(value: string) {
  const [whole, fraction] = value.split(".");
  return (
    whole.replace(/\B(?=(\d{3})+(?!\d))/g, ",") +
    (fraction ? "." + fraction : "")
  );
}
/** Previews represent actual collectibles, never a bid's hypothetical target. */
export function modelMedia(
  order: Pick<Order, "target">,
  options: ReadonlyMap<string, CatalogOption>,
): GiftMedia | undefined {
  const id =
    order.target.kind === "gift" ? order.target.gift.traits.model : undefined;
  const model = id ? options.get(id) : undefined;
  return model?.kind === "model" ? model.media || undefined : undefined;
}
