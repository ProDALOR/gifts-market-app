import { useCallback, useEffect, useState } from "react";
import { stillImages } from "./still-image-cache";
import { hasImageFailure, recoverImage } from "./media-client";

type Status = "loading" | "ready" | "failed";

/** The enclosing artwork is keyed by media identity; each URL owns its retries. */
function useImageSource(
  url: string | undefined,
  active: boolean,
  retainDecoded: boolean,
) {
  const [state, setState] = useState<{ status: Status; attempt: number }>(
    () => ({
      status:
        retainDecoded && url && stillImages.has(url)
          ? "ready"
          : url && hasImageFailure(url)
            ? "failed"
            : "loading",
      attempt: 0,
    }),
  );
  const loaded = useCallback(
    (image?: HTMLImageElement) => {
      // The mounted overscan already retains prefetched images. Pin actually viewed
      // artwork so speculative loads cannot evict the user's recent scroll history.
      if (retainDecoded && url && image && active)
        stillImages.remember(url, image);
      setState((old) =>
        old.status === "ready" ? old : { ...old, status: "ready" },
      );
    },
    [url, active, retainDecoded],
  );
  const failed = useCallback(() => {
    if (retainDecoded && url) stillImages.forget(url);
    setState((old) =>
      old.status === "failed" ? old : { ...old, status: "failed" },
    );
  }, [url, retainDecoded]);
  useEffect(() => {
    if (!url || !active || state.status !== "failed") return;
    const stop = new AbortController();
    void recoverImage(url, stop.signal).then(
      () => {
        if (!stop.signal.aborted)
          setState((old) => ({ status: "loading", attempt: old.attempt + 1 }));
      },
      () => {
        /* Shared failure deadlines govern the next visible use. */
      },
    );
    return () => stop.abort();
  }, [url, active, state.status]);
  return { url, ...state, loaded, failed };
}

export function useArtworkImage(
  preferred: string | undefined,
  fallback: string | undefined,
  active: boolean,
  { retainDecoded = true }: { retainDecoded?: boolean } = {},
) {
  const primary = useImageSource(preferred, active, retainDecoded);
  const showFallback =
    primary.status === "failed" ||
    (primary.attempt > 0 && primary.status !== "ready");
  const secondary = useImageSource(
    fallback !== preferred ? fallback : undefined,
    active && showFallback,
    retainDecoded,
  );
  // Retain the decoded fallback until the recovered preferred image has loaded.
  const sources = [...(showFallback ? [secondary] : []), primary].filter(
    (source) => source.url && source.status !== "failed",
  );
  return {
    sources,
    ready:
      primary.status === "ready" ||
      (showFallback && secondary.status === "ready"),
    loading: sources.some((source) => source.status === "loading"),
  };
}
