import { catalog, collectionCatalog } from "../market.ts";
import type {
  Catalog,
  CatalogOption,
  CatalogVersions,
  Collection,
} from "../market.ts";
import { liveSocket } from "../live-socket.ts";
import type { LiveSocket } from "../live-socket.ts";
import { CatalogResource } from "./resource.ts";
const emptyOptions: ReadonlyMap<string, CatalogOption> = new Map();
const emptyCatalog: Catalog = {
  collections: [],
  venues: [],
  currencies: [],
  books: [],
};
export type CatalogState = {
  selectedId: string;
  types: Collection[];
  venues: Catalog["venues"];
  directory: Catalog["books"];
  item?: Collection;
  options: ReadonlyMap<string, CatalogOption>;
  names: Map<string, string>;
  startupError: string;
  optionsError: string;
  optionsLoading: boolean;
};
type Options = ReadonlyMap<string, CatalogOption>;
type Entry = { resource: CatalogResource<Options>; used: number };
export type CatalogDependencies = {
  catalog: typeof catalog;
  collection: typeof collectionCatalog;
  socket: Pick<LiveSocket, "subscribe" | "update" | "retry">;
  random: () => number;
};
/** Owns catalog requests, revision reconciliation and bounded retention; independent of React. */
export class CatalogController {
  private directory: CatalogResource<Catalog>;
  private collections = new Map<string, Entry>();
  private listeners = new Set<() => void>();
  private selectedId = "";
  private sequence = 0;
  private manifest?: CatalogVersions;
  private fallback = false;
  private fallbackTimer?: ReturnType<typeof setTimeout>;
  private unsubscribe?: () => void;
  private started = false;
  private closed = false;
  private liveError = "";
  private watchSignature = "";
  private materialized = emptyCatalog;
  private types: Collection[] = [];
  private state: CatalogState = {
    selectedId: "",
    types: [],
    venues: [],
    directory: [],
    options: emptyOptions,
    names: new Map(),
    startupError: "",
    optionsError: "",
    optionsLoading: false,
  };
  private deps: CatalogDependencies;
  constructor(deps: Partial<CatalogDependencies> = {}) {
    this.deps = {
      catalog,
      collection: collectionCatalog,
      socket: liveSocket,
      random: Math.random,
      ...deps,
    };
    this.directory = new CatalogResource(
      this.deps.catalog,
      () => this.reconcile(),
      () => {},
      this.deps.random,
    );
  }
  getSnapshot = () => this.state;
  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };
  start() {
    if (this.started || this.closed) return;
    this.started = true;
    this.watchSignature = this.watching();
    this.unsubscribe = this.deps.socket.subscribe(
      {
        type: "catalog.watch",
        id: "catalog",
        collection_ids: JSON.parse(this.watchSignature),
      },
      (frame) => {
        if (frame.type === "catalog.versions") {
          this.liveError = "";
          this.fallback = false;
          this.manifest = frame.versions;
          clearTimeout(this.fallbackTimer);
          this.directory.setTarget(frame.versions.directory);
          for (const [id, entry] of this.collections)
            if (Object.hasOwn(frame.versions.collections, id))
              entry.resource.setTarget(frame.versions.collections[id]);
          this.reconcile();
        } else if (frame.type === "error") {
          this.liveError = `Catalog live updates: ${frame.error.message}`;
          this.fallback = true;
          this.reconcile();
        }
      },
      (status) => {
        if (status === "Reconnecting")
          this.liveError = "Catalog live updates are reconnecting";
        this.publish();
      },
    );
    // HTTP remains usable when a proxy or older deployment cannot establish a catalog watch.
    this.fallbackTimer = setTimeout(() => {
      this.fallback = true;
      this.reconcile();
    }, 1000);
    this.reconcile();
  }
  select(id: string) {
    if (this.selectedId === id || this.closed) return;
    this.selectedId = id;
    const entry = this.collections.get(id);
    if (entry) entry.used = ++this.sequence;
    this.updateWatch();
    this.reconcile();
  }
  retryCatalog = () => {
    if (this.liveError) this.deps.socket.retry("catalog");
    if (this.directory.error || this.directory.value === undefined)
      this.directory.reload();
  };
  retryOptions = () => {
    if (this.liveError) this.deps.socket.retry("catalog");
    this.collections.get(this.selectedId)?.resource.reload();
    if (this.directory.error) this.directory.reload();
  };
  close() {
    this.closed = true;
    clearTimeout(this.fallbackTimer);
    this.unsubscribe?.();
    this.directory.dispose();
    for (const entry of this.collections.values()) entry.resource.dispose();
    this.listeners.clear();
  }
  private watching() {
    const retained = [...this.collections]
      .filter(([, entry]) => entry.resource.value !== undefined)
      .sort((a, b) => b[1].used - a[1].used)
      .map(([id]) => id);
    return JSON.stringify(
      [...new Set([this.selectedId, ...retained].filter(Boolean))]
        .slice(0, 8)
        .sort(),
    );
  }
  private updateWatch() {
    if (!this.started) return;
    const signature = this.watching();
    if (signature === this.watchSignature) return;
    this.watchSignature = signature;
    this.deps.socket.update({
      type: "catalog.watch",
      id: "catalog",
      collection_ids: JSON.parse(signature),
    });
  }
  private entry(id: string) {
    let entry = this.collections.get(id);
    if (entry) return entry;
    const resource = new CatalogResource<Options>(
      async (signal, received) => {
        const page = await this.deps.collection(id, signal, received);
        const rows: CatalogOption[] = [
          ...page.models.map((m) => ({ ...m, kind: "model" as const })),
          ...page.backdrops.map((b) => ({ ...b, kind: "backdrop" as const })),
          ...page.patterns.map((p) => ({ ...p, kind: "pattern" as const })),
        ];
        return new Map(rows.map((row) => [row.id, row]));
      },
      () => this.reconcile(),
      () => this.admit(id),
      this.deps.random,
    );
    entry = { resource, used: ++this.sequence };
    this.collections.set(id, entry);
    return entry;
  }
  private admit(id: string) {
    const others = [...this.collections]
      .filter(([key, e]) => key !== id && e.resource.loading)
      .sort((a, b) => a[1].used - b[1].used);
    while (others.length >= 2) {
      const [key, entry] = others.shift()!;
      entry.resource.dispose();
      this.collections.delete(key);
    }
  }
  private prune() {
    for (const [id, entry] of this.collections)
      if (
        id !== this.selectedId &&
        !entry.resource.loading &&
        entry.resource.value === undefined
      ) {
        entry.resource.dispose();
        this.collections.delete(id);
      }
    const retained = [...this.collections]
      .filter(([, e]) => e.resource.value !== undefined)
      .sort((a, b) => a[1].used - b[1].used);
    for (const [id, entry] of retained)
      if (id !== this.selectedId && entry.resource.value!.size > 10000) {
        entry.resource.dispose();
        this.collections.delete(id);
      }
    const bounded = retained.filter(
      ([id, e]) => this.collections.has(id) && e.resource.value!.size <= 10000,
    );
    let size = bounded.reduce(
      (total, [, e]) => total + e.resource.value!.size,
      0,
    );
    while (bounded.length > 8 || size > 10000) {
      const index = bounded.findIndex(([id]) => id !== this.selectedId);
      if (index < 0) break;
      const [id, entry] = bounded.splice(index, 1)[0];
      size -= entry.resource.value!.size;
      entry.resource.dispose();
      this.collections.delete(id);
    }
  }
  private reconciling = false;
  private reconcile() {
    if (this.closed || this.reconciling) return;
    this.reconciling = true;
    try {
      const directory = this.directory.value;
      if (directory && directory !== this.materialized) {
        const old = new Map(this.types.map((row) => [row.id, row]));
        this.types = directory.collections
          .map((row) => {
            const previous = old.get(row.id);
            return previous && JSON.stringify(previous) === JSON.stringify(row)
              ? previous
              : row;
          })
          .sort((a, b) => a.name.localeCompare(b.name));
        this.materialized = directory;
      }
      const allowed = this.started && (!!this.manifest || this.fallback);
      this.directory.activate(allowed);
      for (const [id, entry] of this.collections)
        if (
          id !== this.selectedId ||
          !allowed ||
          !this.types.some((row) => row.id === id)
        )
          entry.resource.activate(false);
      if (directory && this.types.some((row) => row.id === this.selectedId)) {
        const entry = this.entry(this.selectedId);
        const known = this.manifest?.collections;
        if (known && Object.hasOwn(known, this.selectedId))
          entry.resource.setTarget(known[this.selectedId]);
        entry.resource.activate(
          allowed &&
            ((!!known && Object.hasOwn(known, this.selectedId)) ||
              this.fallback),
        );
      }
      this.prune();
      this.updateWatch();
      this.publish();
    } finally {
      this.reconciling = false;
    }
  }
  private publish() {
    if (this.closed) return;
    const directory = this.directory.value ?? emptyCatalog;
    const item = this.types.find((row) => row.id === this.selectedId);
    const resource = item
      ? this.collections.get(this.selectedId)?.resource
      : undefined;
    const options = resource?.value ?? emptyOptions;
    const previous = this.state;
    const names =
      previous.types === this.types &&
      previous.venues === directory.venues &&
      previous.options === options
        ? previous.names
        : new Map(
            [...this.types, ...directory.venues, ...options.values()].map(
              (row) => [row.id, row.name],
            ),
          );
    const next: CatalogState = {
      selectedId: this.selectedId,
      types: this.types,
      venues: directory.venues,
      directory: directory.books,
      item,
      options,
      names,
      startupError: this.directory.error || this.liveError,
      optionsError: resource?.error ?? "",
      optionsLoading:
        !!item &&
        (!!resource?.loading ||
          (resource?.value === undefined && !resource?.error)),
    };
    if (
      Object.keys(next).some(
        (key) =>
          next[key as keyof CatalogState] !==
          previous[key as keyof CatalogState],
      )
    ) {
      this.state = next;
      for (const listener of this.listeners) listener();
    }
  }
}
