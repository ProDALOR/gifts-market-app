import { selectionFromParams, selectionParams } from "./market-selection.ts";
import type { components } from "./gen/api";
// Mini App schemas are absent from API-only builds; keep the dormant adapter type-safe.
type OptionalSchema<
  Key extends string,
  Fallback,
> = Key extends keyof components["schemas"]
  ? components["schemas"][Key]
  : Fallback;
type User = OptionalSchema<
  "User",
  {
    id: string;
    first_name: string;
    last_name?: string | null;
    username?: string | null;
  }
>;
export type TelegramConfig = OptionalSchema<
  "TelegramConfig",
  {
    auth_enabled: boolean;
    bot_id?: string | null;
    bot_username?: string | null;
    public_url?: string | null;
  }
>;
export type Identity = OptionalSchema<
  "Identity",
  { expires_at: string; user: User }
>;
export type Session = OptionalSchema<"Session", Identity & { token: string }>;
export const miniappEnabled = import.meta.env
  ? import.meta.env.VITE_MARKET_MINIAPP !== "false"
  : true;
export type Insets = {
  top: number;
  right: number;
  bottom: number;
  left: number;
};
export type WebApp = {
  initData: string;
  initDataUnsafe?: { start_param?: string };
  colorScheme: "light" | "dark";
  themeParams: Record<string, string>;
  safeAreaInset?: Insets;
  contentSafeAreaInset?: Insets;
  viewportStableHeight?: number;
  BackButton?: {
    show(): void;
    hide(): void;
    onClick(callback: () => void): void;
    offClick(callback: () => void): void;
  };
  ready(): void;
  expand(): void;
  switchInlineQuery?(query: string, chatTypes?: string[]): void;
  onEvent(event: string, callback: () => void): void;
  offEvent(event: string, callback: () => void): void;
};
declare global {
  interface Window {
    Telegram?: { WebApp?: WebApp };
  }
}
export type TelegramContext = { app?: WebApp; initData: string };
const marker = "market.telegram.context";
const sdk = "https://telegram.org/js/telegram-web-app.js";
const keys = [
  "f",
  "gift",
  "collection",
  "currency",
  "venue",
  "mode",
  "bid_match",
  "sort",
  "filters",
  "min",
  "max",
  "from",
  "until",
];
function readStorage(key: string): string | null {
  try {
    return sessionStorage.getItem(key);
  } catch {
    return null;
  }
}
export function writeStorage(key: string, value: string | null) {
  try {
    if (value === null) sessionStorage.removeItem(key);
    else sessionStorage.setItem(key, value);
  } catch {}
}
function fragmentData(): string {
  return new URLSearchParams(location.hash.slice(1)).get("tgWebAppData") || "";
}
export async function initializeTelegram(): Promise<TelegramContext> {
  if (!miniappEnabled) return { initData: "" };
  const raw = fragmentData();
  const launched = Boolean(
    raw || window.Telegram?.WebApp?.initData || readStorage(marker),
  );
  if (!launched) return { initData: "" };
  writeStorage(marker, "1");
  if (!window.Telegram?.WebApp) {
    await new Promise<void>((resolve) => {
      const script = document.createElement("script");
      let done = false;
      const finish = () => {
        if (!done) {
          done = true;
          clearTimeout(timer);
          resolve();
        }
      };
      const timer = setTimeout(finish, 8000);
      script.src = sdk;
      script.onload = finish;
      script.onerror = finish;
      document.head.append(script);
    });
  }
  const app = window.Telegram?.WebApp;
  if (app) {
    applyTheme(app);
    app.ready();
    app.expand();
  }
  return { app, initData: app?.initData || raw };
}
export function applyTheme(app: WebApp) {
  const root = document.documentElement;
  root.classList.add("telegram");
  root.dataset.telegramTheme = app.colorScheme;
  const colors = {
    bg_color: "--tg-bg",
    secondary_bg_color: "--tg-surface",
    text_color: "--tg-text",
    hint_color: "--tg-hint",
    button_color: "--tg-accent",
    button_text_color: "--tg-accent-text",
  };
  for (const [name, property] of Object.entries(colors)) {
    const color = app.themeParams[name];
    if (color && /^#[0-9a-f]{6}$/i.test(color))
      root.style.setProperty(property, color);
  }
  const inset = (side: keyof Insets) =>
    Math.max(0, app.safeAreaInset?.[side] || 0) +
    Math.max(0, app.contentSafeAreaInset?.[side] || 0);
  for (const side of ["top", "bottom", "left", "right"] as const)
    root.style.setProperty(
      `--tg-safe-${side}`,
      `${Math.min(inset(side), 200)}px`,
    );
}
export function cleanSelection(params: URLSearchParams): URLSearchParams {
  const clean = new URLSearchParams();
  for (const key of keys) {
    if (params.getAll(key).length > 1)
      throw new Error("Repeated selection parameter");
    if (params.has(key)) clean.set(key, params.get(key)!);
  }
  return clean;
}
export function encodeView(params: URLSearchParams): string | null {
  try {
    const bytes = new TextEncoder().encode(
      selectionParams(selectionFromParams(cleanSelection(params))).toString(),
    );
    const encoded =
      "v_" +
      btoa(String.fromCharCode(...bytes))
        .replaceAll("+", "-")
        .replaceAll("/", "_")
        .replace(/=+$/, "");
    return encoded.length <= 512 ? encoded : null;
  } catch {
    return null;
  }
}
export function decodeView(value: string): URLSearchParams | null {
  if (!/^v_[a-zA-Z0-9_-]+$/.test(value) || value.length > 512) return null;
  try {
    const encoded = value.slice(2).replaceAll("-", "+").replaceAll("_", "/");
    const bytes = Uint8Array.from(atob(encoded), (char) => char.charCodeAt(0));
    return cleanSelection(
      new URLSearchParams(
        new TextDecoder("utf-8", { fatal: true }).decode(bytes),
      ),
    );
  } catch {
    return null;
  }
}
export function launchSelection(
  context: TelegramContext,
  search: string,
): URLSearchParams {
  const direct = new URLSearchParams(search);
  const launch =
    context.app?.initDataUnsafe?.start_param ||
    direct.get("tgWebAppStartParam") ||
    new URLSearchParams(context.initData).get("start_param") ||
    "";
  const decoded = decodeView(launch);
  if (decoded) {
    const ownAttributes = ["f", "collection", "filters"].some((key) =>
      direct.has(key),
    );
    for (const [key, value] of decoded)
      if (
        !direct.has(key) &&
        !(ownAttributes && ["f", "collection", "filters"].includes(key))
      )
        direct.set(key, value);
  }
  return direct;
}
export function telegramLink(
  config: TelegramConfig | undefined,
  selection: URLSearchParams,
): string | null {
  if (!config?.bot_username || !config.public_url) return null;
  const payload = encodeView(selection);
  if (payload) return `https://t.me/${config.bot_username}?startapp=${payload}`;
  const url = new URL(config.public_url),
    params = selectionParams(selectionFromParams(cleanSelection(selection)));
  if (!params.has("f")) params.set("f", "");
  url.search = "";
  url.hash = params.toString();
  return url.toString();
}
export async function launchFingerprint(
  context: TelegramContext,
): Promise<string> {
  const bytes = await crypto.subtle.digest(
    "SHA-256",
    new TextEncoder().encode(context.initData),
  );
  return Array.from(new Uint8Array(bytes), (byte) =>
    byte.toString(16).padStart(2, "0"),
  ).join("");
}
export function cachedSession(
  botId: string,
  launch: string,
): { token: string } | null {
  try {
    const value = JSON.parse(
      readStorage(`market.telegram.session.${botId}`) || "null",
    );
    return value?.launch === launch && typeof value.token === "string"
      ? { token: value.token }
      : null;
  } catch {
    return null;
  }
}
export function saveSession(
  botId: string,
  launch: string,
  token: string | null,
) {
  writeStorage(
    `market.telegram.session.${botId}`,
    token ? JSON.stringify({ launch, token }) : null,
  );
}
export function sessionPaused(botId: string, launch: string): boolean {
  return readStorage(`market.telegram.logout.${botId}`) === launch;
}
export function pauseSession(botId: string, launch: string | null) {
  writeStorage(`market.telegram.logout.${botId}`, launch);
}
