import { selectionAt, selectionHref } from "../market-selection.ts";
import { decodePortfolioLaunch } from "../portfolio/links.ts";
import { cleanSelection, launchSelection } from "../telegram.ts";
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
  let portfolioLaunch: URLSearchParams | null = null;
  const direct = selectionAt(url);
  const explicitMarket =
    direct.get("page") === "market" ||
    direct.has("f") ||
    direct.has("filters") ||
    direct.has("collection") ||
    direct.has("gift");
  if (!remembered && !explicitMarket && route.page === "market") {
    const launch =
      context.app?.initDataUnsafe?.start_param ||
      direct.get("tgWebAppStartParam") ||
      new URLSearchParams(context.initData).get("start_param");
    portfolioLaunch = launch ? decodePortfolioLaunch(launch) : null;
    const match = launch && /^portfolio_(\d+)$/.exec(launch);
    const user = portfolioUser(portfolioLaunch?.get("user") || match?.[1]);
    if (user) route = { page: "portfolio", user };
  }
  const params =
    route.page === "market"
      ? cleanSelection(
          remembered || explicitMarket
            ? direct
            : launchSelection(context, direct.toString()),
        )
      : new URLSearchParams(portfolioLaunch || direct);
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
