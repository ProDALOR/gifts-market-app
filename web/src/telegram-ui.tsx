import React, { useEffect, useRef, useState } from "react";
import { ApiFailure, request } from "./market";
import {
  applyTheme,
  miniappEnabled,
  cachedSession,
  launchFingerprint,
  saveSession,
  telegramLink,
  sessionPaused,
  pauseSession,
} from "./telegram";
import type {
  Identity,
  Session,
  TelegramConfig,
  TelegramContext,
} from "./telegram";
export function useTelegram(context: TelegramContext) {
  const [config, setConfig] = useState<TelegramConfig>();
  const [identity, setIdentity] = useState<Identity>();
  const [state, setState] = useState(""),
    [error, setError] = useState("");
  const [attempt, setAttempt] = useState(0);
  const paused = useRef(false);
  const credential = useRef<{ botId: string; launch: string } | undefined>(
    undefined,
  );
  useEffect(() => {
    const app = context.app;
    if (!app) return;
    const update = () => applyTheme(app);
    const events = [
      "themeChanged",
      "safeAreaChanged",
      "contentSafeAreaChanged",
      "viewportChanged",
    ];
    for (const event of events) app.onEvent(event, update);
    return () => {
      for (const event of events) app.offEvent(event, update);
    };
  }, [context]);
  useEffect(() => {
    if (!miniappEnabled) return;
    const abort = new AbortController();
    let timer: ReturnType<typeof setTimeout>;
    void (async () => {
      const config = await request<TelegramConfig>(
        "/telegram",
        undefined,
        abort.signal,
      );
      if (abort.signal.aborted) return;
      setConfig(config);
      if (
        !config.auth_enabled ||
        !config.bot_id ||
        !context.initData ||
        paused.current
      )
        return;
      setState("Signing in…");
      setError("");
      const launch = await launchFingerprint(context);
      credential.current = { botId: config.bot_id, launch };
      if (sessionPaused(config.bot_id, launch)) {
        setState("Sign in");
        return;
      }
      const saved = cachedSession(config.bot_id, launch);
      let authenticated: Identity | undefined;
      if (saved) {
        try {
          authenticated = await request<Identity>(
            "/auth/me",
            undefined,
            abort.signal,
            saved.token,
          );
        } catch (error) {
          if (!(error instanceof ApiFailure) || error.code !== "unauthorized")
            throw error;
          saveSession(config.bot_id, launch, null);
        }
      }
      if (!authenticated) {
        const session = await request<Session>(
          "/auth/telegram",
          { init_data: context.initData },
          abort.signal,
        );
        authenticated = session;
        if (!abort.signal.aborted)
          saveSession(config.bot_id, launch, session.token);
      }
      if (!abort.signal.aborted) {
        setIdentity(authenticated);
        setState("");
        timer = setTimeout(
          () => {
            setIdentity(undefined);
            setState("Sign in");
            saveSession(config.bot_id!, launch, null);
          },
          Math.max(
            0,
            new Date(authenticated.expires_at).getTime() - Date.now(),
          ),
        );
      }
    })().catch((error) => {
      if (!abort.signal.aborted) {
        setState("Sign in");
        setError(
          error instanceof Error
            ? error.message
            : "Telegram sign-in is unavailable",
        );
      }
    });
    return () => {
      abort.abort();
      clearTimeout(timer);
    };
  }, [context, attempt]);
  return {
    config,
    identity,
    state,
    error,
    retry() {
      paused.current = false;
      if (credential.current) pauseSession(credential.current.botId, null);
      setAttempt((value) => value + 1);
    },
    logout() {
      paused.current = true;
      if (credential.current) {
        saveSession(credential.current.botId, credential.current.launch, null);
        pauseSession(credential.current.botId, credential.current.launch);
      }
      setIdentity(undefined);
      setState("Sign in");
      setError("");
    },
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
  const link = telegramLink(telegram.config, selection);
  if (!context.initData)
    return link ? (
      <a className="telegram-link" href={link} target="_blank" rel="noreferrer">
        Telegram ↗
      </a>
    ) : null;
  if (!telegram.config?.auth_enabled) return null;
  const user = telegram.identity?.user;
  return (
    <div className="telegram-account">
      <button
        className="account-button"
        aria-expanded={open}
        aria-label={user ? `Account: ${user.first_name}` : "Telegram sign-in"}
        onClick={() => setOpen(!open)}
      >
        <span className={`account-dot ${user ? "verified" : ""}`} />{" "}
        {user?.first_name || telegram.state || "Sign in"}
      </button>
      {open && (
        <div
          className="account-menu"
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
                  setOpen(false);
                }}
              >
                Sign out
              </button>
            </>
          ) : (
            <>
              <strong>Telegram sign-in</strong>
              <p>
                {telegram.error || "Sign in to use your Telegram identity."}
              </p>
              <button
                onClick={telegram.retry}
                disabled={telegram.state === "Signing in…"}
              >
                Sign in
              </button>
            </>
          )}
        </div>
      )}
    </div>
  );
}
