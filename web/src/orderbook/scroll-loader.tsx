import { useEffect, useRef } from "react";
export function ScrollLoader({
  loading,
  hasMore,
  disabled,
  loadMore,
}: {
  loading: boolean;
  hasMore: boolean;
  disabled: boolean;
  loadMore: () => void;
}) {
  const sentinel = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (loading || !hasMore || disabled || !sentinel.current) return;
    let lastY = window.scrollY,
      lastTouchY = 0,
      frame = 0;
    const check = () => {
      frame = 0;
      const bounds = sentinel.current?.getBoundingClientRect();
      if (
        bounds &&
        bounds.top <= window.innerHeight + 300 &&
        bounds.bottom >= 0
      )
        loadMore();
    };
    const schedule = () => {
      if (!frame) frame = requestAnimationFrame(check);
    };
    const scroll = () => {
      const y = window.scrollY;
      if (y > lastY) schedule();
      lastY = y;
    };
    const wheel = (event: WheelEvent) => {
      if (event.deltaY > 0) schedule();
    };
    const touchStart = (event: TouchEvent) => {
      lastTouchY = event.touches[0]?.clientY ?? 0;
    };
    const touchMove = (event: TouchEvent) => {
      const y = event.touches[0]?.clientY ?? lastTouchY;
      if (y < lastTouchY) schedule();
      lastTouchY = y;
    };
    window.addEventListener("touchstart", touchStart, { passive: true });
    window.addEventListener("touchmove", touchMove, { passive: true });
    window.addEventListener("scroll", scroll, { passive: true });
    window.addEventListener("wheel", wheel, { passive: true });
    return () => {
      window.removeEventListener("touchstart", touchStart);
      window.removeEventListener("touchmove", touchMove);
      window.removeEventListener("scroll", scroll);
      window.removeEventListener("wheel", wheel);
      cancelAnimationFrame(frame);
    };
  }, [loading, hasMore, disabled, loadMore]);
  if (!loading && !hasMore) return null;
  return (
    <div ref={sentinel} className="scroll-loader" role="status">
      {loading
        ? "Loading more gifts…"
        : disabled
          ? "Loading paused — retry above"
          : "Scroll for more"}
    </div>
  );
}
