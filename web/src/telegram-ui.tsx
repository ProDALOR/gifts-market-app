import {
  useCallback,
  useEffect,
  useId,
  useMemo,
  useRef,
  useSyncExternalStore,
} from "react";
import { request } from "./market";
import {
  applyTheme,
  miniappEnabled,
  telegramLink,
  telegramReopenLink,
} from "./telegram";
import type { TelegramContext } from "./telegram";
import { TelegramSessionController } from "./telegram-session";
import { useNavigationBack } from "./app/navigation";
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
    authorized: controller.authorized,
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
export function TelegramAccount({
  telegram,
  context,
  open,
  setOpen,
  selection,
}: {
  telegram: ReturnType<typeof useTelegram>;
  context: TelegramContext;
  open: boolean;
  setOpen: (value: boolean) => void;
  selection: URLSearchParams;
}) {
  const host = useRef<HTMLDivElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const menu = useRef<HTMLDivElement>(null);
  const id = useId();
  const close = useCallback(() => {
    setOpen(false);
    trigger.current?.focus({ preventScroll: true });
  }, [setOpen]);
  useNavigationBack(open, close, 20);
  useEffect(() => {
    if (!open) return;
    menu.current?.focus({ preventScroll: true });
    const outside = (event: PointerEvent) => {
      if (!host.current?.contains(event.target as Node)) setOpen(false);
    };
    const escape = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      event.stopPropagation();
      close();
    };
    document.addEventListener("pointerdown", outside);
    document.addEventListener("keydown", escape);
    return () => {
      document.removeEventListener("pointerdown", outside);
      document.removeEventListener("keydown", escape);
    };
  }, [open, close, setOpen]);
  const link = context.initData
    ? null
    : telegramLink(telegram.config, selection);
  const reopenLink =
    telegram.status === "reopen"
      ? telegramReopenLink(telegram.config, selection)
      : null;
  if (!context.initData)
    return link ? (
      <a className="telegram-link" href={link} target="_blank" rel="noreferrer">
        {new URL(link).hostname === "t.me" ? "Telegram ↗" : "Open market ↗"}
      </a>
    ) : null;
  const user = telegram.identity?.user;
  const pending = telegram.status === "connecting";
  const label = user ? `Account: ${user.first_name}` : "Telegram sign-in";
  const initial = user?.first_name
    .match(/[\p{L}\p{N}]/u)?.[0]
    .toLocaleUpperCase();
  return (
    <div
      className="telegram-account"
      ref={host}
      onBlur={(event) => {
        if (
          event.relatedTarget &&
          !event.currentTarget.contains(event.relatedTarget)
        )
          setOpen(false);
      }}
    >
      <button
        ref={trigger}
        className="account-button"
        aria-expanded={open}
        aria-controls={open ? id : undefined}
        aria-haspopup="dialog"
        aria-label={label}
        aria-busy={pending}
        title={user?.first_name || "Telegram account"}
        onClick={() => setOpen(!open)}
      >
        {initial ? (
          <span className="account-initial" aria-hidden="true">
            {initial}
          </span>
        ) : (
          <svg
            aria-hidden="true"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.7"
          >
            <circle cx="12" cy="8" r="4" />
            <path d="M4 21v-2a8 8 0 0 1 16 0v2" />
          </svg>
        )}
        <span
          aria-hidden="true"
          className={`account-dot ${user ? "verified" : telegram.status}`}
        />
      </button>
      {open && (
        <div
          className="account-menu"
          id={id}
          ref={menu}
          tabIndex={-1}
          role="dialog"
          aria-label="Telegram account"
        >
          {user ? (
            <>
              <strong>
                {user.first_name} {user.last_name}
              </strong>
              {user.username && <span>@{user.username}</span>}
              <small>ID {user.id}</small>
              <button
                onClick={() => {
                  telegram.logout();
                  close();
                }}
              >
                Sign out
              </button>
            </>
          ) : (
            <>
              <strong>
                {telegram.status === "reopen"
                  ? "Reopen in Telegram"
                  : "Telegram sign-in"}
              </strong>
              <p role="status">
                {pending
                  ? "Signing in…"
                  : telegram.error || "Sign in to use your Telegram identity."}
              </p>
              {telegram.status === "reopen" ? (
                reopenLink && (
                  <a
                    className="account-action"
                    href={reopenLink}
                    target="_blank"
                    rel="noreferrer"
                    onClick={(event) => {
                      if (context.app?.openTelegramLink) {
                        event.preventDefault();
                        context.app.openTelegramLink(reopenLink);
                      }
                    }}
                  >
                    Reopen in Telegram
                  </a>
                )
              ) : (
                <button onClick={telegram.retry} disabled={pending}>
                  {telegram.status === "retrying" ||
                  telegram.status === "unavailable"
                    ? "Retry sign-in"
                    : "Sign in"}
                </button>
              )}
            </>
          )}
        </div>
      )}
    </div>
  );
}
