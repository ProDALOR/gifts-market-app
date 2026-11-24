import { useCallback, useLayoutEffect, useMemo, useRef, useState } from "react";
import { usePageSlot } from "../app/navigation";
import { liveSocket } from "../live-socket";
import { PortfolioPager, emptyRows } from "./pager";
import type { PortfolioRows } from "./pager";
import {
  portfolioQuery,
  type PortfolioFilters,
  type PortfolioState,
} from "./model";

export function usePortfolioRows(
  user: number | undefined,
  currency: string,
  filters: PortfolioFilters,
  summary: PortfolioState["valuation"],
  refreshSummary: () => void,
  ready: boolean,
  hold = false,
) {
  const { query, key } = useMemo(() => {
    const query = portfolioQuery(user, currency, filters);
    return { query, key: query ? JSON.stringify(query) : "" };
  }, [user, currency, filters]);
  const cache = usePageSlot<Map<string, PortfolioRows>>(
    "portfolio.pages",
    () => new Map(),
  );
  const pager = useRef<PortfolioPager | undefined>(undefined);
  const [view, setView] = useState<{ key: string; rows: PortfolioRows }>(
    () => ({ key, rows: cache.current.get(key) ?? emptyRows() }),
  );
  const refresh = useRef(refreshSummary);
  refresh.current = refreshSummary;
  useLayoutEffect(() => {
    const initial = cache.current.get(key);
    if (!query) {
      setView({ key, rows: emptyRows() });
      return;
    }
    const controller = new PortfolioPager({
      query,
      initial,
      request: (params, signal) =>
        liveSocket.request({ method: "portfolio.query", params }, signal),
      refreshSummary: () => refresh.current(),
      changed: (rows) => {
        setView({ key, rows });
        if (!rows.loading && !rows.error && rows.version && rows.selection) {
          cache.current.delete(key);
          cache.current.set(key, rows);
          while (cache.current.size > 2)
            cache.current.delete(cache.current.keys().next().value!);
        }
      },
    });
    pager.current = controller;
    setView({ key, rows: controller.current() });
    const visibility = () => {
      if (document.hidden) controller.pause();
    };
    document.addEventListener("visibilitychange", visibility);
    return () => {
      document.removeEventListener("visibilitychange", visibility);
      controller.dispose();
      pager.current = undefined;
    };
  }, [key]);
  useLayoutEffect(() => {
    // Keep the inspected card stable until details close; its valuation has its own lifecycle.
    if (hold || !ready || !summary?.version) {
      pager.current?.pause();
      return;
    }
    if (user && summary?.version && !document.hidden)
      pager.current?.update(summary.version);
  }, [key, summary, user, hold, ready]);
  const loadMore = useCallback(() => pager.current?.loadMore(), []);
  const retry = useCallback(() => pager.current?.retry(), []);
  return {
    ...(view.key === key ? view.rows : emptyRows()),
    queryKey: key,
    loadMore,
    retry,
  };
}
