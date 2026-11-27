import { useEffect, useMemo, useRef, useSyncExternalStore } from "react";
import type { ReactNode } from "react";
import { request } from "./market";
import { applyTheme, miniappEnabled, telegramLink } from "./telegram";
import type { TelegramContext } from "./telegram";
import { TelegramSessionController } from "./telegram-session";
import { AppLink, useNavigation } from "./app/navigation";
import { SharedIcon, VenueIcon } from "./ui/icons";
export function useTelegram(context: TelegramContext) {
  const controller = useMemo(
    () =>
      new TelegramSessionController(
        context,
        request,
        () =>
          document.visibilityState !== "hidden" && navigator.onLine !== false,
      ),
    [context],
  );
  const state = useSyncExternalStore(controller.subscribe, controller.current);
  useEffect(() => {
    const app = context.app;
    const update = () => {
      if (app) applyTheme(app);
    };
    const resume = () => controller.resume();
    const events = [
      "themeChanged",
      "safeAreaChanged",
      "contentSafeAreaChanged",
      "viewportChanged",
    ];
    update();
    for (const event of events) app?.onEvent(event, update);
    app?.onEvent("activated", resume);
    document.addEventListener("visibilitychange", resume);
    window.addEventListener("online", resume);
    if (miniappEnabled) controller.start();
    return () => {
      controller.stop();
      for (const event of events) app?.offEvent(event, update);
      app?.offEvent("activated", resume);
      document.removeEventListener("visibilitychange", resume);
      window.removeEventListener("online", resume);
    };
  }, [context, controller]);
  return {
    ...state,
    retry: controller.retry,
    logout: controller.logout,
  };
}
export function useTelegramBack(
  context: TelegramContext,
  visible: boolean,
  action: () => void,
) {
  const handler = useRef(action);
  handler.current = action;
  useEffect(() => {
    const button = context.app?.BackButton;
    if (!button) return;
    const back = () => handler.current();
    button.onClick(back);
    if (visible) button.show();
    else button.hide();
    return () => {
      button.offClick(back);
      button.hide();
    };
  }, [context, visible]);
}
/** The avatar opens the account page; sign-in and recovery live there too. */
export function TelegramAccount({
  telegram,
  context,
  selection,
  appActions,
}: {
  telegram: ReturnType<typeof useTelegram>;
  context: TelegramContext;
  selection: URLSearchParams;
  appActions?: ReactNode;
}) {
  const profileActive = useNavigation()?.route.page === "profile";
  const user = telegram.identity?.user;
  const initial = user?.first_name
    .match(/[\p{L}\p{N}]/u)?.[0]
    .toLocaleUpperCase();
  const profile = (
    <AppLink
      page="profile"
      className="account-button"
      aria-current={profileActive ? "page" : undefined}
      aria-label={user ? `Profile: ${user.first_name}` : "Profile"}
      title="Profile and settings"
    >
      {initial ? (
        <span className="account-initial" aria-hidden="true">
          {initial}
        </span>
      ) : (
        <SharedIcon name="user" />
      )}
      {context.initData && (
        <span
          aria-hidden="true"
          className={`account-dot ${user ? "verified" : telegram.status}`}
        />
      )}
    </AppLink>
  );
  if (context.initData)
    return (
      <>
        {appActions}
        {profile}
      </>
    );
  const link = telegramLink(telegram.config, selection);
  const isTelegram = link && new URL(link).hostname === "t.me";
  const label = isTelegram ? "Open in Telegram" : "Open market";
  return (
    <>
      {link && (
        <a
          className="telegram-link"
          href={link}
          target="_blank"
          rel="noreferrer"
          aria-label={label}
          title={label}
        >
          {isTelegram ? (
            <VenueIcon name="Telegram" />
          ) : (
            <SharedIcon name="market" />
          )}
        </a>
      )}
      {profile}
    </>
  );
}
