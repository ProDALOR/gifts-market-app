import { useCallback, useLayoutEffect, useRef, useState } from "react";
import type { TraitKey } from "../market";
import { usePreviewEnvironment } from "../previews/preview-environment";

/** One retained panel and one resize observer; animation frames never run React updates. */
export function usePickerMotion(open: TraitKey | undefined, query: string) {
  const [retained, retain] = useState(open);
  const shown = open ?? retained;
  const shell = useRef<HTMLDivElement>(null),
    panel = useRef<HTMLElement>(null),
    content = useRef<HTMLDivElement>(null);
  const heightAnimation = useRef<Animation | undefined>(undefined),
    contentAnimation = useRef<Animation | undefined>(undefined);
  const height = useRef(0),
    latest = useRef(open);
  latest.current = open;
  const { pageVisible, reducedMotion } = usePreviewEnvironment();
  const motion = pageVisible && !reducedMotion;
  const canAnimate = useRef(motion);
  canAnimate.current = motion;
  const cancel = useCallback(() => {
    heightAnimation.current?.cancel();
    contentAnimation.current?.cancel();
    heightAnimation.current = contentAnimation.current = undefined;
  }, []);
  const resize = useCallback(() => {
    const element = shell.current,
      inner = panel.current;
    if (!element || !inner) return;
    const opening = latest.current !== undefined;
    const to = opening
      ? inner.getBoundingClientRect().height +
        parseFloat(getComputedStyle(inner).marginTop)
      : 0;
    const previous = heightAnimation.current;
    if (
      canAnimate.current &&
      height.current === to &&
      previous &&
      (previous.playState === "running" || !opening)
    )
      return;
    const from = previous
      ? element.getBoundingClientRect().height
      : height.current;
    const opacity = Number(getComputedStyle(element).opacity);
    previous?.cancel();
    heightAnimation.current = undefined;
    height.current = to;
    if (!canAnimate.current || !element.animate || Math.abs(from - to) < 0.5) {
      if (!opening) retain(undefined);
      return;
    }
    const animation = element.animate(
      [
        { height: `${from}px`, opacity: from === 0 ? 0 : opacity },
        { height: `${to}px`, opacity: opening ? 1 : 0 },
      ],
      {
        duration: opening ? 180 : 140,
        easing: "cubic-bezier(.2,.7,.2,1)",
        fill: "forwards",
      },
    );
    animation.id = "trait-panel-size";
    heightAnimation.current = animation;
    void animation.finished.then(
      () => {
        if (heightAnimation.current !== animation) return;
        if (latest.current === undefined) retain(undefined);
        else {
          animation.cancel();
          heightAnimation.current = undefined;
        }
      },
      () => {},
    );
  }, []);
  useLayoutEffect(() => {
    if (open) retain(open);
    resize();
  }, [open, motion, resize]);
  const present = shown !== undefined;
  useLayoutEffect(() => {
    if (!present) {
      cancel();
      height.current = 0;
      return;
    }
    const inner = panel.current;
    if (!inner) return;
    const observer = new ResizeObserver(resize);
    observer.observe(inner);
    return () => observer.disconnect();
  }, [present, resize, cancel]);
  useLayoutEffect(() => {
    // Reset only for navigation/search, never for selection or catalog updates.
    if (open && content.current) content.current.scrollTop = 0;
  }, [open, query]);
  useLayoutEffect(() => {
    contentAnimation.current?.cancel();
    contentAnimation.current = undefined;
    if (!open || !motion || !content.current?.animate) return;
    const animation = content.current.animate(
      [
        { opacity: 0.65, transform: "translateY(3px)" },
        { opacity: 1, transform: "translateY(0)" },
      ],
      { duration: 140, easing: "ease-out" },
    );
    animation.id = "trait-panel-content";
    contentAnimation.current = animation;
    void animation.finished.catch(() => {});
  }, [open, query, motion]);
  useLayoutEffect(() => cancel, [cancel]);
  return { shown, shell, panel, content };
}
