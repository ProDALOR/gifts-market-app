import type { components } from "./gen/api";
export type Order = components["schemas"]["Order"];
export type Entry = components["schemas"]["CatalogEntry"];
export type Book = components["schemas"]["BookKey"];
export type Query = components["schemas"]["OrdersQuery"];
export type Predicate = components["schemas"]["Predicate"];
export type AttrValue = components["schemas"]["AttributeValue"];
export type Page<T> = {
  data: T[];
  next_page_token: string;
  cursor: string;
  sync?: components["schemas"]["SyncInfo"];
};
export class ApiFailure extends Error {
  constructor(
    public code: string,
    message: string,
  ) {
    super(message);
  }
}
export async function request<T>(
  path: string,
  body?: unknown,
  signal?: AbortSignal,
  token?: string,
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
  return value;
}
export async function catalog(
  kind: string,
  signal: AbortSignal,
  ids?: string[],
): Promise<Entry[]> {
  const result: Entry[] = [];
  let page_token = "";
  do {
    const page: Page<Entry> = await request(
      "/catalog/query",
      { kind, ids: ids || [], limit: 200, page_token },
      signal,
    );
    result.push(...page.data);
    page_token = page.next_page_token;
  } while (page_token && !signal.aborted);
  return result;
}
export async function books(signal: AbortSignal): Promise<Book[]> {
  const result: Book[] = [];
  let after: Book | null = null;
  do {
    const page: { data: Book[]; next_after: Book | null } = await request(
      "/books?limit=200" +
        (after ? "&after=" + encodeURIComponent(JSON.stringify(after)) : ""),
      undefined,
      signal,
    );
    result.push(...page.data);
    after = page.next_after;
  } while (after && !signal.aborted);
  return result;
}
/** Exact string formatting; no conversion to JavaScript floating point. */
export function amount(value: string) {
  const [whole, fraction] = value.split(".");
  return (
    whole.replace(/\B(?=(\d{3})+(?!\d))/g, ",") +
    (fraction ? "." + fraction : "")
  );
}
export function text(value: AttrValue, names: Map<string, string>): string {
  switch (value.type) {
    case "enum":
      return names.get(value.value) || value.value;
    case "object":
      return Object.entries(value.value || {})
        .map(([k, v]) => `${names.get(k) || k}: ${text(v, names)}`)
        .join(" · ");
    case "list":
      return (value.value || []).map((v) => text(v, names)).join(" · ");
    case "timestamp":
      return new Date(value.value).toLocaleString();
    default:
      return String(value.value);
  }
}

export function dateInput(value: string): string {
  const date = new Date(value);
  if (!value || !Number.isFinite(date.getTime())) return "";
  return new Date(date.getTime() - date.getTimezoneOffset() * 60_000)
    .toISOString()
    .slice(0, 19);
}
export function timestamp(value: string): string {
  return value ? new Date(value).toISOString() : "";
}
