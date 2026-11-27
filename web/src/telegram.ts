import {
  portfolioParams,
  portfolioFiltersFromParams,
} from "./portfolio/links.ts";
import {
  decodeTarget,
  targetFromParams,
  targetAppUrl,
  targetBotUrl,
} from "./targets.ts";
import type { components } from "./gen/api";
// Mini App schemas are absent from API-only builds; keep the dormant adapter type-safe.
export type OptionalSchema<
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
  isVersionAtLeast?(version: string): boolean;
  setHeaderColor?(color: string): void;
  setBackgroundColor?(color: string): void;
  setBottomBarColor?(color: string): void;
  openTelegramLink?(url: string): void;
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
const nativeColors = new WeakMap<WebApp, string>();
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
    else root.style.removeProperty(property);
  }
  const inset = (side: keyof Insets) =>
    Math.max(0, app.safeAreaInset?.[side] || 0) +
    Math.max(0, app.contentSafeAreaInset?.[side] || 0);
  for (const side of ["top", "bottom", "left", "right"] as const)
    root.style.setProperty(
      `--tg-safe-${side}`,
      `${Math.min(inset(side), 200)}px`,
    );
  const color = getComputedStyle(root).getPropertyValue("--paper").trim();
  if (!/^#[0-9a-f]{6}$/i.test(color) || nativeColors.get(app) === color) return;
  nativeColors.set(app, color);
  const supports = (version: string) =>
    !app.isVersionAtLeast || app.isVersionAtLeast(version);
  // Older clients expose SDK methods that their native app does not implement.
  // One unsupported color must not prevent safe-area updates or sign-in.
  if (supports("6.1")) {
    try {
      app.setHeaderColor?.(supports("6.9") ? color : "bg_color");
    } catch {}
    try {
      app.setBackgroundColor?.(color);
    } catch {}
  }
  if (supports("7.10")) {
    try {
      app.setBottomBarColor?.(color);
    } catch {}
  }
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
export function launchTarget(
  context: TelegramContext,
  direct: URLSearchParams,
) {
  return decodeTarget(
    context.app?.initDataUnsafe?.start_param ||
      direct.get("tgWebAppStartParam") ||
      new URLSearchParams(context.initData).get("start_param") ||
      "",
  );
}
export function selectionTarget(selection: URLSearchParams) {
  const p =
    selection.get("page") === "portfolio"
      ? portfolioParams(
          portfolioFiltersFromParams(selection),
          selection.get("currency") || "GRAM",
        )
      : cleanSelection(selection);
  if (selection.get("page") === "portfolio")
    p.set("user", selection.get("user") || "");
  p.set("t", selection.get("page") === "portfolio" ? "portfolio" : "market");
  p.delete("page");
  return targetFromParams(p);
}
export function telegramLink(
  config: TelegramConfig | undefined,
  selection: URLSearchParams,
): string | null {
  if (!config?.bot_username) return null;
  try {
    return targetAppUrl(
      selectionTarget(selection),
      config.bot_username,
      config.public_url ?? undefined,
    );
  } catch {
    return null;
  }
}
/** Oversized targets open a bot draft or complete website link; never discard selection state. */
export function telegramReopenLink(
  config: TelegramConfig | undefined,
  selection: URLSearchParams,
): string | null {
  if (!config?.bot_username) return null;
  const link = telegramLink(config, selection);
  if (link && new URL(link).hostname === "t.me") return link;
  try {
    return targetBotUrl(
      selectionTarget(selection),
      config.bot_username,
      config.public_url ?? undefined,
    );
  } catch {
    return null;
  }
}
export function telegramBotLink(
  config: TelegramConfig | undefined,
  selection: URLSearchParams,
): string | null {
  if (!config?.bot_username) return null;
  try {
    return targetBotUrl(
      selectionTarget(selection),
      config.bot_username,
      config.public_url ?? undefined,
    );
  } catch {
    return null;
  }
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
