import { useEffect, useState, useSyncExternalStore } from "react";
const motion = matchMedia("(prefers-reduced-motion: reduce)");
let snapshot = {
  pageVisible: !document.hidden,
  reducedMotion: motion.matches,
};
const subscribers = new Set<() => void>();
function getSnapshot() {
  const pageVisible = !document.hidden;
  const reducedMotion = motion.matches;
  if (
    pageVisible === snapshot.pageVisible &&
    reducedMotion === snapshot.reducedMotion
  )
    return snapshot;
  snapshot = { pageVisible, reducedMotion };
  return snapshot;
}
function update() {
  getSnapshot();
  // A render can read the new flags before the native change event is delivered.
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
  return useSyncExternalStore(subscribe, getSnapshot);
}
type Intersections = {
  observer: IntersectionObserver;
  targets: Map<Element, Set<(visible: boolean) => void>>;
};
const intersections = new Map<number, Intersections>();
/** Visibility controls playback; a bounded margin admits static painting ahead of scrolling. */
export function usePreviewVisibility(
  ref: { current: Element | null },
  margin = 0,
  enabled = true,
) {
  const [visible, setVisible] = useState(false);
  useEffect(() => {
    const element = ref.current;
    if (!element || !enabled) return;
    let group = intersections.get(margin);
    if (!group) {
      const targets = new Map<Element, Set<(visible: boolean) => void>>();
      group = {
        targets,
        observer: new IntersectionObserver(
          (entries) => {
            for (const entry of entries)
              targets
                .get(entry.target)
                ?.forEach((update) => update(entry.isIntersecting));
          },
          { threshold: 0.01, rootMargin: `${margin}px` },
        ),
      };
      intersections.set(margin, group);
    }
    let updates = group.targets.get(element);
    if (!updates) {
      updates = new Set();
      group.targets.set(element, updates);
      group.observer.observe(element);
    }
    updates.add(setVisible);
    return () => {
      updates.delete(setVisible);
      if (!updates.size) {
        group.observer.unobserve(element);
        group.targets.delete(element);
      }
      if (!group.targets.size) {
        group.observer.disconnect();
        intersections.delete(margin);
      }
    };
  }, [ref, margin, enabled]);
  return enabled && visible;
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
