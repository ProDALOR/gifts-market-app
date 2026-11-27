/** Stateless navigation subjects shared by website, Mini App and bot entry points. */
import {
  selectionFromParams,
  selectionParams,
  MAX_SELECTION_BYTES,
} from "./market-selection.ts";
import type { MarketSelection } from "./market-selection.ts";
import type { PortfolioSort } from "./portfolio/model.ts";
export type PortfolioLink = {
  user_id: number;
  currency_id: string;
  sort?: PortfolioSort;
  filters?: {
    collection_id?: string;
    models?: string[];
    backdrops?: string[];
    patterns?: string[];
    number?: number;
  };
};
export type Target =
  | { type: "gift"; slug: string; currency_id: string }
  | { type: "market"; selection: MarketSelection }
  | { type: "portfolio"; target: PortfolioLink };
const currency = (value: string) =>
  ({ TON: "GRAM", STARS: "XTR" })[value.toUpperCase()] ?? value.toUpperCase();
function user(value: unknown): number {
  if (
    !/^[1-9][0-9]*$/.test(String(value)) ||
    !Number.isSafeInteger(Number(value))
  )
    throw Error("Invalid portfolio user");
  return Number(value);
}
function portfolioSelection(p: PortfolioLink): MarketSelection {
  const f = p.filters;
  return {
    filters: {
      collection_id: f?.collection_id,
      currency_id: currency(p.currency_id),
      venue_id: null,
      models: f?.models ?? [],
      backdrops: f?.backdrops ?? [],
      patterns: f?.patterns ?? [],
      number: f?.number == null ? null : { min: f.number, max: f.number },
      price: null,
    },
  };
}
export function targetParams(target: Target): URLSearchParams {
  let p: URLSearchParams;
  if (target.type === "market") {
    p = new URLSearchParams({ t: "market" });
    for (const [k, v] of selectionParams(
      selectionFromParams(selectionParams(target.selection)),
    ))
      p.append(k, v);
  } else if (target.type === "gift") {
    const checked = selectionFromParams(
      new URLSearchParams({
        gift: target.slug,
        currency: currency(target.currency_id),
      }),
    );
    p = new URLSearchParams({
      t: "gift",
      gift: checked.gift!,
      currency: checked.filters.currency_id,
    });
  } else {
    const selected = selectionFromParams(
        selectionParams(portfolioSelection(target.target)),
      ),
      sort = target.target.sort ?? "value_desc";
    if (!/^(position|number|value)_(asc|desc)$/.test(sort))
      throw Error("Invalid portfolio sort");
    p = new URLSearchParams({
      t: "portfolio",
      user: String(user(target.target.user_id)),
      currency: selected.filters.currency_id,
      f: selectionParams(selected).get("f")!,
      sort,
    });
  }
  if (new TextEncoder().encode(p.toString()).length > MAX_SELECTION_BYTES)
    throw Error("Selection is too large");
  return p;
}
export function targetFromParams(input: URLSearchParams): Target {
  for (const key of input.keys())
    if (input.getAll(key).length > 1) throw Error("Repeated target parameter");
  const p = new URLSearchParams(input),
    type =
      p.get("t") ??
      (p.has("user")
        ? "portfolio"
        : p.has("gift") &&
            !p.get("f") &&
            ![
              "collection",
              "filters",
              "mode",
              "bid_match",
              "sort",
              "venue",
              "min",
              "max",
              "from",
              "until",
            ].some((k) => p.has(k))
          ? "gift"
          : "market");
  p.delete("t");
  p.set("currency", currency(p.get("currency") || "GRAM"));
  if (
    (type === "gift" || type === "market") &&
    [...p.keys()].some(
      (k) =>
        ![
          "gift",
          "currency",
          "f",
          "collection",
          "mode",
          "bid_match",
          "sort",
          "venue",
          "filters",
          "min",
          "max",
          "from",
          "until",
        ].includes(k),
    )
  )
    throw Error("Invalid target");
  if (type === "gift") {
    const s = selectionFromParams(p);
    if (!s.gift) throw Error("Missing gift slug");
    return { type, slug: s.gift, currency_id: s.filters.currency_id };
  }
  if (type === "market") return { type, selection: selectionFromParams(p) };
  if (
    type !== "portfolio" ||
    [...p.keys()].some(
      (k) =>
        !["user", "currency", "f", "sort", "collection", "filters"].includes(k),
    )
  )
    throw Error("Invalid target");
  const id = user(p.get("user")),
    sort = (p.get("sort") || "position_asc").replace(
      /^price_/,
      "value_",
    ) as PortfolioSort;
  p.delete("sort");
  p.delete("user");
  if (!/^(position|number|value)_(asc|desc)$/.test(sort))
    throw Error("Invalid portfolio sort");
  const f = selectionFromParams(p).filters,
    n = f.number;
  if (n && (n.min == null || n.min !== n.max))
    throw Error("Portfolio number must be exact");
  return {
    type,
    target: {
      user_id: id,
      currency_id: f.currency_id,
      sort,
      filters: {
        collection_id: f.collection_id ?? undefined,
        models: f.models,
        backdrops: f.backdrops,
        patterns: f.patterns,
        ...(n?.min == null ? {} : { number: n.min }),
      },
    },
  };
}
export function encodeTarget(target: Target, limit = 512): string | null {
  try {
    const bytes = new TextEncoder().encode(targetParams(target).toString());
    if (bytes.length > Math.floor(((limit - 2) * 3) / 4)) return null;
    const result =
      "t_" +
      btoa(String.fromCharCode(...bytes))
        .replaceAll("+", "-")
        .replaceAll("/", "_")
        .replace(/=+$/, "");
    return result.length <= limit ? result : null;
  } catch {
    return null;
  }
}
export function decodeTarget(value: string): Target | null {
  if (value.length > 512) return null;
  try {
    if (/^portfolio_\d+$/.test(value))
      return targetFromParams(
        new URLSearchParams({
          t: "portfolio",
          user: String(Number(value.slice(10))),
        }),
      );
    if (!/^[tvp]_[A-Za-z0-9_-]+$/.test(value)) return null;
    const bytes = Uint8Array.from(
      atob(value.slice(2).replaceAll("-", "+").replaceAll("_", "/")),
      (c) => c.charCodeAt(0),
    );
    const p = new URLSearchParams(
      new TextDecoder("utf-8", { fatal: true }).decode(bytes),
    );
    if (value.startsWith("v_") || value.startsWith("p_")) {
      if (p.has("t")) return null;
      p.set("t", value.startsWith("v_") ? "market" : "portfolio");
    } else if (!p.has("t")) return null;
    return targetFromParams(p);
  } catch {
    return null;
  }
}
export function targetInlineQuery(target: Target): string {
  const p = targetParams(target);
  if (target.type === "gift") return `${p.get("gift")} ${p.get("currency")}`;
  p.delete("t");
  return `${target.type} ${[...p].map(([k, v]) => `${k}=${k === "f" ? v : new URLSearchParams({ v }).toString().slice(2)}`).join(" ")}`;
}
export function targetWebsiteUrl(target: Target, origin: string): string {
  const url = new URL(origin);
  if (
    !["http:", "https:"].includes(url.protocol) ||
    url.username ||
    url.password
  )
    throw Error("Invalid app URL");
  const p = targetParams(target);
  p.delete("t");
  if (target.type === "portfolio") {
    url.pathname = `${url.pathname.replace(/\/$/, "")}/portfolio/${target.target.user_id}`;
    p.delete("user");
  }
  // An explicit attribute scope prevents Telegram launch state leaking into a direct gift URL.
  if (target.type === "gift") p.set("f", "");
  url.search = "";
  url.hash = p.toString();
  return url.href;
}
function botBase(username: string): URL {
  if (!/^[A-Za-z0-9_]+$/.test(username)) throw Error("Invalid bot username");
  return new URL(`https://t.me/${username}`);
}
export function targetBotUrl(
  target: Target,
  username: string,
  origin?: string,
): string {
  const url = botBase(username),
    payload = encodeTarget(target, 64);
  if (payload) url.searchParams.set("start", payload);
  else {
    const text = targetInlineQuery(target);
    if ([...text].length <= 4096) url.searchParams.set("text", text);
    else {
      if (!origin) throw Error("Selection is too large");
      return targetWebsiteUrl(target, origin);
    }
  }
  return url.href;
}
export function targetAppUrl(
  target: Target,
  username: string,
  origin?: string,
): string | null {
  const url = botBase(username),
    payload = encodeTarget(target);
  if (payload) {
    url.searchParams.set("startapp", payload);
    return url.href;
  }
  return origin ? targetWebsiteUrl(target, origin) : null;
}

/** Direct URL state overrides launch defaults; attribute groups replace each other atomically. */
export function mergeTargetParams(
  input: URLSearchParams,
  target: Target,
): URLSearchParams {
  const direct = new URLSearchParams(input),
    attributes = ["f", "collection", "filters"].some((k) => direct.has(k));
  for (const [key, value] of targetParams(target))
    if (
      !direct.has(key) &&
      !(attributes && ["f", "collection", "filters"].includes(key))
    )
      direct.set(key, value);
  return direct;
}
