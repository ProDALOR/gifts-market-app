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
    const observer = new IntersectionObserver(
      ([entry]) => {
        if (entry.isIntersecting) loadMore();
      },
      { rootMargin: "300px" },
    );
    observer.observe(sentinel.current);
    return () => observer.disconnect();
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
