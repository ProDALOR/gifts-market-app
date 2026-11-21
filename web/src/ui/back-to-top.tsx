import { useEffect, useState } from "react";
import type { RefObject } from "react";
import { usePreviewEnvironment } from "../previews/preview-environment";

/** Observe the page’s top section without an additional scroll handler. */
export function BackToTop({
  target,
  blocked = false,
}: {
  target: RefObject<HTMLElement | null>;
  blocked?: boolean;
}) {
  const [pastTarget, setPastTarget] = useState(false);
  const { reducedMotion } = usePreviewEnvironment();
  useEffect(() => {
    const anchor = target.current;
    if (!anchor) return;
    const desktop = matchMedia("(min-width: 960px)");
    let observer: IntersectionObserver | undefined;
    const observe = () => {
      observer?.disconnect();
      observer = undefined;
      setPastTarget(false);
      if (desktop.matches) return;
      const current = new IntersectionObserver(([entry]) => {
        if (observer !== current) return;
        setPastTarget(
          !entry.isIntersecting &&
            entry.boundingClientRect.height > 0 &&
            entry.boundingClientRect.bottom <= 0,
        );
      });
      observer = current;
      observer.observe(anchor);
    };
    observe();
    desktop.addEventListener("change", observe);
    return () => {
      observer?.disconnect();
      observer = undefined;
      desktop.removeEventListener("change", observe);
    };
  }, [target]);
  if (!pastTarget || blocked) return null;
  return (
    <button
      type="button"
      className="back-to-top"
      aria-label="Back to top"
      title="Back to top"
      aria-controls={target.current?.id || undefined}
      onClick={() => {
        window.scrollTo({
          behavior: reducedMotion ? "auto" : "smooth",
          top: 0,
        });
        target.current?.focus({ preventScroll: true });
      }}
    >
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <path d="m6 9 6-6 6 6M12 3v18" />
      </svg>
    </button>
  );
}
