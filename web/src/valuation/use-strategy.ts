import { useEffect, useState } from "react";
import type { ValuationStrategy } from "../market";
import { strategyClient } from "./strategies";

export function useStrategy(id?: string) {
  const [attempt, setAttempt] = useState(0);
  const [state, setState] = useState<{
    id?: string;
    metadata?: ValuationStrategy;
    error?: string;
  }>({});
  useEffect(() => {
    if (!id) return;
    const stop = new AbortController();
    setState({ id });
    void strategyClient
      .get(id, stop.signal)
      .then((metadata) => {
        if (!stop.signal.aborted) setState({ id, metadata });
      })
      .catch(() => {
        if (!stop.signal.aborted)
          setState({ id, error: "Strategy information is unavailable." });
      });
    return () => stop.abort();
  }, [id, attempt]);
  const current = state.id === id ? state : undefined;
  return {
    metadata: current?.metadata,
    error: current?.error,
    retry: () => {
      strategyClient.invalidate();
      setAttempt((n) => n + 1);
    },
  };
}
