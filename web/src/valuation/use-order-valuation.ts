import { useEffect, useMemo, useState } from "react";
import { useMarketStatus } from "../use-market-status.ts";
import { estimateIsStale, giftBasisChanged } from "./freshness.ts";
import { liveSocket } from "../live-socket.ts";
import { valuation, targetValuation } from "../market";
import type { Order, ValuationView } from "../market";
import { useCollectionValues } from "./market-values";

/** One cancellable read shared by the price summary and calculation breakdown. */
export function useOrderValuation(order: Order) {
  const gift = order.target.kind === "gift" ? order.target : undefined;
  const number = gift?.number;
  const giftId = gift?.id;
  const criteria = order.target.kind === "criteria" ? order.target : undefined;
  const shared = useCollectionValues(
    order.book.collection_id,
    order.book.currency_id,
  );
  const available = !!criteria || (!!number && !!giftId);
  const availability = useMarketStatus(available);
  const key = JSON.stringify([
    number,
    giftId,
    gift?.traits,
    criteria,
    order.book.collection_id,
    order.book.currency_id,
  ]);
  const [attempt, setAttempt] = useState(0);
  const [state, setState] = useState<{
    key: string;
    report?: ValuationView;
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
    if (!criteria && (!number || !giftId)) return;
    const stop = new AbortController();
    const generation = liveSocket.statusSnapshot().generation;
    pending();
    const work: Promise<ValuationView> = criteria
      ? targetValuation(
          {
            target: {
              kind: "criteria",
              collection_id: order.book.collection_id,
              criteria: {
                models: criteria.models,
                backdrops: criteria.backdrops,
                patterns: criteria.patterns,
                number: criteria.number ?? null,
              },
            },
            currency_id: order.book.currency_id,
            strategies: ["ask_floor"],
            period: null,
          },
          stop.signal,
        ).then((value) => {
          const result = value.results.find(
            (row) => row.strategy === "ask_floor",
          );
          if (!result || "error" in result)
            throw new Error(
              result && "error" in result
                ? result.error.message
                : "Matching floor is unavailable",
            );
          return {
            strategy: result.strategy,
            currency_id: value.currency_id,
            evaluated_at: value.evaluated_at,
            stale: value.stale,
            display: result.display,
          };
        })
      : valuation(
          {
            gift: { collection_id: order.book.collection_id, number: number! },
            currency_id: order.book.currency_id,
          },
          stop.signal,
        );
    void work
      .then((report) => {
        if (stop.signal.aborted) return;
        if (
          report.currency_id !== order.book.currency_id ||
          (!criteria &&
            (report.gift?.id !== giftId ||
              report.gift?.number !== number ||
              report.gift?.collection_id !== order.book.collection_id))
        )
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
  const fields = useMemo(() => {
    const all = current?.report?.display.fields ?? [];
    return {
      primary: all.filter((field) => field.primary),
      secondary: all.filter((field) => !field.primary),
    };
  }, [current?.report]);
  return {
    key,
    report: current?.report,
    stale:
      estimateIsStale(current?.report, current?.generation, availability) ||
      (!!current?.report &&
        !!shared &&
        (criteria
          ? Date.parse(shared.evaluatedAt) >
            Date.parse(current.report.evaluated_at)
          : giftBasisChanged(current.report, shared))),
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
