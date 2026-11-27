import { selectionAt, selectionHref } from "../market-selection.ts";
import { mergeTargetParams } from "../targets.ts";
import { cleanSelection, launchTarget } from "../telegram.ts";
import type { TelegramContext } from "../telegram.ts";
export type Page = "market" | "portfolio";
export type Route = { page: Page; user?: number; index: number };
export const portfolioUser = (value: unknown) => {
  if (typeof value !== "number" && typeof value !== "string") return undefined;
  if (typeof value === "string" && !/^[0-9]+$/.test(value)) return undefined;
  const id = Number(value);
  return Number.isSafeInteger(id) && id > 0 ? id : undefined;
};
export function pathRoute(path: string): Omit<Route, "index"> {
  const match = /^\/portfolio(?:\/(\d+))?\/?$/.exec(path);
  return match
    ? { page: "portfolio", user: portfolioUser(match[1]) }
    : { page: "market" };
}
export function initialLocation(
  context: TelegramContext,
  url: URL,
  state: unknown,
) {
  const remembered =
    state && typeof state === "object" && "giftsMarket" in state
      ? (state as { giftsMarket?: Route }).giftsMarket
      : undefined;
  let route = pathRoute(url.pathname);
  const direct = selectionAt(url);
  const explicitMarket =
    direct.get("page") === "market" ||
    direct.has("f") ||
    direct.has("filters") ||
    direct.has("collection") ||
    direct.has("gift");
  const launch =
    !remembered && !explicitMarket && route.page === "market"
      ? launchTarget(context, direct)
      : null;
  if (launch?.type === "portfolio")
    route = { page: "portfolio", user: launch.target.user_id };
  const restored = launch ? mergeTargetParams(direct, launch) : direct;
  const params =
    route.page === "market"
      ? cleanSelection(restored)
      : new URLSearchParams(restored);
  if (route.page === "portfolio") {
    const allowed = new Set(["currency", "f", "collection", "filters", "sort"]);
    for (const key of [...params.keys()])
      if (!allowed.has(key)) params.delete(key);
  }
  const currency = direct.get("currency") || params.get("currency") || "GRAM";
  if (route.page === "portfolio") params.set("currency", currency);
  const path =
    route.page === "portfolio"
      ? `/portfolio${route.user ? `/${route.user}` : ""}`
      : "/";
  return {
    route: {
      ...route,
      index:
        Number.isSafeInteger(remembered?.index) && remembered!.index >= 0
          ? remembered!.index
          : 0,
    },
    href: selectionHref(path, params),
    currency,
  };
}
