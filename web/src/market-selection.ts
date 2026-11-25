/** Public navigation contract. REST and WebSocket order payloads remain unchanged. */
import type { components } from "./gen/api";
import type { TraitFilters, Sort, BidMatch } from "./market";
export type MarketSelection = components["schemas"]["MarketSelection"];
export const MAX_SELECTION_BYTES = 256 * 1024;
const encoder = new TextEncoder();
const fail = (message: string): never => {
  throw new Error(message);
};
const atom = (value: string) =>
  encodeURIComponent(value)
    .replace(
      /[!'()*]/g,
      (c) => `%${c.charCodeAt(0).toString(16).toUpperCase()}`,
    )
    .replaceAll("%40", "@")
    .replaceAll("%3A", ":");
function reference(value: unknown): string {
  if (
    typeof value !== "string" ||
    !value ||
    value.trim() !== value ||
    /[\u0000-\u001f\u007f-\u009f]/.test(value) ||
    encoder.encode(value).length > 256
  )
    return fail("Invalid catalog reference");
  if (value.startsWith("@")) {
    const match = /^@([A-Za-z0-9._-]{1,64}):(.+)$/.exec(value);
    if (
      !match ||
      match[2].trim() !== match[2] ||
      (match[1] === "slug" && !/^[A-Za-z0-9._-]+$/.test(match[2]))
    )
      return fail("Invalid catalog reference");
  }
  return value;
}
function list(value: unknown): string[] {
  if (!Array.isArray(value)) return fail("Invalid attribute selection");
  return value.map(reference);
}
function giftNumber(value: unknown): number {
  if (
    typeof value !== "number" ||
    !Number.isInteger(value) ||
    value < 1 ||
    value > 4294967295
  )
    return fail("Invalid gift number");
  return value;
}
function range(
  value: unknown,
): { min: number | null; max: number | null } | null {
  if (value == null) return null;
  if (typeof value !== "object" || Array.isArray(value))
    return fail("Invalid gift number range");
  const r = value as { min?: unknown; max?: unknown };
  const result = {
    min: r.min == null ? null : giftNumber(r.min),
    max: r.max == null ? null : giftNumber(r.max),
  };
  if (result.min != null && result.max != null && result.min > result.max)
    return fail("Invalid gift number range");
  return result;
}
function exactAmount(value: string): string {
  if (!/^\+?(?:\d+(?:\.\d*)?|\.\d+)$/.test(value)) return fail("Invalid price");
  const [raw, fraction = ""] = value.replace(/^\+/, "").split("."),
    whole = (raw || "0").replace(/^0+(?=\d)/, "");
  const tail = fraction.replace(/0+$/, "");
  if (tail.length > 28 || BigInt(whole + tail) > 79228162514264337593543950335n)
    return fail("Invalid price");
  return whole + (tail ? `.${tail}` : "");
}
export function filterLine(selection: MarketSelection): string {
  const f = selection.filters,
    parts: string[] = [];
  if (f.collection_id) parts.push(`c=${atom(f.collection_id)}`);
  for (const [key, ids] of [
    ["m", f.models],
    ["b", f.backdrops],
    ["s", f.patterns],
  ] as const)
    if (ids?.length) parts.push(`${key}=${ids.map(atom).join(",")}`);
  const n = f.number;
  if (n && (n.min != null || n.max != null))
    parts.push(
      `n=${n.min != null && n.min === n.max ? n.min : `${n.min ?? ""}..${n.max ?? ""}`}`,
    );
  return parts.join("|");
}
function applyLine(selection: MarketSelection, line: string) {
  if (!line) return;
  const seen = new Set<string>();
  for (const part of line.split("|")) {
    const eq = part.indexOf("=");
    if (eq < 1) return fail("Filter groups use key=value");
    const key = part.slice(0, eq),
      value = part.slice(eq + 1);
    if (seen.has(key)) return fail("Repeated filter group");
    seen.add(key);
    if (!value) return fail("Empty filter group");
    if (key === "n") {
      const pieces = value.split("..");
      if (
        pieces.length > 2 ||
        !pieces.some(Boolean) ||
        pieces.some((p) => p && !/^\d+$/.test(p))
      )
        return fail("Invalid gift number");
      selection.filters.number =
        pieces.length === 1
          ? range({ min: Number(value), max: Number(value) })
          : range({
              min: pieces[0] ? Number(pieces[0]) : null,
              max: pieces[1] ? Number(pieces[1]) : null,
            });
      continue;
    }
    const ids = value.split(",").map((v) => reference(decodeURIComponent(v)));
    if (key === "c") {
      if (ids.length !== 1) return fail("Select one collection");
      selection.filters.collection_id = ids[0];
    } else if (key === "m") selection.filters.models = ids;
    else if (key === "b") selection.filters.backdrops = ids;
    else if (key === "s") selection.filters.patterns = ids;
    else return fail("Unknown filter group");
  }
}
export function selectionFromParams(params: URLSearchParams): MarketSelection {
  if (encoder.encode(params.toString()).length > MAX_SELECTION_BYTES)
    return fail("Market selection is too large");
  const keys = [
    "f",
    "collection",
    "currency",
    "venue",
    "mode",
    "bid_match",
    "sort",
    "filters",
    "min",
    "max",
    "from",
    "until",
    "gift",
  ];
  for (const key of keys)
    if (params.getAll(key).length > 1)
      return fail("Repeated selection parameter");
  const get = (key: string) => params.get(key) || undefined;
  const mode = get("mode") ?? "ask";
  const source =
    mode === "ask"
      ? "asks"
      : mode === "bid"
        ? "bids"
        : mode === "executed"
          ? "history"
          : fail("Invalid market mode");
  const sort = get("sort");
  if (
    sort &&
    ![
      "price_asc",
      "price_desc",
      "time_asc",
      "time_desc",
      "number_asc",
      "number_desc",
    ].includes(sort)
  )
    return fail("Invalid sort");
  const match = get("bid_match") ?? "all";
  if (match !== "all" && match !== "any") return fail("Invalid bid matching");
  const selection: MarketSelection = {
    filters: {
      currency_id: reference(get("currency") ?? "GRAM"),
      collection_id:
        !params.has("f") && get("collection")
          ? reference(get("collection"))
          : undefined,
      venue_id: get("venue") ? reference(get("venue")) : null,
      models: [],
      backdrops: [],
      patterns: [],
      number: null,
      price: null,
    },
    source,
    bid_match: match,
    sort: (sort as MarketSelection["sort"]) ?? null,
    period: null,
    gift: get("gift") ?? null,
  };
  if (!params.has("f") && get("filters")) {
    const old = JSON.parse(get("filters")!) as Record<string, unknown>;
    if (
      !old ||
      typeof old !== "object" ||
      Array.isArray(old) ||
      Object.keys(old).some(
        (k) => !["models", "backdrops", "patterns", "number"].includes(k),
      )
    )
      return fail("Invalid legacy filters");
    for (const key of ["models", "backdrops", "patterns"] as const)
      selection.filters[key] = old[key] == null ? [] : list(old[key]);
    selection.filters.number = range(old.number);
  }
  if (params.has("f")) {
    selection.filters.collection_id = undefined;
    applyLine(selection, params.get("f")!);
  }
  const min = get("min"),
    max = get("max");
  if (min || max) {
    const low = min ? exactAmount(min) : null,
      high = max ? exactAmount(max) : null;
    const units = (v: string) => {
      const [w, f = ""] = v.split(".");
      return BigInt(w + f.padEnd(28, "0"));
    };
    if (low != null && high != null && units(low) > units(high))
      return fail("Invalid market filter range");
    selection.filters.price = { min: low, max: high };
  }
  const from = get("from"),
    until = get("until");
  if (from || until) {
    if (
      [from, until].some((v) => v && !Number.isFinite(Date.parse(v))) ||
      (from && until && Date.parse(from) > Date.parse(until))
    )
      return fail("Invalid history period");
    selection.period = { from: from ?? null, until: until ?? null };
  }
  if (
    (selection.filters.models?.length ?? 0) &&
    !selection.filters.collection_id
  )
    return fail("Model filters require a collection");
  if (
    (selection.filters.models?.length ?? 0) +
      (selection.filters.backdrops?.length ?? 0) +
      (selection.filters.patterns?.length ?? 0) >
    4096
  )
    return fail("Market selection is too large");
  if (selection.gift) {
    const m = /^([A-Za-z0-9]{1,128})-([0-9]+)$/.exec(selection.gift);
    if (!m || Number(m[2]) < 1 || Number(m[2]) > 4294967295)
      return fail("Invalid gift slug");
    selection.gift = `${m[1]}-${Number(m[2])}`;
  }
  return selection;
}
export function selectionParams(selection: MarketSelection): URLSearchParams {
  const p = new URLSearchParams(),
    f = filterLine(selection);
  p.set("f", f);
  p.set("currency", selection.filters.currency_id);
  if (selection.filters.venue_id) p.set("venue", selection.filters.venue_id);
  p.set(
    "mode",
    selection.source === "bids"
      ? "bid"
      : selection.source === "history"
        ? "executed"
        : "ask",
  );
  if (selection.source === "bids" || selection.bid_match === "any")
    p.set("bid_match", selection.bid_match ?? "all");
  if (selection.sort) p.set("sort", selection.sort);
  if (selection.filters.price?.min != null)
    p.set("min", selection.filters.price.min);
  if (selection.filters.price?.max != null)
    p.set("max", selection.filters.price.max);
  if (selection.period?.from) p.set("from", selection.period.from);
  if (selection.period?.until) p.set("until", selection.period.until);
  if (selection.gift) p.set("gift", selection.gift);
  if (encoder.encode(p.toString()).length > MAX_SELECTION_BYTES)
    return fail("Market selection is too large");
  return p;
}
export type MarketChoice = {
  collection: string;
  currency: string;
  venue: string;
  mode: "ask" | "bid" | "executed";
  bid_match: BidMatch;
  sort: Exclude<Sort, "identity">;
  min: string;
  max: string;
  from: string;
  until: string;
  filters: TraitFilters;
  number?: MarketSelection["filters"]["number"];
};
export const defaultSort = {
  ask: "price_asc",
  bid: "price_desc",
  executed: "time_desc",
} as const;
export function selectionChoice(selection: MarketSelection): MarketChoice {
  const f = selection.filters,
    mode =
      selection.source === "bids"
        ? "bid"
        : selection.source === "history"
          ? "executed"
          : "ask";
  return {
    collection: f.collection_id ?? "",
    currency: f.currency_id,
    venue: f.venue_id ?? "",
    mode,
    bid_match: selection.bid_match ?? "all",
    sort: (selection.sort as MarketChoice["sort"]) ?? defaultSort[mode],
    min: f.price?.min ?? "",
    max: f.price?.max ?? "",
    from: selection.period?.from ?? "",
    until: selection.period?.until ?? "",
    filters: {
      ...(f.models?.length ? { models: f.models } : {}),
      ...(f.backdrops?.length ? { backdrops: f.backdrops } : {}),
      ...(f.patterns?.length ? { patterns: f.patterns } : {}),
    },
    number: f.number,
  };
}
export function choiceSelection(
  choice: MarketChoice,
  gift?: string,
): MarketSelection {
  return {
    filters: {
      collection_id: choice.collection || undefined,
      currency_id: choice.currency || "GRAM",
      venue_id: choice.venue || null,
      models: choice.filters.models ?? [],
      backdrops: choice.filters.backdrops ?? [],
      patterns: choice.filters.patterns ?? [],
      number: choice.number ?? null,
      price:
        choice.min || choice.max
          ? { min: choice.min || null, max: choice.max || null }
          : null,
    },
    source:
      choice.mode === "bid"
        ? "bids"
        : choice.mode === "executed"
          ? "history"
          : "asks",
    bid_match: choice.bid_match,
    sort: choice.sort,
    period:
      choice.from || choice.until
        ? { from: choice.from || null, until: choice.until || null }
        : null,
    gift: gift ?? null,
  };
}

/** Long state belongs in a fragment: it is not sent to HTTP servers or proxies. */
export function selectionAt(
  url: Pick<URL, "search" | "hash">,
): URLSearchParams {
  const direct = new URLSearchParams(url.search),
    fragment = new URLSearchParams(url.hash.replace(/^#/, ""));
  if (fragment.has("f")) {
    const directKeys = new Set(direct.keys());
    const attrs = ["f", "collection", "filters"].some((key) => direct.has(key));
    for (const [key, value] of fragment)
      if (
        !directKeys.has(key) &&
        !(attrs && ["f", "collection", "filters"].includes(key))
      )
        direct.append(key, value);
  }
  return direct;
}
export function selectionHref(path: string, params: URLSearchParams): string {
  if (!params.size) return path;
  if (params.toString().length > 2048) {
    const all = new URLSearchParams(params);
    if (!all.has("f")) all.set("f", "");
    return `${path}#${all}`;
  }
  return `${path}?${params}`;
}
