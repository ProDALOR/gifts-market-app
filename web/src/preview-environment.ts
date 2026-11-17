import { useEffect, useState, useSyncExternalStore } from "react";
let snapshot = {
  pageVisible: !document.hidden,
  reducedMotion: matchMedia("(prefers-reduced-motion: reduce)").matches,
};
const subscribers = new Set<() => void>();
const motion = matchMedia("(prefers-reduced-motion: reduce)");
function update() {
  const next = { pageVisible: !document.hidden, reducedMotion: motion.matches };
  if (
    next.pageVisible === snapshot.pageVisible &&
    next.reducedMotion === snapshot.reducedMotion
  )
    return;
  snapshot = next;
  subscribers.forEach((subscriber) => subscriber());
}
function subscribe(subscriber: () => void) {
  if (!subscribers.size) {
    motion.addEventListener("change", update);
    document.addEventListener("visibilitychange", update);
    update();
  }
  subscribers.add(subscriber);
  return () => {
    subscribers.delete(subscriber);
    if (!subscribers.size) {
      motion.removeEventListener("change", update);
      document.removeEventListener("visibilitychange", update);
    }
  };
}
export function usePreviewEnvironment() {
  return useSyncExternalStore(subscribe, () => snapshot);
}
const intersections = new Map<Element, (visible: boolean) => void>();
let observer: IntersectionObserver | undefined;
export function usePreviewVisibility(ref: { current: Element | null }) {
  const [visible, setVisible] = useState(false);
  useEffect(() => {
    const element = ref.current;
    if (!element) return;
    observer ??= new IntersectionObserver(
      (entries) => {
        for (const entry of entries)
          intersections.get(entry.target)?.(entry.isIntersecting);
      },
      { threshold: 0.01 },
    );
    intersections.set(element, setVisible);
    observer.observe(element);
    return () => {
      observer?.unobserve(element);
      intersections.delete(element);
      if (!intersections.size) {
        observer?.disconnect();
        observer = undefined;
      }
    };
  }, [ref]);
  return visible;
}

const sizes = new Map<Element, () => void>();
let resizeObserver: ResizeObserver | undefined;
export function observePreviewSize(element: Element, draw: () => void) {
  resizeObserver ??= new ResizeObserver((entries) => {
    for (const entry of entries) sizes.get(entry.target)?.();
  });
  sizes.set(element, draw);
  resizeObserver.observe(element);
  return () => {
    resizeObserver?.unobserve(element);
    sizes.delete(element);
    if (!sizes.size) {
      resizeObserver?.disconnect();
      resizeObserver = undefined;
    }
  };
}
