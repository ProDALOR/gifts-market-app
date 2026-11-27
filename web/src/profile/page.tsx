import { useEffect } from "react";
import type { TelegramContext } from "../telegram";
import { telegramBotLink, telegramReopenLink } from "../telegram";
import { ShareStatus, useSharing } from "../use-sharing";
import { selectionAt, selectionHref } from "../market-selection";
import { AppHeader } from "../app/header";
import {
  AppLink,
  AppNavigation,
  useNavigation,
  useSharedCurrency,
} from "../app/navigation";
import { portfolioUser } from "../app/routes";
import { useSession } from "../app/session";
import { SharedIcon, VenueIcon } from "../ui/icons";
import { DisplaySettings } from "./display-settings";
import { AppearanceSettings } from "./appearance-settings";
import "./profile.css";

/** Account UI uses the existing verified session; opening it starts no market or inventory reads. */
export function ProfilePage({ context }: { context: TelegramContext }) {
  const session = useSession();
  const navigation = useNavigation();
  const [currency, setCurrency] = useSharedCurrency();
  const user = session.identity?.user;
  const owner = portfolioUser(user?.id);
  const initial = user?.first_name
    .match(/[\p{L}\p{N}]/u)?.[0]
    .toLocaleUpperCase();
  const selection = selectionAt(
    new URL(navigation?.href("market") || "/", location.origin),
  );
  selection.set("currency", currency);
  const sharing = useSharing(context, `profile:${owner ?? ""}:${selection}`);
  const botLink = telegramBotLink(session.config, selection);
  const launchLink =
    session.status === "reopen" || !context.initData
      ? telegramReopenLink(session.config, selection)
      : null;
  const pending =
    !!context.initData &&
    (session.status === "idle" ||
      session.status === "connecting" ||
      session.status === "retrying");
  useEffect(() => {
    history.replaceState(
      history.state,
      "",
      selectionHref("/profile", new URLSearchParams({ currency })),
    );
    navigation?.rememberLocation();
  }, [currency, navigation?.rememberLocation]);
  const openTelegram = (
    event: React.MouseEvent<HTMLAnchorElement>,
    url: string,
  ) => {
    if (context.app?.openTelegramLink && new URL(url).hostname === "t.me") {
      event.preventDefault();
      context.app.openTelegramLink(url);
    }
  };
  return (
    <div className="workspace">
      <main className="orderbook market-shell profile-shell">
        <aside className="market-sidebar" aria-label="Profile navigation">
          <AppHeader
            context={context}
            currencies={["GRAM", "XTR"]}
            currency={currency}
            changeCurrency={setCurrency}
            share={
              owner
                ? {
                    controller: sharing,
                    label: "Share my portfolio",
                    target: {
                      type: "portfolio",
                      target: { user_id: owner, currency_id: currency },
                    },
                  }
                : undefined
            }
          />
          <AppNavigation page="profile" />
        </aside>
        <section
          className="market-results profile-content"
          aria-labelledby="profile-title"
        >
          <h1 id="profile-title">Profile</h1>
          <ShareStatus sharing={sharing} />
          {user ? (
            <>
              <section className="profile-card" aria-label="Telegram account">
                <div className="profile-identity">
                  <span className="profile-avatar" aria-hidden="true">
                    {initial || <SharedIcon name="user" />}
                  </span>
                  <div>
                    <h2>
                      {user.first_name} {user.last_name}
                    </h2>
                    {user.username && <p>@{user.username}</p>}
                  </div>
                </div>
                <dl>
                  <dt>Telegram ID</dt>
                  <dd>{user.id}</dd>
                </dl>
              </section>
              <div className="profile-actions">
                {owner && (
                  <AppLink page="portfolio" user={owner}>
                    <SharedIcon name="portfolio" />
                    <span>My portfolio</span>
                    <span aria-hidden="true">›</span>
                  </AppLink>
                )}
                {botLink && (
                  <a
                    href={botLink}
                    target="_blank"
                    rel="noreferrer"
                    onClick={(event) => openTelegram(event, botLink)}
                  >
                    <VenueIcon name="Telegram" />
                    <span>Open in bot</span>
                    <span aria-hidden="true">↗</span>
                  </a>
                )}
              </div>
              <button className="profile-sign-out" onClick={session.logout}>
                Sign out
              </button>
            </>
          ) : (
            <section
              className="profile-card profile-sign-in"
              aria-label="Telegram sign-in"
            >
              <span className="profile-avatar" aria-hidden="true">
                <SharedIcon name="user" />
              </span>
              <p role="status">
                {pending
                  ? "Signing in…"
                  : session.error ||
                    (context.initData
                      ? "Sign in with your Telegram account."
                      : "Open the app in Telegram to view your profile.")}
              </p>
              {launchLink ? (
                <a
                  className="primary"
                  href={launchLink}
                  target="_blank"
                  rel="noreferrer"
                  onClick={(event) => openTelegram(event, launchLink)}
                >
                  {session.status === "reopen"
                    ? "Reopen in Telegram"
                    : "Open in Telegram"}
                </a>
              ) : context.initData && session.status !== "reopen" ? (
                <button
                  className="primary"
                  disabled={pending}
                  onClick={session.retry}
                >
                  {session.status === "unavailable"
                    ? "Retry sign-in"
                    : "Sign in"}
                </button>
              ) : null}
            </section>
          )}
          <AppearanceSettings />
          <DisplaySettings />
          <p className="profile-settings-note">Saved on this device.</p>
        </section>
      </main>
    </div>
  );
}
