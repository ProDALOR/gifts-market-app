import { useEffect, useState } from "react";
import { playbackBudget } from "./playback-budget";
export type PlaybackPolicy =
  "static" | "interaction" | "collection" | "selected" | "detail";
export function useBackgroundPlaybackSuspension(suspended: boolean) {
  useEffect(
    () => (suspended ? playbackBudget.suspendBackground() : undefined),
    [suspended],
  );
}
/** Avoid downloading/creating players for tiles merely passing through a scrollport. */
export function useSettledVisibility(visible: boolean, delay: number) {
  const [settled, setSettled] = useState(false);
  useEffect(() => {
    if (!visible) {
      setSettled(false);
      return;
    }
    const timer = setTimeout(() => setSettled(true), delay);
    return () => clearTimeout(timer);
  }, [visible, delay]);
  return visible && settled;
}
