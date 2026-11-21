import { catalogFloors } from "./catalog-floors.ts";
import { sameCatalogTarget } from "./targets.ts";
import { evaluatedAfter } from "./freshness.ts";
import { liveSocket } from "../live-socket.ts";
import { valuation, ApiFailure } from "../market.ts";
import type {
  MarketStatus,
  ValuationQuery,
  ValuationResponse,
} from "../market.ts";
import type { components } from "../gen/api.ts";
type Values = components["schemas"]["CatalogFloorValuation"];
type CatalogTarget = Extract<ValuationQuery["target"], { kind: "catalog" }>;
type Pair = NonNullable<CatalogTarget["details"]>["pair"];
export type SharedFloor = components["schemas"]["AskFloorValuation"];
export type FloorsEntry = {
  floor: SharedFloor;
  traits?: NonNullable<Values["collections"][number]["traits"]>;
  stale: boolean;
  evaluatedAt: string;
  used: number;
};
/** Trait floors share the selected collection's bounded current-ask projection. */
export function traitFloor(
  entry: FloorsEntry | undefined,
  kind: "model" | "backdrop" | "pattern",
  id: string | null | undefined,
) {
  if (!id) return undefined;
  return kind === "model"
    ? entry?.traits?.models[id]
    : kind === "backdrop"
      ? entry?.traits?.backdrops[id]
      : entry?.traits?.patterns?.[id];
}
export type FloorsState = {
  currency: string;
  selected: string;
  entries: ReadonlyMap<string, FloorsEntry>;
  error: string;
  detail: { pending: boolean; error: string };
};
const REFRESH_MS = 60_000;
const BATCH_SIZE = 8;
type Dependencies = {
  query: typeof valuation;
  watchStatus: (changed: (status: MarketStatus) => void) => () => void;
};
/** All collection summaries, selected traits only, refreshed once a minute over the shared WS. */
export class FloorsController {
  private deps: Dependencies;
  private listeners = new Set<() => void>();
  private cache = new Map<string, FloorsEntry>();
  private collections = new Set<string>();
  private queued = new Set<string>();
  private running = new Map<AbortController, string[]>();
  private detail?: { stop: AbortController };
  private detailsAt = new Map<string, number>();
  private detailsDirty = false;
  private errors = new Map<string, string>();
  private detailError = "";
  private currency = "";
  private selected = "";
  private pair: Pair = null;
  private available = false;
  private unsubscribe?: () => void;
  private timer?: ReturnType<typeof setTimeout>;
  private refreshTimer?: ReturnType<typeof setInterval>;
  private closed = false;
  private used = 0;
  private state: FloorsState = {
    currency: "",
    selected: "",
    entries: new Map(),
    error: "",
    detail: { pending: false, error: "" },
  };
  constructor(deps: Partial<Dependencies> = {}) {
    this.deps = {
      query: valuation,
      watchStatus: (changed) => liveSocket.watchStatus(changed),
      ...deps,
    };
  }
  getSnapshot = () => this.state;
  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };
  private publish() {
    this.state = {
      currency: this.currency,
      selected: this.selected,
      entries: new Map(this.cache),
      error: this.detailError || this.errors.values().next().value || "",
      detail: {
        pending:
          !!this.selected &&
          this.available &&
          (!!this.detail || this.detailsDirty),
        error: this.detailError,
      },
    };
    for (const listener of this.listeners) listener();
  }
  configure(
    currency: string,
    collections: readonly string[],
    selected: string,
    pair: Pair,
  ) {
    if (this.closed) return;
    const ids = new Set(collections);
    const previousSelection = this.selected;
    if (!ids.has(selected)) selected = "";
    const currencyChanged = currency !== this.currency;
    const selectionChanged =
      currencyChanged ||
      selected !== this.selected ||
      pair?.model_id !== this.pair?.model_id ||
      pair?.backdrop_id !== this.pair?.backdrop_id;
    if (
      !selectionChanged &&
      ids.size === this.collections.size &&
      [...ids].every((id) => this.collections.has(id))
    )
      return;
    if (currencyChanged) {
      this.abort();
      this.cache.clear();
      this.detailsAt.clear();
      this.errors.clear();
      this.queued.clear();
      this.currency = currency;
    }
    for (const id of this.collections)
      if (!ids.has(id)) {
        this.cache.delete(id);
        this.detailsAt.delete(id);
        this.errors.delete(id);
        this.queued.delete(id);
      }
    for (const id of ids)
      if (currencyChanged || !this.collections.has(id)) this.queued.add(id);
    this.collections = ids;
    this.selected = selected;
    this.pair = pair;
    if (selectionChanged) {
      if (this.detail && ids.has(previousSelection))
        this.queued.add(previousSelection);
      this.detail?.stop.abort();
      this.detail = undefined;
      this.detailError = "";
      const entry = this.cache.get(selected);
      const cachedPair = entry?.traits?.pair;
      if (
        entry?.traits &&
        (cachedPair?.model_id !== pair?.model_id ||
          cachedPair?.backdrop_id !== pair?.backdrop_id)
      ) {
        const { pair: _old, ...traits } = entry.traits;
        this.cache.set(selected, { ...entry, traits });
        this.detailsAt.delete(selected);
      }
      this.detailsDirty =
        !!selected &&
        (!entry?.traits ||
          entry.stale ||
          Date.now() - (this.detailsAt.get(selected) ?? -Infinity) >=
            REFRESH_MS);
      const current = this.cache.get(selected);
      if (current) this.cache.set(selected, { ...current, used: ++this.used });
    }
    if (!currency || !ids.size) {
      this.abort();
      this.unsubscribe?.();
      this.unsubscribe = undefined;
      clearTimeout(this.timer);
      clearInterval(this.refreshTimer);
      this.refreshTimer = undefined;
      this.available = false;
      this.publish();
      return;
    }
    if (!this.unsubscribe && currency && ids.size) {
      this.unsubscribe = this.deps.watchStatus((status) => {
        if (this.closed) return;
        const available = status === "available";
        if (available === this.available) return;
        this.available = available;
        if (available) this.refresh();
        else {
          this.abort();
          this.markStale();
          this.publish();
        }
      });
    }
    if (!this.refreshTimer && currency && ids.size)
      this.refreshTimer = setInterval(this.refresh, REFRESH_MS);
    this.schedule();
    this.publish();
  }
  private schedule() {
    clearTimeout(this.timer);
    this.timer = setTimeout(() => this.pump(), 40);
  }
  private refresh = () => {
    if (this.closed) return;
    for (const id of this.collections) this.queued.add(id);
    this.detailsDirty = !!this.selected;
    this.schedule();
  };
  private pump() {
    if (this.closed || !this.available || !this.currency) return;
    // A separate foreground request never queues behind the full catalog sweep.
    if (this.selected && this.detailsDirty && !this.detail) this.loadDetails();
    const inFlight = new Set([...this.running.values()].flat());
    while (this.running.size < 2) {
      const ids = [...this.queued]
        .filter((id) => this.collections.has(id) && !inFlight.has(id))
        .slice(0, BATCH_SIZE);
      if (!ids.length) break;
      for (const id of ids) {
        this.queued.delete(id);
        inFlight.add(id);
      }
      const stop = new AbortController();
      const currency = this.currency;
      this.running.set(stop, ids);
      void this.loadSummaries(currency, ids, stop).finally(() => {
        this.running.delete(stop);
        if (!stop.signal.aborted) this.pump();
      });
    }
  }
  private async read(
    currency: string,
    target: CatalogTarget,
    stop: AbortController,
  ) {
    const report = await this.deps.query(
      { currency_id: currency, target, strategies: ["catalog_floor"] },
      stop.signal,
    );
    stop.signal.throwIfAborted();
    if (report.currency_id !== currency)
      throw new Error("Floor results did not match the requested currency");
    if (!sameCatalogTarget(report.target, target))
      throw new Error("Floor results did not match the requested scope");
    const result = report.results.find(
      (row) => row.strategy === "catalog_floor",
    );
    if (!result || "error" in result || !catalogFloors(result.data))
      throw new ApiFailure(
        result && "error" in result ? result.error.code : "unavailable",
        result && "error" in result
          ? result.error.message
          : "Invalid catalog floor results",
      );
    const rows = result.data.collections;
    if (
      rows.length !== target.collection_ids.length ||
      rows.some((row) => {
        if (!target.collection_ids.includes(row.collection_id)) return true;
        const details = target.details;
        if (row.collection_id !== details?.collection_id) return !!row.traits;
        return (
          !row.traits ||
          row.traits.pair?.model_id !== details.pair?.model_id ||
          row.traits.pair?.backdrop_id !== details.pair?.backdrop_id
        );
      })
    )
      throw new Error("Floor results did not match the requested scope");
    return { report, rows };
  }
  private accept({
    report,
    rows,
  }: {
    report: ValuationResponse;
    rows: Values["collections"];
  }) {
    for (const row of rows) {
      if (!this.collections.has(row.collection_id)) continue;
      const previous = this.cache.get(row.collection_id);
      if (previous && evaluatedAfter(previous.evaluatedAt, row.evaluated_at)) {
        if (row.traits && row.collection_id === this.selected)
          this.detailsDirty = true;
        continue;
      }
      const traits =
        row.traits ??
        (previous?.evaluatedAt === row.evaluated_at
          ? previous.traits
          : undefined);
      this.cache.set(row.collection_id, {
        floor: row.floor,
        traits,
        stale: report.stale,
        evaluatedAt: row.evaluated_at,
        used: ++this.used,
      });
      if (row.traits) this.detailsAt.set(row.collection_id, Date.now());
      else if (!traits) {
        this.detailsAt.delete(row.collection_id);
        if (row.collection_id === this.selected && !this.detail)
          this.detailsDirty = true;
      }
      this.errors.delete(row.collection_id);
    }
    this.trim();
    this.publish();
  }
  private async loadSummaries(
    currency: string,
    ids: string[],
    stop: AbortController,
  ): Promise<void> {
    try {
      this.accept(
        await this.read(
          currency,
          { kind: "catalog", collection_ids: ids },
          stop,
        ),
      );
    } catch (error) {
      if (stop.signal.aborted || this.closed) return;
      // Large metadata sets can exceed the combined batch read budget; split only that batch.
      if (
        error instanceof ApiFailure &&
        error.code === "capacity_exceeded" &&
        ids.length > 1
      ) {
        const middle = Math.ceil(ids.length / 2);
        await this.loadSummaries(currency, ids.slice(0, middle), stop);
        if (!stop.signal.aborted)
          await this.loadSummaries(currency, ids.slice(middle), stop);
        return;
      }
      for (const id of ids) {
        if (!this.collections.has(id)) continue;
        this.errors.set(
          id,
          error instanceof Error
            ? error.message
            : "Collection floors unavailable",
        );
        const previous = this.cache.get(id);
        if (previous) this.cache.set(id, { ...previous, stale: true });
      }
      this.publish();
    }
  }
  private loadDetails() {
    const selected = this.selected;
    // The detail projection also supplies this collection's summary.
    this.queued.delete(selected);
    const target: CatalogTarget = {
      kind: "catalog",
      collection_ids: [selected],
      details: { collection_id: selected, pair: this.pair },
    };
    const work = { stop: new AbortController() };
    this.detail = work;
    this.detailsDirty = false;
    this.publish();
    void this.read(this.currency, target, work.stop)
      .then((value) => {
        if (!work.stop.signal.aborted && this.detail === work) {
          this.detailError = "";
          this.accept(value);
        }
      })
      .catch((error) => {
        if (work.stop.signal.aborted || this.closed) return;
        this.detailError =
          error instanceof Error
            ? error.message
            : "Attribute floors unavailable";
        const previous = this.cache.get(selected);
        if (previous) this.cache.set(selected, { ...previous, stale: true });
        this.publish();
      })
      .finally(() => {
        if (this.detail !== work) return;
        this.detail = undefined;
        if (!work.stop.signal.aborted) this.pump();
        this.publish();
      });
  }
  private trim() {
    const traits = [...this.cache]
      .filter(([, entry]) => entry.traits)
      .sort(
        (a, b) =>
          Number(b[0] === this.selected) - Number(a[0] === this.selected) ||
          b[1].used - a[1].used,
      );
    let bytes = 0;
    for (let i = 0; i < traits.length; i++) {
      const [id, entry] = traits[i];
      bytes += JSON.stringify(entry.traits).length * 2;
      if (i >= 8 || bytes > 8 * 1024 * 1024) {
        this.cache.set(id, { ...entry, traits: undefined });
        this.detailsAt.delete(id);
      }
    }
  }
  private markStale() {
    for (const [id, entry] of this.cache)
      this.cache.set(id, { ...entry, stale: true });
  }
  private abort() {
    for (const stop of this.running.keys()) stop.abort();
    this.running.clear();
    this.detail?.stop.abort();
    this.detail = undefined;
  }
  retry = () => this.refresh();
  close() {
    if (this.closed) return;
    this.closed = true;
    clearTimeout(this.timer);
    clearInterval(this.refreshTimer);
    this.abort();
    this.unsubscribe?.();
    this.listeners.clear();
  }
}
