import { Fragment, useLayoutEffect } from "react";
import type { ReactNode } from "react";
import type { TelegramContext } from "../telegram";
import { PortfolioPage } from "../portfolio/page";
import { ProfilePage } from "../profile/page";
import { NavigationProvider } from "./navigation";
import { SessionProvider } from "./session";
import { appearance, applyAppearance } from "./appearance";
import { applyTheme } from "../telegram";
import { CatalogProvider } from "../catalog/provider";
/** Routes consume launch data once. Only the visible page owns subscriptions and media effects. */
export function AppShell({
  context,
  market,
}: {
  context: TelegramContext;
  market: ReactNode;
}) {
  useLayoutEffect(() => {
    const update = () =>
      context.app ? applyTheme(context.app) : applyAppearance();
    update();
    const stop = appearance.subscribe(update);
    const media = matchMedia("(prefers-color-scheme: light)");
    media.addEventListener("change", update);
    return () => {
      stop();
      media.removeEventListener("change", update);
    };
  }, [context.app]);
  return (
    <SessionProvider context={context}>
      <CatalogProvider>
        <NavigationProvider context={context}>
          {(route) => (
            <Fragment key={`${route.page}:${route.user ?? ""}:${route.index}`}>
              {route.page === "market" ? (
                market
              ) : route.page === "portfolio" ? (
                <PortfolioPage context={context} initialUser={route.user} />
              ) : (
                <ProfilePage context={context} />
              )}
            </Fragment>
          )}
        </NavigationProvider>
      </CatalogProvider>
    </SessionProvider>
  );
}
