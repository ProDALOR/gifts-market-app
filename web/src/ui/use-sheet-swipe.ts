import { useLayoutEffect, useRef } from "react";
import type { RefObject } from "react";

/** Pull only from the top of a mobile sheet; leave inner scrolling and controls native. */
export function useSheetSwipe(
  dialog: RefObject<HTMLDialogElement | null>,
  enabled: boolean,
  motion: boolean,
  canStart: () => boolean,
  dismiss: () => void,
) {
  const latest = useRef({ motion, canStart, dismiss });
  latest.current = { motion, canStart, dismiss };
  useLayoutEffect(() => {
    const element = dialog.current;
    if (!enabled || !element) return;
    const sheet = element.querySelector<HTMLElement>(":scope > .modal-sheet");
    if (!sheet) return;
    const mobile = matchMedia("(max-width: 959px)");
    let gesture:
      | {
          id: number;
          x: number;
          y: number;
          at: number;
          distance: number;
          dragging: boolean;
        }
      | undefined;
    let settling: Animation | undefined;
    let frame: number | undefined;
    const cancelFrame = () => {
      if (frame !== undefined) cancelAnimationFrame(frame);
      frame = undefined;
    };
    const paint = () => {
      frame = undefined;
      if (gesture?.dragging)
        sheet.style.transform = `translate3d(0, ${gesture.distance}px, 0)`;
    };
    const reset = (animate = false) => {
      const distance = gesture?.distance ?? 0;
      cancelFrame();
      gesture = undefined;
      delete element.dataset.dragging;
      delete element.dataset.swipeDismiss;
      settling?.cancel();
      sheet.style.removeProperty("transform");
      if (animate && distance && latest.current.motion && sheet.animate)
        settling = sheet.animate(
          [
            { transform: `translate3d(0, ${distance}px, 0)` },
            { transform: "translate3d(0, 0, 0)" },
          ],
          { duration: 160, easing: "cubic-bezier(.2,.7,.2,1)" },
        );
    };
    const start = (event: TouchEvent) => {
      if (gesture) reset();
      if (
        !mobile.matches ||
        !latest.current.canStart() ||
        event.touches.length !== 1
      )
        return;
      const target = event.target;
      if (
        !(target instanceof Element) ||
        target.closest("dialog") !== element ||
        target.closest(
          "a,button,input,select,textarea,[contenteditable],[role=button],[role=slider]",
        )
      )
        return;
      for (let node: Element | null = target; node; node = node.parentElement) {
        if (
          node.scrollTop > 1 &&
          /(auto|scroll|overlay)/.test(getComputedStyle(node).overflowY)
        )
          return;
        if (node === element) break;
      }
      settling?.cancel();
      const touch = event.touches[0];
      gesture = {
        id: touch.identifier,
        x: touch.clientX,
        y: touch.clientY,
        at: event.timeStamp,
        distance: 0,
        dragging: false,
      };
    };
    const move = (event: TouchEvent) => {
      if (!gesture) return;
      if (
        event.touches.length !== 1 ||
        !mobile.matches ||
        !latest.current.canStart()
      )
        return reset();
      const touch = event.touches[0];
      if (touch.identifier !== gesture.id) return reset();
      const dx = Math.abs(touch.clientX - gesture.x),
        dy = touch.clientY - gesture.y;
      if (!gesture.dragging) {
        if (dy < -8 || (dx > 8 && dx >= dy)) return reset();
        if (dy < 8 || dy < dx * 1.25) return;
        if (!event.cancelable) return reset();
        gesture.dragging = true;
        element.dataset.dragging = "true";
      }
      if (!event.cancelable) return reset(true);
      event.preventDefault();
      gesture.distance = Math.max(0, Math.min(dy, innerHeight));
      if (frame === undefined) frame = requestAnimationFrame(paint);
    };
    const end = (event: TouchEvent) => {
      if (!gesture?.dragging) return reset();
      if (event.cancelable) event.preventDefault();
      const { distance, at } = gesture;
      const threshold = Math.min(120, Math.max(72, element.clientHeight * 0.2));
      const flick =
        distance >= 40 && distance / Math.max(1, event.timeStamp - at) >= 0.6;
      if (
        mobile.matches &&
        latest.current.canStart() &&
        (distance >= threshold || flick)
      ) {
        cancelFrame();
        paint();
        gesture = undefined;
        delete element.dataset.dragging;
        element.dataset.swipeDismiss = "true";
        latest.current.dismiss();
      } else reset(true);
    };
    const cancel = () => reset();
    const resize = () => {
      if (!mobile.matches) reset();
    };
    element.addEventListener("touchstart", start, { passive: true });
    element.addEventListener("touchmove", move, { passive: false });
    element.addEventListener("touchend", end, { passive: false });
    element.addEventListener("touchcancel", cancel);
    document.addEventListener("visibilitychange", cancel);
    window.addEventListener("resize", cancel);
    mobile.addEventListener("change", resize);
    return () => {
      element.removeEventListener("touchstart", start);
      element.removeEventListener("touchmove", move);
      element.removeEventListener("touchend", end);
      element.removeEventListener("touchcancel", cancel);
      document.removeEventListener("visibilitychange", cancel);
      window.removeEventListener("resize", cancel);
      mobile.removeEventListener("change", resize);
      reset();
    };
  }, [dialog, enabled]);
}
