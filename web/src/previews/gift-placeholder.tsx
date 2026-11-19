import { useEffect, useId, useState } from "react";
import { loadOutline, outlineCached } from "./outline-resource.ts";

// Available before any network request; an outline refines it when Telegram has one.
const giftSilhouette =
  "M88 200Q88 184 104 184H408Q424 184 424 200V272H88ZM112 296H400V448Q400 464 384 464H128Q112 464 112 448ZM256 184C192 184 136 160 136 112C136 64 224 64 256 144C288 64 376 64 376 112C376 160 320 184 256 184Z";

/** The caller keys this component by URL; no old outline survives a source change. */
export function GiftPlaceholder({
  url,
  active,
  loading,
}: {
  url?: string | null;
  active: boolean;
  loading: boolean;
}) {
  const [path, setPath] = useState<string>();
  const id = `gift-outline-${useId()}`;
  useEffect(() => {
    if (!url || !active) return;
    const stop = new AbortController();
    const load = () => {
      void loadOutline(url, stop.signal).then(
        (value) => {
          if (value && !stop.signal.aborted) setPath(value);
        },
        () => {
          /* Keep the local silhouette on cancellation or queue pressure. */
        },
      );
    };
    const timer = outlineCached(url) ? undefined : setTimeout(load, 150);
    if (timer === undefined) load();
    return () => {
      clearTimeout(timer);
      stop.abort();
    };
  }, [url, active]);
  // Render a controlled path, not an external CSS mask URL: one fetch, no second
  // Origin-dependent image request, and no object-URL lifetime or CSP changes.
  return (
    <svg
      className={`gift-loading-placeholder${path && active ? " gift-model-placeholder" : ""}`}
      viewBox="0 0 512 512"
      aria-hidden="true"
    >
      <defs>
        <clipPath id={`${id}-clip`}>
          <path d={path || giftSilhouette} />
        </clipPath>
        <linearGradient id={`${id}-shine`}>
          <stop offset="20%" stopColor="currentColor" stopOpacity="0" />
          <stop offset="50%" stopColor="currentColor" stopOpacity="0.25" />
          <stop offset="80%" stopColor="currentColor" stopOpacity="0" />
        </linearGradient>
      </defs>
      <g clipPath={`url(#${id}-clip)`}>
        <rect width="512" height="512" fill="currentColor" fillOpacity="0.12" />
        {active && loading && (
          <rect
            className="gift-placeholder-shimmer"
            width="512"
            height="512"
            fill={`url(#${id}-shine)`}
          />
        )}
      </g>
    </svg>
  );
}
