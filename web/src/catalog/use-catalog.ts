import { useEffect, useMemo, useState } from "react";
import { ApiFailure, books, catalog } from "../market";
import type { Book, Entry } from "../market";
import type { Collection, CatalogOption } from "../orderbook";
import { CatalogOptionsCache } from "./options-cache";
type Attribute = Extract<Entry, { kind: "attribute" }>;
export function useCatalog(refresh: number, selectedId: string) {
  const [directory, setDirectory] = useState({
    types: [] as Collection[],
    venues: [] as Entry[],
    directory: [] as Book[],
    attributes: new Map<string, Attribute>(),
  });
  const [startupError, setStartupError] = useState("");
  const [attempt, setAttempt] = useState(0);
  const [cache] = useState(() => new CatalogOptionsCache());
  const [options, setOptions] = useState<ReadonlyMap<string, CatalogOption>>(
    cache.options,
  );
  const [load, setLoad] = useState({ id: "", status: "idle", error: "" });
  const [optionAttempt, setOptionAttempt] = useState(0);
  useEffect(() => {
    const stop = new AbortController();
    let retry: ReturnType<typeof setTimeout>;
    void Promise.all([
      catalog("item_type", stop.signal),
      catalog("venue", stop.signal),
      catalog("attribute", stop.signal),
      books(stop.signal),
    ])
      .then(([items, venues, attrs, directory]) => {
        if (stop.signal.aborted) return;
        setDirectory({
          types: items
            .filter((row): row is Collection => row.kind === "item_type")
            .sort((a, b) => a.name.localeCompare(b.name)),
          venues,
          directory,
          attributes: new Map(
            attrs
              .filter((row): row is Attribute => row.kind === "attribute")
              .map((row) => [row.id, row]),
          ),
        });
        setStartupError("");
      })
      .catch((error) => {
        if (stop.signal.aborted) return;
        setStartupError(error.message);
        if (
          error instanceof ApiFailure &&
          ["unavailable", "capacity_exceeded"].includes(error.code)
        )
          retry = setTimeout(() => setAttempt((value) => value + 1), 2000);
      });
    return () => {
      stop.abort();
      clearTimeout(retry);
    };
  }, [refresh, attempt]);
  const item = directory.types.find((row) => row.id === selectedId);
  useEffect(() => {
    if (!item) return;
    const stop = new AbortController();
    setLoad({ id: item.id, status: "loading", error: "" });
    void cache
      .load(item, stop.signal)
      .then((options) => {
        if (stop.signal.aborted) return;
        setOptions(options);
        setLoad({ id: item.id, status: "ready", error: "" });
      })
      .catch((error) => {
        if (!stop.signal.aborted)
          setLoad({
            id: item.id,
            status: "error",
            error: error.message || "Could not load collection choices",
          });
      });
    return () => stop.abort();
  }, [cache, item, optionAttempt]);
  const names = useMemo(
    () =>
      new Map(
        [
          ...directory.types,
          ...directory.venues,
          ...directory.attributes.values(),
          ...options.values(),
        ].map((row) => [row.id, row.name]),
      ),
    [directory, options],
  );
  const current =
    item && load.id === item.id
      ? load
      : { status: item ? "loading" : "idle", error: "" };
  return {
    ...directory,
    options,
    names,
    item,
    startupError,
    optionsLoading: current.status === "loading",
    optionsError: current.error,
    retryOptions: () => setOptionAttempt((value) => value + 1),
  };
}
