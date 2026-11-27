import { useLayoutEffect, useRef, useState } from "react";
import type { TraitKey } from "../market";
import { usePreviewEnvironment } from "../previews/preview-environment";

/** Natural layout changes once; one opacity animation never resizes the market per frame. */
export function usePickerMotion(open: TraitKey | undefined, query: string) {
  const [retained, retain] = useState(open);
  const shown = open ?? retained;
  const shell = useRef<HTMLDivElement>(null),
    content = useRef<HTMLDivElement>(null);
  const animation = useRef<Animation | undefined>(undefined);
  const latest = useRef(open);
  latest.current = open;
  const { pageVisible, reducedMotion } = usePreviewEnvironment();
  const motion = pageVisible && !reducedMotion;

  useLayoutEffect(() => {
    if (open) retain(open);
    const element = shell.current;
    const previous = animation.current;
    const opacity =
      element && previous
        ? Number(getComputedStyle(element).opacity)
        : open
          ? 0
          : 1;
    previous?.cancel();
    animation.current = undefined;
    if (!element || !motion || !element.animate) {
      if (!open) retain(undefined);
      return;
    }
    const fade = element.animate([{ opacity }, { opacity: open ? 1 : 0 }], {
      duration: open ? 120 : 90,
      easing: "ease-out",
      fill: "forwards",
    });
    fade.id = "trait-panel-visibility";
    animation.current = fade;
    void fade.finished.then(
      () => {
        if (animation.current !== fade) return;
        if (!latest.current) retain(undefined);
        else {
          fade.cancel();
          animation.current = undefined;
        }
      },
      () => {},
    );
  }, [open, motion]);
  useLayoutEffect(() => {
    // Search and navigation reset scrolling without restarting a fade on every keystroke.
    if (open && content.current) content.current.scrollTop = 0;
  }, [open, query]);
  useLayoutEffect(() => {
    if (shown === undefined) {
      animation.current?.cancel();
      animation.current = undefined;
    }
  }, [shown]);
  useLayoutEffect(
    () => () => {
      animation.current?.cancel();
      animation.current = undefined;
    },
    [],
  );
  return { shown, shell, content };
}
