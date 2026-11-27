import { selectionAt, selectionHref } from "../market-selection.ts";
import { mergeTargetParams } from "../targets.ts";
import { cleanSelection, launchTarget } from "../telegram.ts";
import type { TelegramContext } from "../telegram.ts";
export type Page = "market" | "portfolio" | "profile";
export type Route = {
  page: Page;
  user?: number;
  index: number;
  returnTo?: Page;
};
export const returnPage = (value: unknown): Page | undefined =>
  value === "market" || value === "portfolio" || value === "profile"
    ? value
    : undefined;
export function pagePath(page: Page, user?: number) {
  if (page === "portfolio") return `/portfolio${user ? `/${user}` : ""}`;
  return page === "profile" ? "/profile" : "/";
}
export const portfolioUser = (value: unknown) => {
  if (typeof value !== "number" && typeof value !== "string") return undefined;
  if (typeof value === "string" && !/^[0-9]+$/.test(value)) return undefined;
  const id = Number(value);
  return Number.isSafeInteger(id) && id > 0 ? id : undefined;
};
export function pathRoute(path: string): Omit<Route, "index"> {
  if (/^\/profile\/?$/.test(path)) return { page: "profile" };
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
  } else if (route.page === "profile") {
    for (const key of [...params.keys()])
      if (key !== "currency") params.delete(key);
  }
  const currency = direct.get("currency") || params.get("currency") || "GRAM";
  if (route.page !== "market") params.set("currency", currency);
  return {
    route: {
      ...route,
      index:
        Number.isSafeInteger(remembered?.index) && remembered!.index >= 0
          ? remembered!.index
          : 0,
      ...(returnPage(remembered?.returnTo)
        ? { returnTo: returnPage(remembered?.returnTo) }
        : {}),
    },
    href: selectionHref(pagePath(route.page, route.user), params),
    currency,
  };
}
