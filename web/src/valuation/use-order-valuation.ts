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
import { presentResult, type PresentedField } from "./presentation.ts";
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
    valuations?: Omit<OrderValuation, "stale">[];
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
          kind: "selection",
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
    let refreshGeneration = liveSocket.statusSnapshot().generation;
    let nextRefresh = 0;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const schedule = () => {
      clearTimeout(timer);
      if (
        !stop.signal.aborted &&
        !document.hidden &&
        liveSocket.statusSnapshot().status !== "unavailable"
      )
        timer = setTimeout(refresh, Math.max(0, nextRefresh - Date.now()));
    };
    const refresh = async () => {
      if (stop.signal.aborted || document.hidden || running) return;
      if (Date.now() < nextRefresh) {
        schedule();
        return;
      }
      running = true;
      refreshGeneration = liveSocket.statusSnapshot().generation;
      nextRefresh = Date.now() + 60_000;
      pending();
      try {
        const rows = await strategyClient.list(stop.signal);
        stop.signal.throwIfAborted();
        let strategies = compatibleStrategies(rows, target.kind).sort(
          (a, b) =>
            Number(
              b.input.targets.length === 1 && b.input.targets[0] === "gift",
            ) -
            Number(
              a.input.targets.length === 1 && a.input.targets[0] === "gift",
            ),
        );
        if (!strategies.length)
          throw new Error("No compatible valuation strategies are available.");
        const ids = strategies.map((row) => row.id);
        const generation = liveSocket.statusSnapshot().generation;
        refreshGeneration = generation;
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
            ? resolved.kind === "selection" &&
              target.kind === "selection" &&
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
        if (
          report.results.some(
            (result) =>
              strategies.find((row) => row.id === result.strategy)?.revision !==
              result.revision,
          )
        ) {
          // Reuse the completed calculation; only reload stale presentation metadata once.
          strategyClient.invalidate();
          const fresh = await strategyClient.list(stop.signal);
          stop.signal.throwIfAborted();
          strategies = strategies.map(
            (row) => fresh.find((current) => current.id === row.id) ?? row,
          );
        }
        const valuations = strategies.map((strategy) => {
          const result = report.results.find(
            (row) => row.strategy === strategy.id,
          )!;
          const presentation = presentResult(
            strategy,
            result,
            report.currency_id,
          );
          return {
            strategy,
            result,
            ...presentation,
            primaryFields: presentation.fields.filter((field) => field.primary),
          };
        });
        setState({ key, report, valuations, generation, pending: false });
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
        const connection = liveSocket.statusSnapshot();
        if (
          connection.status === "available" &&
          connection.generation !== refreshGeneration
        )
          nextRefresh = 0;
        schedule();
      }
    };
    const visibility = () => {
      clearTimeout(timer);
      if (!document.hidden) void refresh();
    };
    document.addEventListener("visibilitychange", visibility);
    const unsubscribe = liveSocket.watchStatus((status) => {
      if (stop.signal.aborted) return;
      if (status === "unavailable") {
        clearTimeout(timer);
        return;
      }
      if (
        status === "available" &&
        !running &&
        liveSocket.statusSnapshot().generation !== refreshGeneration
      )
        nextRefresh = 0;
      void refresh();
    });
    void refresh();
    return () => {
      stop.abort();
      unsubscribe();
      clearTimeout(timer);
      document.removeEventListener("visibilitychange", visibility);
    };
  }, [key, available]);
  const current = state.key === key ? state : undefined;
  const report = current?.report;
  const outdated = estimateIsStale(report, current?.generation, availability);
  const valuations: OrderValuation[] = (current?.valuations ?? []).map(
    (valuation) => ({
      ...valuation,
      stale:
        outdated ||
        !!(
          shared &&
          (shared.stale ||
            (giftBasisChanged(report!, shared, valuation.strategy.id) ??
              evaluatedAfter(shared.evaluatedAt, report!.evaluated_at)))
        ),
    }),
  );
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
  fields: PresentedField[];
  evidence: string[];
  primaryFields: PresentedField[];
  stale: boolean;
};
