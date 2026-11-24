import { useLayoutEffect, useRef, useState } from "react";
import { usePageSlot } from "../app/navigation";
import { liveSocket } from "../live-socket";
import { PortfolioPager, emptyRows } from "./pager";
import type { PortfolioRows, PortfolioReadQuery } from "./pager";
import type { PortfolioFilters, PortfolioState } from "./model";

export function usePortfolioRows(
  user: number | undefined,
  currency: string,
  filters: PortfolioFilters,
  summary: PortfolioState["valuation"],
  refreshSummary: () => void,
  hold = false,
) {
  const query = {
    user_id: user ?? 1,
    currency_id: currency,
    sort: filters.sort,
    filters: {
      collection_id: filters.collection || undefined,
      models: filters.traits.models || [],
      backdrops: filters.traits.backdrops || [],
      patterns: filters.traits.patterns || [],
      number: filters.number ? Number(filters.number) : undefined,
    },
  } satisfies Omit<PortfolioReadQuery, "offset" | "limit" | "version">;
  const key = JSON.stringify([user, query]);
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
    const controller = new PortfolioPager({
      query,
      initial,
      request: (params, signal) =>
        liveSocket.request({ method: "portfolio.query", params }, signal),
      refreshSummary: () => refresh.current(),
      changed: (rows) => {
        setView({ key, rows });
        if (!rows.loading && rows.version && rows.selection) {
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
    if (hold) {
      pager.current?.pause();
      return;
    }
    if (user && summary?.version && !document.hidden)
      pager.current?.update(summary.version);
  }, [key, summary, user, hold]);
  return {
    ...(view.key === key ? view.rows : emptyRows()),
    loadMore: () => pager.current?.loadMore(),
    retry: () => pager.current?.retry(),
  };
}
