import type { TelegramContext } from "../telegram";
import { useMemo } from "react";
import { selectionTarget } from "../telegram";
import { targetWebsiteUrl } from "../targets";
import type { Target } from "../targets";
import { SharedIcon } from "../ui/icons";
import { ShareButton } from "../use-sharing";
import type { useSharing } from "../use-sharing";
import { TelegramAccount } from "../telegram-ui";
import { BrandMark } from "../brand/brand-mark";
import { CurrencyPicker } from "../ui/pickers";
import { selectionAt } from "../market-selection";
import { pathRoute, portfolioUser } from "./routes";
import { useSession } from "./session";
import { useNavigation } from "./navigation";

/** All pages share the same currency and account entry point. */
export function AppHeader({
  context,
  currencies,
  currency,
  changeCurrency,
  share,
}: {
  context: TelegramContext;
  currencies: readonly string[];
  currency: string;
  changeCurrency: (currency: string) => void;
  share?: {
    controller: ReturnType<typeof useSharing>;
    target: Target;
    label: string;
  };
}) {
  const telegram = useSession();
  const navigation = useNavigation();
  const returnTo = navigation?.route.returnTo;
  const returnName = returnTo === "market" ? "preview" : returnTo;
  const selection = selectionAt(location);
  const route = pathRoute(location.pathname);
  selection.set("currency", currency);
  if (route.page === "portfolio") {
    const user = route.user ?? portfolioUser(telegram.identity?.user.id);
    selection.set("page", "portfolio");
    selection.set("user", user ? String(user) : "");
  }
  const selectionKey = selection.toString();
  const website = useMemo(() => {
    try {
      const base = telegram.config?.public_url || location.origin;
      const link = targetWebsiteUrl(
        share?.target ?? selectionTarget(selection),
        base,
      );
      if (route.page !== "profile") return link;
      const url = new URL(link);
      url.pathname = `${new URL(base).pathname.replace(/\/$/, "")}/profile`;
      url.hash = "";
      url.search = new URLSearchParams({ currency }).toString();
      return url.href;
    } catch {
      // The page may still be discovering its portfolio owner.
      return undefined;
    }
  }, [
    share?.target,
    telegram.config?.public_url,
    selectionKey,
    route.page,
    currency,
  ]);
  return (
    <div className="sidebar-heading">
      {returnTo ? (
        <button
          className="header-back"
          aria-label={`Back to ${returnName}`}
          title={`Back to ${returnName}`}
          onClick={() => history.back()}
        >
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <path d="m14 6-6 6 6 6" />
          </svg>
          <span className="header-back-label">Back</span>
        </button>
      ) : (
        <BrandMark animated />
      )}
      <div className="header-actions">
        <CurrencyPicker
          currencies={currencies}
          value={currency}
          change={changeCurrency}
        />
        <TelegramAccount
          telegram={telegram}
          context={context}
          selection={selection}
          appActions={
            <>
              {website && (
                <a
                  className="header-action"
                  href={website}
                  target="_blank"
                  rel="noreferrer"
                  aria-label="Open website"
                  title="Open website"
                  onClick={(event) => {
                    if (context.app?.openLink) {
                      try {
                        context.app.openLink(website);
                        event.preventDefault();
                      } catch {
                        /* Older clients use the anchor fallback. */
                      }
                    }
                  }}
                >
                  <SharedIcon name="market_link" />
                </a>
              )}
              {share && (
                <ShareButton
                  sharing={share.controller}
                  label={share.label}
                  click={() => void share.controller.share(share.target)}
                />
              )}
            </>
          }
        />
      </div>
    </div>
  );
}
