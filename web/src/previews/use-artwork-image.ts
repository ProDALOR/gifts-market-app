import { useCallback, useEffect, useState } from "react";
import { hasImageFailure, recoverImage } from "./media-client";

type Status = "loading" | "ready" | "failed";

/** The enclosing artwork is keyed by media identity; each URL owns its retries. */
function useImageSource(url: string | undefined, active: boolean) {
  const [state, setState] = useState<{ status: Status; attempt: number }>(
    () => ({
      status: url && hasImageFailure(url) ? "failed" : "loading",
      attempt: 0,
    }),
  );
  const loaded = useCallback(
    () => setState((old) => ({ ...old, status: "ready" })),
    [],
  );
  const failed = useCallback(
    () => setState((old) => ({ ...old, status: "failed" })),
    [],
  );
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
) {
  const primary = useImageSource(preferred, active);
  const showFallback =
    primary.status === "failed" ||
    (primary.attempt > 0 && primary.status !== "ready");
  const secondary = useImageSource(
    fallback !== preferred ? fallback : undefined,
    active && showFallback,
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
