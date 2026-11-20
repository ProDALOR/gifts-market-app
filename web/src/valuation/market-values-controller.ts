import { liveSocket } from "../live-socket.ts";
import type { LiveSocket } from "../live-socket.ts";
import type { components } from "../gen/api.ts";
type Values = components["schemas"]["MarketValues"];
type Request = components["schemas"]["MarketValuesRequest"];
export type SharedFloor = components["schemas"]["AskFloorValuation"];
export type ValuesEntry = {
  floor: SharedFloor;
  traits?: NonNullable<Values["traits"]>;
  stale: boolean;
  evaluatedAt: string;
  used: number;
};
export type ValuesState = {
  currency: string;
  selected: string;
  entries: ReadonlyMap<string, ValuesEntry>;
  error: string;
};
/** One visible-scope subscription, bounded retention and generation guards for all price consumers. */
export class MarketValuesController {
  private listeners = new Set<() => void>();
  private cache = new Map<string, ValuesEntry>();
  private currency = "";
  private selected = "";
  private visible: string[] = [];
  private pair: Request["pair"] = null;
  private unsubscribe?: () => void;
  private signature = "";
  private timer?: ReturnType<typeof setTimeout>;
  private closed = false;
  private used = 0;
  private state: ValuesState = {
    currency: "",
    selected: "",
    entries: new Map(),
    error: "",
  };
  constructor(
    private socket: Pick<
      LiveSocket,
      "subscribe" | "update" | "retry"
    > = liveSocket,
  ) {}
  getSnapshot = () => this.state;
  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };
  private publish(error = this.state.error) {
    this.state = {
      currency: this.currency,
      selected: this.selected,
      entries: new Map(
        [...this.cache]
          .filter(([key]) => key.startsWith(`${this.currency}:`))
          .map(([key, value]) => [key.slice(this.currency.length + 1), value]),
      ),
      error,
    };
    for (const listener of this.listeners) listener();
  }
  select(currency: string, collection: string, pair: Request["pair"]) {
    if (this.closed) return;
    if (
      currency === this.currency &&
      collection === this.selected &&
      pair?.model_id === this.pair?.model_id &&
      pair?.backdrop_id === this.pair?.backdrop_id
    )
      return;
    this.currency = currency;
    this.selected = collection;
    this.pair = pair;
    this.signature = "";
    this.markStale();
    this.schedule();
    this.publish("");
  }
  visibleCollections = (ids: string[]) => {
    if (this.closed) return;
    this.visible = [...new Set(ids)];
    this.schedule();
  };
  private schedule() {
    clearTimeout(this.timer);
    this.timer = setTimeout(() => this.watch(), 40);
  }
  private watch() {
    if (this.closed) return;
    if (!this.currency || !this.selected) {
      this.unsubscribe?.();
      this.unsubscribe = undefined;
      this.signature = "";
      return;
    }
    const ids = [...new Set([this.selected, ...this.visible])]
      .slice(0, 8)
      .sort();
    const params: Request = {
      currency_id: this.currency,
      collection_ids: ids,
      traits_for: this.selected,
      pair: this.pair,
    };
    const signature = JSON.stringify(params);
    if (signature === this.signature) return;
    this.signature = signature;
    this.markStale();
    this.publish("");
    const command = {
      type: "market.values.watch" as const,
      id: "market-values",
      params,
    };
    if (this.unsubscribe) {
      this.socket.update(command);
      return;
    }
    this.unsubscribe = this.socket.subscribe(
      command,
      (frame, sent) => {
        if (
          this.closed ||
          sent.type !== "market.values.watch" ||
          JSON.stringify(sent.params) !== this.signature
        )
          return;
        if (frame.type === "error") {
          this.markStale();
          this.publish(frame.error.message);
          return;
        }
        if (
          frame.type !== "market.values" ||
          frame.data.currency_id !== this.currency
        )
          return;
        const data = frame.data;
        if (
          data.collections.length !== sent.params.collection_ids.length ||
          data.collections.some(
            (row) => !sent.params.collection_ids.includes(row.collection_id),
          ) ||
          (data.traits?.collection_id ?? null) !==
            (sent.params.traits_for ?? null) ||
          JSON.stringify(
            data.traits?.pair
              ? {
                  model_id: data.traits.pair.model_id,
                  backdrop_id: data.traits.pair.backdrop_id,
                }
              : null,
          ) !== JSON.stringify(sent.params.pair ?? null)
        ) {
          this.markStale();
          this.publish("Market values did not match the requested scope");
          return;
        }
        for (const row of data.collections) {
          const key = `${data.currency_id}:${row.collection_id}`;
          this.cache.delete(key);
          this.cache.set(key, {
            floor: row.floor,
            traits:
              data.traits?.collection_id === row.collection_id
                ? data.traits
                : undefined,
            stale: data.stale,
            evaluatedAt: data.evaluated_at,
            used: ++this.used,
          });
        }
        this.trim();
        this.publish("");
      },
      (status) => {
        if (this.closed) return;
        if (status !== "Live") {
          this.markStale();
          this.publish();
        }
      },
    );
  }
  private trim() {
    while (this.cache.size > 128)
      this.cache.delete(this.cache.keys().next().value!);
    const traits = [...this.cache]
      .filter(([, entry]) => entry.traits)
      .sort((a, b) => b[1].used - a[1].used);
    let bytes = 0;
    for (let i = 0; i < traits.length; i++) {
      const [key, entry] = traits[i];
      bytes += JSON.stringify(entry.traits).length * 2;
      if (i >= 8 || bytes > 8 * 1024 * 1024)
        this.cache.set(key, { ...entry, traits: undefined });
    }
  }
  private markStale() {
    for (const [key, entry] of this.cache)
      if (!entry.stale) this.cache.set(key, { ...entry, stale: true });
  }
  retry = () => this.socket.retry("market-values");
  close() {
    if (this.closed) return;
    this.closed = true;
    clearTimeout(this.timer);
    this.unsubscribe?.();
    this.listeners.clear();
  }
}
