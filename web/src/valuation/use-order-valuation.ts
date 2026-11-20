import { useEffect, useMemo, useState } from "react";
import { useMarketStatus } from "../use-market-status.ts";
import {
  estimateIsStale,
  giftBasisChanged,
  evaluatedAfter,
} from "./freshness.ts";
import { liveSocket } from "../live-socket.ts";
import { valuation } from "../market";
import type { Order, ValuationResponse, ValuationQuery } from "../market";
import { useCollectionFloors } from "./floors";

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
  const [attempt, setAttempt] = useState(0);
  const [state, setState] = useState<{
    key: string;
    report?: ValuationResponse;
    error?: string;
    pending: boolean;
    generation?: number;
  }>({ key: "", pending: false });
  const pending = () =>
    setState((previous) =>
      previous.key === key && previous.pending
        ? previous
        : {
            key,
            report: previous.key === key ? previous.report : undefined,
            generation: previous.generation,
            pending: true,
          },
    );
  useEffect(() => {
    if (!available) return;
    const stop = new AbortController(),
      generation = liveSocket.statusSnapshot().generation;
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
    pending();
    void valuation(
      {
        target,
        currency_id: order.book.currency_id,
        strategies: [criteria ? "ask_floor" : "gift_floor"],
        period: null,
      },
      stop.signal,
    )
      .then((report) => {
        if (stop.signal.aborted) return;
        const resolved = report.target;
        const matches =
          (resolved.kind === "gift"
            ? resolved.gift.collection_id
            : resolved.collection_id) === order.book.collection_id &&
          (criteria
            ? resolved.kind === "criteria" &&
              JSON.stringify(resolved.criteria) ===
                JSON.stringify(
                  target.kind === "criteria" ? target.criteria : undefined,
                )
            : resolved.kind === "gift" &&
              resolved.gift.id === gift?.id &&
              resolved.gift.number === gift?.number);
        if (report.currency_id !== order.book.currency_id || !matches)
          throw new Error(
            "Gift details changed. Refresh this market and try again.",
          );
        setState({ key, report, generation, pending: false });
      })
      .catch((error) => {
        if (!stop.signal.aborted)
          setState((previous) => ({
            key,
            report: previous.key === key ? previous.report : undefined,
            generation: previous.generation,
            pending: false,
            error:
              error instanceof Error
                ? error.message
                : "Valuations are unavailable",
          }));
      });
    return () => stop.abort();
  }, [key, attempt]);
  const current = state.key === key ? state : undefined;
  const result = current?.report?.results[0];
  const fields = useMemo(
    () => ({
      primary: result?.display.fields.filter((field) => field.primary) ?? [],
      secondary: result?.display.fields.filter((field) => !field.primary) ?? [],
    }),
    [result],
  );
  return {
    key,
    report: current?.report,
    result,
    stale:
      estimateIsStale(current?.report, current?.generation, availability) ||
      !!(
        current?.report &&
        shared &&
        (criteria
          ? evaluatedAfter(shared.evaluatedAt, current.report.evaluated_at)
          : giftBasisChanged(current.report, shared))
      ),
    fields,
    error: current?.error,
    available,
    loading: available && (!current || current.pending),
    retry: () => {
      pending();
      setAttempt((value) => value + 1);
    },
  };
}
export type OrderValuationState = ReturnType<typeof useOrderValuation>;
