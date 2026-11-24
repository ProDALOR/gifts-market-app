import { usePageSlot } from "../app/navigation";
import { useCallback, useLayoutEffect, useRef, useState } from "react";
import { liveSocket } from "../live-socket";
import type { PortfolioState } from "./model";
let sequence = 0;

/** Watch cached data; only an explicit refresh rechecks Telegram after the first visit. */
export function usePortfolio(user: number | undefined, currency: string) {
  const cache = usePageSlot<Map<string, PortfolioState>>(
    "portfolio.reports",
    () => new Map(),
  );
  const key = `${user}:${currency}`;
  const remember = (value: PortfolioState) => {
    cache.current.delete(key);
    cache.current.set(key, value);
    while (cache.current.size > 2)
      cache.current.delete(cache.current.keys().next().value!);
  };
  const [state, setState] = useState<PortfolioState | undefined>(() =>
    cache.current.get(key),
  );
  const received = useRef(state);
  const [error, setError] = useState("");
  const generation = useRef(0);
  const refreshing = useRef<AbortController | undefined>(undefined);
  const watchId = useRef<string | undefined>(undefined);
  useLayoutEffect(() => {
    const initial = cache.current.get(key);
    received.current = initial;
    setState(initial);
    const accept = (value: PortfolioState) => {
      received.current = value;
      remember(value);
      setState(value);
    };
    setError("");
    let unsubscribe: (() => void) | undefined;
    const detach = () => {
      ++generation.current;
      refreshing.current?.abort();
      refreshing.current = undefined;
      unsubscribe?.();
      unsubscribe = undefined;
      watchId.current = undefined;
    };
    const attach = () => {
      detach();
      if (!user || document.hidden) return;
      const current = generation.current;
      watchId.current = `portfolio.${++sequence}`;
      unsubscribe = liveSocket.subscribe(
        {
          type: "portfolio.watch",
          id: watchId.current,
          params: { user_id: user, currency_id: currency },
        },
        (frame) => {
          if (generation.current !== current) return;
          if (frame.type === "portfolio.snapshot") {
            if (
              frame.data.user_id === user &&
              (!frame.data.valuation ||
                frame.data.valuation.currency_id === currency)
            ) {
              accept(frame.data);
              setError("");
            }
          } else if (frame.type === "portfolio.progress") {
            const old = received.current;
            if (old && frame.refresh_id >= old.refresh_id)
              accept({
                ...old,
                refresh_id: frame.refresh_id,
                progress: frame.progress,
                refresh_after: frame.refresh_after,
                error: frame.error,
              });
          } else if (frame.type === "error") setError(frame.error.message);
        },
      );
    };
    document.addEventListener("visibilitychange", attach);
    attach();
    return () => {
      document.removeEventListener("visibilitychange", attach);
      detach();
    };
  }, [user, currency]);
  const refresh = useCallback(async () => {
    if (!user || refreshing.current || document.hidden) return;
    const current = generation.current;
    const request = new AbortController();
    refreshing.current = request;
    try {
      // The watch owns state. A late acknowledgement must never overwrite newer progress.
      await liveSocket.request(
        {
          method: "portfolio.refresh",
          params: { user_id: user, currency_id: currency },
        },
        request.signal,
      );
      if (current === generation.current) setError("");
    } catch (error) {
      if (!request.signal.aborted && current === generation.current)
        setError(error instanceof Error ? error.message : "Refresh failed");
    } finally {
      if (refreshing.current === request) refreshing.current = undefined;
    }
  }, [user, currency]);
  const currentState =
    state &&
    state.user_id === user &&
    (!state.valuation || state.valuation.currency_id === currency)
      ? state
      : undefined;
  const resubscribe = useCallback(() => {
    if (watchId.current && !document.hidden) liveSocket.retry(watchId.current);
  }, []);
  return { state: currentState, error, refresh, resubscribe };
}
