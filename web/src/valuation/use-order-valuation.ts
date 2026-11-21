import { useEffect, useState } from "react";
import { useMarketStatus } from "../use-market-status.ts";
import {
  estimateIsStale,
  giftBasisChanged,
  evaluatedAfter,
} from "./freshness.ts";
import { liveSocket } from "../live-socket.ts";
import { ApiFailure, valuation } from "../market";
import type {
  Order,
  ValuationResponse,
  ValuationQuery,
  ValuationStrategy,
} from "../market";
import { useCollectionFloors } from "./floors";
import { compatibleStrategies, strategyClient } from "./strategies";
import { sameCriteria } from "./targets.ts";

/** One request/response model for concrete gifts and complete bid conditions. */
export function useOrderValuation(order: Order) {
  const gift = order.target.kind === "gift" ? order.target : undefined;
  const criteria = order.target.kind === "criteria" ? order.target : undefined;
  const shared = useCollectionFloors(
    order.book.collection_id,
    order.book.currency_id,
  );
  const available = !!criteria || !!(gift?.number && gift.id);
  const availability = useMarketStatus(available);
  const key = JSON.stringify([
    order.target,
    order.book.collection_id,
    order.book.currency_id,
  ]);
  const [state, setState] = useState<{
    key: string;
    report?: ValuationResponse;
    strategies?: ValuationStrategy[];
    error?: string;
    pending: boolean;
    generation?: number;
  }>({ key: "", pending: false });
  const pending = () =>
    setState((previous) => ({
      ...(previous.key === key ? previous : { key }),
      error: undefined,
      pending: true,
    }));
  useEffect(() => {
    if (!available) return;
    const stop = new AbortController();
    const target: ValuationQuery["target"] = criteria
      ? {
          kind: "criteria",
          collection_id: order.book.collection_id,
          criteria: {
            models: criteria.models,
            backdrops: criteria.backdrops,
            patterns: criteria.patterns,
            number: criteria.number ?? null,
          },
        }
      : {
          kind: "gift",
          gift: {
            collection_id: order.book.collection_id,
            number: gift!.number!,
          },
        };
    let running = false;
    let nextRefresh = 0;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const schedule = () => {
      clearTimeout(timer);
      if (!stop.signal.aborted && !document.hidden)
        timer = setTimeout(refresh, Math.max(0, nextRefresh - Date.now()));
    };
    const refresh = async () => {
      if (stop.signal.aborted || document.hidden || running) return;
      if (Date.now() < nextRefresh) {
        schedule();
        return;
      }
      running = true;
      nextRefresh = Date.now() + 60_000;
      pending();
      try {
        const rows = await strategyClient.list(stop.signal);
        stop.signal.throwIfAborted();
        const strategies = compatibleStrategies(rows, target.kind).sort(
          (a, b) =>
            Number(b.input === "resolved_gift") -
            Number(a.input === "resolved_gift"),
        );
        if (!strategies.length)
          throw new Error("No compatible valuation strategies are available.");
        const ids = strategies.map((row) => row.id);
        const generation = liveSocket.statusSnapshot().generation;
        const report = await valuation(
          {
            target,
            currency_id: order.book.currency_id,
            strategies: ids,
          },
          stop.signal,
        );
        stop.signal.throwIfAborted();
        const resolved = report.target;
        const matches =
          resolved.kind !== "catalog" &&
          (resolved.kind === "gift"
            ? resolved.gift.collection_id
            : resolved.collection_id) === order.book.collection_id &&
          (criteria
            ? resolved.kind === "criteria" &&
              target.kind === "criteria" &&
              sameCriteria(resolved.criteria, target.criteria)
            : resolved.kind === "gift" &&
              resolved.gift.id === gift?.id &&
              resolved.gift.number === gift?.number);
        if (report.currency_id !== order.book.currency_id || !matches)
          throw new Error(
            "Gift details changed. Refresh this market and try again.",
          );
        if (
          report.results.length !== ids.length ||
          new Set(report.results.map((row) => row.strategy)).size !==
            ids.length ||
          report.results.some((row) => !ids.includes(row.strategy))
        )
          throw new Error(
            "Valuation results did not match the requested strategies.",
          );
        setState({ key, report, strategies, generation, pending: false });
      } catch (error) {
        if (!stop.signal.aborted) {
          // A cached catalog can outlive a deployment. Retry should discover current contracts.
          if (error instanceof ApiFailure && error.code === "invalid_query")
            strategyClient.invalidate();
          setState((previous) => ({
            ...(previous.key === key ? previous : { key }),
            pending: false,
            error:
              error instanceof Error
                ? error.message
                : "Valuations are unavailable",
          }));
        }
      } finally {
        running = false;
        schedule();
      }
    };
    const visibility = () => {
      clearTimeout(timer);
      if (!document.hidden) void refresh();
    };
    document.addEventListener("visibilitychange", visibility);
    void refresh();
    return () => {
      stop.abort();
      clearTimeout(timer);
      document.removeEventListener("visibilitychange", visibility);
    };
  }, [key, available]);
  const current = state.key === key ? state : undefined;
  const strategies = current?.strategies ?? [];
  const report = current?.report;
  const outdated = estimateIsStale(report, current?.generation, availability);
  const valuations: OrderValuation[] = strategies.flatMap((strategy) => {
    const result = report?.results.find((row) => row.strategy === strategy.id);
    if (!result || !report) return [];
    return [
      {
        strategy,
        result,
        primaryFields: result.display.fields.filter((field) => field.primary),
        stale:
          outdated ||
          !!(
            shared &&
            (shared.stale ||
              (giftBasisChanged(report, shared, strategy.id) ??
                evaluatedAfter(shared.evaluatedAt, report.evaluated_at)))
          ),
      },
    ];
  });
  return {
    key,
    report,
    valuations,
    error: current?.error,
    available,
    loading: available && (!current || current.pending),
  };
}
export type OrderValuationState = ReturnType<typeof useOrderValuation>;

export type OrderValuation = {
  strategy: ValuationStrategy;
  result: ValuationResponse["results"][number];
  primaryFields: ValuationResponse["results"][number]["display"]["fields"];
  stale: boolean;
};
