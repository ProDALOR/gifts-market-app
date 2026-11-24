import { Fragment } from "react";
import type { ReactNode } from "react";
import type { TelegramContext } from "../telegram";
import { PortfolioPage } from "../portfolio/page";
import { NavigationProvider } from "./navigation";
import { SessionProvider } from "./session";
/** Routes consume launch data once. Only the visible page owns subscriptions and media effects. */
export function AppShell({
  context,
  market,
}: {
  context: TelegramContext;
  market: ReactNode;
}) {
  return (
    <SessionProvider context={context}>
      <NavigationProvider context={context}>
        {(route) => (
          <Fragment key={`${route.page}:${route.user ?? ""}:${route.index}`}>
            {route.page === "market" ? (
              market
            ) : (
              <PortfolioPage context={context} initialUser={route.user} />
            )}
          </Fragment>
        )}
      </NavigationProvider>
    </SessionProvider>
  );
}
