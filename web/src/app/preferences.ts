import { useSyncExternalStore } from "react";
import { createPreference } from "./preference-store.ts";

export type DisplayMode = "quality" | "performance";
const display = createPreference<DisplayMode>("market.display-mode", (value) =>
  value === "quality" ? "quality" : "performance",
);
export function setDisplayMode(value: DisplayMode) {
  if (display.getSnapshot() !== value) display.set(value, value);
}
/** Per-device rendering preference; independent of authentication and subscriptions. */
export function useDisplayMode() {
  return useSyncExternalStore(display.subscribe, display.getSnapshot);
}
