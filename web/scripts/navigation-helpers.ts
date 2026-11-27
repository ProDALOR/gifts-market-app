/** Test entry points go through the same target conversion and startup routes as the app. */
import { encodeTarget, decodeTarget, targetParams } from "../src/targets.ts";
import { selectionTarget } from "../src/telegram.ts";
import type { TelegramContext } from "../src/telegram.ts";
import { initialLocation } from "../src/app/routes.ts";
import { selectionAt } from "../src/market-selection.ts";
export function marketLaunch(params: URLSearchParams): string | null {
  try {
    return encodeTarget(selectionTarget(params));
  } catch {
    return null;
  }
}
export function decodedMarket(payload: string): URLSearchParams | null {
  const target = decodeTarget(payload);
  return target && target.type !== "portfolio" ? targetParams(target) : null;
}
export function launchParams(
  context: TelegramContext,
  search: string,
): URLSearchParams {
  const url = new URL("https://market.test/");
  url.search = search;
  return selectionAt(
    new URL(initialLocation(context, url, null).href, url.origin),
  );
}
export function portfolioLaunch(params: URLSearchParams): string | null {
  const input = new URLSearchParams(params);
  input.set("page", "portfolio");
  return marketLaunch(input);
}
export function decodedPortfolio(payload: string): URLSearchParams | null {
  const target = decodeTarget(payload);
  return target?.type === "portfolio" ? targetParams(target) : null;
}
