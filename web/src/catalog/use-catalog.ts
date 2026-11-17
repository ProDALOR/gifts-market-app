import { useEffect, useMemo, useState } from "react";
import { ApiFailure, catalog } from "../market";
import type { Book, Catalog, CatalogOption } from "../market";
import type { Collection } from "../orderbook";
import { CollectionCatalogCache } from "./collection-cache";
const emptyOptions: ReadonlyMap<string, CatalogOption> = new Map();
export function useCatalog(refresh: number, selectedId: string) {
  const [directory, setDirectory] = useState({
    types: [] as Collection[],
    venues: [] as Catalog["venues"],
    directory: [] as Book[],
  });
  const [startupError, setStartupError] = useState("");
  const [attempt, setAttempt] = useState(0);
  const cache = useMemo(() => new CollectionCatalogCache(), [refresh]);
  const [loaded, setLoaded] = useState({
    cache,
    id: "",
    options: cache.options,
  });
  const options =
    loaded.cache === cache && loaded.id === selectedId
      ? loaded.options
      : emptyOptions;
  const [load, setLoad] = useState({ id: "", status: "idle", error: "" });
  const [optionAttempt, setOptionAttempt] = useState(0);
  useEffect(() => {
    const stop = new AbortController();
    let retry: ReturnType<typeof setTimeout>;
    void catalog(stop.signal)
      .then((page) => {
        if (stop.signal.aborted) return;
        setDirectory({
          types: page.collections.sort((a, b) => a.name.localeCompare(b.name)),
          venues: page.venues,
          directory: page.books,
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
        setLoaded({ cache, id: item.id, options });
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
        [...directory.types, ...directory.venues, ...options.values()].map(
          (row) => [row.id, row.name],
        ),
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
