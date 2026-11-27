import { useSyncExternalStore } from "react";
import { createPreference } from "./preference-store.ts";

export const palettes = [
  { id: "rose", name: "Rose" },
  { id: "ocean", name: "Ocean" },
  { id: "mint", name: "Mint" },
  { id: "amber", name: "Amber" },
] as const;
export type Palette = (typeof palettes)[number]["id"];
export type ColorMode = "auto" | "dark" | "light";
type Appearance = { palette: Palette; mode: ColorMode };
const defaults: Appearance = { palette: "rose", mode: "auto" };
export function parseAppearance(value: string | null): Appearance {
  try {
    const parsed = JSON.parse(value ?? "null");
    return {
      palette: palettes.some((p) => p.id === parsed?.palette)
        ? parsed.palette
        : defaults.palette,
      mode: ["auto", "dark", "light"].includes(parsed?.mode)
        ? parsed.mode
        : defaults.mode,
    };
  } catch {
    return defaults;
  }
}
export const appearance = createPreference(
  "market.appearance",
  parseAppearance,
);
export function setAppearance(patch: Partial<Appearance>) {
  const previous = appearance.getSnapshot();
  const next = { ...previous, ...patch };
  if (next.mode !== previous.mode || next.palette !== previous.palette)
    appearance.set(next, JSON.stringify(next));
}
export function useAppearance() {
  return useSyncExternalStore(appearance.subscribe, appearance.getSnapshot);
}
/** Theme variables are defined once in palettes.css; gift artwork keeps its own colors. */
export function applyAppearance() {
  const root = document.documentElement;
  const current = appearance.getSnapshot();
  root.dataset.palette = current.palette;
  root.dataset.colorScheme =
    current.mode === "auto"
      ? (root.dataset.telegramTheme ??
        (matchMedia("(prefers-color-scheme: light)").matches
          ? "light"
          : "dark"))
      : current.mode;
  const color = getComputedStyle(root).getPropertyValue("--paper").trim();
  document
    .querySelector('meta[name="theme-color"]')
    ?.setAttribute("content", color);
  return color;
}
