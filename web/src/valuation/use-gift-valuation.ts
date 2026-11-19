import { useEffect, useMemo, useState } from "react";
import { useMarketStatus } from "../use-market-status.ts";
import { estimateIsStale } from "./freshness.ts";
import { liveSocket } from "../live-socket.ts";
import { valuation } from "../market";
import type { Order, ValuationView } from "../market";

/** One cancellable read shared by the price summary and calculation breakdown. */
export function useGiftValuation(order: Order) {
  const gift = order.target.kind === "gift" ? order.target.gift : undefined;
  const number = gift?.number;
  const giftId = gift?.id;
  const available = !!number && !!giftId;
  const availability = useMarketStatus(available);
  const key = JSON.stringify([
    number,
    giftId,
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
    if (!number || !giftId) return;
    const stop = new AbortController();
    const generation = liveSocket.statusSnapshot().generation;
    pending();
    void valuation(
      {
        gift: { collection_id: order.book.collection_id, number },
        currency_id: order.book.currency_id,
      },
      stop.signal,
    )
      .then((report) => {
        if (stop.signal.aborted) return;
        if (
          report.gift.id !== giftId ||
          report.gift.number !== number ||
          report.gift.collection_id !== order.book.collection_id ||
          report.currency_id !== order.book.currency_id
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
    stale: estimateIsStale(current?.report, current?.generation, availability),
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

export type GiftValuationState = ReturnType<typeof useGiftValuation>;
