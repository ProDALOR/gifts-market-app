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
  ValuationResponse,
  ValuationQuery,
  ValuationStrategy,
} from "../market";
import { useCollectionFloors } from "./floors";
import { previewStrategies, strategyClient } from "./strategies";
import { presentResult, type PresentedField } from "./presentation.ts";
import { sameCriteria, sameGiftAttributes } from "./targets.ts";
import { isBestBid } from "./best-bid";
import { sameSlug } from "./slug.ts";
import type { ValuationSubject } from "../order-target";
import { criteriaPresentation, giftPresentation } from "../order-target.ts";

/** One request/response model for concrete gifts and complete bid conditions. */
export function useOrderValuation(
  input:
    ValuationSubject | { slug: string; collection: string; currency: string },
) {
  const order = "target" in input ? input : undefined;
  const lookup = "slug" in input ? input : undefined;
  const book = order?.book ?? {
    collection_id: lookup?.collection || "",
    currency_id: lookup?.currency || "",
  };
  const gift = order && giftPresentation(order);
  const criteria = order && criteriaPresentation(order);
  const shared = useCollectionFloors(book.collection_id, book.currency_id);
  const available = !!lookup || !!criteria || !!gift;
  const availability = useMarketStatus(available);
  const key = JSON.stringify([
    lookup?.slug,
    order?.kind === "bid",
    !!criteria,
    order?.target,
    book.collection_id,
    book.currency_id,
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
    const target: ValuationQuery["target"] = lookup
      ? { kind: "gift", gift: lookup.slug }
      : criteria
        ? {
            kind: "selection",
            collection_id: book.collection_id,
            criteria: {
              models: criteria.models,
              backdrops: criteria.backdrops,
              patterns: criteria.patterns,
              number: criteria.number ?? null,
            },
          }
        : {
            kind: "gift_attributes",
            collection_id: book.collection_id,
            traits: gift!.traits,
            number: gift!.number,
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
        let strategies = previewStrategies(
          rows,
          target.kind,
          order?.kind === "bid",
        );
        if (!strategies.length)
          throw new Error("No compatible valuation strategies are available.");
        const ids = strategies.map((row) => row.id);
        const generation = liveSocket.statusSnapshot().generation;
        refreshGeneration = generation;
        const report = await valuation(
          {
            target,
            currency_id: book.currency_id,
            strategies: ids,
          },
          stop.signal,
        );
        stop.signal.throwIfAborted();
        for (const result of report.results) {
          if (
            result.strategy === "best_bid" &&
            "data" in result &&
            !isBestBid(result.data)
          )
            throw new Error("Invalid best-bid valuation response");
        }
        const resolved = report.target;
        const matches =
          resolved.kind !== "catalog" &&
          (resolved.kind === "gift"
            ? resolved.gift.collection_id
            : resolved.collection_id) === book.collection_id &&
          (lookup
            ? resolved.kind === "gift" && sameSlug(resolved.slug, lookup.slug)
            : criteria
              ? resolved.kind === "selection" &&
                target.kind === "selection" &&
                sameCriteria(resolved.criteria, target.criteria)
              : resolved.kind === "gift_attributes" &&
                target.kind === "gift_attributes" &&
                sameGiftAttributes(resolved, target));
        if (report.currency_id !== book.currency_id || !matches)
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
          valuation.strategy.purpose === "estimate" &&
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
