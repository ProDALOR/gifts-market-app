import { useEffect, useId, useState } from "react";
import { loadOutline } from "./outline-resource.ts";

/** The caller keys this component by URL; no old outline survives a source change. */
export function GiftPlaceholder({
  url,
  active,
  fallback,
}: {
  url?: string | null;
  active: boolean;
  fallback: string;
}) {
  const [path, setPath] = useState<string>();
  const id = `gift-outline-${useId()}`;
  useEffect(() => {
    if (!url || !active) return;
    const stop = new AbortController();
    const timer = setTimeout(() => {
      void loadOutline(url, stop.signal).then(
        (value) => {
          if (value && !stop.signal.aborted) setPath(value);
        },
        () => {
          /* Keep the ordinary fallback on cancellation or queue pressure. */
        },
      );
    }, 150);
    return () => {
      clearTimeout(timer);
      stop.abort();
    };
  }, [url, active]);
  if (!active || !path)
    return <span className="gift-preview-fallback">{fallback}</span>;
  // Render a controlled path, not an external CSS mask URL: one fetch, no second
  // Origin-dependent image request, and no object-URL lifetime or CSP changes.
  return (
    <svg
      className="gift-model-placeholder"
      viewBox="0 0 512 512"
      aria-hidden="true"
    >
      <defs>
        <clipPath id={`${id}-clip`}>
          <path d={path} />
        </clipPath>
        <linearGradient id={`${id}-shine`}>
          <stop offset="20%" stopColor="currentColor" stopOpacity="0" />
          <stop offset="50%" stopColor="currentColor" stopOpacity="0.25" />
          <stop offset="80%" stopColor="currentColor" stopOpacity="0" />
        </linearGradient>
      </defs>
      <g clipPath={`url(#${id}-clip)`}>
        <rect width="512" height="512" fill="currentColor" fillOpacity="0.12" />
        <rect
          className="gift-placeholder-shimmer"
          width="512"
          height="512"
          fill={`url(#${id}-shine)`}
        />
      </g>
    </svg>
  );
}
